use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::buffer::Buffer;
use crate::chunk::reader::{ChunkMessage, chunk_read_owned};
use crate::chunk::state::{ChunkRegistry, DEFAULT_CHUNK_SIZE, DEFAULT_MAX_MSG_LENGTH};
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
/// Cap complete messages handled per `read_messages` call so one TCP recv batch
/// cannot drain thousands of tiny control messages in a single state-machine step.
const MAX_MESSAGES_PER_READ: usize = 256;
/// Hard cap on complete messages handled across all `read_messages` passes in a
/// single `recv` call. Without this, the outer `recv` loop (256 iterations)
/// multiplies the per-pass cap into 65,536 message parses per recv batch.
const MAX_MESSAGES_PER_RECV: usize = 256;
/// Maximum bytes retained in the per-connection inbound staging buffer. When
/// the per-call message budget stops draining faster than the peer sends,
/// complete wire data can otherwise accumulate here up to `BUFFER_MAX_SIZE`
/// (64 MiB) per connection. Sized to comfortably hold one in-flight
/// `DEFAULT_MAX_MSG_LENGTH` message even when reassembled from many small
/// chunks (e.g. a peer that never raises chunk size above the 128-byte
/// default) plus normal per-poll staging, while staying well below the
/// original unbounded-growth risk this cap replaces.
const MAX_RECV_BUFFER_BYTES: usize = 2 * DEFAULT_MAX_MSG_LENGTH as usize;

const SERVER_WINDOW_ACK_SIZE: u32 = 2_500_000;
const SERVER_PEER_BANDWIDTH: u32 = 2_500_000;
const PEER_BANDWIDTH_DYNAMIC: u8 = 2;
const PING_INTERVAL: Duration = Duration::from_secs(5);
const PING_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PENDING_PINGS: usize = 4;
/// Cap inbound Ping-Request reflections per connection to prevent trivial
/// outbound bandwidth/CPU amplification from unauthenticated peers.
const MAX_INBOUND_PING_RESPONSES: usize = 8;
const INBOUND_PING_WINDOW: Duration = Duration::from_secs(1);
/// Cap sub-tags unpacked from a single Aggregate message (mirrors
/// `message::message::MAX_AGGREGATE_SUBTAGS`).
const MAX_AGGREGATE_SUBTAGS: usize = 4096;

pub struct RelayFrame {
    pub frame_type: FrameType,
    pub timestamp: u32,
    pub payload: Vec<u8>,
    pub app: String,
    pub stream_name: String,
    pub publisher_conn_id: u64,
}

