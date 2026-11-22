//! librtmp2 — A Rust port of the librtmp2 RTMP/RTMPS protocol library.
//!
//! `librtmp2` speaks **Legacy RTMP** and **Enhanced RTMP (E-RTMP) v1/v2**: the
//! handshake, chunking, AMF0/AMF3 commands, live publish/play relay, and RTMPS
//! (RTMP over TLS). It is a protocol library, not a media server: the host
//! application installs callbacks and the library delivers semantic events.
//!
//! It exposes two APIs over one implementation:
//!
//! - an idiomatic **Rust API** ([`server::Server`], [`client::Client`],
//!   [`session::conn::Conn`]), and
//! - a stable **`extern "C"` FFI layer** (mainly [`lrtmp2_server_create`] and
//!   [`lrtmp2_client_create`]) for C, Go, Python, PHP and others.
//!
//! # What it does and does not do
//!
//! Supported today:
//!
//! - Live ingest and relay for OBS/FFmpeg-style workflows: H.264/AAC, plus
//!   Enhanced RTMP passthrough for HEVC/AV1/Opus
//! - A minimal RTMP server and client (`connect` → `createStream` →
//!   `publish`/`play`)
//! - RTMPS via the `tls` feature (OpenSSL), **enabled by default**
//! - E-RTMP v1/v2 parsers and serializers: `capsEx` negotiation, multitrack,
//!   ModEx, `fourCcList`, and reconnect signalling
//!
//! Not implemented: the encrypted/Adobe-digest handshake, VOD commands, and
//! multi-client shared-object attribute sync (the envelope is delivered, the
//! policy is left to the host). See the README's *Implementation status* table
//! for the code-accurate breakdown.
//!
//! # Installation
//!
//! ```toml
//! [dependencies]
//! librtmp2 = "0.17"
//! ```
//!
//! For a plaintext-only build without OpenSSL:
//!
//! ```toml
//! [dependencies]
//! librtmp2 = { version = "0.17", default-features = false }
//! ```
//!
//! # Cargo features
//!
//! | Feature | Default | Effect |
//! |---------|---------|--------|
//! | `tls`   | yes     | Enables RTMPS and pulls in `openssl`. Without it, `rtmps://` URLs fail with [`types::ErrorCode::Unsupported`] and [`tls_supported`] returns `false`. |
//!
//! # Example: a minimal relay server
//!
//! ```
//! use librtmp2::server::Server;
//! use librtmp2::types::ServerConfig;
//!
//! # fn main() -> Result<(), librtmp2::types::ErrorCode> {
//! let mut server = Server::from_config(ServerConfig::default(), None)?;
//! // Plaintext RTMP on the standard port. Use `listen_tls` for RTMPS.
//! server.listen("127.0.0.1:1935")?;
//!
//! // Drive the accept/poll loop from your own thread or event loop.
//! // `poll` blocks up to `timeout_ms` and never returns a per-connection error.
//! loop {
//!     server.poll(100)?;
//!     # break;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Example: publishing from the client
//!
//! ```no_run
//! use librtmp2::client::Client;
//! use librtmp2::types::FrameType;
//!
//! # fn main() -> Result<(), librtmp2::types::ErrorCode> {
//! let mut client = Client::new();
//! // Connect performs TCP (and TLS for rtmps://), the handshake, and the
//! // `connect` + `createStream` command exchange in one call.
//! client.connect("rtmp://127.0.0.1:1935/live/cam1")?;
//! client.publish()?;
//!
//! // Send an already-framed audio/video payload (this is an opaque RTMP
//! // tag body, not a raw codec frame).
//! client.send_frame_payload(FrameType::Video, 0, &[0x17, 0x00, 0x00, 0x00, 0x00])?;
//!
//! // Flush outbound data and process inbound control traffic.
//! client.poll(100)?;
//! # Ok(())
//! # }
//! ```
//!
//! # Error handling
//!
//! Nearly every fallible function returns [`types::Result<T>`], i.e.
//! `Result<T, `[`types::ErrorCode`]`>`. [`ErrorCode`] is a
//! small `#[repr(C)]` enum whose discriminants are the negative values the FFI
//! layer returns, so the same classification works across the Rust and C APIs.
//! Use [`error_string`] (or [`types::ErrorCode::as_str`]) for a human-readable
//! description.
//!
//! ```
//! use librtmp2::{error_string, types::ErrorCode};
//!
//! let err = ErrorCode::Auth;
//! assert_eq!(error_string(err), "Authentication error");
//! assert_eq!(err as i32, -8);
//! ```
//!
//! # Threading
//!
//! The library is single-threaded per connection: chunk state and session state
//! are per-connection, never global, so a client and a server can coexist in one
//! process or on separate threads. Drive each [`session::conn::Conn`] or
//! [`client::Client`] from one thread at a time.
//!
//! # Module map
//!
//! Every protocol layer is a module, lowest first:
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | [`alloc`], [`bytes`], [`buffer`], [`log`], [`types`] | Growable buffers, big-endian helpers, logging, error codes and shared types |
//! | [`net`] | Per-OS socket layer and the [`net::RawSocket`] handle type |
//! | [`transport`] | Plaintext and TLS transport on top of [`net`] |
//! | [`handshake`] | C0/C1/C2 ↔ S0/S1/S2 handshake |
//! | [`chunk`] | Chunk reader/writer and per-CSID state |
//! | [`message`] | Reassembled message dispatch: control, commands, shared objects |
//! | [`amf`] | AMF0 (mandatory) and AMF3 |
//! | [`flv`] | FLV audio/video/script tag parsing |
//! | [`ertmp`] | E-RTMP v1 (ExVideo/ExAudio, FourCC, HDR) and v2 (capsEx, reconnect, multitrack, ModEx) |
//! | [`media`] | Init-frame classification, delivery hints, codec config validation |
//! | [`session`] | Connection object, state machine, publish/play flows |
//! | [`server`] | Listening sockets, accept loop, per-connection poll, relay |
//! | [`client`] | Outbound connect → publish/play |
//!
//! Further reading: the README, `docs/protocol-mapping-legacy.md`,
//! `docs/protocol-mapping-ertmp-v1.md` and
//! `docs/protocol-mapping-ertmp-v2.md`, plus runnable programs under
//! `examples/`.
//!
//! # FFI safety
//!
//! The `lrtmp2_*` functions are the C ABI. They take raw pointers, return
//! negative [`ErrorCode`] values on failure, and carry a
//! `# Safety` section describing exactly what the caller must guarantee. A null
//! handle never aborts; it is rejected with an error or a no-op.

