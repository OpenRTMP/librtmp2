use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::buffer::Buffer;
use crate::chunk::reader::{ChunkMessage, chunk_read_owned};
use crate::chunk::state::{
    ChunkRegistry, DEFAULT_CHUNK_SIZE, DEFAULT_MAX_MSG_LENGTH, RTMP_WIRE_MAX_MSG_LENGTH,
};
use crate::chunk::writer::chunk_write;
use crate::ertmp::connect_amf::{negotiate_caps, write_negotiated_caps};
use crate::ertmp::multitrack_media::{first_track_fourcc, foreach_track, is_multitrack_container};
use crate::handshake::{self, Handshake, HandshakeState};
use crate::media::{
    is_on_metadata_payload, normalize_modex_payload, parse_video_metadata_hdr, populate_av_frame,
    populate_multitrack_frame, ERTMP_PACKET_TYPE_MODEX,
};
use crate::message::command;
use crate::message::control::{
    self, UCTRL_PING_REQUEST, UCTRL_PING_RESPONSE, UCTRL_SET_BUFFER_LENGTH, UCTRL_STREAM_BEGIN,
    UCTRL_STREAM_EOF,
};
use crate::message::message as msg_dispatch;
use crate::message::shared_object::{self, SharedObjectMessage};
use crate::session::publish_route::PublishRouteRegistry;
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

