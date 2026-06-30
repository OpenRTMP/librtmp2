//! Plaintext + optional TLS byte transport.
//!
//! Mirrors `src/core/transport.h` and `src/core/transport.c`.
//!
//! The plaintext path is always available. The TLS path is feature-gated
//! behind the "tls" feature (OpenSSL).

use crate::types::Result;
use crate::types::ErrorCode;

/// Transport wraps a connected socket fd and presents a single send/recv API.
pub struct Transport {
    /// underlying socket file descriptor
    fd: i32,
    /// whether TLS is enabled
    is_tls: bool,
    /// TLS handshake state (server-side)
    #[cfg(feature = "tls")]
    tls_hs_state: i8,
    #[cfg(feature = "tls")]
    tls_hs_want_write: bool,
}

/// Server-side TLS context: holds the certificate/key shared across connections.
pub struct TlsCtx {
    #[cfg(feature = "tls")]
    cert_file: String,
    #[cfg(feature = "tls")]
    key_file: String,
}

impl Transport {
    /// Wrap a connected fd as a plaintext transport.
    pub fn new_plain(fd: i32) -> Self {
        Self {
            fd,
            is_tls: false,
            #[cfg(feature = "tls")]
            tls_hs_state: 0,
            #[cfg(feature = "tls")]
            tls_hs_want_write: false,
        }
    }

    /// Get the underlying file descriptor.
    pub fn fd(&self) -> i32 {
        self.fd
    }

    /// Check if this transport uses TLS.
    pub fn is_tls(&self) -> bool {
        self.is_tls
    }

    /// Non-blocking receive.
    ///
    /// Returns the number of bytes read (>0), 0 on clean peer shutdown, or -1 on error.
    /// On -1, `again` indicates a transient would-block:
    ///   1 = wait for readable (EAGAIN / TLS WANT_READ)
    ///   2 = wait for writable (TLS WANT_WRITE during a read)
    ///   0 = fatal error.
    pub fn recv(&self, buf: &mut [u8], again: &mut i32) -> isize {
        unsafe {
            let n = libc::recv(
                self.fd,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                libc::MSG_DONTWAIT,
            );
            if n < 0 {
                let err = *libc::__errno_location();
                if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                    *again = 1;
                }
            }
            n as isize
        }
    }

    /// Non-blocking send. Returns bytes written, or 0 when the socket is not
    /// writable (EAGAIN/EWOULDBLOCK). Used by the server poll loop so one
    /// slow peer cannot stall all connections.
    pub fn try_send(&self, data: &[u8]) -> Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        unsafe {
            let n = libc::send(
                self.fd,
                data.as_ptr() as *const libc::c_void,
                data.len(),
                libc::MSG_DONTWAIT,
            );
            if n < 0 {
                let err = *libc::__errno_location();
                if err == libc::EINTR {
                    return Ok(0);
                }
                if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                    return Ok(0);
                }
                return Err(ErrorCode::Io);
            }
            Ok(n as usize)
        }
    }

    /// Blocking send of the whole buffer (client-side synchronous I/O).
    pub fn send(&self, data: &[u8]) -> Result<()> {
        let mut sent = 0;
        while sent < data.len() {
            let n = self.try_send(&data[sent..])?;
            if n == 0 {
                let mut pfd = libc::pollfd {
                    fd: self.fd,
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let rc = unsafe { libc::poll(&mut pfd, 1, 10_000) };
                if rc == 0 {
                    return Err(ErrorCode::Timeout);
                }
                if rc < 0 {
                    return Err(ErrorCode::Io);
                }
                continue;
            }
            sent += n;
        }
        Ok(())
    }

    /// Number of bytes already buffered inside the transport (0 for plaintext).
    pub fn pending(&self) -> i32 {
        0
    }
}

impl TlsCtx {
    /// Build a server TLS context from PEM cert-chain and private-key files.
    #[cfg(feature = "tls")]
    pub fn new_server(cert_file: &str, key_file: &str) -> Result<Self> {
        Ok(Self {
            cert_file: cert_file.to_string(),
            key_file: key_file.to_string(),
        })
    }

    /// Build a server TLS context (no-op without TLS feature).
    #[cfg(not(feature = "tls"))]
    pub fn new_server(_cert_file: &str, _key_file: &str) -> Result<Self> {
        Err(ErrorCode::Unsupported)
    }
}

/// Check if TLS support is available.
pub fn tls_available() -> bool {
    cfg!(feature = "tls")
}
