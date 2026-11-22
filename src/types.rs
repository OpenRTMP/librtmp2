//! Core type definitions, error codes, and protocol constants.
//!
//! This module holds the vocabulary the rest of the crate is built from:
//!
//! - [`ErrorCode`] and the crate-wide [`Result`] alias.
//! - The [`ConnState`] connection state machine.
//! - Codec and frame classification ([`FrameType`], [`AudioCodec`],
//!   [`VideoCodec`], [`FourCc`], [`Frame`]).
//! - Enhanced RTMP (E-RTMP) v1/v2 constants: packet types, `capsEx` mask bits,
//!   FourCC capability flags, and the negotiation structs
//!   ([`NegotiatedCaps`], [`ConnectInfo`], [`Reconnect`]).
//! - FLV tag views ([`AudioTag`], [`VideoTag`], [`ScriptTag`]).
//!
//! Most items here mirror the C header `include/librtmp2/types.h` so that the
//! `extern "C"` layer and Rust callers share one definition. Rust-only
//! additions (such as [`ResourceLimits`]) are marked as such.
//!
//! ```
//! use librtmp2::types::{ErrorCode, FrameType};
//!
//! assert_eq!(ErrorCode::Ok as i32, 0);
//! assert_eq!(ErrorCode::Auth.as_str(), "Authentication error");
//! assert_eq!(FrameType::Video as i32, 1);
//! ```

#![allow(dead_code)]

use std::ffi::c_int;

/* ── Opaque forward-declared types ── */
// These are opaque in the C API; in Rust they are pub(crate) structs.

/// Opaque server handle
#[repr(C)]
pub struct Server {
    _private: [u8; 0],
}

/// Opaque client handle
#[repr(C)]
pub struct Client {
    _private: [u8; 0],
}

/// Opaque connection handle
#[repr(C)]
pub struct Conn {
    _private: [u8; 0],
}

/// Opaque stream handle
#[repr(C)]
pub struct Stream {
    _private: [u8; 0],
}

/* ── Error codes ── */

/// Error codes returned by librtmp2 functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[repr(C)]
pub enum ErrorCode {
    /// Operation succeeded
    Ok = 0,
    /// I/O error
    Io = -1,
    /// Timeout
    Timeout = -2,
    /// Protocol error
    Protocol = -3,
    /// Handshake error
    Handshake = -4,
    /// Chunk error
    Chunk = -5,
    /// AMF error
    Amf = -6,
    /// Unsupported feature
    Unsupported = -7,
    /// Authentication error
    Auth = -8,
    /// Internal error
    Internal = -9,
}

impl ErrorCode {
    /// Convert an error code to a human-readable string.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Ok => "OK",
            ErrorCode::Io => "I/O error",
            ErrorCode::Timeout => "Timeout",
            ErrorCode::Protocol => "Protocol error",
            ErrorCode::Handshake => "Handshake error",
            ErrorCode::Chunk => "Chunk error",
            ErrorCode::Amf => "AMF error",
            ErrorCode::Unsupported => "Unsupported",
            ErrorCode::Auth => "Authentication error",
            ErrorCode::Internal => "Internal error",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Result type used throughout the library.
pub type Result<T> = std::result::Result<T, ErrorCode>;

/* ── Connection state machine ── */

/// Connection state machine states.
///
/// A connection advances monotonically through these states as the RTMP
/// session is established, used, and torn down. See the [`crate::session`]
/// module docs for the surrounding flow; the [`ConnState::CapsNegotiated`]
/// state exists only when Enhanced RTMP v2 capability exchange runs and is
/// skipped for legacy connects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub enum ConnState {
    /// TCP connection accepted; no handshake bytes exchanged yet.
    TcpAccepted = 0,
    /// Performing the C0/C1/C2 ↔ S0/S1/S2 handshake.
    Handshake,
    /// Handshake complete; ready for the `connect` command.
    Connected,
    /// E-RTMP v2 capabilities negotiated (between [`Self::Connected`] and
    /// [`Self::AppConnected`]).
    CapsNegotiated,
    /// `NetConnection.Connect.Success` sent/acknowledged; the app is attached.
    AppConnected,
    /// `createStream` succeeded and a stream id was assigned.
    StreamCreated,
    /// This connection is publishing a stream.
    Publishing,
    /// This connection is playing a stream.
    Playing,
    /// Teardown started; remaining queued output is being flushed.
    Closing,
    /// Connection fully closed; no further I/O happens.
    Closed,
}

/* ── Asynchronous publish/play authorization ── */

/// Outcome of a play/publish authorization check.
///
/// `on_publish_cb`/`on_play_cb` (and their FFI equivalents) can only answer
/// synchronously with a `bool`.
/// [`Conn::on_publish_auth_cb`](crate::session::conn::Conn::on_publish_auth_cb)/
/// [`Conn::on_play_auth_cb`](crate::session::conn::Conn::on_play_auth_cb)
/// return this instead, so an
/// integrator whose authorization work is itself asynchronous (e.g. a
/// database lookup dispatched to a worker thread) can return `Pending`
/// without blocking the RTMP processing loop. The connection then waits --
/// sending and receiving no media -- until the integrator calls
/// `complete_publish_authorization`/`complete_play_authorization` (on
/// [`Conn`] or [`crate::server::Server`]) with the final `Allow`/`Deny`
/// decision, or until the pending-authorization timeout denies it.
///
/// [`Conn`]: crate::session::conn::Conn
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationResult {
    /// Authorization succeeded; proceed with the normal publish/play flow.
    Allow,
    /// Authorization failed; proceed with the normal rejection flow.
    Deny,
    /// Authorization is still in progress. The connection is held open with
    /// media blocked until a later `complete_*_authorization` call (or a
    /// timeout) resolves it.
    ///
    /// `request_id` is the integrator's own identifier for this request, echoed
    /// back to `complete_publish_authorization`/`complete_play_authorization`,
    /// so a completion resolves only the request it was issued for: a decision
    /// for a request the connection has already discarded -- torn down,
    /// replaced, or superseded when the opposite role was granted -- is ignored
    /// rather than applied to whatever request holds the slot by then. It must
    /// be unique per connection and role; a request reusing an id this
    /// connection already answered is refused, because its completion could not
    /// be told apart from the one owed for the earlier request.
    Pending {
        /// The integrator's own identifier for this authorization request,
        /// echoed back to `complete_publish_authorization` /
        /// `complete_play_authorization`. Must be unique per connection and
        /// role.
        request_id: u64,
    },
}

