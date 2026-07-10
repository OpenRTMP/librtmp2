//! librtmp2 — A Rust port of the librtmp2 RTMP/RTMPS protocol library.
//!
//! This is a complete 1:1 Rust port of the C library `librtmp2`.
//! It provides server, client, handshake, chunk, message, AMF, FLV, and E-RTMP functionality.

#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(clippy::all)]

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
pub mod message;
pub mod net;
pub mod server;
pub mod session;
pub mod transport;
pub mod types;

// Re-exports for convenience
pub use types::*;

/// Library version string.
pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

/// Get the library version string.
pub fn version_string() -> &'static str {
    VERSION_STRING
}

/// Get the major version number.
pub fn version_major() -> i32 {
    i32::try_from(types::VERSION_MAJOR).unwrap_or(0)
}

/// Get the minor version number.
pub fn version_minor() -> i32 {
    i32::try_from(types::VERSION_MINOR).unwrap_or(0)
}

/// Get the patch version number.
pub fn version_patch() -> i32 {
    i32::try_from(types::VERSION_PATCH).unwrap_or(0)
}

/// Check if TLS support is available.
pub fn tls_supported() -> bool {
    transport::tls_available()
}

/// Get the error string for an error code.
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
/// discriminant for the enum — so the FFI boundary must validate the raw
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

#[cfg(test)]
mod ffi_tests {
    use super::*;
    use crate::server::Server;
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
        };
        let server = Server::new(config).unwrap();
        assert_eq!(server.config.max_connections, 0);
    }
}

use std::ffi::c_int;

/// Default connection cap applied at the FFI boundary when `max_connections`
/// is exactly zero — the usual pattern from `calloc`/`{0}` initialization in
/// C embedders, which would otherwise disable all connection limiting. A
/// negative value is left as-is: `Server` already treats `max_connections <=
/// 0` as unlimited, and callers may pass a negative value deliberately to
/// request that, so only the zero-initialized case needs the default.
const DEFAULT_FFI_MAX_CONNECTIONS: c_int = 256;

/// Create a server (FFI-compatible).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_create(config: *const ServerConfig) -> *mut server::Server {
    if config.is_null() {
        return std::ptr::null_mut();
    }
    let mut cfg = unsafe { *config };
    if cfg.max_connections == 0 {
        cfg.max_connections = DEFAULT_FFI_MAX_CONNECTIONS;
    }
    match server::Server::new(cfg) {
        Ok(s) => Box::into_raw(Box::new(s)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Destroy a server (FFI-compatible).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_destroy(server: *mut server::Server) {
    if !server.is_null() {
        drop(unsafe { Box::from_raw(server) });
    }
}

/// Start listening (FFI-compatible).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_listen(
    server: *mut server::Server,
    bind_addr: *const u8,
) -> i32 {
    if server.is_null() || bind_addr.is_null() {
        return ErrorCode::Internal as i32;
    }
    let s = unsafe { &mut *server };
    let addr = unsafe { std::ffi::CStr::from_ptr(bind_addr as *const std::ffi::c_char) };
    match s.listen(addr.to_str().unwrap_or("")) {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Poll for events (FFI-compatible).
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_server_stop(server: *mut server::Server) {
    if !server.is_null() {
        unsafe { (*server).stop() };
    }
}

/// Create a client (FFI-compatible).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_create(_config: *const ServerConfig) -> *mut client::Client {
    let c = client::Client::new();
    Box::into_raw(Box::new(c))
}

/// Destroy a client (FFI-compatible).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_destroy(c: *mut client::Client) {
    if !c.is_null() {
        drop(unsafe { Box::from_raw(c) });
    }
}

/// Connect (FFI-compatible).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_connect(c: *mut client::Client, url: *const u8) -> i32 {
    if c.is_null() || url.is_null() {
        return ErrorCode::Internal as i32;
    }
    let client = unsafe { &mut *c };
    let url_str = unsafe { std::ffi::CStr::from_ptr(url as *const std::ffi::c_char) };
    match client.connect(url_str.to_str().unwrap_or("")) {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Publish (FFI-compatible).
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_client_send_frame(
    c: *mut client::Client,
    frame: *const Frame,
) -> i32 {
    if c.is_null() || frame.is_null() {
        return ErrorCode::Internal as i32;
    }
    let frame_ref = unsafe { &*frame };
    if unsafe { (*c).state } != client::ClientState::Publishing {
        return ErrorCode::Protocol as i32;
    }
    if frame_ref.size > 0 && frame_ref.data.is_null() {
        return ErrorCode::Internal as i32;
    }
    if frame_ref.size as usize > client::MAX_CLIENT_FRAME_BYTES {
        return ErrorCode::Protocol as i32;
    }
    let payload = if frame_ref.size == 0 || frame_ref.data.is_null() {
        Vec::new()
    } else {
        unsafe {
            std::slice::from_raw_parts(frame_ref.data, frame_ref.size as usize).to_vec()
        }
    };
    match unsafe {
        (*c).send_frame_payload(
            frame_ref.frame_type,
            frame_ref.timestamp,
            &payload,
        )
    } {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Poll client (FFI-compatible).
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_conn_get_fd(_conn: *const session::conn::Conn) -> i32 {
    -1
}

/// Check TLS support (FFI-compatible).
#[unsafe(no_mangle)]
pub extern "C" fn lrtmp2_tls_supported() -> i32 {
    if tls_supported() {
        1
    } else {
        0
    }
}

/// Get version string (FFI-compatible).
#[unsafe(no_mangle)]
pub extern "C" fn lrtmp2_version_string() -> *const u8 {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr()
}

/// Get major version (FFI-compatible).
#[unsafe(no_mangle)]
pub extern "C" fn lrtmp2_version_major() -> i32 {
    version_major()
}

/// Get minor version (FFI-compatible).
#[unsafe(no_mangle)]
pub extern "C" fn lrtmp2_version_minor() -> i32 {
    version_minor()
}

/// Get patch version (FFI-compatible).
#[unsafe(no_mangle)]
pub extern "C" fn lrtmp2_version_patch() -> i32 {
    version_patch()
}

/// Get error string (FFI-compatible).
///
/// Takes a raw `i32` rather than `ErrorCode` so that an out-of-range value
/// from a caller (e.g. a weakly-typed binding forwarding an unrelated int)
/// cannot construct an invalid enum discriminant, which would be undefined
/// behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrtmp2_error_string(code: i32) -> *const u8 {
    error_c_string(error_code_from_raw(code)).as_ptr()
}
