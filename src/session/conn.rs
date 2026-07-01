use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::buffer::Buffer;
use crate::chunk::reader::{chunk_read, ChunkMessage};
use crate::chunk::state::ChunkRegistry;
use crate::chunk::writer::chunk_write;
use crate::handshake::{self, Handshake, HandshakeState};
use crate::message::command;
use crate::message::control::{self, UCTRL_PING_REQUEST, UCTRL_PING_RESPONSE};
use crate::message::message as msg_dispatch;
use crate::session::state_machine;
use crate::session::stream::Stream;
use crate::transport::Transport;
use crate::types::*;

pub const MAX_STREAMS_PER_CONN: u32 = 16;
pub const MAX_PENDING_RELAY_FRAMES: usize = 1024;
pub const MAX_PENDING_RELAY_BYTES: usize = 8 * 1024 * 1024;

const SERVER_WINDOW_ACK_SIZE: u32 = 2_500_000;
const SERVER_PEER_BANDWIDTH: u32 = 2_500_000;
const PEER_BANDWIDTH_DYNAMIC: u8 = 2;
const PING_INTERVAL: Duration = Duration::from_secs(5);
const PING_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PENDING_PINGS: usize = 4;

pub struct RelayFrame {
    pub frame_type: FrameType,
    pub timestamp: u32,
    pub payload: Vec<u8>,
    pub app: String,
    pub stream_name: String,
}

pub struct Conn {
    pub state: ConnState,
    pub handshake: Handshake,
    pub recv_buffer: Buffer,
    pub send_buffer: Buffer,
    pub chunk_reg: ChunkRegistry,
    pub chunk_size: u32,
    pub window_ack_size: u32,
    pub bytes_received: u32,
    pub bytes_at_last_ack: u32,
    /// Audio/video payload bytes received (excludes handshake/control overhead).
    pub media_bytes_received: u64,
    /// Audio/video payload bytes sent to this peer.
    pub media_bytes_sent: u64,
    pub client_fd: i32,
    /// Stable per-connection id (monotonic, never reused while the server runs).
    pub conn_id: u64,
    /// Peer socket address for logging (not persisted).
    pub remote_addr: String,
    pub transport: Option<Transport>,
    pub app: String,
    /// Canonical relay route key. When set, publisher/player media is matched
    /// on this value instead of the RTMP stream name (e.g. separate publish/play keys).
    pub relay_key: String,
    pub next_stream_id: u32,
    pub current_stream: Option<Box<Stream>>,
    pub connect_cb_fired: bool,
    pub send_mutex: Mutex<()>,
    pub pending_relay: Vec<RelayFrame>,
    pub needs_init_frames: bool,
    pub detected_video_codec: Option<String>,
    pub detected_audio_codec: Option<String>,
    pub relay_enabled: bool,
    /// When true, media relay stays off until the integrator sets `relay_enabled`
    /// after its own post-auth bookkeeping (used by librtmp2-server).
    pub defer_media_relay: bool,
    pub on_frame_cb: Option<fn(&Frame)>,
    /// When set, must return true before audio/video is queued for relay.
    pub on_media_cb: Option<fn(u64, FrameType, Option<&str>) -> bool>,
    pub on_connect_cb: Option<fn()>,
    pub on_publish_cb: Option<fn(conn_id: u64, app: &str, stream_name: &str) -> bool>,
    pub on_play_cb: Option<fn(conn_id: u64, app: &str, stream_name: &str) -> bool>,
    /// Last measured client↔server RTT in milliseconds (RTMP UserControl ping).
    pub rtt_ms: f64,
    pending_pings: HashMap<u32, Instant>,
    last_ping_sent: Option<Instant>,
    next_ping_token: u32,
}