/* ── Frame types ── */

/// Frame type classification.
///
/// Distinguishes the four kinds of RTMP message the library delivers as a
/// [`Frame`]. Audio and video frames carry codec payloads; script and
/// metadata frames carry AMF-encoded side data (e.g. `onMetaData`).
///
/// ```
/// use librtmp2::types::FrameType;
///
/// assert_eq!(FrameType::Audio as i32, 0);
/// assert_eq!(FrameType::Video as i32, 1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum FrameType {
    /// Audio payload (RTMP message type `0x08`).
    Audio = 0,
    /// Video payload (RTMP message type `0x09`).
    Video = 1,
    /// Script data / metadata (RTMP message types `0x12`, `0x0F`).
    Script = 2,
    /// Metadata side channel (Enhanced RTMP metadata packet).
    Metadata = 3,
}

/* ── Audio codec IDs ── */

/// Audio codec identifiers.
///
/// The discriminants are the FLV `SoundFormat` values (high nibble of the first
/// audio tag byte) used by legacy RTMP audio, so they match the wire format
/// directly. SoundFormat `9` is Enhanced RTMP's ExHeader discriminator rather
/// than a codec, and `12`/`13`/`15` are reserved by the FLV specification.
///
/// Enhanced RTMP identifies codecs through a FourCC instead — see
/// [`crate::ertmp::fourcc`] — and reports them in [`Frame::audio_fourcc`]. Note
/// that [`AudioCodec::Opus`] is reachable only through a FourCC: legacy
/// SoundFormat `14` means MP3 at 8 kHz, not Opus.
///
/// ```
/// use librtmp2::types::AudioCodec;
///
/// assert_eq!(AudioCodec::Aac as u8, 10);
/// assert_eq!(AudioCodec::default(), AudioCodec::Pcm);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum AudioCodec {
    /// Uncompressed linear PCM, platform-endian.
    Pcm = 0,
    /// ADPCM.
    Adpcm = 1,
    /// MP3.
    Mp3 = 2,
    /// Linear PCM, little-endian.
    PcmLe = 3,
    /// Nellymoser 16 kHz.
    Nelly16k = 4,
    /// Nellymoser 8 kHz.
    Nelly8k = 5,
    /// Nellymoser.
    Nelly = 6,
    /// G.711 A-law.
    G711A = 7,
    /// G.711 µ-law.
    G711U = 8,
    /// AAC.
    Aac = 10,
    /// Speex.
    Speex = 11,
    /// Opus (Enhanced RTMP `Opus` FourCC only; no legacy SoundFormat).
    Opus = 14,
}

impl Default for AudioCodec {
    fn default() -> Self {
        AudioCodec::Pcm
    }
}

/* ── Video codec IDs (legacy) ── */

/// Video codec identifiers (legacy).
///
/// The discriminants are the FLV `CodecID` values (low nibble of the first
/// video tag byte) for legacy RTMP video. Enhanced RTMP identifies codecs
/// through a FourCC instead — see [`VideoFourCcInfoMap`] and
/// [`crate::ertmp::fourcc`] — and reports them in [`Frame::video_fourcc`].
///
/// ```
/// use librtmp2::types::VideoCodec;
///
/// assert_eq!(VideoCodec::H264 as u8, 7);
/// assert_eq!(VideoCodec::default(), VideoCodec::Jpeg);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum VideoCodec {
    /// JPEG (still image).
    Jpeg = 1,
    /// Sorenson H.263.
    Sorenson = 2,
    /// Screen video.
    Screen = 3,
    /// On2 VP6.
    Vp6 = 4,
    /// On2 VP6 with alpha channel.
    Vp6a = 5,
    /// Screen video v2.
    Screen2 = 6,
    /// AVC / H.264.
    H264 = 7,
    /// HEVC / H.265 (Enhanced RTMP only).
    H265 = 12,
    /// AV1 (Enhanced RTMP only).
    Av1 = 13,
    /// VP9 (Enhanced RTMP only).
    Vp9 = 14,
}

impl Default for VideoCodec {
    fn default() -> Self {
        VideoCodec::Jpeg
    }
}

/* ── FourCC for E-RTMP ── */

/// FourCC code for Enhanced RTMP.
///
/// A four-character codec identifier such as `"avc1"` or `"Opus"`, stored
/// NUL-terminated so it can be handed to C unchanged.
///
/// # Examples
///
/// ```
/// use librtmp2::types::FourCc;
///
/// let avc = FourCc { cc: *b"avc1\0" };
/// assert_eq!(&avc.cc[..4], b"avc1");
/// assert_eq!(avc.cc[4], 0); // NUL terminator
///
/// // The default is an all-zero (unset) code.
/// assert_eq!(FourCc::default().cc, [0u8; 5]);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct FourCc {
    /// null-terminated, e.g. "hvc1"
    pub cc: [u8; 5],
}

/* ── Parsed frame ── */

