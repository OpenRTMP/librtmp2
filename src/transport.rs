//! Plaintext + optional TLS byte transport.
//!
//! Mirrors `src/core/transport.h` and `src/core/transport.c`.
//!
//! The plaintext path is always available. The TLS path is feature-gated
//! behind the "tls" feature (OpenSSL).

use crate::types::ErrorCode;
use crate::types::Result;

#[cfg(feature = "tls")]
use openssl::ssl::{
    HandshakeError, MidHandshakeSslStream, SslAcceptor, SslFiletype, SslMethod, SslStream,
};
#[cfg(feature = "tls")]
use std::net::TcpStream;
#[cfg(feature = "tls")]
use std::os::unix::io::{AsRawFd, FromRawFd};
#[cfg(feature = "tls")]
use std::sync::Arc;

enum TransportInner {
    Plain(i32),
    #[cfg(feature = "tls")]
    Tls {
        stream: SslStream<TcpStream>,
        /// Cached raw fd, used only for identification and `fd()` / `poll()`.
        fd: i32,
    },
}

/// Transport wraps a connected socket fd and presents a single send/recv API.
///
/// The transport OWNS the file descriptor: it is closed when the transport
/// is dropped (plain: explicit `close(2)`; TLS: via `TcpStream` drop inside
/// the SSL stream).
pub struct Transport {
    inner: TransportInner,
}

/// Server-side TLS context: holds the validated SSL acceptor shared across
/// connections.
///
/// Cheaply `Clone`: it's just an `Arc` bump, so a `Server` with multiple TLS
/// listeners can hand each accepted connection its own owned handle without
/// re-validating the certificate/key per listener.
#[derive(Clone)]
pub struct TlsCtx {
    #[cfg(feature = "tls")]
    pub(crate) acceptor: Arc<SslAcceptor>,
}

/// A TLS handshake that could not complete immediately on a non-blocking
/// accepted socket. The server keeps these between `poll()` calls so one slow
/// RTMPS peer cannot block accepting plaintext peers or processing sessions.
#[cfg(feature = "tls")]
pub(crate) struct PendingTlsAccept {
    stream: MidHandshakeSslStream<TcpStream>,
    fd: i32,
}

#[cfg(feature = "tls")]
pub(crate) enum TlsAcceptOutcome {
    Complete(Transport),
    WouldBlock(PendingTlsAccept),
}

fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

impl Transport {
    /// Wrap an owned fd as a plaintext transport.
    pub fn new_plain(fd: i32) -> Self {
        Self {
            inner: TransportInner::Plain(fd),
        }
    }

    #[cfg(feature = "tls")]
    fn new_tls(stream: SslStream<TcpStream>) -> Result<Self> {
        let raw_fd = stream.get_ref().as_raw_fd();
        stream
            .get_ref()
            .set_nonblocking(true)
            .map_err(|_| ErrorCode::Io)?;
        Ok(Self {
            inner: TransportInner::Tls { stream, fd: raw_fd },
        })
    }

    /// Return the underlying file descriptor (used for `poll(2)` and as a
    /// connection identifier; I/O is performed through this struct).
    pub fn fd(&self) -> i32 {
        match &self.inner {
            TransportInner::Plain(fd) => *fd,
            #[cfg(feature = "tls")]
            TransportInner::Tls { fd, .. } => *fd,
        }
    }

    /// Check if this transport uses TLS.
    pub fn is_tls(&self) -> bool {
        match &self.inner {
            TransportInner::Plain(_) => false,
            #[cfg(feature = "tls")]
            TransportInner::Tls { .. } => true,
        }
    }