impl Conn {
    pub fn new() -> Self {
        let mut chunk_reg = ChunkRegistry::new();
        chunk_reg.init();
        Self {
            state: ConnState::TcpAccepted,
            handshake: Handshake::default(),
            recv_buffer: Buffer::new(),
            send_buffer: Buffer::new(),
            chunk_reg,
            chunk_size: 128,
            window_ack_size: 0,
            bytes_received: 0,
            bytes_at_last_ack: 0,
            media_bytes_received: 0,
            media_bytes_sent: 0,
            client_fd: -1,
            conn_id: 0,
            remote_addr: String::new(),
            transport: None,
            app: String::new(),
            relay_key: String::new(),
            next_stream_id: 0,
            current_stream: None,
            connect_cb_fired: false,
            send_mutex: Mutex::new(()),
            pending_relay: Vec::new(),
            needs_init_frames: false,
            detected_video_codec: None,
            detected_audio_codec: None,
            relay_enabled: false,
            defer_media_relay: false,
            on_frame_cb: None,
            on_media_cb: None,
            on_connect_cb: None,
            on_publish_cb: None,
            on_play_cb: None,
            rtt_ms: 0.0,
            pending_pings: HashMap::new(),
            last_ping_sent: None,
            next_ping_token: 1,
        }
    }

    fn pending_relay_bytes(&self) -> usize {
        self.pending_relay.iter().map(|f| f.payload.len()).sum()
    }

    /// Key used to route relayed media between publishers and players.
    pub fn relay_route_key(&self) -> String {
        if !self.relay_key.is_empty() {
            return self.relay_key.clone();
        }
        self.current_stream
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }

    fn queue_relay_frame(&mut self, frame_type: FrameType, timestamp: u32, payload: &[u8]) -> Result<()> {
        if self.pending_relay.len() >= MAX_PENDING_RELAY_FRAMES
            || self.pending_relay_bytes() + payload.len() > MAX_PENDING_RELAY_BYTES
        {
            return Err(ErrorCode::Internal);
        }
        self.pending_relay.push(RelayFrame {
            frame_type,
            timestamp,
            payload: payload.to_vec(),
            app: self.app.clone(),
            stream_name: self.relay_route_key(),
        });
        Ok(())
    }

    fn media_allowed(&self, frame_type: FrameType) -> bool {
        let Some(cb) = self.on_media_cb else {
            return true;
        };
        let codec = match frame_type {
            FrameType::Video => self.detected_video_codec.as_deref(),
            FrameType::Audio => self.detected_audio_codec.as_deref(),
            _ => None,
        };
        cb(self.conn_id, frame_type, codec)
    }

    fn handle_media_frame(
        &mut self,
        frame_type: FrameType,
        timestamp: u32,
        payload: &[u8],
    ) -> Result<()> {
        if !self.relay_enabled
            || !self
                .current_stream
                .as_ref()
                .map(|s| s.is_publishing)
                .unwrap_or(false)
        {
            return Ok(());
        }

        match frame_type {
            FrameType::Video if self.detected_video_codec.is_none() => {
                self.detected_video_codec = detect_video_codec(payload);
            }
            FrameType::Audio if self.detected_audio_codec.is_none() => {
                self.detected_audio_codec = detect_audio_codec(payload);
            }
            _ => {}
        }

        if !self.media_allowed(frame_type) {
            return Err(ErrorCode::Auth);
        }

        self.media_bytes_received = self
            .media_bytes_received
            .saturating_add(payload.len() as u64);

        if self.queue_relay_frame(frame_type, timestamp, payload).is_err() {
            return Err(ErrorCode::Internal);
        }

        if let Some(cb) = self.on_frame_cb {
            let relay = self.pending_relay.last().unwrap();
            let mut frame = Frame {
                frame_type,
                timestamp,
                ..Default::default()
            };
            frame.data = relay.payload.as_ptr();
            frame.size = relay.payload.len() as u32;
            cb(&frame);
        }
        Ok(())
    }

    pub fn get_fd(&self) -> i32 { self.client_fd }