/// A parsed frame.
///
/// Input to the send path (`lrtmp2_client_send_frame`) and the value delivered
/// to `on_frame` callbacks. The Rust client and server deliver parsed frames
/// through an `on_frame` callback, which the C ABI does not yet expose.
///
/// # Data pointer lifetime
///
/// [`Frame::data`] points at a payload owned by the connection that produced
/// the frame (for callbacks) or by the caller (for the send path). It is valid
/// only for the duration of the callback, or for the duration of the
/// `send_frame` call when sending; copy the bytes if you need to retain them.
///
/// # Example
///
/// ```
/// use librtmp2::types::{Frame, FrameType};
///
/// let frame = Frame {
///     frame_type: FrameType::Video,
///     timestamp: 1000,
///     size: 0,
///     ..Frame::default()
/// };
/// assert_eq!(frame.frame_type, FrameType::Video);
/// // `track_id` defaults to 255, meaning "not a multitrack frame".
/// assert_eq!(frame.track_id, u8::MAX);
/// ```
#[derive(Debug, Clone)]
#[repr(C)]
pub struct Frame {
    /// Audio, video, script, or metadata classification.
    pub frame_type: FrameType,
    /// RTMP message timestamp in milliseconds.
    pub timestamp: u32,
    /// Composition time offset in milliseconds (video only; 0 otherwise).
    pub composition_time: u32,
    /// Length of the payload at [`Frame::data`], in bytes.
    pub size: u32,
    /// Pointer to the payload bytes; validity is described on [`Frame`].
    pub data: *const u8,
    /// Audio-specific: codec identified from the legacy FLV sound format.
    pub audio_codec: AudioCodec,
    /// Audio-specific: sample rate in Hz (`0` when unknown).
    pub audio_sample_rate: u32,
    /// Audio-specific: channel count.
    pub audio_channels: u8,
    /// Audio-specific: bit depth in bits (e.g. `8` or `16`).
    pub audio_bit_depth: u8,
    /// Enhanced: FourCC from ExAudioTagHeader
    pub audio_fourcc: FourCc,
    /// Video-specific: codec identified from the legacy FLV codec id.
    pub video_codec: VideoCodec,
    /// Video-specific: FourCC from the Enhanced RTMP video tag header.
    pub video_fourcc: FourCc,
    /// 1=keyframe, 2=inter, etc.
    pub video_frame_type: u8,
    /// Script/metadata flag
    pub is_metadata: u8,
    /// E-RTMP multitrack id (0 = default track; 255 = not set / single-track).
    pub track_id: u8,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            frame_type: FrameType::Audio,
            timestamp: 0,
            composition_time: 0,
            size: 0,
            data: std::ptr::null(),
            audio_codec: AudioCodec::default(),
            audio_sample_rate: 0,
            audio_channels: 0,
            audio_bit_depth: 0,
            audio_fourcc: FourCc::default(),
            video_codec: VideoCodec::default(),
            video_fourcc: FourCc::default(),
            video_frame_type: 0,
            is_metadata: 0,
            track_id: u8::MAX,
        }
    }
}

/* ── Error info ── */

/// Error information struct.
///
/// Pairs an [`ErrorCode`] with a fixed-size, NUL-padded human-readable message.
/// The buffer is `[u8; 256]` to keep the struct `#[repr(C)]`-compatible with
/// `include/librtmp2/types.h`.
///
/// # Examples
///
/// ```
/// use librtmp2::types::{ErrorCode, ErrorInfo};
///
/// let info = ErrorInfo {
///     code: ErrorCode::Timeout,
///     ..ErrorInfo::default()
/// };
/// assert_eq!(info.code, ErrorCode::Timeout);
/// // The message buffer is zero-initialised, so it is a valid empty C string.
/// assert_eq!(info.message[0], 0);
/// ```
#[derive(Debug, Clone)]
#[repr(C)]
pub struct ErrorInfo {
    /// Machine-readable error classification.
    pub code: ErrorCode,
    /// NUL-terminated UTF-8 message; unused trailing bytes are zero.
    pub message: [u8; 256],
}

impl Default for ErrorInfo {
    fn default() -> Self {
        Self {
            code: ErrorCode::Ok,
            message: [0; 256],
        }
    }
}

/* ── Server config ── */

/// Per-connection and server-wide memory limits (Rust-only; not part of the C ABI).
///
/// Caps how much memory a hostile or misbehaving peer can make the library
/// retain. The defaults are tuned for a busy relay; lower them first when
/// embedding somewhere with a tight memory budget.
///
/// # Examples
///
/// ```
/// use librtmp2::types::ResourceLimits;
///
/// let defaults = ResourceLimits::default();
/// assert_eq!(defaults.max_reassembly_bytes, 32 * 1024 * 1024);
///
/// // Tighten the per-connection reassembly ceiling; keep the rest.
/// let tight = ResourceLimits {
///     max_reassembly_bytes: 1024 * 1024,
///     ..ResourceLimits::default()
/// };
/// assert_eq!(tight.max_reassembly_bytes, 1024 * 1024);
/// assert_eq!(tight.max_stream_cache_bytes, defaults.max_stream_cache_bytes);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    /// Total bytes retained across all stream-cache entries.
    pub max_stream_cache_bytes: usize,
    /// Max reassembly buffer bytes per RTMP connection.
    pub max_reassembly_bytes: usize,
    /// Max queued relay payload bytes per publisher connection.
    pub max_pending_relay_bytes: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_stream_cache_bytes: 64 * 1024 * 1024,
            max_reassembly_bytes: 32 * 1024 * 1024,
            max_pending_relay_bytes: 8 * 1024 * 1024,
        }
    }
}

/// Server configuration struct.
///
/// The `tls_*_file` fields are raw C string pointers. Passing this struct to
/// the raw constructor ([`crate::server::Server::new`]) or to the C ABI entry
/// point requires each non-null pointer to reference a NUL-terminated C string;
/// see that function's `# Safety` section. Rust callers should prefer
/// [`crate::server::Server::from_config`], which takes TLS paths as `&Path`
/// and never dereferences these fields.
///
/// ```
/// use librtmp2::types::ServerConfig;
///
/// // Plaintext RTMP listener on 0.0.0.0 with library defaults:
/// let cfg = ServerConfig {
///     chunk_size: 4096,
///     ..ServerConfig::default()
/// };
/// assert_eq!(cfg.tls_enabled, 0);
/// assert!(cfg.tls_cert_file.is_null());
/// ```
#[derive(Clone, Copy)]
#[repr(C)]
pub struct ServerConfig {
    /// Maximum number of concurrent accepted connections. `0` means "use the
    /// library default" at the C ABI boundary; a negative value means
    /// unlimited.
    pub max_connections: c_int,
    /// Outbound chunk size announced to peers via `SetChunkSize`, in bytes.
    /// Clamped by the server to a sane range.
    pub chunk_size: c_int,
    /// TLS / RTMPS fields
    pub tls_enabled: c_int,
    /// Server: PEM certificate chain file.
    ///
    /// `null` or a pointer to a NUL-terminated C string (valid UTF-8).
    pub tls_cert_file: *const u8,
    /// Server: PEM private key file.
    ///
    /// `null` or a pointer to a NUL-terminated C string (valid UTF-8).
    pub tls_key_file: *const u8,
    /// Client: CA bundle for verification (or NULL)
    pub tls_ca_file: *const u8,
    /// Client: skip certificate verification
    pub tls_insecure: c_int,
    /// Max incomplete TLS handshakes retained per remote peer IP before the
    /// oldest is evicted to admit a new one. `0` (or negative) uses the
    /// built-in default (see `server::DEFAULT_MAX_PENDING_TLS_PER_ADDR`).
    /// Appended field: see `docs/abi-policy.md` (config struct fields may be
    /// appended between minor releases).
    pub max_pending_tls_per_addr: c_int,
    /// Max active (already-accepted) connections retained per remote peer
    /// IP. `0` (or negative) uses the built-in default (see
    /// `server::DEFAULT_MAX_CONNECTIONS_PER_ADDR`). Kept independent of
    /// `max_pending_tls_per_addr` so tuning one does not change the other:
    /// they defend different resources (the connection table vs. the
    /// pending-handshake queue). Appended field: see `docs/abi-policy.md`.
    pub max_connections_per_addr: c_int,
}