#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(clippy::all)]
// Every public item must be documented. The CI enforces this as an error via
// `RUSTDOCFLAGS="-D warnings" cargo doc` and `RUSTFLAGS="-D missing_docs"`.
#![warn(missing_docs)]

pub mod alloc;
pub mod amf;
pub mod buffer;
pub mod bytes;
pub mod chunk;
pub mod client;
pub mod ertmp;
pub mod flv;
pub mod handshake;
pub mod log;
pub mod media;
pub mod message;
pub mod net;
pub mod server;
pub mod session;
pub(crate) mod sha256;
pub mod transport;
pub mod types;

// Re-exports for convenience
pub use media::DeliveryHint;
pub use session::RelayFrame;
pub use types::*;

/// Library version string.
///
/// The same value as [`types::VERSION_STRING`]; both read `CARGO_PKG_VERSION`
/// so the crate version has a single source of truth.
pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

/// Get the library version string.
///
/// ```
/// assert_eq!(librtmp2::version_string(), env!("CARGO_PKG_VERSION"));
/// ```
pub fn version_string() -> &'static str {
    VERSION_STRING
}

/// Get the major version number.
///
/// ```
/// let major: i32 = librtmp2::version_major();
/// assert!(major >= 0);
/// ```
pub fn version_major() -> i32 {
    i32::try_from(types::VERSION_MAJOR).unwrap_or(0)
}

/// Get the minor version number.
///
/// # Examples
///
/// ```
/// assert!(librtmp2::version_minor() >= 0);
/// ```
pub fn version_minor() -> i32 {
    i32::try_from(types::VERSION_MINOR).unwrap_or(0)
}

/// Get the patch version number.
///
/// # Examples
///
/// ```
/// assert!(librtmp2::version_patch() >= 0);
/// ```
pub fn version_patch() -> i32 {
    i32::try_from(types::VERSION_PATCH).unwrap_or(0)
}

/// Check if TLS support is available.
///
/// Returns `false` when the crate was built with `--no-default-features`, in
/// which case `rtmps://` URLs fail with [`types::ErrorCode::Unsupported`].
///
/// ```
/// // Matches the `tls` feature this test binary was built with.
/// assert_eq!(librtmp2::tls_supported(), cfg!(feature = "tls"));
/// ```
pub fn tls_supported() -> bool {
    transport::tls_available()
}

/// Get the error string for an error code.
///
/// Equivalent to [`types::ErrorCode::as_str`]. The returned string is static
/// and NUL-free.
///
/// # Examples
///
/// ```
/// use librtmp2::types::ErrorCode;
///
/// assert_eq!(librtmp2::error_string(ErrorCode::Timeout), "Timeout");
/// assert_eq!(librtmp2::error_string(ErrorCode::Amf), "AMF error");
/// ```
pub fn error_string(code: ErrorCode) -> &'static str {
    code.as_str()
}

fn error_c_string(code: ErrorCode) -> &'static [u8] {
    match code {
        ErrorCode::Ok => b"OK\0",
        ErrorCode::Io => b"I/O error\0",
        ErrorCode::Timeout => b"Timeout\0",
        ErrorCode::Protocol => b"Protocol error\0",
        ErrorCode::Handshake => b"Handshake error\0",
        ErrorCode::Chunk => b"Chunk error\0",
        ErrorCode::Amf => b"AMF error\0",
        ErrorCode::Unsupported => b"Unsupported\0",
        ErrorCode::Auth => b"Authentication error\0",
        ErrorCode::Internal => b"Internal error\0",
    }
}

/// Map a raw FFI error code integer to `ErrorCode`, falling back to
/// `Internal` for any value a caller passes that isn't one of ours.
/// Constructing an `ErrorCode` directly from an arbitrary caller-supplied
/// value would be undefined behavior, since not every `i32` is a valid
/// discriminant for the enum â€” so the FFI boundary must validate the raw
/// `i32` first.
fn error_code_from_raw(code: i32) -> ErrorCode {
    match code {
        0 => ErrorCode::Ok,
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
    }
}

/// Map a raw `FrameType` discriminant without constructing an invalid enum
/// value (which would be undefined behavior at the FFI boundary).
fn frame_type_from_raw(raw: i32) -> Result<FrameType> {
    match raw {
        0 => Ok(FrameType::Audio),
        1 => Ok(FrameType::Video),
        2 => Ok(FrameType::Script),
        3 => Ok(FrameType::Metadata),
        _ => Err(ErrorCode::Internal),
    }
}

/// Leading fields of [`Frame`] read as plain scalars so invalid enum
/// discriminants never instantiate a `Frame` reference.
#[repr(C)]
struct FrameSendFields {
    frame_type: i32,
    timestamp: u32,
    composition_time: u32,
    size: u32,
    data: *const u8,
}