    pub fn recv(&mut self, data: &[u8]) -> Result<()> {
        self.recv_buffer.write(data).map_err(|_| ErrorCode::Internal)?;
        self.bytes_received = self.bytes_received.wrapping_add(data.len() as u32);
        let mut max_iter = 65536;
        let mut no_progress = 0;
        while max_iter > 0 {
            max_iter -= 1;
            let avail = self.recv_buffer.available();
            if avail == 0 && self.state != ConnState::Handshake { break; }
            let before = avail;
            let rc = self.process();
            if rc < 0 {
                return Err(match rc {
                    -1 => ErrorCode::Io,
                    -2 => ErrorCode::Timeout,
                    -3 => ErrorCode::Protocol,
                    -4 => ErrorCode::Handshake,
                    -5 => ErrorCode::Chunk,
                    -6 => ErrorCode::Amf,
                    -7 => ErrorCode::Unsupported,
                    -8 => ErrorCode::Auth,
                    -9 => ErrorCode::Internal,
                    _ => ErrorCode::Internal,
                });
            }
            if rc == 0 {
                let after = self.recv_buffer.available();
                if after == before {
                    no_progress += 1;
                    if no_progress > 3 { break; }
                } else {
                    no_progress = 0;
                }
                if after == 0 && self.state < ConnState::Closing { break; }
            } else {
                no_progress = 0;
            }
        }
        if self.window_ack_size > 0
            && self.bytes_received.wrapping_sub(self.bytes_at_last_ack) >= self.window_ack_size
        {
            self.send_acknowledgement(self.bytes_received)?;
            self.bytes_at_last_ack = self.bytes_received;
        }
        Ok(())
    }

    pub fn process(&mut self) -> i32 {
        match self.state {
            ConnState::TcpAccepted | ConnState::Handshake => self.do_handshake(),
            ConnState::Connected
            | ConnState::AppConnected
            | ConnState::StreamCreated
            | ConnState::Publishing
            | ConnState::Playing
            | ConnState::CapsNegotiated => self.read_messages(),
            ConnState::Closing | ConnState::Closed => 0,
        }
    }

    pub fn do_handshake(&mut self) -> i32 {
        match self.handshake.state {
            HandshakeState::ServerWaitC0 => {
                handshake::server_init(&mut self.handshake);
                match handshake::server_read_c0(&mut self.handshake, &mut self.recv_buffer) {
                    Ok(()) => { self.state = ConnState::Handshake; self.do_handshake_recurse() }
                    Err(ErrorCode::Io) => 0,
                    Err(e) => e as i32,
                }
            }
            HandshakeState::ServerWaitC1 => self.do_handshake_recurse(),
            HandshakeState::ServerWaitC2 => match handshake::server_read_c2(&mut self.handshake, &mut self.recv_buffer) {
                Ok(()) => { self.state = ConnState::Connected; 1 }
                Err(ErrorCode::Io) => 0,
                Err(e) => e as i32,
            },
            HandshakeState::Done => { self.state = ConnState::Connected; 1 }
            _ => -1,
        }
    }

    fn do_handshake_recurse(&mut self) -> i32 {
        match handshake::server_read_c1(&mut self.handshake, &mut self.recv_buffer) {
            Ok(()) => {
                if self.client_fd >= 0 {
                    let s0 = [0x03u8];
                    if self.send_buffer.write(&s0).is_err() { return ErrorCode::Internal as i32; }
                    let out_data = self.handshake.out.peek();
                    if self.send_buffer.write(out_data).is_err() { return ErrorCode::Internal as i32; }
                }
                self.handshake.out.reset();
                1
            }
            Err(ErrorCode::Io) => 0,
            Err(e) => e as i32,
        }
    }

    pub fn read_messages(&mut self) -> i32 {
        loop {
            let mut msg = ChunkMessage::default();
            let mut payload_ptr: *const u8 = std::ptr::null();
            let mut payload_len = 0;
            match chunk_read(&mut self.recv_buffer, &mut self.chunk_reg, None, &mut msg, &mut payload_ptr, &mut payload_len) {
                Ok(0) => break,
                Ok(1) => {
                    if msg.is_complete {
                        let payload_slice = if payload_ptr.is_null() || payload_len == 0 {
                            &[]
                        } else {
                            unsafe { std::slice::from_raw_parts(payload_ptr, payload_len) }
                        };
                        if let Err(e) = self.handle_message(&msg, payload_slice) {
                            return match e {
                                ErrorCode::Auth => -8,
                                _ => -3,
                            };
                        }
                        let _ = self.flush();
                    }
                }
                Ok(_) => break,
                Err(_) => return -1,
            }
        }
        1
    }