impl Default for ServerConfig {
    /// Plaintext (no TLS), library defaults for every limit.
    ///
    /// The raw TLS path pointers are null, so the config is safe to pass to
    /// [`crate::server::Server::new`] from inside an `unsafe` block or to
    /// [`crate::server::Server::from_config`].
    fn default() -> Self {
        Self {
            // Match the C ABI's documented default: 0 means "use the library
            // default" (256), not "unlimited". Safe Rust callers get the same
            // behaviour as a zero-initialised `ServerConfig` passed through
            // `lrtmp2_server_create`.
            max_connections: 256,
            chunk_size: 128,
            tls_enabled: 0,
            tls_cert_file: std::ptr::null(),
            tls_key_file: std::ptr::null(),
            tls_ca_file: std::ptr::null(),
            tls_insecure: 0,
            max_pending_tls_per_addr: 0,
            max_connections_per_addr: 0,
        }
    }
}

/* ── Callback types ── */

/// Server-side publish/play authorization callback installed through the C ABI.
/// Return `0` to allow and any non-zero value to deny.
pub(crate) type CAuthCallback = unsafe extern "C" fn(
    conn_id: u64,
    app: *const std::ffi::c_char,
    stream_name: *const std::ffi::c_char,
    userdata: *mut std::ffi::c_void,
) -> std::ffi::c_int;

/// C callback plus its opaque context, kept together for Server/Conn copies.
#[derive(Clone, Copy, Default)]
pub(crate) struct CAuthState {
    pub(crate) callback: Option<CAuthCallback>,
    pub(crate) userdata: usize,
}

/// on_connect callback type
pub type OnConnectCb = Option<unsafe extern "C" fn(conn: *mut Conn, userdata: *mut u8) -> c_int>;
/// on_publish callback type
pub type OnPublishCb = Option<
    unsafe extern "C" fn(
        conn: *mut Conn,
        app: *const u8,
        stream_key: *const u8,
        userdata: *mut u8,
    ) -> c_int,
>;
/// on_play callback type
pub type OnPlayCb = Option<
    unsafe extern "C" fn(
        conn: *mut Conn,
        app: *const u8,
        stream_key: *const u8,
        userdata: *mut u8,
    ) -> c_int,
>;
/// on_frame callback type
pub type OnFrameCb =
    Option<unsafe extern "C" fn(conn: *mut Conn, frame: *const Frame, userdata: *mut u8) -> c_int>;
/// on_close callback type
pub type OnCloseCb = Option<unsafe extern "C" fn(conn: *mut Conn, userdata: *mut u8)>;
/// on_send_data callback type
pub type OnSendDataCb = Option<
    unsafe extern "C" fn(conn: *mut Conn, data: *const u8, len: usize, userdata: *mut u8) -> c_int,
>;

/* ── Version constants ── */

/// Parses a plain-digit version component (as provided by Cargo's
/// `CARGO_PKG_VERSION_*` env vars) into a `u32` at compile time.
const fn parse_version_component(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut value: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        value = value * 10 + (bytes[i] - b'0') as u32;
        i += 1;
    }
    value
}

/// Major version number of the crate, parsed from `CARGO_PKG_VERSION_MAJOR`
/// at compile time.
pub const VERSION_MAJOR: u32 = parse_version_component(env!("CARGO_PKG_VERSION_MAJOR"));
/// Minor version number of the crate, parsed from `CARGO_PKG_VERSION_MINOR`
/// at compile time.
pub const VERSION_MINOR: u32 = parse_version_component(env!("CARGO_PKG_VERSION_MINOR"));
/// Patch version number of the crate, parsed from `CARGO_PKG_VERSION_PATCH`
/// at compile time.
pub const VERSION_PATCH: u32 = parse_version_component(env!("CARGO_PKG_VERSION_PATCH"));
/// Full version string, e.g. `"0.17.2"`.
pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

/* ── E-RTMP constants ── */

/// Enhanced RTMP v1 VideoPacketType: start of a codec sequence header.
pub const ERTMP_PACKET_TYPE_SEQUENCE_START: u8 = 0;
/// Enhanced RTMP v1 VideoPacketType: a coded frame.
pub const ERTMP_PACKET_TYPE_CODED_FRAMES: u8 = 1;
/// Enhanced RTMP v1 VideoPacketType: end of a codec sequence.
pub const ERTMP_PACKET_TYPE_SEQUENCE_END: u8 = 2;
/// Enhanced RTMP v1 VideoPacketType: coded frames with an extended header.
pub const ERTMP_PACKET_TYPE_CODED_FRAMES_X: u8 = 3;
/// Enhanced RTMP v1 VideoPacketType: metadata (e.g. HDR `colorInfo`).
pub const ERTMP_PACKET_TYPE_METADATA: u8 = 4;
/// Enhanced RTMP v1 VideoPacketType: start of an MPEG-2 TS sequence.
pub const ERTMP_PACKET_TYPE_MPEG2TS_SEQUENCE_START: u8 = 5;

/// Enhanced RTMP v1/v2 AudioPacketType: start of an audio codec sequence header
/// (e.g. AAC AudioSpecificConfig).
pub const ERTMP_AUDIO_PACKET_TYPE_SEQUENCE_START: u8 = 0;
/// Enhanced RTMP v1/v2 AudioPacketType: a coded audio frame.
pub const ERTMP_AUDIO_PACKET_TYPE_CODED_FRAMES: u8 = 1;
/// Enhanced RTMP v1/v2 AudioPacketType: end of an audio codec sequence.
pub const ERTMP_AUDIO_PACKET_TYPE_SEQUENCE_END: u8 = 2;
/// Enhanced RTMP v1/v2 AudioPacketType: metadata.
pub const ERTMP_AUDIO_PACKET_TYPE_METADATA: u8 = 3;
/// Enhanced RTMP v2 AudioPacketType: multichannel configuration.
pub const ERTMP_AUDIO_PACKET_TYPE_MULTICHANNEL: u8 = 4;
/// Enhanced RTMP v2 AudioPacketType: multitrack container.
pub const ERTMP_AUDIO_PACKET_TYPE_MULTITRACK: u8 = 5;
/// Enhanced RTMP v2 VideoPacketType: multitrack container.
pub const ERTMP_VIDEO_PACKET_TYPE_MULTITRACK: u8 = 6;