// `FrameSendFields` intentionally reinterprets the leading `FrameType` field
// as its raw C ABI integer. Keep every layout assumption explicit so changes
// to either struct fail compilation instead of silently shifting FFI reads.
const _: [(); std::mem::size_of::<i32>()] = [(); std::mem::size_of::<FrameType>()];
const _: [(); std::mem::align_of::<i32>()] = [(); std::mem::align_of::<FrameType>()];
const _: [(); std::mem::offset_of!(Frame, frame_type)] =
    [(); std::mem::offset_of!(FrameSendFields, frame_type)];
const _: [(); std::mem::offset_of!(Frame, timestamp)] =
    [(); std::mem::offset_of!(FrameSendFields, timestamp)];
const _: [(); std::mem::offset_of!(Frame, composition_time)] =
    [(); std::mem::offset_of!(FrameSendFields, composition_time)];
const _: [(); std::mem::offset_of!(Frame, size)] =
    [(); std::mem::offset_of!(FrameSendFields, size)];
const _: [(); std::mem::offset_of!(Frame, data)] =
    [(); std::mem::offset_of!(FrameSendFields, data)];
const _: [(); std::mem::size_of::<FrameSendFields>()] =
    [(); std::mem::offset_of!(Frame, audio_codec)];

#[cfg(test)]
mod ffi_tests {
    use super::*;
    use crate::server::Server;
    use crate::session::conn::Conn;
    use std::ptr::NonNull;

    #[test]
    fn server_create_applies_default_max_connections_for_zero_config() {
        let config = ServerConfig {
            max_connections: 0,
            chunk_size: 128,
            tls_enabled: 0,
            tls_cert_file: std::ptr::null(),
            tls_key_file: std::ptr::null(),
            tls_ca_file: std::ptr::null(),
            tls_insecure: 0,
            max_pending_tls_per_addr: 0,
            max_connections_per_addr: 0,
        };
        let server = NonNull::new(unsafe { lrtmp2_server_create(&config) })
            .expect("lrtmp2_server_create returned null");
        unsafe {
            assert_eq!(
                server.as_ref().config.max_connections,
                DEFAULT_FFI_MAX_CONNECTIONS
            );
            lrtmp2_server_destroy(server.as_ptr());
        }
    }

    /// Test callback that allows every request.
    unsafe extern "C" fn allow_cb(
        _conn_id: u64,
        _app: *const std::ffi::c_char,
        _stream: *const std::ffi::c_char,
        _userdata: *mut std::ffi::c_void,
    ) -> std::ffi::c_int {
        0
    }

    #[test]
    fn server_c_auth_callback_setters_store_the_callback_and_userdata() {
        let config = ServerConfig::default();
        let mut server = NonNull::new(unsafe { lrtmp2_server_create(&config) })
            .expect("lrtmp2_server_create returned null");
        let userdata = 0x1234usize as *mut std::ffi::c_void;
        unsafe {
            // Simulate a connection accepted before the callback is installed.
            let mut conn = Conn::new();
            conn.c_publish_auth.userdata = 0x99;
            conn.c_play_auth.userdata = 0x99;
            server.as_mut().connections.push(conn);

            assert_eq!(
                lrtmp2_server_set_publish_auth_cb(server.as_ptr(), Some(allow_cb), userdata),
                0
            );
            assert_eq!(
                lrtmp2_server_set_play_auth_cb(server.as_ptr(), Some(allow_cb), userdata),
                0
            );
            assert_eq!(server.as_ref().c_publish_auth.userdata, userdata as usize);
            assert_eq!(server.as_ref().c_play_auth.userdata, userdata as usize);
            assert!(server.as_ref().c_publish_auth.callback.is_some());
            assert!(server.as_ref().c_play_auth.callback.is_some());
            assert_eq!(
                server.as_ref().connections[0].c_publish_auth.userdata,
                userdata as usize
            );
            assert_eq!(
                server.as_ref().connections[0].c_play_auth.userdata,
                userdata as usize
            );
            assert!(
                server.as_ref().connections[0]
                    .c_publish_auth
                    .callback
                    .is_some()
            );
            assert!(
                server.as_ref().connections[0]
                    .c_play_auth
                    .callback
                    .is_some()
            );

            assert_eq!(
                lrtmp2_server_set_publish_auth_cb(server.as_ptr(), None, std::ptr::null_mut()),
                0
            );
            assert!(server.as_ref().c_publish_auth.callback.is_none());
            assert!(
                server.as_ref().connections[0]
                    .c_publish_auth
                    .callback
                    .is_none()
            );
            assert_eq!(
                lrtmp2_server_set_play_auth_cb(std::ptr::null_mut(), None, std::ptr::null_mut()),
                -1
            );
            lrtmp2_server_destroy(server.as_ptr());
        }
    }

    #[test]
    fn server_create_preserves_negative_max_connections_as_unlimited() {
        let config = ServerConfig {
            max_connections: -1,
            chunk_size: 128,
            tls_enabled: 0,
            tls_cert_file: std::ptr::null(),
            tls_key_file: std::ptr::null(),
            tls_ca_file: std::ptr::null(),
            tls_insecure: 0,
            max_pending_tls_per_addr: 0,
            max_connections_per_addr: 0,
        };
        let server = NonNull::new(unsafe { lrtmp2_server_create(&config) })
            .expect("lrtmp2_server_create returned null");
        unsafe {
            // Only the zero-initialized case gets the default; an explicit
            // negative value must pass through unchanged since `Server`
            // already treats `max_connections <= 0` as unlimited.
            assert_eq!(server.as_ref().config.max_connections, -1);
            lrtmp2_server_destroy(server.as_ptr());
        }
    }