#[derive(Debug, Clone)]
struct QueuedPing {
    token: u32,
    queued_at: Instant,
    /// Bytes that must drain from the front of `send_buffer` before this ping
    /// is considered fully on the wire (bytes queued ahead of the ping at
    /// queue time plus the ping itself, but not data appended afterward).
    bytes_until_flushed: usize,
}
/// Cap inbound Ping-Request reflections per connection to prevent trivial
/// outbound bandwidth/CPU amplification from unauthenticated peers.
const MAX_INBOUND_PING_RESPONSES: usize = 8;
/// Minimum wall-clock gap between init-frame replay requests caused by
/// switching play routes. This prevents a client from alternating stream
/// names to force cached headers and keyframes to be resent every poll batch.
const INIT_REPLAY_COOLDOWN: Duration = Duration::from_secs(1);
/// Close publishers that claimed a route but never sent media. Shorter than
/// [`RTMP_SESSION_SETUP_TIMEOUT`] so squatters cannot block legitimate
/// publishers for the full post-connect grace window.
const PUBLISH_MEDIA_REQUIRED_TIMEOUT: Duration = Duration::from_secs(2);
const INBOUND_PING_WINDOW: Duration = Duration::from_secs(1);
/// Close inbound sessions that never complete the AMF `connect` exchange or
/// never start publishing/playing. Matches the RTMPS accept deadline so a
/// peer cannot hold a connection slot indefinitely with a partial legacy
/// handshake, an idle post-handshake TCP session, or ping responses alone
/// after `connect`.
pub(crate) const RTMP_SESSION_SETUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Cap sub-tags unpacked from a single Aggregate message (mirrors
/// `message::message::MAX_AGGREGATE_SUBTAGS`).
const MAX_AGGREGATE_SUBTAGS: usize = 4096;

/// One media/script frame queued for local relay, stream-cache update, and
/// optional export to integrators.
///
/// Constructed by publishers (via the session media path), by
/// [`Conn::inject_relay_frame`], or by [`crate::server::Server::inject_relay_frame`].
/// Integrators that drain export buffers receive clones of these frames.
#[derive(Clone)]
pub struct RelayFrame {
    /// Audio, video, script (`onMetaData`), or metadata classification.
    pub frame_type: FrameType,
    /// RTMP message timestamp (milliseconds).
    pub timestamp: u32,
    /// Wire payload as received or injected (relayed to players unchanged).
    pub payload: Vec<u8>,
    /// ModEx-normalized bytes used only for codec parsing and cache classification.
    /// `None` means the normalized bytes are identical to `payload`.
    pub cache_payload: Option<Vec<u8>>,
    /// Application name used as the first half of the relay route key.
    pub app: String,
    /// Stream / relay-route key used to match publishers to players.
    pub stream_name: String,
    /// Owning publisher connection id. Socket-less injects use a high-bit
    /// external id ([`crate::server::is_external_publisher_id`]); the sentinel
    /// [`crate::server::EXTERNAL_RELAY_PUBLISHER_ID`] remains `u64::MAX`.
    pub publisher_conn_id: u64,
}

impl RelayFrame {
    /// Bytes used for codec parsing and cache classification.
    pub fn cache_payload(&self) -> &[u8] {
        self.cache_payload.as_deref().unwrap_or(&self.payload)
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.payload
            .len()
            .saturating_add(
                self.cache_payload
                    .as_ref()
                    .map(|payload| payload.len())
                    .unwrap_or(0),
            )
            .saturating_add(self.app.len())
            .saturating_add(self.stream_name.len())
    }
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
    pub bytes_received: u64,
    pub bytes_at_last_ack: u64,
    /// Audio/video payload bytes received (excludes handshake/control overhead).
    pub media_bytes_received: u64,
    /// Bytes accepted via [`Conn::inject_relay_frame`] (AV + script/metadata;
    /// socket telemetry stays in `media_bytes_received`). Counts trusted
    /// injects toward publisher liveness and deferred-relay drain so route
    /// squatters still time out and metadata is not stuck while relay is deferred.
    pub injected_media_bytes: u64,
    /// Audio/video payload bytes sent to this peer.
    pub media_bytes_sent: u64,
    /// Snapshot of `media_bytes_sent` consumed by the last pause-grace reset.
    /// A later pause may refresh setup grace only after additional relay bytes
    /// have been sent, so historical activity cannot keep a slot alive forever.
    pause_grace_media_bytes_sent: u64,
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
    /// Last play-route change that requested cached init frames.
    last_init_replay_request: Option<Instant>,
    pub detected_video_codec: Option<String>,
    pub detected_audio_codec: Option<String>,
    pub detected_video_width: Option<u32>,
    pub detected_video_height: Option<u32>,
    pub detected_video_framerate: Option<f64>,
    pub detected_audio_sample_rate: Option<u32>,
    pub detected_audio_channels: Option<u32>,
    /// HDR `colorInfo` parsed from the publisher's most recent enhanced video
    /// Metadata packet (`ERTMP_PACKET_TYPE_METADATA`), if any. Rust-only --
    /// not part of `Frame`'s ABI-stable `#[repr(C)]` layout (see
    /// `docs/abi-policy.md`).
    pub detected_hdr_info: Option<HdrInfo>,
    pub relay_enabled: bool,
    /// When true, media relay stays off until the integrator sets `relay_enabled`
    /// after its own post-auth bookkeeping (used by librtmp2-server).
    pub defer_media_relay: bool,
    /// Cap on queued relay payload bytes for this connection.
    pub max_pending_relay_bytes: usize,
    pub on_frame_cb: Option<fn(&Frame)>,
    /// When set, must return true before audio/video is queued for relay.
    /// For a multitrack (`ManyTracks`/`ManyTracksManyCodecs`) container, this
    /// is called once per track with that track's codec, not once per frame —
    /// any single denial rejects the whole frame.
    pub on_media_cb: Option<fn(u64, FrameType, Option<&str>) -> bool>,
    pub on_connect_cb: Option<fn()>,
    pub on_publish_cb: Option<fn(conn_id: u64, app: &str, stream_name: &str) -> bool>,
    pub on_play_cb: Option<fn(conn_id: u64, app: &str, stream_name: &str) -> bool>,
    /// Must return true before a parsed AMF3 Shared Object message is delivered
    /// to [`Self::on_shared_object_cb`]. When unset while `on_publish_cb` or
    /// `on_play_cb` is configured, inbound shared objects are dropped so a
    /// peer authorized only for publish/play cannot inject shared-object events.
    pub on_shared_object_auth_cb: Option<fn(conn_id: u64, so: &SharedObjectMessage) -> bool>,
    /// Fired for every parsed AMF3 Shared Object message (RTMP message type
    /// `0x10`) received on this connection. The library only delivers the
    /// parsed envelope -- multi-client attribute sync/persistence is left to
    /// the host application, consistent with this crate's "deliver the
    /// event, not the policy" design.
    pub on_shared_object_cb: Option<fn(conn_id: u64, so: &SharedObjectMessage)>,
    /// Must return true to allow `releaseStream` to force-evict *another*
    /// connection's publish-route claim for `stream_name` (e.g. reclaiming a
    /// route after a reconnecting encoder's old TCP session went stale but
    /// hasn't timed out yet). Unset (the default) makes `releaseStream` a
    /// no-op: unlike `on_media_cb`/`on_publish_cb`, this has no permissive
    /// default, since evicting a still-live publisher would otherwise let
    /// any authenticated peer hijack another connection's stream by name.
    pub on_release_stream_cb: Option<fn(conn_id: u64, app: &str, stream_name: &str) -> bool>,
    /// When set by the built-in server, enforces single-publisher-per-route.
    pub(crate) publish_routes: Option<PublishRouteRegistry>,
    /// Exact stream-name key currently held in `publish_routes` for this conn.
    /// Must be released on teardown even when `relay_key` later diverges from
    /// the RTMP publish name (librtmp2-server pins `relay_key` to the DB id).
    claimed_publish_route: Option<String>,
    /// Cache keys to evict after the publisher renames its stream.
    pub pending_cache_evictions: Vec<(String, String)>,
    /// Last measured client↔server RTT in milliseconds (RTMP UserControl ping).
    pub rtt_ms: f64,
    pending_pings: HashMap<u32, Instant>,
    /// Ping queued in `send_buffer` but not yet fully flushed.
    queued_ping: Option<QueuedPing>,
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
    /// Start of the current "must become active" grace window: set when
    /// this inbound TCP session was accepted, and reset every time the
    /// session goes idle after publishing/playing (`FCUnpublish`,
    /// `deleteStream`, `closeStream`) so a legitimate client that stops and
    /// later republishes on the same connection gets a fresh window instead
    /// of being reaped for the unpublished gap. Used by
    /// [`Conn::session_setup_timed_out`].
    session_setup_started: Instant,
    /// E-RTMP caps agreed during connect (empty when legacy connect).
    pub negotiated_caps: NegotiatedCaps,
    /// Player-side buffer length from SetBufferLength user control (ms).
    pub buffer_length_ms: u32,
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
            injected_media_bytes: 0,
            media_bytes_sent: 0,
            pause_grace_media_bytes_sent: 0,
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
            last_init_replay_request: None,
            detected_video_codec: None,
            detected_audio_codec: None,
            detected_video_width: None,
            detected_video_height: None,
            detected_video_framerate: None,
            detected_audio_sample_rate: None,
            detected_audio_channels: None,
            detected_hdr_info: None,
            relay_enabled: false,
            defer_media_relay: false,
            max_pending_relay_bytes: MAX_PENDING_RELAY_BYTES,
            on_frame_cb: None,
            on_media_cb: None,
            on_connect_cb: None,
            on_publish_cb: None,
            on_play_cb: None,
            on_shared_object_auth_cb: None,
            on_shared_object_cb: None,
            on_release_stream_cb: None,
            publish_routes: None,
            claimed_publish_route: None,
            pending_cache_evictions: Vec::new(),
            rtt_ms: 0.0,
            pending_pings: HashMap::new(),
            queued_ping: None,
            last_ping_sent: None,
            next_ping_token: 1,
            inbound_ping_responses: 0,
            inbound_ping_window_start: None,
            frame_cb_scratch: Vec::new(),
            budget_exhausted: false,
            session_setup_started: Instant::now(),
            negotiated_caps: NegotiatedCaps::default(),
            buffer_length_ms: 3000,
        }
    }

    pub fn has_buffered_messages(&self) -> bool {
        self.budget_exhausted
    }

    fn pending_relay_bytes(&self) -> usize {
        self.pending_relay.iter().map(RelayFrame::retained_bytes).sum()
    }

    pub fn relay_route_key(&self) -> String {
        if !self.relay_key.is_empty() {
            return self.relay_key.clone();
        }
        self.current_stream
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }

    pub fn accepts_multitrack(&self) -> bool {
        !self.negotiated_caps.has_caps_ex || self.negotiated_caps.multitrack_enabled
    }

    fn evict_active_publish_route(&mut self) {
        let was_publishing = self
            .current_stream
            .as_ref()
            .map(|s| s.is_publishing)
            .unwrap_or(false);
        if !was_publishing {
            let claimed_route = self.claimed_publish_route.clone();
            self.release_claimed_publish_route();
            if let Some(route_key) = claimed_route
                && !route_key.is_empty()
            {
                self.pending_cache_evictions
                    .push((self.app.clone(), route_key));
            }
            self.injected_media_bytes = 0;
            return;
        }
        let claimed_route = self.claimed_publish_route.clone();
        self.release_claimed_publish_route();
        let live_route_key = self.relay_route_key();
        let mut evicted = std::collections::HashSet::new();
        for route_key in [claimed_route, Some(live_route_key)].into_iter().flatten() {
            if !route_key.is_empty() && evicted.insert(route_key.clone()) {
                self.pending_cache_evictions
                    .push((self.app.clone(), route_key));
            }
        }
        self.clear_detected_stream_metadata();
        self.injected_media_bytes = 0;
    }

    fn release_claimed_publish_route(&mut self) {
        if let Some(claimed) = self.claimed_publish_route.take() {
            if let Some(routes) = self.publish_routes.as_ref() {
                routes.release(self.conn_id, &self.app, &claimed);
            }
        }
    }

    fn claim_publish_route(&mut self, stream: &str) -> bool {
        let Some(routes) = self.publish_routes.as_ref() else {
            self.claimed_publish_route = Some(stream.to_string());
            return true;
        };
        if !routes.claim(self.conn_id, &self.app, stream) {
            return false;
        }
        match self.claimed_publish_route.take() {
            Some(prev) if prev == stream => self.claimed_publish_route = Some(prev),
            Some(prev) => {
                routes.release(self.conn_id, &self.app, &prev);
                if !prev.is_empty() {
                    self.pending_cache_evictions.push((self.app.clone(), prev));
                }
                self.claimed_publish_route = Some(stream.to_string());
            }
            None => self.claimed_publish_route = Some(stream.to_string()),
        }
        true
    }

    fn clear_detected_stream_metadata(&mut self) {
        self.detected_video_codec = None;
        self.detected_audio_codec = None;
        self.detected_video_width = None;
        self.detected_video_height = None;
        self.detected_video_framerate = None;
        self.detected_audio_sample_rate = None;
        self.detected_audio_channels = None;
        self.detected_hdr_info = None;
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

    fn handle_publisher_data_message(
        &mut self,
        msg_stream_id: u32,
        timestamp: u32,
        payload: &[u8],
    ) -> Result<()> {
        if !self.publishing_metadata_allowed(msg_stream_id) {
            return Ok(());
        }
        let relay_metadata = is_on_metadata_payload(payload)
            && self.relay_enabled
            && self
                .current_stream
                .as_ref()
                .map(|s| s.is_publishing)
                .unwrap_or(false);
        if relay_metadata && !self.media_allowed(FrameType::Script, None) {
            return Err(ErrorCode::Auth);
        }
        self.handle_data_message(payload)?;
        if relay_metadata {
            if let Some(cb) = self.on_frame_cb {
                self.frame_cb_scratch.clear();
                self.frame_cb_scratch.extend_from_slice(payload);
                let frame = Frame {
                    frame_type: FrameType::Script,
                    timestamp,
                    size: self.frame_cb_scratch.len() as u32,
                    data: self.frame_cb_scratch.as_ptr(),
                    is_metadata: 1,
                    ..Default::default()
                };
                cb(&frame);
            }
            self.queue_relay_frame(FrameType::Script, timestamp, payload, payload)?;
        }
        Ok(())
    }

    pub fn inject_relay_frame(
        &mut self,
        frame_type: FrameType,
        timestamp: u32,
        payload: &[u8],
    ) -> Result<()> {
        let max_len = self.chunk_reg.max_msg_length.min(RTMP_WIRE_MAX_MSG_LENGTH) as usize;
        if payload.len() > max_len {
            return Err(ErrorCode::Internal);
        }
        if let Some(stream) = self.current_stream.as_ref() {
            if stream.is_playing && !stream.is_publishing {
                return Err(ErrorCode::Internal);
            }
        }
        let stream_name = self.relay_route_key();
        if self.app.is_empty() || stream_name.is_empty() {
            return Err(ErrorCode::Internal);
        }
        if self.app.len() > 1024 || stream_name.len() > 1024 {
            return Err(ErrorCode::Internal);
        }
        let previous_claim = self.claimed_publish_route.clone();
        let eviction_len_before = self.pending_cache_evictions.len();
        let already_owned_route =
            self.claimed_publish_route.as_deref() == Some(stream_name.as_str());
        if !self.claim_publish_route(&stream_name) {
            if let Some(ref claimed) = self.claimed_publish_route {
                self.relay_key = claimed.clone();
            }
            return Err(ErrorCode::Internal);
        }
        let normalized = normalize_modex_payload(payload, self.negotiated_caps.caps_ex_mask);
        if let Err(e) = self.queue_relay_frame(frame_type, timestamp, payload, normalized.as_ref()) {
            if !already_owned_route {
                self.release_claimed_publish_route();
                self.pending_cache_evictions.truncate(eviction_len_before);
                if let Some(prev) = previous_claim {
                    if let Some(routes) = self.publish_routes.as_ref() {
                        let _ = routes.claim(self.conn_id, &self.app, &prev);
                    }
                    self.claimed_publish_route = Some(prev.clone());
                    self.relay_key = prev;
                }
            }
            return Err(e);
        }
        self.injected_media_bytes = self
            .injected_media_bytes
            .saturating_add((payload.len() as u64).max(1));
        Ok(())
    }

    fn queue_relay_frame(
        &mut self,
        frame_type: FrameType,
        timestamp: u32,
        payload: &[u8],
        cache_payload: &[u8],
    ) -> Result<()> {
        let max_len = self.chunk_reg.max_msg_length.min(RTMP_WIRE_MAX_MSG_LENGTH) as usize;
        if payload.len() > max_len {
            return Err(ErrorCode::Internal);
        }
        let cache_payload = if cache_payload.len() == payload.len()
            && std::ptr::eq(cache_payload.as_ptr(), payload.as_ptr())
        {
            None
        } else {
            Some(cache_payload.to_vec())
        };
        let app = self.app.clone();
        let stream_name = self.relay_route_key();
        let retained_bytes = payload
            .len()
            .saturating_add(
                cache_payload
                    .as_ref()
                    .map(|payload| payload.len())
                    .unwrap_or(0),
            )
            .saturating_add(app.len())
            .saturating_add(stream_name.len());
        if self.pending_relay.len() >= MAX_PENDING_RELAY_FRAMES
            || self.pending_relay_bytes().saturating_add(retained_bytes)
                > self.max_pending_relay_bytes
        {
            return Err(ErrorCode::Internal);
        }
        self.pending_relay.push(RelayFrame {
            frame_type,
            timestamp,
            payload: payload.to_vec(),
            cache_payload,
            app,
            stream_name,
            publisher_conn_id: self.conn_id,
        });
        Ok(())
    }

    fn media_allowed(&self, frame_type: FrameType, codec: Option<&str>) -> bool {
        let Some(cb) = self.on_media_cb else {
            return true;
        };
        if matches!(frame_type, FrameType::Video | FrameType::Audio) && codec.is_none() {
            return false;
        }
        cb(self.conn_id, frame_type, codec)
    }

    fn request_init_replay(&mut self) {
        let now = Instant::now();
        if self
            .last_init_replay_request
            .is_some_and(|last| now.duration_since(last) < INIT_REPLAY_COOLDOWN)
        {
            return;
        }
        self.last_init_replay_request = Some(now);
        self.needs_init_frames = true;
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

        // Authorization must inspect ModEx even if the peer omitted capsEx;
        // otherwise wrapper metadata can impersonate an allowed outer codec.
        let auth_payload = normalize_modex_payload(payload, CAPS_EX_MASK_MODEX);
        let auth_parse_payload = auth_payload.as_ref();

        // Keep forced normalization for video, where the enhanced header is
        // unambiguous. For audio, legacy headers such as G.711 mu-law 0x87
        // collide with the ModEx marker, so callbacks/cache may only peel
        // wrappers when ModEx was actually negotiated.
        let downstream_caps = if frame_type == FrameType::Audio {
            self.negotiated_caps.caps_ex_mask
        } else {
            CAPS_EX_MASK_MODEX
        };
        let normalized_payload = normalize_modex_payload(payload, downstream_caps);
        let parse_payload = normalized_payload.as_ref();

        let current_codec = match frame_type {
            FrameType::Video => detect_video_codec(auth_parse_payload),
            FrameType::Audio => detect_audio_codec(auth_parse_payload),
            _ => None,
        };

        let auth_is_multitrack = is_multitrack_container(frame_type, auth_parse_payload);
        let is_multitrack = is_multitrack_container(frame_type, parse_payload);

        if auth_is_multitrack {
            let mut auth_denied = false;
            let tracks_valid = foreach_track(frame_type, auth_parse_payload, |track| {
                if auth_denied {
                    return;
                }
                let track_codec = media_fourcc_auth_label(&track.fourcc);
                if !self.media_allowed(frame_type, track_codec.as_deref()) {
                    auth_denied = true;
                }
            });
            if !tracks_valid {
                return Err(ErrorCode::Protocol);
            }
            if auth_denied {
                return Err(ErrorCode::Auth);
            }
        } else if !self.media_allowed(frame_type, current_codec.as_deref()) {
            return Err(ErrorCode::Auth);
        }

        match frame_type {
            FrameType::Video if self.detected_video_codec.is_none() => {
                self.detected_video_codec = current_codec.clone();
            }
            FrameType::Audio if self.detected_audio_codec.is_none() => {
                self.detected_audio_codec = current_codec.clone();
            }
            _ => {}
        }

        if frame_type == FrameType::Video && !is_multitrack {
            if let Some(color_info) = parse_video_metadata_hdr(parse_payload) {
                self.detected_hdr_info = Some(color_info);
            }
        }

        self.media_bytes_received = self
            .media_bytes_received
            .saturating_add(payload.len() as u64);

        let cb = self.on_frame_cb;
        let parsed_multitrack = foreach_track(frame_type, parse_payload, |track| {
            if let Some(cb) = cb {
                self.invoke_multitrack_on_frame_cb(
                    cb,
                    frame_type,
                    timestamp,
                    track.track_id,
                    track.fourcc,
                    track.packet_type,
                    track.video_frame_type,
                    track.payload,
                );
            }
        });
        if is_multitrack && !parsed_multitrack {
            return Err(ErrorCode::Protocol);
        }
        if !is_multitrack {
            if let Some(cb) = cb {
                self.invoke_on_frame_cb(cb, frame_type, timestamp, u8::MAX, parse_payload);
            }
        }
        if self
            .queue_relay_frame(frame_type, timestamp, payload, parse_payload)
            .is_err()
        {
            return Err(ErrorCode::Internal);
        }
        Ok(())
    }

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
                    self.handle_publisher_data_message(msg_stream_id, out_ts, tag_payload)?;
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
        self.recv_buffer.write(data).map_err(|_| ErrorCode::Internal)?;
        self.bytes_received = self.bytes_received.saturating_add(data.len() as u64);
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
            && self.bytes_received.saturating_sub(self.bytes_at_last_ack)
                >= self.window_ack_size as u64
        {
            self.send_acknowledgement(self.bytes_received as u32)?;
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
            msg_dispatch::RTMP_MSG_AUDIO => self.handle_media_frame(
                msg.msg_stream_id,
                FrameType::Audio,
                msg.timestamp,
                payload,
            ),
            msg_dispatch::RTMP_MSG_VIDEO => self.handle_media_frame(
                msg.msg_stream_id,
                FrameType::Video,
                msg.timestamp,
                payload,
            ),
            msg_dispatch::RTMP_MSG_AGGREGATE => {
                self.handle_aggregate(msg.msg_stream_id, msg.timestamp, payload)
            }
            msg_dispatch::RTMP_MSG_AMF0_DATA => {
                self.handle_publisher_data_message(msg.msg_stream_id, msg.timestamp, payload)
            }
            msg_dispatch::RTMP_MSG_AMF3_DATA => {
                if !payload.is_empty() && payload[0] == 0x00 {
                    self.handle_publisher_data_message(
                        msg.msg_stream_id,
                        msg.timestamp,
                        &payload[1..],
                    )
                } else {
                    self.handle_publisher_data_message(msg.msg_stream_id, msg.timestamp, payload)
                }
            }
            msg_dispatch::RTMP_MSG_AMF3_SHARED_OBJECT => {
                let data = if !payload.is_empty() && payload[0] == 0x00 {
                    &payload[1..]
                } else {
                    payload
                };
                self.handle_amf3_shared_object(data)
            }
            _ => Ok(()),
        }
    }

    fn handle_amf3_shared_object(&mut self, payload: &[u8]) -> Result<()> {
        if self.state < ConnState::AppConnected {
            return Ok(());
        }
        let Ok(so) = shared_object::parse(payload) else {
            return Ok(());
        };
        if self.on_shared_object_cb.is_none() {
            return Ok(());
        }
        match self.on_shared_object_auth_cb {
            Some(auth_cb) if !auth_cb(self.conn_id, &so) => return Ok(()),
            None if self.on_publish_cb.is_some() || self.on_play_cb.is_some() => return Ok(()),
            _ => {}
        }
        if let Some(cb) = self.on_shared_object_cb {
            cb(self.conn_id, &so);
        }
        Ok(())
    }

    pub fn send_shared_object(&mut self, so: &SharedObjectMessage) -> Result<()> {
        let mut amf_buf = Buffer::with_capacity(256);
        shared_object::write(so, &mut amf_buf)?;
        let mut wire = Vec::with_capacity(amf_buf.available() + 1);
        wire.push(0x00);
        wire.extend_from_slice(amf_buf.as_slice());

        let mut cmsg = ChunkMessage::default();
        cmsg.msg_length = wire.len() as u32;
        cmsg.msg_stream_id = 0;
        cmsg.fmt = 0;
        cmsg.csid = 3;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_AMF3_SHARED_OBJECT;
        chunk_write(
            &mut self.send_buffer,
            &cmsg,
            &wire,
            wire.len(),
            self.active_chunk_size as usize,
        )
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
        let event_type = ((payload[0] as u16) << 8) | (payload[1] as u16);
        let (event_type, param1, param2) = if event_type == UCTRL_SET_BUFFER_LENGTH {
            control::read_user_control(payload, true)?
        } else {
            let (ty, p1, _) = control::read_user_control(payload, false)?;
            (ty, p1, None)
        };
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
            UCTRL_STREAM_BEGIN => {
                let _ = param1;
            }
            UCTRL_STREAM_EOF => {
                let _ = param1;
            }
            UCTRL_SET_BUFFER_LENGTH => {
                if let Some(ms) = param2 {
                    self.buffer_length_ms = ms;
                }
            }
            _ => {}
        }
        Ok(())
    }

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
        if let Some(queued) = &self.queued_ping {
            if now.duration_since(queued.queued_at) >= PING_TIMEOUT {
                return Err(ErrorCode::Protocol);
            }
            return Ok(());
        }

        let had_stale_ping = self
            .pending_pings
            .values()
            .any(|sent| now.duration_since(*sent) >= PING_TIMEOUT);
        self.pending_pings
            .retain(|_, sent| now.duration_since(*sent) < PING_TIMEOUT);
        if had_stale_ping {
            return Err(ErrorCode::Protocol);
        }
        if self.pending_pings.len() + usize::from(self.queued_ping.is_some()) >= MAX_PENDING_PINGS {
            return Err(ErrorCode::Protocol);
        }

        let token = self.next_ping_token;
        self.next_ping_token = self.next_ping_token.wrapping_add(1);
        self.send_user_control_ping_request(token)?;
        self.queued_ping = Some(QueuedPing {
            token,
            queued_at: now,
            bytes_until_flushed: self.send_buffer.available(),
        });
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
                if self.state >= ConnState::AppConnected {
                    return Ok(());
                }
                let mut info = ConnectInfo::default();
                if command::read_connect(&mut buf, &mut info).is_err() {
                    self.send_command_error(
                        info.transaction_id,
                        "NetConnection.Connect.Rejected",
                        "Invalid connect command or capability negotiation.",
                    )?;
                    return Ok(());
                }
                self.app = match command::decode_route_amf_string(&info.app) {
                    Ok(app) => app,
                    Err(_) => {
                        self.send_command_error(
                            info.transaction_id,
                            "NetConnection.Connect.Rejected",
                            "Invalid connect app name.",
                        )?;
                        return Ok(());
                    }
                };
                if self.app.is_empty() {
                    self.send_command_error(
                        info.transaction_id,
                        "NetConnection.Connect.Rejected",
                        "Empty connect app name.",
                    )?;
                    return Ok(());
                }
                let needs_caps = info.has_four_cc_list
                    || info.has_caps_ex
                    || info.has_video_four_cc_info_map
                    || info.has_reconnect;
                if needs_caps {
                    let _ = state_machine::conn_transition(&mut self.state, ConnState::CapsNegotiated);
                }
                let negotiated = if needs_caps {
                    let caps = negotiate_caps(&info);
                    self.negotiated_caps = caps.clone();
                    Some(caps)
                } else {
                    None
                };
                let _ = state_machine::conn_transition(&mut self.state, ConnState::AppConnected);
                self.send_connect_response(info.transaction_id, negotiated.as_ref())?;
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
                    self.evict_active_publish_route();
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
                let name_str = match command::decode_route_amf_string(&stream_name) {
                    Ok(name) => name,
                    Err(_) => {
                        return self.send_onstatus(
                            0,
                            "error",
                            "NetStream.Publish.BadName",
                            "Invalid stream name",
                        );
                    }
                };
                if name_str.is_empty() {
                    return self.send_onstatus(
                        0,
                        "error",
                        "NetStream.Publish.BadName",
                        "Empty stream name",
                    );
                }
                if self.current_stream.is_none() {
                    return self.send_onstatus(
                        0,
                        "error",
                        "NetStream.Publish.BadConnection",
                        "No stream created",
                    );
                }
                if self.on_publish_cb.is_none() && self.on_play_cb.is_some() {
                    return self.send_onstatus(
                        0,
                        "error",
                        "NetStream.Publish.BadName",
                        "Publish not authorized",
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
                let renaming_route = was_publishing
                    && !prev_route_key.is_empty()
                    && prev_route_key != next_route_key;
                if !self.claim_publish_route(&next_route_key) {
                    return self.send_onstatus(
                        0,
                        "error",
                        "NetStream.Publish.BadName",
                        "Route already publishing",
                    );
                }
                if !was_publishing || renaming_route {
                    self.session_setup_started = Instant::now();
                    self.injected_media_bytes = 0;
                }
                if renaming_route {
                    self.pending_cache_evictions
                        .push((self.app.clone(), prev_route_key));
                }
                if !self.defer_media_relay {
                    self.relay_enabled = true;
                }
                {
                    if let Some(ref mut stream) = self.current_stream {
                        stream.is_publishing = true;
                        stream.is_playing = false;
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
                let name_str = match command::decode_route_amf_string(&stream_name) {
                    Ok(name) => name,
                    Err(_) => {
                        return self.send_onstatus(
                            0,
                            "error",
                            "NetStream.Play.Failed",
                            "Invalid stream name",
                        );
                    }
                };
                if name_str.is_empty() {
                    return self.send_onstatus(
                        0,
                        "error",
                        "NetStream.Play.Failed",
                        "Empty stream name",
                    );
                }
                if self.current_stream.is_none() {
                    return self.send_onstatus(
                        0,
                        "error",
                        "NetStream.Play.BadConnection",
                        "No stream created",
                    );
                }
                if self.on_play_cb.is_none() && self.on_publish_cb.is_some() {
                    return self.send_onstatus(
                        0,
                        "error",
                        "NetStream.Play.Failed",
                        "Play not authorized",
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
                if !self.defer_media_relay {
                    self.relay_enabled = true;
                }
                {
                    self.evict_active_publish_route();
                    let already_playing_same = self
                        .current_stream
                        .as_ref()
                        .map(|s| s.is_playing && s.name == name_str)
                        .unwrap_or(false);
                    if let Some(ref mut stream) = self.current_stream {
                        stream.is_playing = true;
                        stream.is_publishing = false;
                        stream.name = name_str;
                    }
                    if !already_playing_same {
                        self.pause_grace_media_bytes_sent = self.media_bytes_sent;
                        self.request_init_replay();
                    }
                    let _ = state_machine::conn_transition(&mut self.state, ConnState::Playing);
                    let sid = self
                        .current_stream
                        .as_ref()
                        .map(|s| s.stream_id)
                        .unwrap_or(0);
                    self.send_onstatus(sid, "status", "NetStream.Play.Start", "Playing")?;
                    self.send_stream_lifecycle_begin(sid)?;
                }
            }
            "FCUnpublish" | "deleteStream" => {
                let was_active = self
                    .current_stream
                    .as_ref()
                    .is_some_and(|s| s.is_publishing || s.is_playing);
                self.evict_active_publish_route();
                if let Some(ref mut stream) = self.current_stream {
                    stream.is_publishing = false;
                    stream.is_playing = false;
                    stream.paused = false;
                }
                if was_active {
                    self.session_setup_started = Instant::now();
                }
                self.relay_enabled = false;
                if let Some(sid) = self.current_stream.as_ref().map(|s| s.stream_id) {
                    let _ = self.send_stream_lifecycle_eof(sid);
                }
            }
            "FCPublish" => {}
            "releaseStream" => {
                let mut stream_name = [0u8; 256];
                if command::read_release_stream(&mut buf, &mut stream_name).is_ok() {
                    if let Ok(name_str) = command::decode_route_amf_string(&stream_name) {
                        if !name_str.is_empty() {
                            let authorized = self
                                .on_release_stream_cb
                                .is_some_and(|cb| cb(self.conn_id, &self.app, &name_str));
                            if authorized {
                                if let Some(ref routes) = self.publish_routes {
                                    routes.force_release(&self.app, &name_str);
                                }
                            }
                        }
                    }
                }
            }
            "pause" => {
                if let Ok(pause_flag) = command::read_pause(&mut buf) {
                    if let Some(ref mut stream) = self.current_stream {
                        if pause_flag && stream.is_playing && !stream.paused {
                            if self.media_bytes_sent > self.pause_grace_media_bytes_sent {
                                self.session_setup_started = Instant::now();
                                self.pause_grace_media_bytes_sent = self.media_bytes_sent;
                            }
                        }
                        stream.paused = pause_flag;
                    }
                    let sid = self
                        .current_stream
                        .as_ref()
                        .map(|s| s.stream_id)
                        .unwrap_or(0);
                    let (code, desc) = if pause_flag {
                        ("NetStream.Pause.Notify", "Paused")
                    } else {
                        ("NetStream.Unpause.Notify", "Unpaused")
                    };
                    self.send_onstatus(sid, "status", code, desc)?;
                }
            }
            "seek" => {
                let _millis = command::read_seek(&mut buf).unwrap_or(0.0);
                let sid = self
                    .current_stream
                    .as_ref()
                    .map(|s| s.stream_id)
                    .unwrap_or(0);
                self.send_onstatus(sid, "status", "NetStream.Seek.Notify", "Seeking")?;
            }
            "receiveAudio" => {
                if let Ok(flag) = command::read_bool_command(&mut buf) {
                    if let Some(ref mut stream) = self.current_stream {
                        stream.receive_audio = flag;
                    }
                }
            }
            "receiveVideo" => {
                if let Ok(flag) = command::read_bool_command(&mut buf) {
                    if let Some(ref mut stream) = self.current_stream {
                        stream.receive_video = flag;
                    }
                }
            }
            "closeStream" => {
                let target_id = command::read_close_stream(&mut buf)
                    .ok()
                    .flatten()
                    .or_else(|| self.current_stream.as_ref().map(|s| s.stream_id))
                    .unwrap_or(0);
                if self.current_stream.as_ref().map(|s| s.stream_id) == Some(target_id) {
                    let was_active = self
                        .current_stream
                        .as_ref()
                        .is_some_and(|s| s.is_publishing || s.is_playing);
                    self.evict_active_publish_route();
                    if let Some(ref mut stream) = self.current_stream {
                        stream.is_playing = false;
                        stream.is_publishing = false;
                        stream.paused = false;
                    }
                    if was_active {
                        self.session_setup_started = Instant::now();
                    }
                    self.relay_enabled = false;
                    let _ = self.send_stream_lifecycle_eof(target_id);
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn send_connect_response(
        &mut self,
        transaction_id: f64,
        caps: Option<&NegotiatedCaps>,
    ) -> Result<()> {
        let win = SERVER_WINDOW_ACK_SIZE.to_be_bytes();
        self.send_control(0x05, &win)?;
        let mut bw = [0u8; 5];
        let bw_val = SERVER_PEER_BANDWIDTH.to_be_bytes();
        bw[..4].copy_from_slice(&bw_val);
        bw[4] = PEER_BANDWIDTH_DYNAMIC;
        self.send_control(0x06, &bw)?;
        let cs = self.chunk_size.to_be_bytes();
        self.send_control(0x01, &cs)?;
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
        if let Some(caps) = caps {
            write_negotiated_caps(&mut amf_buf, caps)?;
        }
        crate::amf::amf0::write_object_end(&mut amf_buf)?;
        self.send_command(0, amf_buf.as_slice())
    }

    pub fn send_command_error(
        &mut self,
        transaction_id: f64,
        code: &str,
        description: &str,
    ) -> Result<()> {
        let mut amf_buf = Buffer::with_capacity(256);
        command::build_error(&mut amf_buf, transaction_id, code, description)?;
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

    pub fn send_reconnect_request(
        &mut self,
        tc_url: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        let mut amf_buf = Buffer::with_capacity(512);
        command::build_reconnect_request(&mut amf_buf, tc_url, description)?;
        self.send_command(0, amf_buf.as_slice())
    }

    pub fn flush(&mut self) -> Result<()> {
        if self.client_fd < 0 || self.send_buffer.available() == 0 {
            self.commit_flushed_ping();
            return Ok(());
        }
        let Some(ref mut transport) = self.transport else {
            self.commit_flushed_ping();
            return Ok(());
        };
        while self.send_buffer.available() > 0 {
            let pending = self.send_buffer.peek();
            let n = transport.try_send(pending, &mut 0i32)?;
            if n == 0 {
                break;
            }
            self.send_buffer.drain(n);
            if let Some(ref mut queued) = self.queued_ping {
                queued.bytes_until_flushed = queued.bytes_until_flushed.saturating_sub(n);
            }
        }
        self.commit_flushed_ping();
        Ok(())
    }

    pub fn disconnect_transport(&mut self) {
        self.transport = None;
        self.client_fd = -1;
    }

    fn commit_flushed_ping(&mut self) {
        let Some(queued) = self.queued_ping.as_ref() else {
            return;
        };
        if queued.bytes_until_flushed > 0 {
            return;
        }
        let now = Instant::now();
        self.pending_pings.insert(queued.token, now);
        self.last_ping_sent = Some(now);
        self.queued_ping = None;
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

    pub fn send_data_message(&mut self, timestamp: u32, payload: &[u8]) -> Result<()> {
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
        cmsg.csid = 5;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_AMF0_DATA;
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

    fn send_stream_lifecycle_begin(&mut self, stream_id: u32) -> Result<()> {
        let mut buf = Buffer::with_capacity(14);
        control::write_user_control_stream_begin(&mut buf, stream_id)?;
        self.send_control(msg_dispatch::RTMP_MSG_USER_CONTROL, buf.as_slice())?;
        buf.reset();
        control::write_user_control_set_buffer_length(&mut buf, stream_id, self.buffer_length_ms)?;
        self.send_control(msg_dispatch::RTMP_MSG_USER_CONTROL, buf.as_slice())
    }

    fn send_stream_lifecycle_eof(&mut self, stream_id: u32) -> Result<()> {
        let mut buf = Buffer::with_capacity(6);
        control::write_user_control_stream_eof(&mut buf, stream_id)?;
        self.send_control(msg_dispatch::RTMP_MSG_USER_CONTROL, buf.as_slice())
    }

    fn invoke_multitrack_on_frame_cb(
        &mut self,
        cb: fn(&Frame),
        frame_type: FrameType,
        timestamp: u32,
        track_id: u8,
        fourcc: [u8; 4],
        packet_type: u8,
        video_frame_type: u8,
        payload: &[u8],
    ) {
        self.frame_cb_scratch.clear();
        self.frame_cb_scratch.extend_from_slice(payload);
        let mut frame = Frame {
            frame_type,
            timestamp,
            size: self.frame_cb_scratch.len() as u32,
            data: self.frame_cb_scratch.as_ptr(),
            track_id,
            ..Default::default()
        };
        populate_multitrack_frame(&mut frame, fourcc, packet_type, video_frame_type);
        cb(&frame);
    }

    fn invoke_on_frame_cb(
        &mut self,
        cb: fn(&Frame),
        frame_type: FrameType,
        timestamp: u32,
        track_id: u8,
        payload: &[u8],
    ) {
        self.frame_cb_scratch.clear();
        self.frame_cb_scratch.extend_from_slice(payload);
        let mut frame = Frame {
            frame_type,
            timestamp,
            size: self.frame_cb_scratch.len() as u32,
            data: self.frame_cb_scratch.as_ptr(),
            track_id,
            ..Default::default()
        };
        populate_av_frame(&mut frame, &self.frame_cb_scratch);
        cb(&frame);
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

fn is_wildcard_media_fourcc(fourcc: &[u8; 4]) -> bool {
    fourcc[0] == b'*'
}

fn media_fourcc_auth_label(fourcc: &[u8; 4]) -> Option<String> {
    if is_wildcard_media_fourcc(fourcc) {
        return None;
    }
    Some(fourcc_auth_label(fourcc))
}

fn fourcc_auth_label(fourcc: &[u8; 4]) -> String {
    match std::str::from_utf8(fourcc) {
        Ok(label) if !label.is_empty() => label.to_string(),
        _ => format!(
            "fourcc:{:02x}{:02x}{:02x}{:02x}",
            fourcc[0], fourcc[1], fourcc[2], fourcc[3]
        ),
    }
}

fn detect_video_codec(payload: &[u8]) -> Option<String> {
    if let Some(cc) = first_track_fourcc(FrameType::Video, payload) {
        return media_fourcc_auth_label(&cc);
    }
    let mut hdr = VideoHeader::default();
    if crate::ertmp::exvideo::exvideo_parse(payload, &mut hdr).is_err() {
        return None;
    }
    if hdr.is_ex_header != 0 {
        if hdr.packet_type == ERTMP_PACKET_TYPE_MODEX {
            return None;
        }
        let mut fourcc = [0u8; 4];
        fourcc.copy_from_slice(&hdr.fourcc[..4]);
        return media_fourcc_auth_label(&fourcc);
    } else {
        match payload[0] & 0x0F {
            7 => Some("avc1".to_string()),
            12 => Some("hvc1".to_string()),
            13 => Some("av01".to_string()),
            nibble => Some(format!("legacy:{nibble:x}")),
        }
    }
}

fn detect_audio_codec(payload: &[u8]) -> Option<String> {
    if let Some(cc) = first_track_fourcc(FrameType::Audio, payload) {
        return media_fourcc_auth_label(&cc);
    }
    // Detect a still-wrapped ModEx marker before `exaudio_parse` tries to
    // disambiguate the enhanced header by looking for a registered FourCC.
    // Deep chains that hit the peel limit otherwise fall back to the legacy
    // SoundFormat nibble and can masquerade as AAC.
    if payload
        .first()
        .is_some_and(|b| b & 0x80 != 0 && b & 0x0F == ERTMP_PACKET_TYPE_MODEX)
    {
        return None;
    }
    let mut hdr = AudioHeader::default();
    if crate::ertmp::exaudio::exaudio_parse(payload, &mut hdr).is_err() {
        return None;
    }
    if hdr.is_ex_header != 0 {
        if hdr.packet_type == ERTMP_PACKET_TYPE_MODEX {
            return None;
        }
        let mut fourcc = [0u8; 4];
        fourcc.copy_from_slice(&hdr.fourcc[..4]);
        return media_fourcc_auth_label(&fourcc);
    } else {
        match hdr.audio_codec {
            AudioCodec::Aac => Some("mp4a".to_string()),
            AudioCodec::Mp3 => Some("mp3".to_string()),
            AudioCodec::Opus => Some("Opus".to_string()),
            other => Some(format!("legacy:{other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amf::amf0;
    use crate::session::stream::Stream;

    #[test]
    fn relay_budget_counts_actual_retained_bytes() {
        let mut conn = Conn::new();
        conn.max_pending_relay_bytes = 6;
        assert!(conn.queue_relay_frame(FrameType::Video, 0, b"data", b"data").is_ok());
        assert_eq!(conn.pending_relay_bytes(), 4);
    }

    #[test]
    fn modex_wrapper_cannot_bypass_codec_deny_list_without_caps_ex() {
        fn allow_avc1_only(_: u64, frame_type: FrameType, codec: Option<&str>) -> bool {
            frame_type == FrameType::Video && codec == Some("avc1")
        }
        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        conn.current_stream.as_mut().unwrap().is_publishing = true;
        conn.on_media_cb = Some(allow_avc1_only);
        let payload = vec![
            0x97, b'a', b'v', b'c', b'1', 0x02, 0, 1, 2, 0x01, 0x90, b'v', b'p', b'0', b'9', 0,
            0, 0, 0xBB,
        ];
        assert_eq!(
            conn.handle_media_frame(1, FrameType::Video, 0, &payload),
            Err(ErrorCode::Auth)
        );
    }

    #[test]
    fn excessive_audio_modex_chain_is_unknown_for_authorization() {
        fn allow_aac_only(_: u64, frame_type: FrameType, codec: Option<&str>) -> bool {
            frame_type == FrameType::Audio && codec == Some("mp4a")
        }
        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.negotiated_caps.has_caps_ex = true;
        conn.negotiated_caps.caps_ex_mask = CAPS_EX_MASK_MODEX;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        conn.current_stream.as_mut().unwrap().is_publishing = true;
        conn.on_media_cb = Some(allow_aac_only);
        let mut payload = vec![0x97];
        for _ in 0..40 {
            payload.extend_from_slice(&[0x00, 0x00, 0x07]);
        }
        payload.extend_from_slice(&[0x90, b'O', b'p', b'u', b's', 0xBB]);
        assert_eq!(
            conn.handle_media_frame(1, FrameType::Audio, 0, &payload),
            Err(ErrorCode::Auth)
        );
    }

    #[test]
    fn unnegotiated_legacy_g711u_payload_is_preserved() {
        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        conn.current_stream.as_mut().unwrap().is_publishing = true;
        conn.on_frame_cb = Some(|_| {});
        let payload = vec![0x87, 0x00, 0x00, 0x00];
        conn.handle_media_frame(1, FrameType::Audio, 0, &payload)
            .unwrap();
        assert_eq!(conn.frame_cb_scratch, payload);
        assert_eq!(conn.pending_relay[0].payload, payload);
        assert!(conn.pending_relay[0].cache_payload.is_none());
    }
}