/// E-RTMP v2 `CapsExMask` bits carried in the connect `capsEx` AMF number.
///
/// Reconnect support requested by the client.
pub const CAPS_EX_MASK_RECONNECT: u32 = 0x01;
/// Multitrack (per-track audio/video containers) support.
pub const CAPS_EX_MASK_MULTITRACK: u32 = 0x02;
/// ModEx (modular extension wrapper) support.
pub const CAPS_EX_MASK_MODEX: u32 = 0x04;
/// Nanosecond timestamp-offset (`TimestampOffsetNano`) support.
pub const CAPS_EX_MASK_TIMESTAMP_NANO: u32 = 0x08;
/// Capability mask the built-in server advertises by default.
pub const CAPS_EX_MASK_SERVER_DEFAULT: u32 =
    CAPS_EX_MASK_MULTITRACK | CAPS_EX_MASK_MODEX | CAPS_EX_MASK_TIMESTAMP_NANO;

/// Enhanced RTMP v1 AudioSampleRate (legacy): 5512 Hz.
pub const ERTMP_AUDIO_RATE_5500: u8 = 0;
/// 11025 Hz sample rate.
pub const ERTMP_AUDIO_RATE_11025: u8 = 1;
/// 22050 Hz sample rate.
pub const ERTMP_AUDIO_RATE_22050: u8 = 2;
/// 44100 Hz sample rate.
pub const ERTMP_AUDIO_RATE_44100: u8 = 3;

/// Enhanced RTMP v1 AudioSampleSize (legacy): 8-bit samples.
pub const ERTMP_AUDIO_SAMPLE_8BIT: u8 = 0;
/// 16-bit samples.
pub const ERTMP_AUDIO_SAMPLE_16BIT: u8 = 1;

/* ── Parsed video tag header ── */

/// Parsed video tag header.
///
/// The result of decoding the first bytes of an RTMP video payload, legacy FLV
/// or Enhanced RTMP alike. See [`crate::ertmp::exvideo`] for the parser.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::exvideo::exvideo_parse;
/// use librtmp2::types::VideoHeader;
///
/// // 0x17 = legacy AVC keyframe (bit 7 clear), then AVCPacketType 0x01.
/// let mut hdr = VideoHeader::default();
/// exvideo_parse(&[0x17, 0x01, 0x00, 0x00, 0x00], &mut hdr).unwrap();
///
/// assert_eq!(hdr.is_ex_header, 0); // legacy tag
/// assert_eq!(hdr.frame_type, 1);   // keyframe
/// // `header_size` is where the codec payload begins.
/// assert!(hdr.header_size >= 5);
/// ```
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct VideoHeader {
    /// Non-zero when the tag used an Enhanced RTMP (ExHeader) header.
    pub is_ex_header: u8,
    /// Enhanced RTMP [`ERTMP_PACKET_TYPE_*`](ERTMP_PACKET_TYPE_CODED_FRAMES)
    /// packet type; `0` for legacy tags.
    pub packet_type: u8,
    /// FourCC codec identifier, NUL-terminated (Enhanced RTMP only).
    pub fourcc: [u8; 5],
    /// FLV frame type nibble (1 = keyframe, 2 = inter frame, ...).
    pub frame_type: u8,
    /// Composition time offset in milliseconds.
    pub composition_time: u32,
    /// Number of header bytes consumed; the codec payload starts here.
    pub header_size: usize,
}

/* ── Parsed audio tag header ── */

/// Parsed audio tag header.
///
/// The result of decoding the first bytes of an RTMP audio payload, legacy FLV
/// or Enhanced RTMP alike. See [`crate::ertmp::exaudio`] for the parser.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::exaudio::exaudio_parse;
/// use librtmp2::types::{AudioCodec, AudioHeader};
///
/// let mut hdr = AudioHeader::default();
/// // Legacy AAC sequence header: 0xAF = AAC, 44 kHz, 16-bit, stereo.
/// exaudio_parse(&[0xAF, 0x00], &mut hdr).unwrap();
///
/// assert_eq!(hdr.is_ex_header, 0);
/// assert_eq!(hdr.audio_codec, AudioCodec::Aac);
/// assert_eq!(hdr.aac_packet_type, 0); // sequence header
/// ```
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct AudioHeader {
    /// Non-zero when the tag used an Enhanced RTMP (ExHeader) header.
    pub is_ex_header: u8,
    /// Enhanced RTMP
    /// [`ERTMP_AUDIO_PACKET_TYPE_*`](ERTMP_AUDIO_PACKET_TYPE_CODED_FRAMES)
    /// packet type; `0` for legacy tags.
    pub packet_type: u8,
    /// FourCC codec identifier, NUL-terminated (Enhanced RTMP only).
    pub fourcc: [u8; 5],
    /// Codec identified from the legacy FLV sound format.
    pub audio_codec: AudioCodec,
    /// Raw
    /// [`ERTMP_AUDIO_RATE_*`](ERTMP_AUDIO_RATE_44100) sample-rate code.
    pub sample_rate: u8,
    /// Raw
    /// [`ERTMP_AUDIO_SAMPLE_*`](ERTMP_AUDIO_SAMPLE_16BIT) sample-size code.
    pub sample_size: u8,
    /// Number of audio channels (`1` = mono, `2` = stereo).
    pub channels: u8,
    /// AAC packet type (0 = sequence header, 1 = raw) for legacy AAC tags.
    pub aac_packet_type: u8,
    /// Number of header bytes consumed; the codec payload starts here.
    pub header_size: usize,
}

/* ── HDR color info ── */