    #[test]
    fn server_new_preserves_zero_max_connections_for_rust_api() {
        let config = ServerConfig {
            max_connections: 0,
            chunk_size: 128,
            tls_enabled: 0,
            tls_cert_file: std::ptr::null(),
            tls_key_file: std::ptr::null(),
            tls_ca_file: std::ptr::null(),
            tls_insecure: 0,
            max_pending_tls_per_addr: 0,
            max_connections_per_addr: 0,
        };
        let server = Server::from_config(config, None).unwrap();
        assert_eq!(server.config.max_connections, 0);
    }

    #[test]
    fn server_listen_rejects_invalid_utf8_bind_addr() {
        let config = ServerConfig {
            max_connections: 8,
            chunk_size: 128,
            tls_enabled: 0,
            tls_cert_file: std::ptr::null(),
            tls_key_file: std::ptr::null(),
            tls_ca_file: std::ptr::null(),
            tls_insecure: 0,
            max_pending_tls_per_addr: 0,
            max_connections_per_addr: 0,
        };
        let server = NonNull::new(unsafe { lrtmp2_server_create(&config) })
            .expect("lrtmp2_server_create returned null");
        let invalid = [0xFF, b':', b'1', b'9', b'3', b'5', 0];
        let rc = unsafe {
            lrtmp2_server_listen(server.as_ptr(), invalid.as_ptr() as *const std::ffi::c_char)
        };
        unsafe {
            lrtmp2_server_destroy(server.as_ptr());
        }
        assert_eq!(rc, ErrorCode::Internal as i32);
    }

    #[test]
    fn frame_type_from_raw_rejects_invalid_discriminants() {
        assert_eq!(frame_type_from_raw(0).unwrap(), FrameType::Audio);
        assert_eq!(frame_type_from_raw(3).unwrap(), FrameType::Metadata);
        assert!(frame_type_from_raw(4).is_err());
        assert!(frame_type_from_raw(-1).is_err());
    }

    #[test]
    fn client_send_frame_checks_state_before_reading_frame() {
        let client = Box::into_raw(Box::new(client::Client::new()));
        let dangling_frame = NonNull::<Frame>::dangling().as_ptr();

        let rc = unsafe { lrtmp2_client_send_frame(client, dangling_frame) };

        unsafe { lrtmp2_client_destroy(client) };
        assert_eq!(rc, ErrorCode::Protocol as i32);
    }
}

use std::ffi::c_int;

/// Default connection cap applied at the FFI boundary when `max_connections`
/// is exactly zero â€” the usual pattern from `calloc`/`{0}` initialization in
/// C embedders, which would otherwise disable all connection limiting. A
/// negative value is left as-is: `Server` already treats `max_connections <=
/// 0` as unlimited, and callers may pass a negative value deliberately to
/// request that, so only the zero-initialized case needs the default.
const DEFAULT_FFI_MAX_CONNECTIONS: c_int = 256;