    fn handle_message(&mut self, msg: &ChunkMessage, payload: &[u8]) -> Result<()> {
        match msg.msg_type_id {
            msg_dispatch::RTMP_MSG_SET_CHUNK_SIZE
            | msg_dispatch::RTMP_MSG_ABORT_MESSAGE
            | msg_dispatch::RTMP_MSG_ACKNOWLEDGEMENT
            | msg_dispatch::RTMP_MSG_WINDOW_ACK_SIZE
            | msg_dispatch::RTMP_MSG_SET_PEER_BANDWIDTH => self.handle_control(msg.msg_type_id, payload),
            msg_dispatch::RTMP_MSG_USER_CONTROL => self.handle_user_control(payload),
            msg_dispatch::RTMP_MSG_AMF0_COMMAND => self.handle_command(payload),
            msg_dispatch::RTMP_MSG_AMF3_COMMAND => {
                if !payload.is_empty() && payload[0] == 0x00 {
                    self.handle_command(&payload[1..])
                } else {
                    self.handle_command(payload)
                }
            }
            msg_dispatch::RTMP_MSG_AUDIO => self.handle_media_frame(FrameType::Audio, msg.timestamp, payload),
            msg_dispatch::RTMP_MSG_VIDEO => self.handle_media_frame(FrameType::Video, msg.timestamp, payload),
            _ => Ok(()),
        }
    }

    fn handle_control(&mut self, msg_type_id: u8, payload: &[u8]) -> Result<()> {
        match msg_type_id {
            msg_dispatch::RTMP_MSG_SET_CHUNK_SIZE => {
                if payload.len() >= 4 {
                    if let Ok(cs) = control::read_set_chunk_size(payload) {
                        self.apply_chunk_size(cs);
                    }
                }
            }
            msg_dispatch::RTMP_MSG_ABORT_MESSAGE => {
                if payload.len() >= 4 {
                    if let Ok(csid) = control::read_abort_message(payload) {
                        self.chunk_reg.reset_stream(csid);
                    }
                }
            }
            msg_dispatch::RTMP_MSG_WINDOW_ACK_SIZE => {
                if payload.len() >= 4 {
                    if let Ok(win) = control::read_window_ack_size(payload) {
                        self.window_ack_size = win;
                    }
                }
            }
            msg_dispatch::RTMP_MSG_ACKNOWLEDGEMENT => {
                let _ = control::read_acknowledgement_size(payload);
            }
            msg_dispatch::RTMP_MSG_SET_PEER_BANDWIDTH => {
                let _ = control::read_set_peer_bandwidth(payload);
            }
            _ => {}
        }
        Ok(())
    }

    /// Apply an outbound/inbound chunk size to this connection's registry.
    pub fn apply_chunk_size(&mut self, chunk_size: u32) {
        self.chunk_size = chunk_size;
        self.chunk_reg.set_all_chunk_size(chunk_size);
    }