/// HDR color information.
///
/// Mirrors the Enhanced RTMP `colorInfo` object: the three CTA-861 style
/// enumerations that describe an HDR signal. Values are passed through
/// unchanged from the wire.
///
/// # Examples
///
/// ```
/// use librtmp2::types::HdrInfo;
///
/// // An HDR10 (PQ / BT.2020) signal.
/// let hdr = HdrInfo {
///     color_primaries: 9,  // BT.2020
///     transfer_chars: 16,  // PQ
///     matrix_coeffs: 9,    // BT.2020 non-constant
/// };
/// assert_eq!(hdr.transfer_chars, 16);
///
/// // The default is all-zero, i.e. "unspecified".
/// assert_eq!(HdrInfo::default().color_primaries, 0);
/// ```
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct HdrInfo {
    /// CTA-861 color primaries code (e.g. `9` = BT.2020).
    pub color_primaries: u16,
    /// CTA-861 transfer-characteristics code (e.g. `16` = PQ).
    pub transfer_chars: u16,
    /// CTA-861 matrix-coefficients code (e.g. `9` = BT.2020 non-constant).
    pub matrix_coeffs: u16,
}

/* ── fourCcList ── */

/// Maximum number of FourCC entries in a list.
pub const MAX_FOURCCS: usize = 16;

/// A list of FourCC codes.
///
/// Used in E-RTMP v2 capability negotiation (`fourCcList` in the connect
/// command) to advertise which codecs an endpoint supports.
///
/// ```
/// use librtmp2::types::{FourCc, FourCcList, MAX_FOURCCS};
///
/// let mut list = FourCcList::default();
/// list.entries[0] = FourCc { cc: *b"avc1\0" };
/// list.count = 1;
/// assert!(list.count <= MAX_FOURCCS);
/// ```
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct FourCcList {
    /// Codec entries; only the first [`FourCcList::count`] are meaningful.
    pub entries: [FourCc; MAX_FOURCCS],
    /// Number of populated entries; never exceeds [`MAX_FOURCCS`].
    pub count: usize,
}

/* ── E-RTMP v2 capability negotiation ── */

/// E-RTMP v2 per-codec capability flags.
///
/// Bit flags OR-ed together in [`VideoFourCcInfoMap::masks`] to describe what
/// an endpoint can do with a given video codec.
///
/// This endpoint can decode the codec.
pub const FOUR_CC_INFO_CAN_DECODE: u32 = 0x01;
/// This endpoint can encode the codec.
pub const FOUR_CC_INFO_CAN_ENCODE: u32 = 0x02;
/// This endpoint can forward the codec without decoding it.
pub const FOUR_CC_INFO_CAN_FORWARD: u32 = 0x04;
/// Union of every capability flag.
pub const FOUR_CC_INFO_ALL: u32 =
    FOUR_CC_INFO_CAN_DECODE | FOUR_CC_INFO_CAN_ENCODE | FOUR_CC_INFO_CAN_FORWARD;

/// Video FourCC info map. Each entry's mask uses `FOUR_CC_INFO_*`.
///
/// # Examples
///
/// ```
/// use librtmp2::types::{
///     FourCc, VideoFourCcInfoMap, FOUR_CC_INFO_CAN_DECODE, FOUR_CC_INFO_CAN_ENCODE,
/// };
///
/// let mut map = VideoFourCcInfoMap::default();
/// map.entries[0] = FourCc { cc: *b"avc1\0" };
/// map.masks[0] = FOUR_CC_INFO_CAN_DECODE | FOUR_CC_INFO_CAN_ENCODE;
/// map.count = 1;
///
/// assert_eq!(map.count, 1);
/// assert_eq!(map.masks[0], 0x03);
/// ```
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct VideoFourCcInfoMap {
    /// Codec FourCC codes; only the first [`Self::count`] are meaningful.
    pub entries: [FourCc; MAX_FOURCCS],
    /// Capability mask per entry, using `FOUR_CC_INFO_*` bits.
    pub masks: [u32; MAX_FOURCCS],
    /// Number of populated entries; never exceeds [`MAX_FOURCCS`].
    pub count: usize,
}

/// Capability negotiation struct.
///
/// Sent as the `capsEx` numbers in the E-RTMP v2 `connect` command; carries the
/// protocol version and the legacy 32-bit codec bitmasks.
///
/// # Examples
///
/// ```
/// use librtmp2::types::CapsExit;
///
/// let caps = CapsExit {
///     version: 2,
///     video_codec_32: 0x0000_0080, // legacy H.264 bit
///     audio_codec_32: 0x0000_0400, // legacy AAC bit
/// };
/// assert_eq!(caps.version, 2);
/// assert_ne!(caps.video_codec_32, 0);
/// ```
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct CapsExit {
    /// E-RTMP version the peer implements.
    pub version: u32,
    /// Legacy 32-bit video codec bitmask, passed through unchanged.
    pub video_codec_32: i32,
    /// Legacy 32-bit audio codec bitmask, passed through unchanged.
    pub audio_codec_32: i32,
}

/* ── E-RTMP v2 reconnect ── */

/// Reconnect request struct.
///
/// E-RTMP v2 `reconnect` value advertised in `connect`. See
/// [`crate::ertmp::reconnect`] for the wire codec.
///
/// # Examples
///
/// ```
/// use librtmp2::types::Reconnect;
///
/// // Ask the server to replay buffered data on reconnect, up to 64 KiB.
/// let rc = Reconnect { replay: 1, limit: 64 * 1024 };
/// assert_eq!(rc.replay, 1);
///
/// // The default disables replay.
/// assert_eq!(Reconnect::default().replay, 0);
/// ```
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct Reconnect {
    /// Non-zero when the endpoint wants the server to replay cached data on
    /// reconnect.
    pub replay: u32,
    /// Replay limit (number of frames or bytes) the endpoint can absorb.
    pub limit: u32,
}

/* ── E-RTMP v2 multitrack ── */

/// Multitrack type.
///
/// Classifies what a multitrack container carries. See
/// [`crate::ertmp::multitrack`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum MultitrackType {
    /// Audio tracks.
    Audio = 0,
    /// Video tracks.
    Video = 1,
    /// Metadata tracks.
    Metadata = 2,
}

impl Default for MultitrackType {
    fn default() -> Self {
        MultitrackType::Audio
    }
}

/// Multitrack descriptor.
///
/// Describes one track in a multitrack container: its type and a UTF-8/ASCII
/// name such as `"mono"` or `"camera2"`.
///
/// # Examples
///
/// ```
/// use librtmp2::types::{Multitrack, MultitrackType};
///
/// let mut track = Multitrack::default();
/// track.track_type = MultitrackType::Video;
/// track.track_name[..6].copy_from_slice(b"camera");
/// track.track_name[6] = b'2';
///
/// assert_eq!(track.track_type, MultitrackType::Video);
/// assert_eq!(&track.track_name[..7], b"camera2");
/// ```
#[derive(Debug, Clone)]
#[repr(C)]
pub struct Multitrack {
    /// What the track carries.
    pub track_type: MultitrackType,
    /// NUL-terminated track name.
    pub track_name: [u8; 64],
}