    /// Non-blocking receive.
    ///
    /// Returns the number of bytes read (>0), 0 on clean peer shutdown, or -1
    /// on error. On -1, `again` indicates a transient would-block:
    ///   1 = wait for readable (EAGAIN / TLS WANT_READ)
    ///   2 = wait for writable (TLS WANT_WRITE during a read)
    ///   0 = fatal error.
    pub fn recv(&mut self, buf: &mut [u8], again: &mut i32) -> isize {
        match &mut self.inner {
            TransportInner::Plain(fd) => unsafe {
                let n = libc::recv(
                    *fd,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                    libc::MSG_DONTWAIT,
                );
                if n < 0 {
                    let err = last_errno();
                    if err == libc::EINTR || err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                        *again = 1;
                    }
                }
                n as isize
            },
            #[cfg(feature = "tls")]
            TransportInner::Tls { stream, .. } => {
                use openssl::ssl::ErrorCode as SslErr;
                match stream.ssl_read(buf) {
                    Ok(n) => n as isize,
                    Err(e) => match e.code() {
                        SslErr::WANT_READ => {
                            *again = 1;
                            -1
                        }
                        SslErr::WANT_WRITE => {
                            *again = 2;
                            -1
                        }
                        SslErr::ZERO_RETURN => 0,
                        _ => -1,
                    },
                }
            }
        }
    }

    /// Non-blocking send. Returns bytes written, or 0 when the socket is not
    /// ready. On `Ok(0)`, `again` is set to indicate the poll direction:
    ///   1 = wait for readable (TLS WANT_READ during write, e.g. renegotiation)
    ///   2 = wait for writable (EAGAIN/EWOULDBLOCK/EINTR/TLS WANT_WRITE)
    /// Used by the server poll loop so one slow peer cannot stall all connections.
    pub fn try_send(&mut self, data: &[u8], again: &mut i32) -> Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        match &mut self.inner {
            TransportInner::Plain(fd) => {
                let n = unsafe {
                    libc::send(
                        *fd,
                        data.as_ptr() as *const libc::c_void,
                        data.len(),
                        libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                    )
                };
                if n < 0 {
                    let err = last_errno();
                    if err == libc::EINTR || err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                        *again = 2;
                        return Ok(0);
                    }
                    return Err(ErrorCode::Io);
                }
                Ok(n as usize)
            }
            #[cfg(feature = "tls")]
            TransportInner::Tls { stream, .. } => {
                use openssl::ssl::ErrorCode as SslErr;
                match stream.ssl_write(data) {
                    Ok(n) => Ok(n),
                    Err(e) => match e.code() {
                        SslErr::WANT_WRITE => {
                            *again = 2;
                            Ok(0)
                        }
                        // TLS renegotiation: ssl_write needs read readiness.
                        SslErr::WANT_READ => {
                            *again = 1;
                            Ok(0)
                        }
                        _ => Err(ErrorCode::Io),
                    },
                }
            }
        }
    }

    /// Blocking send of the whole buffer (client-side synchronous I/O).
    ///
    /// Uses a 10-second poll timeout rather than an infinite wait so a peer
    /// that stops reading cannot block the caller indefinitely. Correctly
    /// handles TLS WANT_READ during writes (e.g. renegotiation) by polling
    /// for read readiness instead of write readiness.
    pub fn send(&mut self, data: &[u8]) -> Result<()> {
        let mut sent = 0;
        while sent < data.len() {
            let mut again = 0i32;
            let n = self.try_send(&data[sent..], &mut again)?;
            if n == 0 {
                let fd = self.fd();
                let events = if again == 1 {
                    libc::POLLIN
                } else {
                    libc::POLLOUT
                };
                let mut pfd = libc::pollfd {
                    fd,
                    events,
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

impl Drop for Transport {
    fn drop(&mut self) {
        // For plain transports, explicitly close the owned fd.
        // For TLS transports, SslStream<TcpStream> closes the fd when it drops.
        match &self.inner {
            TransportInner::Plain(fd) => {
                if *fd >= 0 {
                    unsafe { libc::close(*fd) };
                }
            }
            #[cfg(feature = "tls")]
            TransportInner::Tls { .. } => {}
        }
    }
}

impl TlsCtx {
    /// Build a server TLS context from PEM cert-chain and private-key files.
    ///
    /// Validates the certificate and private key immediately; returns an error
    /// if the files cannot be read, are malformed, or the key doesn't match the
    /// certificate.
    #[cfg(feature = "tls")]
    pub fn new_server(cert_file: &str, key_file: &str) -> Result<Self> {
        let mut builder =
            SslAcceptor::mozilla_intermediate(SslMethod::tls()).map_err(|_| ErrorCode::Internal)?;
        builder
            .set_certificate_chain_file(cert_file)
            .map_err(|_| ErrorCode::Internal)?;
        builder
            .set_private_key_file(key_file, SslFiletype::PEM)
            .map_err(|_| ErrorCode::Internal)?;
        builder
            .check_private_key()
            .map_err(|_| ErrorCode::Internal)?;
        Ok(Self {
            acceptor: Arc::new(builder.build()),
        })
    }

    /// TLS is not available in this build.
    #[cfg(not(feature = "tls"))]
    pub fn new_server(_cert_file: &str, _key_file: &str) -> Result<Self> {
        Err(ErrorCode::Unsupported)
    }

    /// Begin or finish a TLS server handshake without blocking the caller.
    ///
    /// If the peer has not provided enough handshake data yet, the returned
    /// [`PendingTlsAccept`] can be stored and retried on a later `poll()` call.
    #[cfg(feature = "tls")]
    pub(crate) fn accept_nonblocking(&self, fd: i32) -> Result<TlsAcceptOutcome> {
        let tcp = unsafe { TcpStream::from_raw_fd(fd) };
        tcp.set_nonblocking(true).map_err(|_| ErrorCode::Io)?;
        match self.acceptor.accept(tcp) {
            Ok(ssl) => Ok(TlsAcceptOutcome::Complete(Transport::new_tls(ssl)?)),
            Err(HandshakeError::WouldBlock(stream)) => {
                Ok(TlsAcceptOutcome::WouldBlock(PendingTlsAccept { stream, fd }))
            }
            Err(_) => Err(ErrorCode::Handshake),
        }
    }

    /// Perform a TLS server handshake on the given fd and return a TLS
    /// [`Transport`] that owns the fd.
    ///
    /// This convenience helper may block for up to 10 seconds while waiting for
    /// handshake readiness. The server's accept loop uses `accept_nonblocking`
    /// instead so one stalled RTMPS peer cannot freeze other listeners.
    ///
    /// On failure the fd is closed (via the dropped `TcpStream` inside the
    /// error value) — the caller must not close it again.
    #[cfg(feature = "tls")]
    pub fn accept(&self, fd: i32) -> Result<Transport> {
        match self.accept_nonblocking(fd)? {
            TlsAcceptOutcome::Complete(transport) => Ok(transport),
            TlsAcceptOutcome::WouldBlock(mut pending) => loop {
                let mut pfd = libc::pollfd {
                    fd: pending.fd(),
                    events: libc::POLLIN | libc::POLLOUT,
                    revents: 0,
                };
                let rc = unsafe { libc::poll(&mut pfd, 1, 10_000) };
                if rc == 0 {
                    return Err(ErrorCode::Timeout);
                }
                if rc < 0 {
                    return Err(ErrorCode::Io);
                }
                match pending.progress()? {
                    TlsAcceptOutcome::Complete(transport) => return Ok(transport),
                    TlsAcceptOutcome::WouldBlock(next) => pending = next,
                }
            },
        }
    }

    #[cfg(not(feature = "tls"))]
    pub fn accept(&self, _fd: i32) -> Result<Transport> {
        Err(ErrorCode::Unsupported)
    }
}

#[cfg(feature = "tls")]
impl PendingTlsAccept {
    pub(crate) fn fd(&self) -> i32 {
        self.fd
    }

    pub(crate) fn progress(self) -> Result<TlsAcceptOutcome> {
        let fd = self.fd;
        match self.stream.handshake() {
            Ok(ssl) => Ok(TlsAcceptOutcome::Complete(Transport::new_tls(ssl)?)),
            Err(HandshakeError::WouldBlock(stream)) => {
                Ok(TlsAcceptOutcome::WouldBlock(PendingTlsAccept { stream, fd }))
            }
            Err(_) => Err(ErrorCode::Handshake),
        }
    }
}

/// Check if TLS support is available.
pub fn tls_available() -> bool {
    cfg!(feature = "tls")
}