pub struct Conn {
    pub state: ConnState,
    pub handshake: Handshake,
    pub recv_buffer: Buffer,
    pub send_buffer: Buffer,
    pub chunk_reg: ChunkRegistry,
    /// Target chunk size announced to the peer (from server config).
    pub chunk_size: u32,
    /// Active outbound chunk size; stays at the RTMP default until SetChunkSize
    /// is negotiated on the wire.
    active_chunk_size: u32,
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
    pub detected_video_width: Option<u32>,
    pub detected_video_height: Option<u32>,
    pub detected_video_framerate: Option<f64>,
    pub detected_audio_sample_rate: Option<u32>,
    pub detected_audio_channels: Option<u32>,
    pub relay_enabled: bool,
    /// When true, media relay stays off until the integrator sets `relay_enabled`
    /// after its own post-auth bookkeeping (used by librtmp2-server).
    pub defer_media_relay: bool,
    /// Cap on queued relay payload bytes for this connection.
    pub max_pending_relay_bytes: usize,
    pub on_frame_cb: Option<fn(&Frame)>,
    /// When set, must return true before audio/video is queued for relay.
    pub on_media_cb: Option<fn(u64, FrameType, Option<&str>) -> bool>,
    pub on_connect_cb: Option<fn()>,
    pub on_publish_cb: Option<fn(conn_id: u64, app: &str, stream_name: &str) -> bool>,
    pub on_play_cb: Option<fn(conn_id: u64, app: &str, stream_name: &str) -> bool>,
    /// Cache keys to evict after the publisher renames its stream.
    pub pending_cache_evictions: Vec<(String, String)>,
    /// Last measured client↔server RTT in milliseconds (RTMP UserControl ping).
    pub rtt_ms: f64,
    pending_pings: HashMap<u32, Instant>,
    last_ping_sent: Option<Instant>,
    next_ping_token: u32,
    inbound_ping_responses: usize,
    inbound_ping_window_start: Option<Instant>,
    /// Retains the last frame payload delivered through `on_frame_cb` so
    /// `Frame.data` stays valid until the next callback on this connection.
    frame_cb_scratch: Vec<u8>,
    /// Set when `read_messages` stops because `MAX_MESSAGES_PER_READ` or
    /// `MAX_MESSAGES_PER_RECV` was hit while `recv_buffer` still holds
    /// complete, unprocessed messages -- as opposed to stopping because the
    /// buffer ran out of data. Callers must keep draining (e.g. via another
    /// `recv(&[])`) until this clears, or a batch that exceeds the budget can
    /// sit unprocessed until the peer happens to send more bytes.
    budget_exhausted: bool,
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
            chunk_size: DEFAULT_CHUNK_SIZE,
            active_chunk_size: DEFAULT_CHUNK_SIZE,
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
            detected_video_width: None,
            detected_video_height: None,
            detected_video_framerate: None,
            detected_audio_sample_rate: None,
            detected_audio_channels: None,
            relay_enabled: false,
            defer_media_relay: false,
            max_pending_relay_bytes: MAX_PENDING_RELAY_BYTES,
            on_frame_cb: None,
            on_media_cb: None,
            on_connect_cb: None,
            on_publish_cb: None,
            on_play_cb: None,
            pending_cache_evictions: Vec::new(),
            rtt_ms: 0.0,
            pending_pings: HashMap::new(),
            last_ping_sent: None,
            next_ping_token: 1,
            inbound_ping_responses: 0,
            inbound_ping_window_start: None,
            frame_cb_scratch: Vec::new(),
            budget_exhausted: false,
        }
    }

    /// True if the last `recv`/`process` pass stopped early because the
    /// per-call message budget was exhausted while complete messages were
    /// still buffered. Callers (e.g. the server poll loop) should keep
    /// calling `recv(&[])` until this returns false so a batch larger than
    /// the budget doesn't stall waiting on the peer to send more bytes.
    pub fn has_buffered_messages(&self) -> bool {
        self.budget_exhausted
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

    /// Queue eviction of the current publish route's cache key when
    /// `current_stream` is about to be repurposed or replaced while still
    /// marked as actively publishing (a fresh `createStream`, or switching
    /// to `play`, both abandon whatever was being published). Unlike a
    /// `publish` rename there is no new route to keep serving, so this
    /// always evicts when the connection was publishing. No-op otherwise.
    fn evict_active_publish_route(&mut self) {
        let was_publishing = self
            .current_stream
            .as_ref()
            .map(|s| s.is_publishing)
            .unwrap_or(false);
        if !was_publishing {
            return;
        }
        let route_key = self.relay_route_key();
        if !route_key.is_empty() {
            self.pending_cache_evictions
                .push((self.app.clone(), route_key));
        }
        self.clear_detected_stream_metadata();
    }

    fn clear_detected_stream_metadata(&mut self) {
        self.detected_video_width = None;
        self.detected_video_height = None;
        self.detected_video_framerate = None;
        self.detected_audio_sample_rate = None;
        self.detected_audio_channels = None;
    }

    fn publishing_metadata_allowed(&self, msg_stream_id: u32) -> bool {
        let expected_stream_id = self
            .current_stream
            .as_ref()
            .filter(|s| s.is_publishing)
            .map(|s| s.stream_id)
            .unwrap_or(0);
        msg_stream_id == expected_stream_id && expected_stream_id != 0
    }

    fn handle_publisher_data_message(&mut self, msg_stream_id: u32, payload: &[u8]) -> Result<()> {
        if !self.publishing_metadata_allowed(msg_stream_id) {
            return Ok(());
        }
        self.handle_data_message(payload)
    }

    fn queue_relay_frame(
        &mut self,
        frame_type: FrameType,
        timestamp: u32,
        payload: &[u8],
    ) -> Result<()> {
        if self.pending_relay.len() >= MAX_PENDING_RELAY_FRAMES
            || self.pending_relay_bytes() + payload.len() > self.max_pending_relay_bytes
        {
            return Err(ErrorCode::Internal);
        }
        self.pending_relay.push(RelayFrame {
            frame_type,
            timestamp,
            payload: payload.to_vec(),
            app: self.app.clone(),
            stream_name: self.relay_route_key(),
            publisher_conn_id: self.conn_id,
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
        msg_stream_id: u32,
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

        let expected_stream_id = self
            .current_stream
            .as_ref()
            .map(|s| s.stream_id)
            .unwrap_or(0);
        if msg_stream_id != expected_stream_id {
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

        if let Some(cb) = self.on_frame_cb {
            self.frame_cb_scratch.clear();
            self.frame_cb_scratch.extend_from_slice(payload);
            let frame = Frame {
                frame_type,
                timestamp,
                size: self.frame_cb_scratch.len() as u32,
                data: self.frame_cb_scratch.as_ptr(),
                ..Default::default()
            };
            cb(&frame);
        }

        if self
            .queue_relay_frame(frame_type, timestamp, payload)
            .is_err()
        {
            return Err(ErrorCode::Internal);
        }
        Ok(())
    }

    /// Unpack an RTMP Aggregate message (type 0x16) into its constituent FLV
    /// sub-tags and route each audio/video sub-tag through the same relay path
    /// as a standalone `RTMP_MSG_AUDIO`/`RTMP_MSG_VIDEO` message.
    ///
    /// Each sub-tag is `tag_type(1) + data_size(3) + timestamp(3) + timestamp_ext(1)
    /// + stream_id(3) + data(data_size) + prev_tag_size(4)`. Sub-tag timestamps are
    /// relative to the first sub-tag's timestamp; that delta is added to the
    /// aggregate message's own chunk timestamp per the FLV/RTMP aggregate spec.
    fn handle_aggregate(
        &mut self,
        msg_stream_id: u32,
        base_timestamp: u32,
        payload: &[u8],
    ) -> Result<()> {
        let mut pos = 0;
        let mut have_base = false;
        let mut sub_base_ts: u32 = 0;
        let mut subtags = 0usize;

        while pos + 11 <= payload.len() {
            if subtags >= MAX_AGGREGATE_SUBTAGS {
                return Err(ErrorCode::Protocol);
            }
            subtags += 1;

            let tag_type = payload[pos];
            let data_size = ((payload[pos + 1] as u32) << 16)
                | ((payload[pos + 2] as u32) << 8)
                | (payload[pos + 3] as u32);
            let ts = ((payload[pos + 4] as u32) << 16)
                | ((payload[pos + 5] as u32) << 8)
                | (payload[pos + 6] as u32)
                | ((payload[pos + 7] as u32) << 24);

            let body = pos + 11;
            let data_size = data_size as usize;
            if body + data_size > payload.len() {
                return Err(ErrorCode::Protocol);
            }

            if !have_base {
                sub_base_ts = ts;
                have_base = true;
            }
            // Use wrapping arithmetic: a sub-tag ts less than the first sub-tag's
            // ts (malformed but possible) must not panic in debug or silently
            // wrap in an unexpected direction in release.
            let out_ts = base_timestamp.wrapping_add(ts.wrapping_sub(sub_base_ts));
            let tag_payload = &payload[body..body + data_size];

            match tag_type {
                msg_dispatch::RTMP_MSG_AUDIO => {
                    self.handle_media_frame(msg_stream_id, FrameType::Audio, out_ts, tag_payload)?;
                }
                msg_dispatch::RTMP_MSG_VIDEO => {
                    self.handle_media_frame(msg_stream_id, FrameType::Video, out_ts, tag_payload)?;
                }
                msg_dispatch::RTMP_MSG_AMF0_DATA => {
                    self.handle_publisher_data_message(msg_stream_id, tag_payload)?;
                }
                _ => {}
            }

            pos = body + data_size + 4;
        }

        Ok(())
    }

    pub fn get_fd(&self) -> i32 {
        self.client_fd
    }

    pub fn recv(&mut self, data: &[u8]) -> Result<()> {
        if !data.is_empty()
            && self.recv_buffer.available().saturating_add(data.len()) > MAX_RECV_BUFFER_BYTES
        {
            return Err(ErrorCode::Protocol);
        }
        self.recv_buffer
            .write(data)
            .map_err(|_| ErrorCode::Internal)?;
        self.bytes_received = self.bytes_received.wrapping_add(data.len() as u32);
        self.budget_exhausted = false;
        let mut max_iter = 256;
        let mut no_progress = 0;
        let mut messages_budget = MAX_MESSAGES_PER_RECV;
        while max_iter > 0 {
            if messages_budget == 0 {
                self.budget_exhausted = true;
                break;
            }
            max_iter -= 1;
            let avail = self.recv_buffer.available();
            if avail == 0 && self.state != ConnState::Handshake {
                break;
            }
            let before = avail;
            let rc = self.process(&mut messages_budget);
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
                    if no_progress > 3 {
                        break;
                    }
                } else {
                    no_progress = 0;
                }
                if after == 0 && self.state < ConnState::Closing {
                    break;
                }
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

    pub fn process(&mut self, messages_budget: &mut usize) -> i32 {
        match self.state {
            ConnState::TcpAccepted | ConnState::Handshake => self.do_handshake(),
            ConnState::Connected
            | ConnState::AppConnected
            | ConnState::StreamCreated
            | ConnState::Publishing
            | ConnState::Playing
            | ConnState::CapsNegotiated => self.read_messages(messages_budget),
            ConnState::Closing | ConnState::Closed => 0,
        }
    }

    pub fn do_handshake(&mut self) -> i32 {
        match self.handshake.state {
            HandshakeState::ServerWaitC0 => {
                handshake::server_init(&mut self.handshake);
                match handshake::server_read_c0(&mut self.handshake, &mut self.recv_buffer) {
                    Ok(()) => {
                        self.state = ConnState::Handshake;
                        self.do_handshake_recurse()
                    }
                    Err(ErrorCode::Io) => 0,
                    Err(e) => e as i32,
                }
            }
            HandshakeState::ServerWaitC1 => self.do_handshake_recurse(),
            HandshakeState::ServerWaitC2 => {
                match handshake::server_read_c2(&mut self.handshake, &mut self.recv_buffer) {
                    Ok(()) => {
                        self.state = ConnState::Connected;
                        1
                    }
                    Err(ErrorCode::Io) => 0,
                    Err(e) => e as i32,
                }
            }
            HandshakeState::Done => {
                self.state = ConnState::Connected;
                1
            }
            _ => -1,
        }
    }

    fn do_handshake_recurse(&mut self) -> i32 {
        match handshake::server_read_c1(&mut self.handshake, &mut self.recv_buffer) {
            Ok(()) => {
                if self.client_fd >= 0 {
                    let s0 = [0x03u8];
                    if self.send_buffer.write(&s0).is_err() {
                        return ErrorCode::Internal as i32;
                    }
                    let out_data = self.handshake.out.peek();
                    if self.send_buffer.write(out_data).is_err() {
                        return ErrorCode::Internal as i32;
                    }
                }
                self.handshake.out.reset();
                1
            }
            Err(ErrorCode::Io) => 0,
            Err(e) => e as i32,
        }
    }

    pub fn read_messages(&mut self, messages_budget: &mut usize) -> i32 {
        let mut processed = 0usize;
        loop {
            if processed >= MAX_MESSAGES_PER_READ || *messages_budget == 0 {
                // Stopped because the budget ran out, not because the buffer
                // is empty -- there may still be complete messages sitting in
                // `recv_buffer` that the caller needs to keep draining.
                self.budget_exhausted = true;
                break;
            }
            let mut msg = ChunkMessage::default();
            match chunk_read_owned(&mut self.recv_buffer, &mut self.chunk_reg, &mut msg) {
                Ok((0, _)) => break,
                Ok((1, payload_owned)) => {
                    if msg.is_complete {
                        processed += 1;
                        *messages_budget = messages_budget.saturating_sub(1);
                        if let Err(e) = self.handle_message(&msg, &payload_owned) {
                            return match e {
                                ErrorCode::Auth => -8,
                                _ => -3,
                            };
                        }
                        let _ = self.flush();
                    }
                }
                Ok(_) => break,
                Err(ErrorCode::Chunk) => return -5,
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
            | msg_dispatch::RTMP_MSG_SET_PEER_BANDWIDTH => {
                self.handle_control(msg.msg_type_id, payload)
            }
            msg_dispatch::RTMP_MSG_USER_CONTROL => self.handle_user_control(payload),
            msg_dispatch::RTMP_MSG_AMF0_COMMAND => self.handle_command(payload),
            msg_dispatch::RTMP_MSG_AMF3_COMMAND => {
                if !payload.is_empty() && payload[0] == 0x00 {
                    self.handle_command(&payload[1..])
                } else {
                    self.handle_command(payload)
                }
            }
            msg_dispatch::RTMP_MSG_AUDIO => {
                self.handle_media_frame(msg.msg_stream_id, FrameType::Audio, msg.timestamp, payload)
            }
            msg_dispatch::RTMP_MSG_VIDEO => {
                self.handle_media_frame(msg.msg_stream_id, FrameType::Video, msg.timestamp, payload)
            }
            msg_dispatch::RTMP_MSG_AGGREGATE => {
                self.handle_aggregate(msg.msg_stream_id, msg.timestamp, payload)
            }
            msg_dispatch::RTMP_MSG_AMF0_DATA => {
                self.handle_publisher_data_message(msg.msg_stream_id, payload)
            }
            msg_dispatch::RTMP_MSG_AMF3_DATA => {
                if !payload.is_empty() && payload[0] == 0x00 {
                    self.handle_publisher_data_message(msg.msg_stream_id, &payload[1..])
                } else {
                    self.handle_publisher_data_message(msg.msg_stream_id, payload)
                }
            }
            _ => Ok(()),
        }
    }

    fn handle_data_message(&mut self, payload: &[u8]) -> Result<()> {
        let mut buf = Buffer::from_slice(payload);
        let first_byte = match buf.peek().first().copied() {
            Some(b) => b,
            None => return Ok(()),
        };
        if first_byte != crate::amf::amf0::Amf0Type::String as u8
            && first_byte != crate::amf::amf0::Amf0Type::LongString as u8
        {
            return Ok(());
        }

        let mut name = [0u8; 64];
        let Some(name_len) = read_data_event_name(
            &mut buf,
            first_byte == crate::amf::amf0::Amf0Type::String as u8,
            &mut name,
        ) else {
            return Ok(());
        };
        let name_str = std::str::from_utf8(&name[..name_len]).unwrap_or("");

        if name_str == "@setDataFrame" {
            let next_byte = match buf.peek().first().copied() {
                Some(b) => b,
                None => return Ok(()),
            };
            if next_byte != crate::amf::amf0::Amf0Type::String as u8
                && next_byte != crate::amf::amf0::Amf0Type::LongString as u8
            {
                return Ok(());
            }
            let mut inner = [0u8; 64];
            let Some(inner_len) = read_data_event_name(
                &mut buf,
                next_byte == crate::amf::amf0::Amf0Type::String as u8,
                &mut inner,
            ) else {
                return Ok(());
            };
            let inner_str = std::str::from_utf8(&inner[..inner_len]).unwrap_or("");
            if inner_str != "onMetaData" {
                return Ok(());
            }
        } else if name_str != "onMetaData" {
            return Ok(());
        }

        self.clear_detected_stream_metadata();
        self.parse_on_metadata_object(&mut buf)
    }

    fn parse_on_metadata_object(&mut self, buf: &mut Buffer) -> Result<()> {
        let ty = match crate::amf::amf0::read_type(buf) {
            Ok(t) => t,
            Err(_) => return Ok(()),
        };
        if ty != crate::amf::amf0::Amf0Type::Object && ty != crate::amf::amf0::Amf0Type::EcmaArray {
            return Ok(());
        }
        if ty == crate::amf::amf0::Amf0Type::EcmaArray {
            let mut count_bytes = [0u8; 4];
            if buf.read(&mut count_bytes).is_err() {
                return Ok(());
            }
        }

        let mut keys = 0usize;
        while !crate::amf::amf0::is_object_end(buf) {
            keys += 1;
            if keys > crate::amf::amf0::MAX_OBJECT_KEYS {
                return Ok(());
            }
            let mut key = [0u8; 256];
            if crate::amf::amf0::read_object_key(buf, &mut key).is_err() {
                return Ok(());
            }
            let key_len = key.iter().position(|&b| b == 0).unwrap_or(key.len());
            let key_str = std::str::from_utf8(&key[..key_len]).unwrap_or("");
            if !self.apply_metadata_key(key_str, buf) {
                return Ok(());
            }
        }
        let mut end = [0u8; 3];
        if buf.read(&mut end).is_err() {
            return Ok(());
        }
        Ok(())
    }

    fn apply_metadata_key(&mut self, key: &str, buf: &mut Buffer) -> bool {
        let ty = match crate::amf::amf0::read_type(buf) {
            Ok(t) => t,
            Err(_) => return false,
        };
        match key {
            "width" => {
                if ty == crate::amf::amf0::Amf0Type::Number {
                    let v = match crate::amf::amf0::read_number(buf) {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    if let Some(w) = positive_f64_to_u32(v) {
                        self.detected_video_width = Some(w);
                    }
                } else {
                    return crate::amf::amf0::skip_value_after_type(buf, ty).is_ok();
                }
            }
            "height" => {
                if ty == crate::amf::amf0::Amf0Type::Number {
                    let v = match crate::amf::amf0::read_number(buf) {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    if let Some(h) = positive_f64_to_u32(v) {
                        self.detected_video_height = Some(h);
                    }
                } else {
                    return crate::amf::amf0::skip_value_after_type(buf, ty).is_ok();
                }
            }
            "framerate" | "videoframerate" => {
                if ty == crate::amf::amf0::Amf0Type::Number {
                    let v = match crate::amf::amf0::read_number(buf) {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    if sane_framerate(v) {
                        self.detected_video_framerate = Some(v);
                    }
                } else {
                    return crate::amf::amf0::skip_value_after_type(buf, ty).is_ok();
                }
            }
            "audiosamplerate" => {
                if ty == crate::amf::amf0::Amf0Type::Number {
                    let v = match crate::amf::amf0::read_number(buf) {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    if let Some(sr) = positive_f64_to_u32(v) {
                        self.detected_audio_sample_rate = Some(sr);
                    }
                } else {
                    return crate::amf::amf0::skip_value_after_type(buf, ty).is_ok();
                }
            }
            "audiochannels" => {
                if ty == crate::amf::amf0::Amf0Type::Number {
                    let v = match crate::amf::amf0::read_number(buf) {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    if let Some(ch) = positive_f64_to_u32(v) {
                        if ch > 0 && ch <= 32 {
                            self.detected_audio_channels = Some(ch);
                        }
                    }
                } else {
                    return crate::amf::amf0::skip_value_after_type(buf, ty).is_ok();
                }
            }
            "stereo" => {
                if ty == crate::amf::amf0::Amf0Type::Boolean {
                    let stereo = match crate::amf::amf0::read_boolean(buf) {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    self.detected_audio_channels = Some(if stereo { 2 } else { 1 });
                } else {
                    return crate::amf::amf0::skip_value_after_type(buf, ty).is_ok();
                }
            }
            _ => return crate::amf::amf0::skip_value_after_type(buf, ty).is_ok(),
        }
        true
    }

    fn handle_control(&mut self, msg_type_id: u8, payload: &[u8]) -> Result<()> {
        match msg_type_id {
            msg_dispatch::RTMP_MSG_SET_CHUNK_SIZE => {
                if payload.len() >= 4 {
                    if let Ok(cs) = control::read_set_chunk_size(payload) {
                        self.chunk_reg.set_all_chunk_size(cs);
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
                if payload.len() >= 4 {
                    let _ = control::read_acknowledgement_size(payload);
                }
            }
            msg_dispatch::RTMP_MSG_SET_PEER_BANDWIDTH => {
                if payload.len() >= 5 {
                    let _ = control::read_set_peer_bandwidth(payload);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Apply a chunk size to outbound writes and inbound reassembly.
    pub fn apply_chunk_size(&mut self, chunk_size: u32) {
        self.chunk_size = chunk_size;
        self.active_chunk_size = chunk_size;
        self.chunk_reg.set_all_chunk_size(chunk_size);
    }

    fn activate_announced_chunk_size(&mut self) {
        self.active_chunk_size = self.chunk_size;
        self.chunk_reg.set_all_chunk_size(self.chunk_size);
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
                let now = Instant::now();
                if let Some(start) = self.inbound_ping_window_start {
                    if now.duration_since(start) >= INBOUND_PING_WINDOW {
                        self.inbound_ping_window_start = Some(now);
                        self.inbound_ping_responses = 0;
                    }
                } else {
                    self.inbound_ping_window_start = Some(now);
                }
                if self.inbound_ping_responses >= MAX_INBOUND_PING_RESPONSES {
                    return Err(ErrorCode::Protocol);
                }
                self.inbound_ping_responses += 1;
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
        if command::peek_name(&mut buf, &mut name_buf).is_err() {
            return Ok(());
        }
        let name = std::str::from_utf8(&name_buf)
            .unwrap_or("")
            .trim_end_matches('\0');
        match name {
            "connect" => {
                // A connection may only negotiate its app namespace once. Without
                // this guard a peer that's already publishing/playing under an
                // authorized app could send a second `connect` with a different
                // app, silently repointing `self.app` (and therefore relay/cache
                // routing) to a namespace `on_publish_cb`/`on_play_cb` never
                // authorized -- cache poisoning / cross-app playback.
                if self.state >= ConnState::AppConnected {
                    return Ok(());
                }
                let mut info = ConnectInfo::default();
                command::read_connect(&mut buf, &mut info)?;
                let app_len = info.app.iter().position(|&b| b == 0).unwrap_or(0);
                self.app = std::str::from_utf8(&info.app[..app_len])
                    .unwrap_or("")
                    .to_string();
                let _ = state_machine::conn_transition(&mut self.state, ConnState::AppConnected);
                self.send_connect_response(info.transaction_id)?;
                if !self.connect_cb_fired {
                    self.connect_cb_fired = true;
                    if let Some(cb) = self.on_connect_cb {
                        cb();
                    }
                }
            }
            "createStream" => {
                if self.state < ConnState::AppConnected {
                    return self.send_onstatus(
                        0,
                        "error",
                        "NetStream.Failed",
                        "connect required before createStream",
                    );
                }
                let txn = command::read_create_stream(&mut buf)?;
                if self.next_stream_id >= MAX_STREAMS_PER_CONN {
                    self.send_onstatus(0, "error", "NetStream.Failed", "Too many streams")?;
                } else {
                    // A fresh createStream replaces current_stream outright;
                    // if the stream it's replacing was actively publishing,
                    // that route is being abandoned and must be evicted now
                    // rather than left as a dangling cache-key tracking
                    // entry until this connection eventually disconnects.
                    self.evict_active_publish_route();
                    self.next_stream_id += 1;
                    let stream_id = self.next_stream_id;
                    self.current_stream = Some(Box::new(Stream::new(stream_id)));
                    let _ =
                        state_machine::conn_transition(&mut self.state, ConnState::StreamCreated);
                    self.send_create_stream_response(txn, stream_id)?;
                }
            }
            "publish" => {
                let mut stream_name = [0u8; 256];
                let mut publish_type = [0u8; 64];
                command::read_publish(&mut buf, &mut stream_name, &mut publish_type)?;
                let name_str = std::str::from_utf8(&stream_name)
                    .unwrap_or("")
                    .trim_end_matches('\0')
                    .to_string();
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
                        return self.send_onstatus(
                            0,
                            "error",
                            "NetStream.Publish.BadName",
                            "Publish not authorized",
                        );
                    }
                }
                // Use current_stream.is_publishing rather than self.state
                // here: conn_transition() only allows forward moves through
                // ConnState, so a play-then-publish sequence leaves
                // self.state stuck at Playing (Publishing < Playing) even
                // though this stream is genuinely publishing again. `play`
                // explicitly clears is_publishing when it takes over a
                // stream, so the flag accurately reflects whether this
                // specific current_stream is the one actively publishing.
                let was_publishing = self
                    .current_stream
                    .as_ref()
                    .map(|s| s.is_publishing)
                    .unwrap_or(false);
                let prev_route_key = self.relay_route_key();
                let next_route_key = if !self.relay_key.is_empty() {
                    self.relay_key.clone()
                } else {
                    name_str.clone()
                };
                if was_publishing && !prev_route_key.is_empty() && prev_route_key != next_route_key
                {
                    self.pending_cache_evictions
                        .push((self.app.clone(), prev_route_key));
                }
                if !self.defer_media_relay || self.on_publish_cb.is_none() {
                    self.relay_enabled = true;
                }
                {
                    if let Some(ref mut stream) = self.current_stream {
                        stream.is_publishing = true;
                        stream.name = name_str;
                    }
                    self.clear_detected_stream_metadata();
                    let _ = state_machine::conn_transition(&mut self.state, ConnState::Publishing);
                    let sid = self
                        .current_stream
                        .as_ref()
                        .map(|s| s.stream_id)
                        .unwrap_or(0);
                    self.send_onstatus(sid, "status", "NetStream.Publish.Start", "Publishing")?;
                }
            }
            "play" => {
                let mut stream_name = [0u8; 256];
                command::read_play(&mut buf, &mut stream_name)?;
                let name_str = std::str::from_utf8(&stream_name)
                    .unwrap_or("")
                    .trim_end_matches('\0')
                    .to_string();
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
                        return self.send_onstatus(
                            0,
                            "error",
                            "NetStream.Play.Failed",
                            "Play not authorized",
                        );
                    }
                }
                if !self.defer_media_relay || self.on_play_cb.is_none() {
                    self.relay_enabled = true;
                }
                {
                    // `play` supersedes any publish role this current_stream
                    // held: evict the abandoned publish route's cache key
                    // now (there's no "next" publish route to preserve it
                    // for), and clear is_publishing so it can't be
                    // mistaken for an active publish by a later command.
                    self.evict_active_publish_route();
                    if let Some(ref mut stream) = self.current_stream {
                        stream.is_playing = true;
                        stream.is_publishing = false;
                        stream.name = name_str;
                    }
                    self.needs_init_frames = true;
                    let _ = state_machine::conn_transition(&mut self.state, ConnState::Playing);
                    let sid = self
                        .current_stream
                        .as_ref()
                        .map(|s| s.stream_id)
                        .unwrap_or(0);
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
        // Negotiation complete: subsequent server chunks and client sends use
        // the announced size (connect AMF above was still at 128).
        self.activate_announced_chunk_size();
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

    pub fn send_create_stream_response(
        &mut self,
        transaction_id: f64,
        stream_id: u32,
    ) -> Result<()> {
        let mut amf_buf = Buffer::with_capacity(256);
        command::build_create_stream_result(&mut amf_buf, transaction_id, stream_id as f64)?;
        self.send_command(0, amf_buf.as_slice())
    }

    pub fn send_onstatus(
        &mut self,
        stream_id: u32,
        level: &str,
        code: &str,
        description: &str,
    ) -> Result<()> {
        let mut amf_buf = Buffer::with_capacity(512);
        command::build_onstatus(&mut amf_buf, level, code, description)?;
        self.send_command(stream_id, amf_buf.as_slice())
    }

    pub fn flush(&mut self) -> Result<()> {
        if self.client_fd < 0 || self.send_buffer.available() == 0 {
            return Ok(());
        }
        let Some(ref mut transport) = self.transport else {
            return Ok(());
        };
        while self.send_buffer.available() > 0 {
            let pending = self.send_buffer.peek();
            let n = transport.try_send(pending, &mut 0i32)?;
            if n == 0 {
                break;
            }
            self.send_buffer.drain(n);
        }
        Ok(())
    }

    pub fn send_frame(
        &mut self,
        frame_type: FrameType,
        timestamp: u32,
        payload: &[u8],
    ) -> Result<()> {
        let stream_id = self
            .current_stream
            .as_ref()
            .map(|s| s.stream_id)
            .unwrap_or(1);
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
            self.active_chunk_size as usize,
        )?;
        self.media_bytes_sent = self.media_bytes_sent.saturating_add(payload.len() as u64);
        Ok(())
    }

    fn send_control(&mut self, ty: u8, data: &[u8]) -> Result<()> {
        let mut msg = ChunkMessage::default();
        msg.csid = 2;
        msg.fmt = 0;
        msg.msg_length = data.len() as u32;
        msg.msg_type_id = ty;
        msg.msg_stream_id = 0;
        chunk_write(
            &mut self.send_buffer,
            &msg,
            data,
            data.len(),
            self.active_chunk_size as usize,
        )
    }

    fn send_command(&mut self, msg_stream_id: u32, amf_data: &[u8]) -> Result<()> {
        let mut cmd_msg = ChunkMessage::default();
        cmd_msg.csid = 3;
        cmd_msg.fmt = 0;
        cmd_msg.timestamp = 0;
        cmd_msg.msg_length = amf_data.len() as u32;
        cmd_msg.msg_type_id = 0x14;
        cmd_msg.msg_stream_id = msg_stream_id;
        chunk_write(
            &mut self.send_buffer,
            &cmd_msg,
            amf_data,
            amf_data.len(),
            self.active_chunk_size as usize,
        )
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
    fn default() -> Self {
        Self::new()
    }
}

fn positive_f64_to_u32(v: f64) -> Option<u32> {
    if !v.is_finite() || v < 0.0 || v > u32::MAX as f64 {
        return None;
    }
    Some(v as u32)
}

fn sane_framerate(v: f64) -> bool {
    v.is_finite() && v > 0.0 && v <= 1000.0
}

fn read_data_event_name(buf: &mut Buffer, is_string: bool, out: &mut [u8; 64]) -> Option<usize> {
    match if is_string {
        crate::amf::amf0::read_string(buf, out)
    } else {
        crate::amf::amf0::read_long_string(buf, out)
    } {
        Ok(n) => Some(n),
        Err(_) => None,
    }
}

fn detect_video_codec(payload: &[u8]) -> Option<String> {
    if payload.is_empty() {
        return None;
    }
    if payload[0] & 0x80 != 0 {
        if payload.len() >= 5 {
            if let Ok(s) = std::str::from_utf8(&payload[1..5]) {
                return Some(s.to_string());
            }
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
    if payload.is_empty() {
        return None;
    }
    if (payload[0] & 0xF0) == 0x90 && payload.len() >= 5 {
        if let Ok(s) = std::str::from_utf8(&payload[1..5]) {
            return Some(s.to_string());
        }
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
    fn connect_rejects_app_names_longer_than_routing_buffer() {
        let mut conn = Conn::new();
        let mut buf = Buffer::with_capacity(512);
        let long_app = "a".repeat(256);
        command::build_connect(
            &mut buf,
            &long_app,
            "rtmp://host/app",
            "",
            "",
            "FMLE/3.0",
            0,
            0,
        )
        .unwrap();
        assert_eq!(conn.handle_command(buf.as_slice()), Err(ErrorCode::Amf));
        assert!(conn.app.is_empty());
    }

    #[test]
    fn connect_after_app_connected_does_not_repoint_app_namespace() {
        let mut conn = Conn::new();

        let mut buf = Buffer::with_capacity(256);
        command::build_connect(
            &mut buf,
            "public",
            "rtmp://host/public",
            "",
            "",
            "FMLE/3.0",
            0,
            0,
        )
        .unwrap();
        conn.handle_command(buf.as_slice()).unwrap();
        assert_eq!(conn.app, "public");
        assert_eq!(conn.state, ConnState::AppConnected);

        // A second `connect` after the app namespace is already authorized
        // must be ignored -- it must not silently repoint `self.app` (and
        // therefore relay/cache routing) to a namespace that was never
        // passed through on_publish_cb/on_play_cb.
        let mut buf2 = Buffer::with_capacity(256);
        command::build_connect(
            &mut buf2,
            "private",
            "rtmp://host/private",
            "",
            "",
            "FMLE/3.0",
            0,
            0,
        )
        .unwrap();
        conn.handle_command(buf2.as_slice()).unwrap();
        assert_eq!(conn.app, "public");
        assert_eq!(conn.state, ConnState::AppConnected);
    }

    #[test]
    fn apply_chunk_size_updates_outbound_and_inbound() {
        let mut conn = Conn::new();
        conn.apply_chunk_size(4096);
        assert_eq!(conn.chunk_size, 4096);
        assert_eq!(conn.active_chunk_size, 4096);
        assert_eq!(conn.chunk_reg.default_chunk_size, 4096);
    }

    #[test]
    fn new_connection_starts_at_rtmp_default_chunk_size() {
        let conn = Conn::new();
        assert_eq!(conn.chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(conn.active_chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(conn.chunk_reg.default_chunk_size, DEFAULT_CHUNK_SIZE);
    }

    #[test]
    fn enhanced_av1_media_frames_accepted_while_publishing() {
        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        if let Some(s) = conn.current_stream.as_mut() {
            s.is_publishing = true;
        }

        let av1_seq = vec![0x90, b'a', b'v', b'0', b'1', 0x01, 0x02, 0x03];
        assert!(
            conn.handle_media_frame(1, FrameType::Video, 0, &av1_seq)
                .is_ok()
        );

        let aac_seq = vec![0xAF, 0x00, 0x12, 0x10];
        assert!(
            conn.handle_media_frame(1, FrameType::Audio, 0, &aac_seq)
                .is_ok()
        );

        let av1_frame = vec![0x91, b'a', b'v', b'0', b'1', 0xDE, 0xAD, 0xBE, 0xEF];
        assert!(
            conn.handle_media_frame(1, FrameType::Video, 40, &av1_frame)
                .is_ok()
        );
    }

    #[test]
    fn publish_rename_with_relay_key_does_not_evict_stale_rtmp_name() {
        let mut conn = Conn::new();
        conn.app = "live".to_string();
        conn.relay_key = "route-1".to_string();
        conn.current_stream = Some(Box::new(Stream::new(1)));

        let mut buf = Buffer::with_capacity(128);
        command::build_publish(&mut buf, "A", "live").unwrap();
        conn.handle_command(buf.as_slice()).unwrap();
        assert!(conn.pending_cache_evictions.is_empty());
        assert_eq!(conn.current_stream.as_ref().unwrap().name, "A");

        let mut buf = Buffer::with_capacity(128);
        command::build_publish(&mut buf, "B", "live").unwrap();
        conn.handle_command(buf.as_slice()).unwrap();

        // relay_route_key() is pinned to relay_key, so republishing under a new
        // RTMP stream name must NOT queue an eviction for the stale RTMP name
        // ("A") -- the real cache key ("route-1") never changed.
        assert!(conn.pending_cache_evictions.is_empty());
        assert_eq!(conn.current_stream.as_ref().unwrap().name, "B");
    }

    #[test]
    fn publish_rename_without_relay_key_evicts_old_route_key() {
        let mut conn = Conn::new();
        conn.app = "live".to_string();
        conn.current_stream = Some(Box::new(Stream::new(1)));

        let mut buf = Buffer::with_capacity(128);
        command::build_publish(&mut buf, "A", "live").unwrap();
        conn.handle_command(buf.as_slice()).unwrap();
        assert!(conn.pending_cache_evictions.is_empty());

        let mut buf = Buffer::with_capacity(128);
        command::build_publish(&mut buf, "B", "live").unwrap();
        conn.handle_command(buf.as_slice()).unwrap();

        assert_eq!(
            conn.pending_cache_evictions,
            vec![("live".to_string(), "A".to_string())]
        );
    }

    #[test]
    fn play_then_publish_does_not_evict_foreign_cache_key() {
        let mut conn = Conn::new();
        conn.app = "live".to_string();
        conn.current_stream = Some(Box::new(Stream::new(1)));

        let mut buf = Buffer::with_capacity(128);
        command::build_play(&mut buf, "victim").unwrap();
        conn.handle_command(buf.as_slice()).unwrap();
        assert_eq!(conn.current_stream.as_ref().unwrap().name, "victim");
        assert!(!conn.current_stream.as_ref().unwrap().is_publishing);

        let mut buf = Buffer::with_capacity(128);
        command::build_publish(&mut buf, "other", "live").unwrap();
        conn.handle_command(buf.as_slice()).unwrap();

        // This connection only ever played "victim" -- it never published it
        // -- so publishing "other" afterwards must not queue an eviction for
        // a cache key this connection never created.
        assert!(conn.pending_cache_evictions.is_empty());
        assert_eq!(conn.current_stream.as_ref().unwrap().name, "other");
    }

    #[test]
    fn publish_then_play_then_publish_does_not_evict_played_stream_key() {
        let mut conn = Conn::new();
        conn.app = "live".to_string();
        conn.current_stream = Some(Box::new(Stream::new(1)));

        let mut buf = Buffer::with_capacity(128);
        command::build_publish(&mut buf, "A", "live").unwrap();
        conn.handle_command(buf.as_slice()).unwrap();
        assert!(conn.pending_cache_evictions.is_empty());

        // `play` supersedes the active publish of "A" on this stream, so it
        // must evict "A" itself right away (there's no future publish
        // rename to hang that eviction off of, and is_publishing is cleared
        // so this stream can no longer be mistaken for an active publisher
        // of "A").
        let mut buf = Buffer::with_capacity(128);
        command::build_play(&mut buf, "victim").unwrap();
        conn.handle_command(buf.as_slice()).unwrap();
        assert_eq!(conn.current_stream.as_ref().unwrap().name, "victim");
        assert!(!conn.current_stream.as_ref().unwrap().is_publishing);
        assert_eq!(
            conn.pending_cache_evictions,
            vec![("live".to_string(), "A".to_string())]
        );
        conn.pending_cache_evictions.clear();

        let mut buf = Buffer::with_capacity(128);
        command::build_publish(&mut buf, "other", "live").unwrap();
        conn.handle_command(buf.as_slice()).unwrap();

        // This connection was never actively publishing "victim" -- it was
        // only playing it -- so publishing "other" must not queue a second
        // eviction for a cache key it never owned.
        assert!(conn.pending_cache_evictions.is_empty());
        assert_eq!(conn.current_stream.as_ref().unwrap().name, "other");
    }

    #[test]
    fn create_stream_evicts_active_publish_route() {
        let mut conn = Conn::new();
        conn.app = "live".to_string();
        conn.state = ConnState::AppConnected;
        conn.current_stream = Some(Box::new(Stream::new(1)));

        let mut buf = Buffer::with_capacity(128);
        command::build_publish(&mut buf, "A", "live").unwrap();
        conn.handle_command(buf.as_slice()).unwrap();
        assert!(conn.pending_cache_evictions.is_empty());
        assert!(conn.current_stream.as_ref().unwrap().is_publishing);

        // A fresh createStream replaces current_stream outright, abandoning
        // the active publish of "A" with no future rename to hang an
        // eviction off of -- it must be evicted immediately here, not left
        // as a dangling publisher_cache_keys tracking entry until this
        // connection eventually disconnects.
        let mut buf = Buffer::with_capacity(128);
        command::build_create_stream(&mut buf, 4.0).unwrap();
        conn.handle_command(buf.as_slice()).unwrap();

        assert_eq!(
            conn.pending_cache_evictions,
            vec![("live".to_string(), "A".to_string())]
        );
        assert!(!conn.current_stream.as_ref().unwrap().is_publishing);
        assert_eq!(conn.current_stream.as_ref().unwrap().name, "");
    }

    #[test]
    fn handle_control_peer_set_chunk_size_updates_inbound_only() {
        let mut conn = Conn::new();
        conn.chunk_size = 4096;
        conn.handle_control(
            msg_dispatch::RTMP_MSG_SET_CHUNK_SIZE,
            &8192u32.to_be_bytes(),
        )
        .unwrap();
        assert_eq!(conn.chunk_size, 4096);
        assert_eq!(conn.active_chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(conn.chunk_reg.default_chunk_size, 8192);
    }

    #[test]
    fn inbound_ping_requests_are_rate_limited() {
        let mut conn = Conn::new();
        conn.client_fd = 0;
        conn.transport = None;
        let mut ping = |token: u32| {
            let mut buf = Buffer::with_capacity(6);
            control::write_user_control_ping_request(&mut buf, token).unwrap();
            conn.handle_user_control(buf.as_slice())
        };
        for token in 0..8 {
            assert!(ping(token).is_ok(), "token {token} should be accepted");
        }
        assert!(
            matches!(ping(99), Err(ErrorCode::Protocol)),
            "9th ping in one second must be rejected"
        );
    }

    #[test]
    fn media_frames_on_wrong_msg_stream_id_are_ignored() {
        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        if let Some(s) = conn.current_stream.as_mut() {
            s.is_publishing = true;
        }

        let payload = vec![0x17, 0x01, 0x00, 0x00, 0x00];
        conn.handle_media_frame(99, FrameType::Video, 0, &payload)
            .unwrap();
        assert!(conn.pending_relay.is_empty());
    }

    #[test]
    fn read_messages_respects_recv_level_message_budget() {
        use crate::chunk::writer::chunk_write;

        let mut conn = Conn::new();
        conn.state = ConnState::Connected;

        let mut wire = Buffer::new();
        for i in 0..300usize {
            let payload = [i as u8];
            let msg = ChunkMessage {
                csid: 2,
                fmt: 0,
                timestamp: 0,
                msg_length: 1,
                msg_type_id: 0x03,
                msg_stream_id: 0,
                is_complete: false,
            };
            chunk_write(&mut wire, &msg, &payload, 1, 128).unwrap();
        }
        conn.recv_buffer = wire;

        let mut budget = MAX_MESSAGES_PER_RECV;
        let _ = conn.read_messages(&mut budget);
        assert_eq!(budget, 0);
        assert!(conn.recv_buffer.available() > 0);
    }

    #[test]
    fn recv_rejects_recv_buffer_growth_past_cap() {
        let mut conn = Conn::new();
        conn.recv_buffer
            .write(&vec![0u8; MAX_RECV_BUFFER_BYTES])
            .unwrap();
        assert!(matches!(conn.recv(&[1]), Err(ErrorCode::Protocol)));
    }

    #[test]
    fn on_frame_cb_scratch_retains_payload_after_delivery() {
        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        if let Some(s) = conn.current_stream.as_mut() {
            s.is_publishing = true;
        }
        conn.on_frame_cb = Some(|_| {});

        let payload = vec![0x17, 0x42];
        conn.handle_media_frame(1, FrameType::Video, 0, &payload)
            .unwrap();
        assert_eq!(conn.frame_cb_scratch.as_slice(), payload.as_slice());
    }

    fn flv_subtag(tag_type: u8, timestamp: u32, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(tag_type);
        let len = data.len() as u32;
        v.extend_from_slice(&[(len >> 16) as u8, (len >> 8) as u8, len as u8]);
        v.extend_from_slice(&[
            (timestamp >> 16) as u8,
            (timestamp >> 8) as u8,
            timestamp as u8,
            (timestamp >> 24) as u8,
        ]);
        v.extend_from_slice(&[0, 0, 0]); // stream id (always 0 on the wire)
        v.extend_from_slice(data);
        let prev_tag_size = (11 + data.len()) as u32;
        v.extend_from_slice(&prev_tag_size.to_be_bytes());
        v
    }

    #[test]
    fn aggregate_message_unpacks_audio_and_video_subtags_into_relay() {
        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        if let Some(s) = conn.current_stream.as_mut() {
            s.is_publishing = true;
        }

        let audio_payload = vec![0xAF, 0x01, 0x11, 0x22];
        let video_payload = vec![0x17, 0x01, 0x00, 0x00, 0x00, 0xAA];
        let mut aggregate = Vec::new();
        aggregate.extend(flv_subtag(0x08, 100, &audio_payload));
        aggregate.extend(flv_subtag(0x09, 140, &video_payload));

        conn.handle_aggregate(1, 1000, &aggregate).unwrap();

        assert_eq!(conn.pending_relay.len(), 2);
        assert_eq!(conn.pending_relay[0].frame_type, FrameType::Audio);
        assert_eq!(conn.pending_relay[0].timestamp, 1000);
        assert_eq!(conn.pending_relay[0].payload, audio_payload);
        assert_eq!(conn.pending_relay[1].frame_type, FrameType::Video);
        // Second sub-tag's ts (140) is 40 past the first sub-tag's ts (100),
        // so its relayed timestamp is the aggregate's own timestamp (1000) + 40.
        assert_eq!(conn.pending_relay[1].timestamp, 1040);
        assert_eq!(conn.pending_relay[1].payload, video_payload);
    }

    #[test]
    fn aggregate_message_rejects_subtag_size_overrunning_payload() {
        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        if let Some(s) = conn.current_stream.as_mut() {
            s.is_publishing = true;
        }

        // Claims a data_size far larger than the bytes actually present.
        let mut aggregate = vec![0x08, 0xFF, 0xFF, 0xFF];
        aggregate.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0]);
        aggregate.extend_from_slice(b"short");

        assert_eq!(
            conn.handle_aggregate(1, 0, &aggregate),
            Err(ErrorCode::Protocol)
        );
    }

    #[test]
    fn aggregate_dispatch_reaches_handle_aggregate_via_handle_message() {
        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        if let Some(s) = conn.current_stream.as_mut() {
            s.is_publishing = true;
        }

        let video_payload = vec![0x17, 0x01, 0x00, 0x00, 0x00, 0xBB];
        let aggregate = flv_subtag(0x09, 0, &video_payload);
        let msg = ChunkMessage {
            csid: 6,
            fmt: 0,
            timestamp: 500,
            msg_length: aggregate.len() as u32,
            msg_type_id: msg_dispatch::RTMP_MSG_AGGREGATE,
            msg_stream_id: 1,
            is_complete: true,
        };

        conn.handle_message(&msg, &aggregate).unwrap();

        assert_eq!(conn.pending_relay.len(), 1);
        assert_eq!(conn.pending_relay[0].frame_type, FrameType::Video);
        assert_eq!(conn.pending_relay[0].timestamp, 500);
        assert_eq!(conn.pending_relay[0].payload, video_payload);
    }

    fn amf0_string(s: &str) -> Vec<u8> {
        let mut buf = Buffer::with_capacity(64);
        crate::amf::amf0::write_string(&mut buf, s).unwrap();
        buf.as_slice().to_vec()
    }

    fn amf0_object_end() -> [u8; 3] {
        [0, 0, 0x09]
    }

    fn publishing_conn(stream_id: u32) -> Conn {
        let mut conn = Conn::new();
        conn.current_stream = Some(Box::new(Stream::new(stream_id)));
        if let Some(s) = conn.current_stream.as_mut() {
            s.is_publishing = true;
        }
        conn
    }

    fn build_on_metadata_payload(
        with_set_data_frame: bool,
        entries: &[(&str, f64)],
        extra: &[(&str, bool)],
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        if with_set_data_frame {
            payload.extend(amf0_string("@setDataFrame"));
        }
        payload.extend(amf0_string("onMetaData"));
        payload.push(crate::amf::amf0::Amf0Type::Object as u8);
        for (key, value) in entries {
            let mut buf = Buffer::with_capacity(32);
            crate::amf::amf0::write_object_key(&mut buf, key).unwrap();
            crate::amf::amf0::write_number(&mut buf, *value).unwrap();
            payload.extend_from_slice(buf.as_slice());
        }
        for (key, value) in extra {
            let mut buf = Buffer::with_capacity(32);
            crate::amf::amf0::write_object_key(&mut buf, key).unwrap();
            crate::amf::amf0::write_boolean(&mut buf, *value).unwrap();
            payload.extend_from_slice(buf.as_slice());
        }
        payload.extend_from_slice(&amf0_object_end());
        payload
    }

    #[test]
    fn on_metadata_populates_conn_fields() {
        let mut conn = Conn::new();
        let payload = build_on_metadata_payload(
            false,
            &[
                ("width", 1920.0),
                ("height", 1080.0),
                ("framerate", 30.0),
                ("audiosamplerate", 48000.0),
                ("audiochannels", 2.0),
            ],
            &[],
        );
        conn.handle_data_message(&payload).unwrap();
        assert_eq!(conn.detected_video_width, Some(1920));
        assert_eq!(conn.detected_video_height, Some(1080));
        assert_eq!(conn.detected_video_framerate, Some(30.0));
        assert_eq!(conn.detected_audio_sample_rate, Some(48000));
        assert_eq!(conn.detected_audio_channels, Some(2));
    }

    #[test]
    fn on_metadata_with_set_data_frame_prefix() {
        let mut conn = Conn::new();
        let payload = build_on_metadata_payload(
            true,
            &[
                ("width", 1280.0),
                ("height", 720.0),
                ("videoframerate", 60.0),
            ],
            &[("stereo", true)],
        );
        conn.handle_data_message(&payload).unwrap();
        assert_eq!(conn.detected_video_width, Some(1280));
        assert_eq!(conn.detected_video_height, Some(720));
        assert_eq!(conn.detected_video_framerate, Some(60.0));
        assert_eq!(conn.detected_audio_channels, Some(2));
        assert_eq!(conn.detected_audio_sample_rate, None);
    }

    #[test]
    fn on_metadata_missing_keys_leave_fields_none() {
        let mut conn = Conn::new();
        let payload = build_on_metadata_payload(false, &[("width", 640.0)], &[]);
        conn.handle_data_message(&payload).unwrap();
        assert_eq!(conn.detected_video_width, Some(640));
        assert_eq!(conn.detected_video_height, None);
        assert_eq!(conn.detected_video_framerate, None);
        assert_eq!(conn.detected_audio_sample_rate, None);
        assert_eq!(conn.detected_audio_channels, None);
    }

    #[test]
    fn on_metadata_skips_unknown_extra_keys() {
        let mut conn = Conn::new();
        let mut payload = amf0_string("onMetaData");
        payload.push(crate::amf::amf0::Amf0Type::Object as u8);
        let mut width = Buffer::with_capacity(32);
        crate::amf::amf0::write_object_key(&mut width, "width").unwrap();
        crate::amf::amf0::write_number(&mut width, 800.0).unwrap();
        payload.extend_from_slice(width.as_slice());
        let mut unknown = Buffer::with_capacity(64);
        crate::amf::amf0::write_object_key(&mut unknown, "customTag").unwrap();
        crate::amf::amf0::write_string(&mut unknown, "ignored").unwrap();
        payload.extend_from_slice(unknown.as_slice());
        payload.extend_from_slice(&amf0_object_end());

        conn.handle_data_message(&payload).unwrap();
        assert_eq!(conn.detected_video_width, Some(800));
    }

    #[test]
    fn on_metadata_rejects_out_of_range_dimensions() {
        let mut conn = Conn::new();
        let payload = build_on_metadata_payload(
            false,
            &[("width", -1.0), ("height", (u32::MAX as f64) + 1.0)],
            &[],
        );
        conn.handle_data_message(&payload).unwrap();
        assert_eq!(conn.detected_video_width, None);
        assert_eq!(conn.detected_video_height, None);
    }

    #[test]
    fn amf3_data_message_strips_leading_zero_byte() {
        let mut conn = publishing_conn(1);
        let mut body =
            build_on_metadata_payload(false, &[("width", 1024.0), ("height", 576.0)], &[]);
        let mut payload = vec![0x00];
        payload.append(&mut body);
        let msg = ChunkMessage {
            csid: 4,
            fmt: 0,
            timestamp: 0,
            msg_length: payload.len() as u32,
            msg_type_id: msg_dispatch::RTMP_MSG_AMF3_DATA,
            msg_stream_id: 1,
            is_complete: true,
        };
        conn.handle_message(&msg, &payload).unwrap();
        assert_eq!(conn.detected_video_width, Some(1024));
        assert_eq!(conn.detected_video_height, Some(576));
    }

    #[test]
    fn on_metadata_ignores_oversized_leading_event_name() {
        let mut conn = Conn::new();
        let long_name = "x".repeat(65);
        let mut payload = amf0_string(&long_name);
        payload.push(crate::amf::amf0::Amf0Type::Object as u8);
        payload.extend_from_slice(&amf0_object_end());

        conn.handle_data_message(&payload).unwrap();
        assert_eq!(conn.detected_video_width, None);
    }

    #[test]
    fn on_metadata_ignores_oversized_set_data_frame_inner_name() {
        let mut conn = Conn::new();
        let long_name = "y".repeat(65);
        let mut payload = amf0_string("@setDataFrame");
        payload.extend(amf0_string(&long_name));
        payload.push(crate::amf::amf0::Amf0Type::Object as u8);
        payload.extend_from_slice(&amf0_object_end());

        conn.handle_data_message(&payload).unwrap();
        assert_eq!(conn.detected_video_width, None);
    }

    #[test]
    fn on_metadata_keeps_partial_fields_when_unknown_value_cannot_be_skipped() {
        let mut conn = Conn::new();
        let mut payload = amf0_string("onMetaData");
        payload.push(crate::amf::amf0::Amf0Type::Object as u8);
        let mut width = Buffer::with_capacity(32);
        crate::amf::amf0::write_object_key(&mut width, "width").unwrap();
        crate::amf::amf0::write_number(&mut width, 1280.0).unwrap();
        payload.extend_from_slice(width.as_slice());
        let mut unknown = Buffer::with_capacity(8);
        crate::amf::amf0::write_object_key(&mut unknown, "vendor").unwrap();
        unknown
            .write(&[crate::amf::amf0::Amf0Type::Recordset as u8])
            .unwrap();
        payload.extend_from_slice(unknown.as_slice());
        payload.extend_from_slice(&amf0_object_end());

        conn.handle_data_message(&payload).unwrap();
        assert_eq!(conn.detected_video_width, Some(1280));
    }

    #[test]
    fn on_metadata_ignored_while_not_publishing() {
        let mut conn = Conn::new();
        let payload = build_on_metadata_payload(false, &[("width", 1920.0)], &[]);
        let msg = ChunkMessage {
            csid: 4,
            fmt: 0,
            timestamp: 0,
            msg_length: payload.len() as u32,
            msg_type_id: msg_dispatch::RTMP_MSG_AMF0_DATA,
            msg_stream_id: 1,
            is_complete: true,
        };
        conn.handle_message(&msg, &payload).unwrap();
        assert_eq!(conn.detected_video_width, None);
    }

    #[test]
    fn on_metadata_ignored_on_wrong_stream_id() {
        let mut conn = publishing_conn(1);
        let payload = build_on_metadata_payload(false, &[("width", 1920.0)], &[]);
        let msg = ChunkMessage {
            csid: 4,
            fmt: 0,
            timestamp: 0,
            msg_length: payload.len() as u32,
            msg_type_id: msg_dispatch::RTMP_MSG_AMF0_DATA,
            msg_stream_id: 99,
            is_complete: true,
        };
        conn.handle_message(&msg, &payload).unwrap();
        assert_eq!(conn.detected_video_width, None);
    }

    #[test]
    fn later_on_metadata_replaces_stale_values() {
        let mut conn = publishing_conn(1);
        let first = build_on_metadata_payload(false, &[("width", 1920.0), ("height", 1080.0)], &[]);
        let second = build_on_metadata_payload(false, &[("width", 1280.0), ("height", 720.0)], &[]);
        conn.handle_data_message(&first).unwrap();
        conn.handle_data_message(&second).unwrap();
        assert_eq!(conn.detected_video_width, Some(1280));
        assert_eq!(conn.detected_video_height, Some(720));
    }

    #[test]
    fn aggregate_script_subtag_populates_metadata() {
        let mut conn = publishing_conn(1);
        let metadata =
            build_on_metadata_payload(false, &[("width", 720.0), ("height", 480.0)], &[]);
        let aggregate = flv_subtag(msg_dispatch::RTMP_MSG_AMF0_DATA, 0, &metadata);
        conn.handle_aggregate(1, 0, &aggregate).unwrap();
        assert_eq!(conn.detected_video_width, Some(720));
        assert_eq!(conn.detected_video_height, Some(480));
    }

    #[test]
    fn on_metadata_tolerates_excess_keys() {
        let mut conn = publishing_conn(1);
        let mut payload = amf0_string("onMetaData");
        payload.push(crate::amf::amf0::Amf0Type::Object as u8);
        let mut width = Buffer::with_capacity(32);
        crate::amf::amf0::write_object_key(&mut width, "width").unwrap();
        crate::amf::amf0::write_number(&mut width, 800.0).unwrap();
        payload.extend_from_slice(width.as_slice());
        for i in 0..crate::amf::amf0::MAX_OBJECT_KEYS {
            let mut entry = Buffer::with_capacity(32);
            crate::amf::amf0::write_object_key(&mut entry, &format!("k{i}")).unwrap();
            crate::amf::amf0::write_null(&mut entry).unwrap();
            payload.extend_from_slice(entry.as_slice());
        }
        payload.extend_from_slice(&amf0_object_end());

        conn.handle_data_message(&payload).unwrap();
        assert_eq!(conn.detected_video_width, Some(800));
    }
}