impl Default for Multitrack {
    fn default() -> Self {
        Self {
            track_type: MultitrackType::Audio,
            track_name: [0; 64],
        }
    }
}

/* ── E-RTMP v2 ModEx ── */

/// ModEx extension type (E-RTMP v2 `VideoPacketModExType` /
/// `AudioPacketModExType`).
///
/// Only `TimestampOffsetNano` (wire `modExType` 0) is defined by the v2 spec;
/// every other wire value is surfaced as [`ModexType::Nop`] so callers degrade
/// gracefully instead of rejecting the frame. The discriminants below are an
/// internal Rust representation, not the wire values — use
/// [`crate::media::modex::modex_write`] to serialize.
///
/// # Examples
///
/// ```
/// use librtmp2::types::ModexType;
///
/// // A consumer-side "ignore this extension".
/// assert_eq!(ModexType::default(), ModexType::Nop);
///
/// // The only extension the v2 spec defines.
/// assert_eq!(ModexType::TimestampOffsetNano as u8, 0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum ModexType {
    /// v2 `TimestampOffsetNano` (`modExType` 0). [`Modex::offset`] carries the
    /// UI24 nanosecond offset from `modExData`.
    TimestampOffsetNano = 0,
    /// No recognized ModEx extension (reserved/unknown `modExType`, or a
    /// consumer-side "ignore this extension"). Ignored per the spec's
    /// graceful-degradation rule; it has no v2 wire representation.
    Nop = 1,
}

impl Default for ModexType {
    fn default() -> Self {
        ModexType::Nop
    }
}

impl ModexType {
    /// Deprecated alias for [`ModexType::TimestampOffsetNano`].
    ///
    /// The pre-v2 API called this `Timestamp` and encoded an eight-byte
    /// offset; the v2 wire format uses a UI24 `TimestampOffsetNano` instead.
    #[deprecated(note = "renamed to `ModexType::TimestampOffsetNano`; v2 encodes a UI24 offset")]
    #[allow(non_upper_case_globals)]
    pub const Timestamp: ModexType = ModexType::TimestampOffsetNano;
}

/// ModEx extension wrapper (E-RTMP v2 ModEx signalling).
///
/// A wrapper is `sizeMinusOne` + `modExData` + one option byte holding
/// `modExType:UB[4] | nextPacketType:UB[4]`; see
/// [`crate::media::modex`].
///
/// # Examples
///
/// ```
/// use librtmp2::types::{Modex, ModexType};
///
/// // A 1 ms nanosecond offset followed by a coded frame (packet type 1).
/// let modex = Modex {
///     modex_type: ModexType::TimestampOffsetNano,
///     offset: 1_000_000,
///     next_packet_type: 1,
/// };
/// assert_eq!(modex.modex_type, ModexType::TimestampOffsetNano);
/// // Only the low 24 bits reach the wire.
/// assert_eq!(modex.offset & 0xFF_FFFF, 1_000_000);
/// ```
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct Modex {
    /// Recognized ModEx extension type.
    pub modex_type: ModexType,
    /// `TimestampOffsetNano` value. Only the low 24 bits are written, matching
    /// the spec's UI24.
    pub offset: u64,
    /// Low nibble of the option byte: the packet type that follows this
    /// wrapper. [`crate::media::modex::ERTMP_PACKET_TYPE_MODEX`] (7) chains
    /// another ModEx wrapper.
    pub next_packet_type: u8,
}

/* ── FLV tag types ── */

/// FLV audio tag.
///
/// A borrowed view of a parsed FLV audio tag. [`AudioTag::data`] points into the
/// parser's input buffer and is only valid while that buffer lives.
///
/// # Examples
///
/// ```
/// use librtmp2::flv::audio_tag;
/// use librtmp2::types::{AudioCodec, AudioTag};
///
/// let payload = [0xAF, 0x01, 0x21, 0x10]; // AAC, raw frame
/// let mut tag = AudioTag::default();
/// audio_tag::parse(&payload, &mut tag).unwrap();
///
/// assert_eq!(tag.codec, AudioCodec::Aac);
/// assert_eq!(tag.aac_packet_type, 1);
/// // `data`/`size` cover the whole payload passed in.
/// assert_eq!(tag.size, payload.len());
/// ```
#[derive(Debug, Clone)]
#[repr(C)]
pub struct AudioTag {
    /// Codec identified from the FLV sound format.
    pub codec: AudioCodec,
    /// Raw FLV sample-rate code (see [`ERTMP_AUDIO_RATE_44100`]).
    pub sample_rate: u8,
    /// Sample size in bits.
    pub bit_depth: u8,
    /// Channel count.
    pub channels: u8,
    /// AAC packet type (0 = sequence header, 1 = raw); meaningful for AAC only.
    pub aac_packet_type: u8,
    /// Pointer to the codec payload.
    pub data: *const u8,
    /// Length of the payload at [`AudioTag::data`], in bytes.
    pub size: usize,
}

impl Default for AudioTag {
    fn default() -> Self {
        Self {
            codec: AudioCodec::default(),
            sample_rate: 0,
            bit_depth: 0,
            channels: 0,
            aac_packet_type: 0,
            data: std::ptr::null(),
            size: 0,
        }
    }
}

/// FLV video tag.
///
/// A borrowed view of a parsed FLV video tag. [`VideoTag::data`] points into the
/// parser's input buffer and is only valid while that buffer lives.
///
/// # Examples
///
/// ```
/// use librtmp2::flv::video_tag;
/// use librtmp2::types::{VideoCodec, VideoTag};
///
/// // AVC keyframe: 0x17 = keyframe + AVC, 0x01 = NALU, cts = 0.
/// let payload = [0x17, 0x01, 0x00, 0x00, 0x00, 0xAA, 0xBB];
/// let mut tag = VideoTag::default();
/// video_tag::parse(&payload, &mut tag).unwrap();
///
/// assert_eq!(tag.codec, VideoCodec::H264);
/// assert_eq!(tag.frame_type, 1);
/// assert_eq!(tag.avc_packet_type, 1);
/// assert_eq!(tag.composition_time, 0);
/// ```
#[derive(Debug, Clone)]
#[repr(C)]
pub struct VideoTag {
    /// FLV frame type nibble (1 = keyframe, 2 = inter frame, ...).
    pub frame_type: u8,
    /// Codec identified from the FLV codec id.
    pub codec: VideoCodec,
    /// AVC/HEVC packet type (0 = sequence header, 1 = NALU, 2 = end of seq).
    pub avc_packet_type: u8,
    /// Composition time offset in milliseconds.
    pub composition_time: u32,
    /// Pointer to the codec payload.
    pub data: *const u8,
    /// Length of the payload at [`VideoTag::data`], in bytes.
    pub size: usize,
}