/// Create a server (FFI-compatible).
///
/// # Safety
///
/// `config` must point to a readable [`ServerConfig`]. When
/// `(*config).tls_enabled != 0`, `tls_cert_file` and `tls_key_file` must each be
/// a pointer to a NUL-terminated C string (valid UTF-8) that stays alive for
/// the duration of this call; see [`server::Server::new`].
///
/// # Examples
///
/// ```
/// # use librtmp2::types::ServerConfig;
/// # unsafe {
/// // A zero-initialised config is the usual C pattern; `max_connections: 0`
/// // means "use the library default" at this boundary.
/// let cfg = ServerConfig::default();
/// let server = librtmp2::lrtmp2_server_create(&cfg);
/// assert!(!server.is_null());
/// librtmp2::lrtmp2_server_destroy(server);
///
/// // A null config is rejected rather than dereferenced.
/// assert!(librtmp2::lrtmp2_server_create(std::ptr::null()).is_null());
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_create(config: *const ServerConfig) -> *mut server::Server {
    if config.is_null() {
        return std::ptr::null_mut();
    }
    let mut cfg = unsafe { *config };
    if cfg.max_connections == 0 {
        cfg.max_connections = DEFAULT_FFI_MAX_CONNECTIONS;
    }
    // SAFETY: the caller guarantees the pointers in `cfg` are valid, NUL-
    // terminated C strings (see this function's `# Safety` section).
    match unsafe { server::Server::new(cfg) } {
        Ok(s) => Box::into_raw(Box::new(s)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Install a C publish-authorization callback (FFI-compatible).
///
/// The callback runs synchronously during the RTMP `publish` command. Return
/// `0` to allow the publish and any non-zero value to deny it. `userdata` is
/// passed back unchanged and must stay valid until the callback is replaced or
/// the server is destroyed. Passing a null callback clears it and restores the
/// library's default allow behavior for embedders that have no callback.
///
/// Do not call any function on the same server from the callback. Do not call
/// this setter while `lrtmp2_server_poll` is running on that server.
///
/// Returns `0` on success and a negative value when `server` is null.
///
/// # Safety
///
/// `server` must be a live pointer returned by [`lrtmp2_server_create`] that
/// has not been destroyed, and it must not be in use by a concurrent
/// `lrtmp2_server_poll` on another thread. When `cb` is non-null, `userdata`
/// must stay valid until the callback is replaced or the server is destroyed.
///
/// # Examples
///
/// ```
/// use librtmp2::types::ServerConfig;
///
/// unsafe extern "C" fn allow(
///     _conn_id: u64,
///     _app: *const std::ffi::c_char,
///     _stream: *const std::ffi::c_char,
///     _userdata: *mut std::ffi::c_void,
/// ) -> std::ffi::c_int {
///     0 // 0 = allow
/// }
///
/// # unsafe {
/// let server = librtmp2::lrtmp2_server_create(&ServerConfig::default());
/// assert_eq!(
///     librtmp2::lrtmp2_server_set_publish_auth_cb(server, Some(allow), std::ptr::null_mut()),
///     0
/// );
/// // Passing a null callback restores the default allow policy.
/// assert_eq!(
///     librtmp2::lrtmp2_server_set_publish_auth_cb(server, None, std::ptr::null_mut()),
///     0
/// );
/// librtmp2::lrtmp2_server_destroy(server);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_set_publish_auth_cb(
    server: *mut server::Server,
    cb: Option<
        unsafe extern "C" fn(
            conn_id: u64,
            app: *const std::ffi::c_char,
            stream_name: *const std::ffi::c_char,
            userdata: *mut std::ffi::c_void,
        ) -> std::ffi::c_int,
    >,
    userdata: *mut std::ffi::c_void,
) -> i32 {
    if server.is_null() {
        return -1;
    }
    // SAFETY: the caller passed a live server pointer created by
    // `lrtmp2_server_create` and keeps it valid for this call.
    let s = unsafe { &mut *server };
    s.c_publish_auth = crate::types::CAuthState {
        callback: cb,
        userdata: userdata as usize,
    };
    // Connections copy the callback at accept time. Update the live ones too:
    // otherwise a replaced callback keeps using the old userdata after the
    // embedder has freed it, and a newly installed policy is ignored.
    for conn in &mut s.connections {
        conn.c_publish_auth = s.c_publish_auth;
    }
    0
}

/// Install a C play-authorization callback (FFI-compatible).
///
/// See [`lrtmp2_server_set_publish_auth_cb`] for the return convention,
/// `userdata` lifetime and re-entrancy rules. Passing a null callback clears
/// it.
///
/// # Safety
///
/// `server` must be a live pointer returned by [`lrtmp2_server_create`] that
/// has not been destroyed, and it must not be in use by a concurrent
/// `lrtmp2_server_poll` on another thread. When `cb` is non-null, `userdata`
/// must stay valid until the callback is replaced or the server is destroyed.
///
/// # Examples
///
/// ```
/// use librtmp2::types::ServerConfig;
///
/// unsafe extern "C" fn allow(
///     _conn_id: u64,
///     _app: *const std::ffi::c_char,
///     _stream: *const std::ffi::c_char,
///     _userdata: *mut std::ffi::c_void,
/// ) -> std::ffi::c_int {
///     0
/// }
///
/// # unsafe {
/// let server = librtmp2::lrtmp2_server_create(&ServerConfig::default());
/// assert_eq!(
///     librtmp2::lrtmp2_server_set_play_auth_cb(server, Some(allow), std::ptr::null_mut()),
///     0
/// );
/// // A null server pointer is rejected with a negative value.
/// assert_eq!(
///     librtmp2::lrtmp2_server_set_play_auth_cb(std::ptr::null_mut(), None, std::ptr::null_mut()),
///     -1
/// );
/// librtmp2::lrtmp2_server_destroy(server);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_set_play_auth_cb(
    server: *mut server::Server,
    cb: Option<
        unsafe extern "C" fn(
            conn_id: u64,
            app: *const std::ffi::c_char,
            stream_name: *const std::ffi::c_char,
            userdata: *mut std::ffi::c_void,
        ) -> std::ffi::c_int,
    >,
    userdata: *mut std::ffi::c_void,
) -> i32 {
    if server.is_null() {
        return -1;
    }
    // SAFETY: see `lrtmp2_server_set_publish_auth_cb`.
    let s = unsafe { &mut *server };
    s.c_play_auth = crate::types::CAuthState {
        callback: cb,
        userdata: userdata as usize,
    };
    for conn in &mut s.connections {
        conn.c_play_auth = s.c_play_auth;
    }
    0
}

/// Destroy a server (FFI-compatible).
///
/// A null pointer is accepted and does nothing.
///
/// # Safety
///
/// `server` must be either null or a pointer returned by
/// [`lrtmp2_server_create`] that has not already been destroyed. After this
/// call the pointer is dangling and must not be used again — in particular, no
/// connection callback installed on the server may retain it.
///
/// # Examples
///
/// ```
/// # use librtmp2::types::ServerConfig;
/// # unsafe {
/// let server = librtmp2::lrtmp2_server_create(&ServerConfig::default());
/// librtmp2::lrtmp2_server_destroy(server);
///
/// // Destroying null is a no-op, so a failed create needs no special case.
/// librtmp2::lrtmp2_server_destroy(std::ptr::null_mut());
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_destroy(server: *mut server::Server) {
    if !server.is_null() {
        drop(unsafe { Box::from_raw(server) });
    }
}

/// Start listening (FFI-compatible).
///
/// `bind_addr` is a NUL-terminated UTF-8 `"host:port"` string (port defaults to
/// 1935). Returns `0` on success or a negative [`ErrorCode`] value; a null
/// pointer or invalid UTF-8 yields [`ErrorCode::Internal`] rather than binding
/// `0.0.0.0`.
///
/// # Safety
///
/// `server` must be a live pointer returned by [`lrtmp2_server_create`] that has
/// not been destroyed. `bind_addr` must be null or point to a NUL-terminated C
/// string valid for the duration of the call. The server must not be in use by a
/// concurrent `lrtmp2_server_poll` on another thread.
///
/// # Examples
///
/// ```
/// use std::ffi::CString;
/// use librtmp2::types::ServerConfig;
///
/// # unsafe {
/// let server = librtmp2::lrtmp2_server_create(&ServerConfig::default());
/// let addr = CString::new("127.0.0.1:0").unwrap();
/// assert_eq!(librtmp2::lrtmp2_server_listen(server, addr.as_ptr()), 0);
///
/// // A null address is rejected instead of binding 0.0.0.0.
/// assert_eq!(
///     librtmp2::lrtmp2_server_listen(server, std::ptr::null()),
///     librtmp2::types::ErrorCode::Internal as i32
/// );
/// librtmp2::lrtmp2_server_destroy(server);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_listen(
    server: *mut server::Server,
    bind_addr: *const std::ffi::c_char,
) -> i32 {
    if server.is_null() || bind_addr.is_null() {
        return ErrorCode::Internal as i32;
    }
    let s = unsafe { &mut *server };
    let addr = unsafe { std::ffi::CStr::from_ptr(bind_addr) };
    let addr_str = match addr.to_str() {
        Ok(s) => s,
        // Invalid UTF-8 must not fall back to "" (which binds 0.0.0.0).
        Err(_) => return ErrorCode::Internal as i32,
    };
    match s.listen(addr_str) {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Poll for events (FFI-compatible).
///
/// Blocks up to `timeout_ms` milliseconds, then accepts connections, advances
/// every connection, and relays media. Returns `0` on success or a negative
/// [`ErrorCode`] value.
///
/// # Safety
///
/// `server` must be a live pointer returned by [`lrtmp2_server_create`] that has
/// not been destroyed. Because this call mutates the whole server, no other
/// function may be called on the same server concurrently (including from a
/// callback it fires), and `lrtmp2_server_destroy` must not run until the call
/// returns.
///
/// # Examples
///
/// ```
/// use std::ffi::CString;
/// use librtmp2::types::ServerConfig;
///
/// # unsafe {
/// let server = librtmp2::lrtmp2_server_create(&ServerConfig::default());
/// let addr = CString::new("127.0.0.1:0").unwrap();
/// librtmp2::lrtmp2_server_listen(server, addr.as_ptr());
///
/// // Non-blocking tick: returns 0 when nothing happened.
/// assert_eq!(librtmp2::lrtmp2_server_poll(server, 0), 0);
/// librtmp2::lrtmp2_server_destroy(server);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_poll(server: *mut server::Server, timeout_ms: i32) -> i32 {
    if server.is_null() {
        return ErrorCode::Internal as i32;
    }
    let s = unsafe { &mut *server };
    match s.poll(timeout_ms) {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Stop the server (FFI-compatible).
///
/// Clears the running flag so a subsequent [`lrtmp2_server_poll`] returns
/// [`ErrorCode::Internal`]. Safe on a null pointer (no-op).
///
/// # Safety
///
/// `server` must be null or a live pointer returned by [`lrtmp2_server_create`]
/// that has not been destroyed, and it must not be in use by a concurrent
/// `lrtmp2_server_poll` on another thread.
///
/// # Examples
///
/// ```
/// # use librtmp2::types::ServerConfig;
/// # unsafe {
/// let server = librtmp2::lrtmp2_server_create(&ServerConfig::default());
/// librtmp2::lrtmp2_server_stop(server);
///
/// // After stopping, polling reports an error rather than blocking.
/// assert_ne!(librtmp2::lrtmp2_server_poll(server, 0), 0);
/// librtmp2::lrtmp2_server_destroy(server);
///
/// // Stopping null is a no-op.
/// librtmp2::lrtmp2_server_stop(std::ptr::null_mut());
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_stop(server: *mut server::Server) {
    if !server.is_null() {
        unsafe { (*server).stop() };
    }
}

/// Create a client (FFI-compatible). `config` may be NULL to use defaults
/// (verify `rtmps://` peers against the system trust store). When non-NULL,
/// `config.tls_ca_file` (a CA bundle to trust instead of the system store)
/// and `config.tls_insecure` (must be exactly `1` to skip verification, for
/// testing only) control `rtmps://` verification for this client. Returns NULL if
/// `tls_ca_file` is non-NULL but not valid UTF-8, rather than silently
/// falling back to default verification.
///
/// # Safety
///
/// `config` must be null or point to a readable [`ServerConfig`] valid for the
/// duration of the call. When `config.tls_ca_file` is non-null it must point to
/// a NUL-terminated C string (invalid UTF-8 is rejected with a null return). The
/// other pointer fields are not dereferenced by this function.
///
/// # Examples
///
/// ```
/// # unsafe {
/// // A null config means "use defaults", i.e. verify rtmps:// against the
/// // system trust store.
/// let client = librtmp2::lrtmp2_client_create(std::ptr::null());
/// assert!(!client.is_null());
/// librtmp2::lrtmp2_client_destroy(client);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_create(config: *const ServerConfig) -> *mut client::Client {
    let mut c = client::Client::new();
    if !config.is_null() {
        let cfg = unsafe { &*config };
        let ca_file = if cfg.tls_ca_file.is_null() {
            None
        } else {
            match unsafe { std::ffi::CStr::from_ptr(cfg.tls_ca_file as *const std::ffi::c_char) }
                .to_str()
            {
                Ok(s) => Some(s.to_string()),
                Err(_) => return std::ptr::null_mut(),
            }
        };
        // Require an explicit `1` so uninitialized stack garbage cannot silently
        // enable certificate verification bypass (see issue #89 pattern for
        // `max_connections`).
        c.set_tls_client_config(ca_file, cfg.tls_insecure == 1);
    }
    Box::into_raw(Box::new(c))
}

/// Destroy a client (FFI-compatible).
///
/// A null pointer is accepted and does nothing.
///
/// # Safety
///
/// `c` must be either null or a pointer returned by [`lrtmp2_client_create`]
/// that has not already been destroyed. After this call the pointer is dangling
/// and must not be used again, including from any callback it installed.
///
/// # Examples
///
/// ```
/// # unsafe {
/// let client = librtmp2::lrtmp2_client_create(std::ptr::null());
/// librtmp2::lrtmp2_client_destroy(client);
///
/// // Destroying null is a no-op.
/// librtmp2::lrtmp2_client_destroy(std::ptr::null_mut());
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_destroy(c: *mut client::Client) {
    if !c.is_null() {
        drop(unsafe { Box::from_raw(c) });
    }
}

/// Connect (FFI-compatible).
///
/// `url` is a NUL-terminated `rtmp://…` or `rtmps://…` URL. Returns `0` on
/// success or a negative [`ErrorCode`] value.
///
/// # Safety
///
/// `c` must be a live pointer returned by [`lrtmp2_client_create`] that has not
/// been destroyed, and `url` must be null or point to a NUL-terminated C string
/// valid for the duration of the call. A non-UTF-8 `url` is treated as an empty
/// URL and fails the connect. The client must not be used concurrently from
/// another thread.
///
/// # Examples
///
/// ```
/// use std::ffi::CString;
///
/// # unsafe {
/// let client = librtmp2::lrtmp2_client_create(std::ptr::null());
/// let url = CString::new("rtmp://127.0.0.1:1/live/cam1").unwrap();
///
/// // Nothing is listening on port 1, so this fails; the point is the call shape.
/// assert_ne!(librtmp2::lrtmp2_client_connect(client, url.as_ptr()), 0);
///
/// // A null URL is rejected.
/// assert_eq!(
///     librtmp2::lrtmp2_client_connect(client, std::ptr::null()),
///     librtmp2::types::ErrorCode::Internal as i32
/// );
/// librtmp2::lrtmp2_client_destroy(client);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_connect(
    c: *mut client::Client,
    url: *const std::ffi::c_char,
) -> i32 {
    if c.is_null() || url.is_null() {
        return ErrorCode::Internal as i32;
    }
    let client = unsafe { &mut *c };
    let url_str = unsafe { std::ffi::CStr::from_ptr(url) };
    match client.connect(url_str.to_str().unwrap_or("")) {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Publish (FFI-compatible).
///
/// Requires a successful [`lrtmp2_client_connect`] first. Returns `0` on success
/// or a negative [`ErrorCode`] value.
///
/// # Safety
///
/// `c` must be a live pointer returned by [`lrtmp2_client_create`] that has not
/// been destroyed, and it must not be used concurrently from another thread.
///
/// # Examples
///
/// ```
/// # unsafe {
/// let client = librtmp2::lrtmp2_client_create(std::ptr::null());
///
/// // Publishing before connecting is a protocol error, not a crash.
/// assert_eq!(
///     librtmp2::lrtmp2_client_publish(client),
///     librtmp2::types::ErrorCode::Protocol as i32
/// );
/// librtmp2::lrtmp2_client_destroy(client);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_publish(c: *mut client::Client) -> i32 {
    if c.is_null() {
        return ErrorCode::Internal as i32;
    }
    match unsafe { (*c).publish() } {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Play (FFI-compatible).
///
/// Requires a successful [`lrtmp2_client_connect`] first. Returns `0` on success
/// or a negative [`ErrorCode`] value. Received frames are delivered to the Rust
/// `on_frame_cb`, which has no C equivalent yet.
///
/// # Safety
///
/// `c` must be a live pointer returned by [`lrtmp2_client_create`] that has not
/// been destroyed, and it must not be used concurrently from another thread.
///
/// # Examples
///
/// ```
/// # unsafe {
/// let client = librtmp2::lrtmp2_client_create(std::ptr::null());
///
/// // Playing before connecting is a protocol error.
/// assert_eq!(
///     librtmp2::lrtmp2_client_play(client),
///     librtmp2::types::ErrorCode::Protocol as i32
/// );
/// librtmp2::lrtmp2_client_destroy(client);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_play(c: *mut client::Client) -> i32 {
    if c.is_null() {
        return ErrorCode::Internal as i32;
    }
    match unsafe { (*c).play() } {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Send a frame (FFI-compatible).
///
/// `frame` must describe a complete RTMP tag body. `frame.data`/`frame.size`
/// are copied before the call returns, so they need only be valid during it.
/// Frames larger than `client::MAX_CLIENT_FRAME_BYTES` are rejected with
/// [`ErrorCode::Protocol`]. Requires the client to be publishing. Returns `0` on
/// success or a negative [`ErrorCode`] value.
///
/// # Safety
///
/// `c` must be a live pointer returned by [`lrtmp2_client_create`] that has not
/// been destroyed. `frame` must point to a readable [`Frame`] whose enum
/// discriminants are valid and whose `data` points to `size` readable bytes (or
/// is null when `size` is `0`) for the duration of the call. The client must not
/// be used concurrently from another thread.
///
/// # Examples
///
/// ```
/// use librtmp2::types::{Frame, FrameType};
///
/// # unsafe {
/// let client = librtmp2::lrtmp2_client_create(std::ptr::null());
///
/// // Sending before the client is Publishing is rejected before the frame is
/// // ever read, so `frame.data` need not be valid here.
/// assert_eq!(
///     librtmp2::lrtmp2_client_send_frame(client, std::ptr::null()),
///     librtmp2::types::ErrorCode::Internal as i32
/// );
///
/// // A well-formed frame is still refused while disconnected.
/// let payload = [0x17u8, 0x01];
/// let frame = Frame {
///     frame_type: FrameType::Video,
///     size: payload.len() as u32,
///     data: payload.as_ptr(),
///     ..Frame::default()
/// };
/// assert_eq!(
///     librtmp2::lrtmp2_client_send_frame(client, &frame),
///     librtmp2::types::ErrorCode::Protocol as i32
/// );
/// librtmp2::lrtmp2_client_destroy(client);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_send_frame(
    c: *mut client::Client,
    frame: *const Frame,
) -> i32 {
    if c.is_null() || frame.is_null() {
        return ErrorCode::Internal as i32;
    }
    if unsafe { (*c).state } != client::ClientState::Publishing {
        return ErrorCode::Protocol as i32;
    }
    let fields = unsafe { &*(frame as *const Frame as *const FrameSendFields) };
    let frame_type = match frame_type_from_raw(fields.frame_type) {
        Ok(t) => t,
        Err(e) => return e as i32,
    };
    if fields.size > 0 && fields.data.is_null() {
        return ErrorCode::Internal as i32;
    }
    if fields.size as usize > client::MAX_CLIENT_FRAME_BYTES {
        return ErrorCode::Protocol as i32;
    }
    let payload = if fields.size == 0 || fields.data.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(fields.data, fields.size as usize).to_vec() }
    };
    match unsafe { (*c).send_frame_payload(frame_type, fields.timestamp, &payload) } {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Poll client (FFI-compatible).
///
/// Blocks up to `timeout_ms` milliseconds, flushes outbound data, and services
/// inbound control traffic. Returns `0` on success or a negative [`ErrorCode`]
/// value.
///
/// # Safety
///
/// `c` must be a live pointer returned by [`lrtmp2_client_create`] that has not
/// been destroyed, and it must not be used concurrently from another thread or
/// from inside a callback it fires.
///
/// # Examples
///
/// ```
/// # unsafe {
/// let client = librtmp2::lrtmp2_client_create(std::ptr::null());
///
/// // Polling a client that never connected reports an error.
/// assert_ne!(librtmp2::lrtmp2_client_poll(client, 0), 0);
///
/// // A null pointer is rejected.
/// assert_eq!(
///     librtmp2::lrtmp2_client_poll(std::ptr::null_mut(), 0),
///     librtmp2::types::ErrorCode::Internal as i32
/// );
/// librtmp2::lrtmp2_client_destroy(client);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_poll(c: *mut client::Client, timeout_ms: i32) -> i32 {
    if c.is_null() {
        return ErrorCode::Internal as i32;
    }
    match unsafe { (*c).poll(timeout_ms) } {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Get connection fd (FFI-compatible).
///
/// Reserved for ABI compatibility: it currently always returns `-1` and never
/// touches `_conn`.
///
/// # Safety
///
/// The pointer is never dereferenced, so any pointer value — including null — is
/// accepted. It is still marked `unsafe` to keep the C signature stable.
///
/// # Examples
///
/// ```
/// # unsafe {
/// // Reserved for ABI compatibility: the pointer is never dereferenced and the
/// // call always reports "no fd".
/// assert_eq!(librtmp2::lrtmp2_conn_get_fd(std::ptr::null()), -1);
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_conn_get_fd(_conn: *const session::conn::Conn) -> i32 {
    -1
}

/// Check TLS support (FFI-compatible).
///
/// # Examples
///
/// ```
/// let v = librtmp2::lrtmp2_tls_supported();
/// assert!(v == 0 || v == 1);
/// assert_eq!(v == 1, librtmp2::tls_supported());
/// ```
#[unsafe(no_mangle)]
pub extern "C" fn lrtmp2_tls_supported() -> i32 {
    if tls_supported() { 1 } else { 0 }
}

/// Get version string (FFI-compatible).
///
/// # Examples
///
/// ```
/// # unsafe {
/// let p = librtmp2::lrtmp2_version_string();
/// assert!(!p.is_null());
/// // NUL-terminated and stable for the life of the process.
/// let s = std::ffi::CStr::from_ptr(p).to_str().unwrap();
/// assert!(!s.is_empty());
/// # }
/// ```
#[unsafe(no_mangle)]
pub extern "C" fn lrtmp2_version_string() -> *const std::ffi::c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const std::ffi::c_char
}

/// Get major version (FFI-compatible).
///
/// # Examples
///
/// ```
/// assert_eq!(librtmp2::lrtmp2_version_major(), librtmp2::version_major());
/// ```
#[unsafe(no_mangle)]
pub extern "C" fn lrtmp2_version_major() -> i32 {
    version_major()
}

/// Get minor version (FFI-compatible).
///
/// # Examples
///
/// ```
/// assert_eq!(librtmp2::lrtmp2_version_minor(), librtmp2::version_minor());
/// ```
#[unsafe(no_mangle)]
pub extern "C" fn lrtmp2_version_minor() -> i32 {
    version_minor()
}

/// Get patch version (FFI-compatible).
///
/// # Examples
///
/// ```
/// assert_eq!(librtmp2::lrtmp2_version_patch(), librtmp2::version_patch());
/// ```
#[unsafe(no_mangle)]
pub extern "C" fn lrtmp2_version_patch() -> i32 {
    version_patch()
}

/// Get error string (FFI-compatible).
///
/// Takes a raw `i32` rather than `ErrorCode` so that an out-of-range value
/// from a caller (e.g. a weakly-typed binding forwarding an unrelated int)
/// cannot construct an invalid enum discriminant, which would be undefined
/// behavior. Out-of-range values map to [`ErrorCode::Internal`].
///
/// # Safety
///
/// The returned pointer is a `'static`, NUL-terminated string owned by the
/// library. It must not be modified or freed, and stays valid for the whole
/// process, so callers may keep it.
///
/// # Examples
///
/// ```
/// # unsafe {
/// let p = librtmp2::lrtmp2_error_string(librtmp2::types::ErrorCode::Timeout as i32);
/// assert!(!p.is_null());
/// let s = std::ffi::CStr::from_ptr(p).to_str().unwrap();
/// assert_eq!(s, "Timeout");
///
/// // Out-of-range codes map to the internal error rather than misbehaving.
/// let p = librtmp2::lrtmp2_error_string(9999);
/// assert_eq!(std::ffi::CStr::from_ptr(p).to_str().unwrap(), "Internal error");
/// # }
/// ```
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_error_string(code: i32) -> *const std::ffi::c_char {
    error_c_string(error_code_from_raw(code)).as_ptr() as *const std::ffi::c_char
}