    fn handle_user_control(&mut self, payload: &[u8]) -> Result<()> {
        if payload.len() < 6 {
            return Ok(());
        }
        let (event_type, param1, _) = control::read_user_control(payload, false)?;
        match event_type {
            UCTRL_PING_RESPONSE => {
                if let Some(sent_at) = self.pending_pings.remove(&param1) {
                    self.rtt_ms = sent_at.elapsed().as_secs_f64() * 1000.0;
                }
            }
            UCTRL_PING_REQUEST => {
                self.send_user_control_ping_response(param1)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Send an RTMP ping when due and measure RTT from the client's response.
    pub fn maybe_send_ping(&mut self) -> Result<()> {
        if self.state < ConnState::AppConnected {
            return Ok(());
        }
        let now = Instant::now();
        if self
            .last_ping_sent
            .is_some_and(|t| now.duration_since(t) < PING_INTERVAL)
        {
            return Ok(());
        }

        self.pending_pings
            .retain(|_, sent| now.duration_since(*sent) < PING_TIMEOUT);
        if self.pending_pings.len() >= MAX_PENDING_PINGS {
            return Ok(());
        }

        let token = self.next_ping_token;
        self.next_ping_token = self.next_ping_token.wrapping_add(1);
        self.send_user_control_ping_request(token)?;
        self.pending_pings.insert(token, now);
        self.last_ping_sent = Some(now);
        Ok(())
    }

    pub fn handle_command(&mut self, payload: &[u8]) -> Result<()> {
        let mut buf = Buffer::from_slice(payload);
        let mut name_buf = [0u8; 64];
        if command::peek_name(&mut buf, &mut name_buf).is_err() { return Ok(()); }
        let name = std::str::from_utf8(&name_buf).unwrap_or("").trim_end_matches('\0');
        match name {
            "connect" => {
                let mut info = ConnectInfo::default();
                command::read_connect(&mut buf, &mut info)?;
                let app_len = info.app.iter().position(|&b| b == 0).unwrap_or(0);
                self.app = std::str::from_utf8(&info.app[..app_len]).unwrap_or("").to_string();
                let _ = state_machine::conn_transition(&mut self.state, ConnState::AppConnected);
                self.send_connect_response(info.transaction_id)?;
                if !self.connect_cb_fired {
                    self.connect_cb_fired = true;
                    if let Some(cb) = self.on_connect_cb { cb(); }
                }
            }
            "createStream" => {
                if self.state < ConnState::AppConnected {
                    return self.send_onstatus(0, "error", "NetStream.Failed", "connect required before createStream");
                }
                let txn = command::read_create_stream(&mut buf)?;
                if self.next_stream_id >= MAX_STREAMS_PER_CONN {
                    self.send_onstatus(0, "error", "NetStream.Failed", "Too many streams")?;
                } else {
                    self.next_stream_id += 1;
                    let stream_id = self.next_stream_id;
                    self.current_stream = Some(Box::new(Stream::new(stream_id)));
                    let _ = state_machine::conn_transition(&mut self.state, ConnState::StreamCreated);
                    self.send_create_stream_response(txn, stream_id)?;
                }
            }
            "publish" => {
                let mut stream_name = [0u8; 256];
                let mut publish_type = [0u8; 64];
                command::read_publish(&mut buf, &mut stream_name, &mut publish_type)?;
                let name_str = std::str::from_utf8(&stream_name).unwrap_or("").trim_end_matches('\0').to_string();
                if self.current_stream.is_none() {
                    return self.send_onstatus(
                        0,
                        "error",
                        "NetStream.Publish.BadConnection",
                        "No stream created",
                    );
                }
                if let Some(cb) = self.on_publish_cb {
                    if !cb(self.conn_id, &self.app, &name_str) {
                        return self.send_onstatus(0, "error", "NetStream.Publish.BadName", "Publish not authorized");
                    }
                }
                if !self.defer_media_relay || self.on_publish_cb.is_none() {
                    self.relay_enabled = true;
                }
                {
                    if let Some(ref mut stream) = self.current_stream {
                        stream.is_publishing = true;
                        stream.name = name_str;
                    }
                    let _ = state_machine::conn_transition(&mut self.state, ConnState::Publishing);
                    let sid = self.current_stream.as_ref().map(|s| s.stream_id).unwrap_or(0);
                    self.send_onstatus(sid, "status", "NetStream.Publish.Start", "Publishing")?;
                }
            }
            "play" => {
                let mut stream_name = [0u8; 256];
                command::read_play(&mut buf, &mut stream_name)?;
                let name_str = std::str::from_utf8(&stream_name).unwrap_or("").trim_end_matches('\0').to_string();
                if self.current_stream.is_none() {
                    return self.send_onstatus(
                        0,
                        "error",
                        "NetStream.Play.BadConnection",
                        "No stream created",
                    );
                }
                if let Some(cb) = self.on_play_cb {
                    if !cb(self.conn_id, &self.app, &name_str) {
                        return self.send_onstatus(0, "error", "NetStream.Play.Failed", "Play not authorized");
                    }
                }
                if !self.defer_media_relay || self.on_play_cb.is_none() {
                    self.relay_enabled = true;
                }
                {
                    if let Some(ref mut stream) = self.current_stream {
                        stream.is_playing = true;
                        stream.name = name_str;
                    }
                    self.needs_init_frames = true;
                    let _ = state_machine::conn_transition(&mut self.state, ConnState::Playing);
                    let sid = self.current_stream.as_ref().map(|s| s.stream_id).unwrap_or(0);
                    self.send_onstatus(sid, "status", "NetStream.Play.Start", "Playing")?;
                }
            }
            "FCPublish" | "FCUnpublish" | "releaseStream" | "deleteStream" => {}
            _ => {}
        }
        Ok(())
    }

    pub fn send_connect_response(&mut self, transaction_id: f64) -> Result<()> {
        let win = SERVER_WINDOW_ACK_SIZE.to_be_bytes();
        self.send_control(0x05, &win)?;
        let mut bw = [0u8; 5];
        let bw_val = SERVER_PEER_BANDWIDTH.to_be_bytes();
        bw[..4].copy_from_slice(&bw_val);
        bw[4] = PEER_BANDWIDTH_DYNAMIC;
        self.send_control(0x06, &bw)?;
        let cs = self.chunk_size.to_be_bytes();
        self.send_control(0x01, &cs)?;
        let mut amf_buf = Buffer::with_capacity(512);
        crate::amf::amf0::write_string(&mut amf_buf, "_result")?;
        crate::amf::amf0::write_number(&mut amf_buf, transaction_id)?;
        crate::amf::amf0::write_null(&mut amf_buf)?;
        crate::amf::amf0::write_object_begin(&mut amf_buf)?;
        crate::amf::amf0::write_object_key(&mut amf_buf, "level")?;
        crate::amf::amf0::write_string(&mut amf_buf, "status")?;
        crate::amf::amf0::write_object_key(&mut amf_buf, "code")?;
        crate::amf::amf0::write_string(&mut amf_buf, "NetConnection.Connect.Success")?;
        crate::amf::amf0::write_object_key(&mut amf_buf, "description")?;
        crate::amf::amf0::write_string(&mut amf_buf, "Connection succeeded.")?;
        crate::amf::amf0::write_object_end(&mut amf_buf)?;
        self.send_command(0, amf_buf.as_slice())
    }

    pub fn send_create_stream_response(&mut self, transaction_id: f64, stream_id: u32) -> Result<()> {
        let mut amf_buf = Buffer::with_capacity(256);
        command::build_create_stream_result(&mut amf_buf, transaction_id, stream_id as f64)?;
        self.send_command(0, amf_buf.as_slice())
    }

    pub fn send_onstatus(&mut self, stream_id: u32, level: &str, code: &str, description: &str) -> Result<()> {
        let mut amf_buf = Buffer::with_capacity(512);
        command::build_onstatus(&mut amf_buf, level, code, description)?;
        self.send_command(stream_id, amf_buf.as_slice())
    }

    pub fn flush(&mut self) -> Result<()> {
        if self.client_fd < 0 || self.send_buffer.available() == 0 { return Ok(()); }
        let Some(ref mut transport) = self.transport else { return Ok(()); };
        while self.send_buffer.available() > 0 {
            let pending = self.send_buffer.peek();
            let n = transport.try_send(pending, &mut 0i32)?;
            if n == 0 { break; }
            self.send_buffer.drain(n);
        }
        Ok(())
    }

    pub fn send_frame(&mut self, frame_type: FrameType, timestamp: u32, payload: &[u8]) -> Result<()> {
        let stream_id = self.current_stream.as_ref().map(|s| s.stream_id).unwrap_or(1);
        let mut cmsg = ChunkMessage::default();
        cmsg.timestamp = timestamp;
        cmsg.msg_length = payload.len() as u32;
        cmsg.msg_stream_id = stream_id;
        cmsg.fmt = 0;
        if frame_type == FrameType::Audio {
            cmsg.csid = 4;
            cmsg.msg_type_id = 0x08;
        } else {
            cmsg.csid = 6;
            cmsg.msg_type_id = 0x09;
        }
        chunk_write(
            &mut self.send_buffer,
            &cmsg,
            payload,
            payload.len(),
            self.chunk_size as usize,
        )?;
        self.media_bytes_sent = self
            .media_bytes_sent
            .saturating_add(payload.len() as u64);
        Ok(())
    }

    fn send_control(&mut self, ty: u8, data: &[u8]) -> Result<()> {
        let mut msg = ChunkMessage::default();
        msg.csid = 2;
        msg.fmt = 0;
        msg.msg_length = data.len() as u32;
        msg.msg_type_id = ty;
        msg.msg_stream_id = 0;
        chunk_write(&mut self.send_buffer, &msg, data, data.len(), self.chunk_size as usize)
    }

    fn send_command(&mut self, msg_stream_id: u32, amf_data: &[u8]) -> Result<()> {
        let mut cmd_msg = ChunkMessage::default();
        cmd_msg.csid = 3;
        cmd_msg.fmt = 0;
        cmd_msg.timestamp = 0;
        cmd_msg.msg_length = amf_data.len() as u32;
        cmd_msg.msg_type_id = 0x14;
        cmd_msg.msg_stream_id = msg_stream_id;
        chunk_write(&mut self.send_buffer, &cmd_msg, amf_data, amf_data.len(), self.chunk_size as usize)
    }

    fn send_acknowledgement(&mut self, seq: u32) -> Result<()> {
        self.send_control(0x03, &seq.to_be_bytes())
    }

    fn send_user_control_ping_request(&mut self, timestamp: u32) -> Result<()> {
        let mut buf = Buffer::with_capacity(6);
        control::write_user_control_ping_request(&mut buf, timestamp)?;
        self.send_control(msg_dispatch::RTMP_MSG_USER_CONTROL, buf.as_slice())
    }

    fn send_user_control_ping_response(&mut self, timestamp: u32) -> Result<()> {
        let mut buf = Buffer::with_capacity(6);
        control::write_user_control_ping_response(&mut buf, timestamp)?;
        self.send_control(msg_dispatch::RTMP_MSG_USER_CONTROL, buf.as_slice())
    }
}

impl Default for Conn {
    fn default() -> Self { Self::new() }
}

fn detect_video_codec(payload: &[u8]) -> Option<String> {
    if payload.is_empty() { return None; }
    if payload[0] & 0x80 != 0 {
        if payload.len() >= 5 {
            if let Ok(s) = std::str::from_utf8(&payload[1..5]) { return Some(s.to_string()); }
        }
        return None;
    }
    Some(match payload[0] & 0x0F {
        7 => "avc1".to_string(),
        12 => "hvc1".to_string(),
        13 => "av01".to_string(),
        _ => return None,
    })
}

fn detect_audio_codec(payload: &[u8]) -> Option<String> {
    if payload.is_empty() { return None; }
    if (payload[0] & 0xF0) == 0x90 && payload.len() >= 5 {
        if let Ok(s) = std::str::from_utf8(&payload[1..5]) { return Some(s.to_string()); }
    }
    Some(match (payload[0] >> 4) & 0x0F {
        10 => "mp4a".to_string(),
        2 => "mp3".to_string(),
        14 => "Opus".to_string(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::stream::Stream;

    #[test]
    fn relay_route_key_prefers_relay_key_over_rtmp_name() {
        let mut conn = Conn::new();
        conn.relay_key = "stream-db-id".to_string();
        conn.current_stream = Some(Box::new(Stream::new(1)));
        if let Some(ref mut stream) = conn.current_stream {
            stream.name = "pub_or_play_key".to_string();
        }
        assert_eq!(conn.relay_route_key(), "stream-db-id");
    }

    #[test]
    fn relay_route_key_falls_back_to_rtmp_stream_name() {
        let mut conn = Conn::new();
        conn.current_stream = Some(Box::new(Stream::new(1)));
        if let Some(ref mut stream) = conn.current_stream {
            stream.name = "legacy_name".to_string();
        }
        assert_eq!(conn.relay_route_key(), "legacy_name");
    }

    #[test]
    fn apply_chunk_size_updates_connection_registry() {
        let mut conn = Conn::new();
        conn.apply_chunk_size(4096);
        assert_eq!(conn.chunk_size, 4096);
        assert_eq!(conn.chunk_reg.default_chunk_size, 4096);
    }

    #[test]
    fn handle_control_applies_peer_set_chunk_size() {
        let mut conn = Conn::new();
        conn.handle_control(
            msg_dispatch::RTMP_MSG_SET_CHUNK_SIZE,
            &4096u32.to_be_bytes(),
        )
        .unwrap();
        assert_eq!(conn.chunk_size, 4096);
        assert_eq!(conn.chunk_reg.default_chunk_size, 4096);
    }
}