impl Default for VideoTag {
    fn default() -> Self {
        Self {
            frame_type: 0,
            codec: VideoCodec::default(),
            avc_packet_type: 0,
            composition_time: 0,
            data: std::ptr::null(),
            size: 0,
        }
    }
}

/// FLV script tag.
///
/// A borrowed view of a parsed FLV script (metadata) tag. [`ScriptTag::data`]
/// points into the parser's input buffer and is only valid while that buffer
/// lives.
///
/// # Examples
///
/// ```
/// use librtmp2::flv::script_tag;
/// use librtmp2::types::ScriptTag;
///
/// // AMF0 string "onMetaData" then an empty object.
/// let mut payload = vec![0x02, 0x00, 0x0A];
/// payload.extend_from_slice(b"onMetaData");
/// payload.extend_from_slice(&[0x03, 0x00, 0x00, 0x09]);
///
/// let mut tag = ScriptTag::default();
/// script_tag::parse(&payload, &mut tag).unwrap();
/// assert_eq!(&tag.name[..10], b"onMetaData");
/// assert!(tag.size > 0);
/// ```
#[derive(Debug, Clone)]
#[repr(C)]
pub struct ScriptTag {
    /// NUL-terminated script/tag name, e.g. `"onMetaData"`.
    pub name: [u8; 64],
    /// Pointer to the AMF-encoded script body.
    pub data: *const u8,
    /// Length of the payload at [`ScriptTag::data`], in bytes.
    pub size: usize,
}

impl Default for ScriptTag {
    fn default() -> Self {
        Self {
            name: [0; 64],
            data: std::ptr::null(),
            size: 0,
        }
    }
}

/* ── Connect info ── */

/// Negotiated E-RTMP capability set returned in connect `_result`.
///
/// The subset of E-RTMP v2 capabilities both peers agreed on during the
/// `connect` handshake. Each capability carries a `has_*` flag because the
/// corresponding struct field is meaningless when the capability was not
/// negotiated. See [`crate::ertmp::connect_amf`].
#[derive(Debug, Clone, Default)]
pub struct NegotiatedCaps {
    /// Codecs advertised through `fourCcList`.
    pub four_cc_list: FourCcList,
    /// Version and legacy codec bitmasks from `capsEx`.
    pub caps_ex: CapsExit,
    /// Per-codec encode/decode/forward capabilities.
    pub video_four_cc_info_map: VideoFourCcInfoMap,
    /// Reconnect/replay parameters.
    pub reconnect: Reconnect,
    /// Raw `capsEx` mask both peers agreed on; see [`CAPS_EX_MASK_RECONNECT`].
    pub caps_ex_mask: u32,
    /// Whether [`Self::four_cc_list`] was negotiated.
    pub has_four_cc_list: bool,
    /// Whether [`Self::caps_ex`] was negotiated.
    pub has_caps_ex: bool,
    /// Whether [`Self::video_four_cc_info_map`] was negotiated.
    pub has_video_four_cc_info_map: bool,
    /// Whether [`Self::reconnect`] was negotiated.
    pub has_reconnect: bool,
    /// Whether multitrack containers may be exchanged on this connection.
    pub multitrack_enabled: bool,
}

/// Connect command information.
///
/// The parsed fields of an RTMP `connect` command, including the E-RTMP
/// capability objects a peer sent. Fixed-size byte arrays hold strings so the
/// struct stays `#[repr(C)]`-compatible; unused trailing bytes are zero.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct ConnectInfo {
    /// Command name, normally `"connect"`.
    pub name: [u8; 64],
    /// AMF transaction id of the `connect` command.
    pub transaction_id: f64,
    /// Application name.
    pub app: [u8; 256],
    /// `tcUrl` from the command object.
    pub tc_url: [u8; 512],
    /// `pageUrl` from the command object.
    pub page_url: [u8; 512],
    /// `swfUrl` from the command object.
    pub swf_url: [u8; 512],
    /// Flash player version string.
    pub flash_ver: [u8; 64],
    /// Legacy 32-bit audio codec bitmask.
    pub audio_codecs: i32,
    /// Legacy 32-bit video codec bitmask.
    pub video_codecs: i32,
    /// Codecs advertised through `fourCcList`.
    pub four_cc_list: FourCcList,
    /// Version and legacy codec bitmasks from `capsEx`.
    pub caps_ex: CapsExit,
    /// Per-codec encode/decode/forward capabilities.
    pub video_four_cc_info_map: VideoFourCcInfoMap,
    /// Reconnect/replay parameters.
    pub reconnect: Reconnect,
    /// Raw `capsEx` mask; see [`CAPS_EX_MASK_RECONNECT`].
    pub caps_ex_mask: u32,
    /// Whether [`Self::four_cc_list`] was present.
    pub has_four_cc_list: bool,
    /// Whether [`Self::caps_ex`] was present.
    pub has_caps_ex: bool,
    /// Whether [`Self::video_four_cc_info_map`] was present.
    pub has_video_four_cc_info_map: bool,
    /// Whether [`Self::reconnect`] was present.
    pub has_reconnect: bool,
}

impl Default for ConnectInfo {
    fn default() -> Self {
        Self {
            name: [0; 64],
            transaction_id: 0.0,
            app: [0; 256],
            tc_url: [0; 512],
            page_url: [0; 512],
            swf_url: [0; 512],
            flash_ver: [0; 64],
            audio_codecs: 0,
            video_codecs: 0,
            four_cc_list: FourCcList::default(),
            caps_ex: CapsExit::default(),
            video_four_cc_info_map: VideoFourCcInfoMap::default(),
            reconnect: Reconnect::default(),
            caps_ex_mask: 0,
            has_four_cc_list: false,
            has_caps_ex: false,
            has_video_four_cc_info_map: false,
            has_reconnect: false,
        }
    }
}
