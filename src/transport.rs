//! Plaintext + optional TLS byte transport.
//!
//! Mirrors `src/core/transport.h` and `src/core/transport.c`.
//!
//! The plaintext path is always available. The TLS path is feature-gated
//! behind the "tls" feature (OpenSSL).
//!
//! All OS socket calls go through [`crate::net`], which selects the Unix or
//! Windows backend at compile time.

use crate::net::{self, Interest, RawSocket, SockIo, Wait};
use crate::types::ErrorCode;
use crate::types::Result;

#[cfg(feature = "tls")]
use openssl::ssl::{
    HandshakeError, MidHandshakeSslStream, SslAcceptor, SslFiletype, SslMethod, SslStream,
    SslVerifyMode,
};
use std::net::TcpStream;
use std::sync::Arc;
#[cfg(feature = "tls")]
use std::time::Duration;
use std::time::Instant;

#[cfg(feature = "tls")]
const TLS_ACCEPT_TIMEOUT_SECS: u64 = 10;

/// Most buffers [`Transport::try_send_vectored`] passes to one system call.
pub(crate) use crate::net::MAX_SEND_PARTS;

enum TransportInner {
    Plain(RawSocket),
    #[cfg(feature = "tls")]
    Tls {
        stream: SslStream<TcpStream>,
        /// Cached raw handle, used only for identification and `fd()` / `poll()`.
        fd: RawSocket,
    },
}

/// Transport wraps a connected socket and presents a single send/recv API.
///
/// The transport OWNS the socket: it is closed exactly once, when the
/// transport is dropped (plain: explicit `close(2)` / `closesocket`; TLS: via
/// `TcpStream` drop inside the SSL stream).
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
    fd: RawSocket,
    interest: Interest,
}

#[cfg(feature = "tls")]
fn tls_poll_interest(stream: &MidHandshakeSslStream<TcpStream>) -> Interest {
    use openssl::ssl::ErrorCode as SslErr;
    match stream.error().code() {
        SslErr::WANT_WRITE => Interest::Write,
        _ => Interest::Read,
    }
}

#[cfg(feature = "tls")]
pub(crate) enum TlsAcceptOutcome {
    Complete(Transport),
    WouldBlock(PendingTlsAccept),
}

impl Transport {
    /// Wrap an owned, connected socket as a plaintext transport. The
    /// transport takes ownership and closes it on drop.
    ///
    /// On Unix `fd` is a file descriptor and its blocking mode is left
    /// untouched (every call passes `MSG_DONTWAIT`). On Windows `fd` is a
    /// Winsock `SOCKET`, which this switches to non-blocking mode because
    /// Winsock has no per-call non-blocking flag.
    pub fn new_plain(fd: RawSocket) -> Self {
        net::prepare_transport_socket(fd);
        Self {
            inner: TransportInner::Plain(fd),
        }
    }

    /// Wrap a connected `TcpStream` as a plaintext transport, taking
    /// ownership of its socket. Portable equivalent of
    /// [`Transport::new_plain`] that needs no raw handle.
    pub fn from_tcp_stream(stream: TcpStream) -> Self {
        Self::new_plain(net::into_raw(stream))
    }

    #[cfg(feature = "tls")]
    fn new_tls(stream: SslStream<TcpStream>) -> Result<Self> {
        // OpenSSL's write path doesn't set MSG_NOSIGNAL the way the plaintext
        // path does, so a peer resetting mid-write can raise SIGPIPE and kill
        // the whole host process. Ignore it once, process-wide: RTMP(S)
        // connections always report broken peers via EPIPE/an OpenSSL error
        // return, so the signal itself carries no information we need.
        //
        // Only do this if the disposition is still the default: an embedding
        // application that already installed its own SIGPIPE handler (or
        // explicitly ignored it) made that choice deliberately, and we
        // shouldn't clobber it.
        //
        // Note for embedders: SIG_IGN is inherited across fork/exec. A host
        // process that forks child processes after opening a TLS connection
        // through this library, and that relies on the default SIGPIPE
        // behavior in those children, will need to restore SIG_DFL itself.
        //
        // Windows has no SIGPIPE: a write to a reset peer just fails with
        // WSAECONNRESET, so there is nothing to ignore there.
        #[cfg(unix)]
        {
            static SET_SIGPIPE_DISPOSITION: std::sync::Once = std::sync::Once::new();
            SET_SIGPIPE_DISPOSITION.call_once(|| unsafe {
                let current = libc::signal(libc::SIGPIPE, libc::SIG_IGN);
                if current != libc::SIG_DFL && current != libc::SIG_ERR {
                    libc::signal(libc::SIGPIPE, current);
                }
            });
        }

        let raw_fd = net::as_raw(stream.get_ref());
        stream
            .get_ref()
            .set_nonblocking(true)
            .map_err(|_| ErrorCode::Io)?;
        Ok(Self {
            inner: TransportInner::Tls { stream, fd: raw_fd },
        })
    }

    /// Perform a blocking TLS client handshake over an already-connected
    /// `stream` (RTMPS). By default validates the server certificate against
    /// the system trust store and checks `host` against the certificate (SNI
    /// + hostname verification), matching standard TLS client behavior.
    ///
    /// - `ca_file` (PEM bundle): verification trusts *only* the CAs in this
    ///   bundle, replacing the system trust store rather than adding to it —
    ///   so a caller pinning a private CA doesn't also accept publicly
    ///   trusted certificates for the same hostname.
    /// - `insecure = true`: skip certificate verification entirely (only for
    ///   testing against self-signed deployments; never use in production).
    ///   The system default verify paths are never touched in this mode, so
    ///   a host with no usable default CA store still connects.
    ///
    /// `stream` must not already be set non-blocking; this performs a
    /// synchronous handshake, matching [`Client`](crate::client::Client)'s
    /// otherwise-blocking connect sequence. The handshake itself is bounded by
    /// a read/write timeout so a peer that completes the TCP connect but then
    /// stalls mid-handshake cannot hang the caller indefinitely (mirroring the
    /// bound `send()` places on writes post-handshake). Uses a fixed default
    /// timeout; call [`Transport::connect_tls_with_timeout`] to bound it by
    /// an existing deadline instead.
    #[cfg(feature = "tls")]
    pub fn connect_tls(
        stream: TcpStream,
        host: &str,
        ca_file: Option<&str>,
        insecure: bool,
    ) -> Result<Self> {
        Transport::connect_tls_with_timeout(
            stream,
            host,
            ca_file,
            insecure,
            Duration::from_secs(TLS_ACCEPT_TIMEOUT_SECS),
        )
    }

    /// Same as [`Transport::connect_tls`], but `timeout` bounds the whole
    /// handshake instead of the fixed default. Pass the caller's remaining
    /// connect deadline here — otherwise the TLS handshake could run well
    /// past the overall connect timeout the caller already spent on DNS/TCP
    /// connect.
    #[cfg(feature = "tls")]
    pub fn connect_tls_with_timeout(
        stream: TcpStream,
        host: &str,
        ca_file: Option<&str>,
        insecure: bool,
        timeout: Duration,
    ) -> Result<Self> {
        use openssl::ssl::{Ssl, SslContextBuilder, SslMode};
        use openssl::x509::X509;
        use openssl::x509::store::X509StoreBuilder;
        use openssl::x509::verify::X509CheckFlags;

        if timeout.is_zero() {
            return Err(ErrorCode::Timeout);
        }
        // A single fixed read/write socket timeout only bounds each
        // individual I/O call, not the handshake as a whole — a peer that
        // drip-feeds one byte just before each timeout could stall the
        // handshake far past `timeout`. Drive the handshake non-blocking
        // instead (mirroring the server-side `accept_nonblocking` pattern)
        // and poll against one absolute deadline so the wall-clock budget
        // can't be reset by partial I/O.
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(ErrorCode::Internal)?;
        stream.set_nonblocking(true).map_err(|_| ErrorCode::Io)?;

        let mut ctx = SslContextBuilder::new(SslMethod::tls()).map_err(|_| ErrorCode::Internal)?;
        ctx.set_cipher_list(
            "DEFAULT:!aNULL:!eNULL:!MD5:!3DES:!DES:!RC4:!IDEA:!SEED:!aDSS:!SRP:!PSK",
        )
        .map_err(|_| ErrorCode::Internal)?;
        // The non-blocking publish path (Client::try_flush_send_buffer) may
        // retry a WANT_READ/WANT_WRITE SSL_write() with a send_buffer that
        // has grown (a new frame appended after the pending one) since the
        // previous attempt. ACCEPT_MOVING_WRITE_BUFFER permits the retry
        // buffer to differ in location/length as long as the previously
        // unwritten bytes are still present at the front, which holds here
        // since Buffer only ever appends after its unread portion.
        ctx.set_mode(SslMode::ACCEPT_MOVING_WRITE_BUFFER);
        if insecure {
            // Verification is disabled outright, so don't touch the system
            // verify-path configuration at all: a minimal host without a
            // usable default CA store must still be able to connect.
            ctx.set_verify(SslVerifyMode::NONE);
        } else {
            ctx.set_verify(SslVerifyMode::PEER);
            match ca_file {
                Some(ca) => {
                    // Build a replacement trust store containing only the
                    // caller-supplied CA(s), instead of augmenting the
                    // system default store: a custom CA is meant to
                    // restrict verification, not merely extend it.
                    let pem = std::fs::read(ca).map_err(|_| ErrorCode::Internal)?;
                    let certs = X509::stack_from_pem(&pem).map_err(|_| ErrorCode::Internal)?;
                    let mut store = X509StoreBuilder::new().map_err(|_| ErrorCode::Internal)?;
                    for cert in certs {
                        store.add_cert(cert).map_err(|_| ErrorCode::Internal)?;
                    }
                    ctx.set_cert_store(store.build());
                }
                None => {
                    ctx.set_default_verify_paths()
                        .map_err(|_| ErrorCode::Internal)?;
                }
            }
        }
        let ssl_ctx = ctx.build();

        let mut ssl = Ssl::new(&ssl_ctx).map_err(|_| ErrorCode::Internal)?;
        if !insecure {
            let param = ssl.param_mut();
            param.set_hostflags(X509CheckFlags::NO_PARTIAL_WILDCARDS);
            match host.parse() {
                Ok(ip) => param.set_ip(ip).map_err(|_| ErrorCode::Internal)?,
                Err(_) => param.set_host(host).map_err(|_| ErrorCode::Internal)?,
            }
        }
        ssl.set_hostname(host).map_err(|_| ErrorCode::Internal)?;

        let mut pending = match ssl.connect(stream) {
            Ok(ssl_stream) => return Transport::new_tls(ssl_stream),
            Err(HandshakeError::WouldBlock(mid)) => mid,
            Err(_) => return Err(ErrorCode::Handshake),
        };
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(ErrorCode::Timeout);
            };
            // `poll(2)`/`WSAPoll`'s granularity is milliseconds; round a sub-ms
            // remainder down to an expired deadline instead of up to a full
            // 1ms wait, so the caller's absolute deadline can't be overshot.
            if remaining.as_millis() == 0 {
                return Err(ErrorCode::Timeout);
            }
            // `poll(2)`'s timeout is a 32-bit millisecond count, so a
            // remaining budget past ~24.8 days must be clamped; `rc == 0`
            // then only means "this clamped wait expired", not "the real
            // deadline passed" — loop and recheck instead of timing out
            // early.
            let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
            let fd = net::as_raw(pending.get_ref());
            match net::poll_one(fd, tls_poll_interest(&pending), timeout_ms) {
                Wait::Ready => {}
                Wait::TimedOut => {
                    if Instant::now() >= deadline {
                        return Err(ErrorCode::Timeout);
                    }
                    continue;
                }
                Wait::Interrupted => continue,
                Wait::Failed => return Err(ErrorCode::Io),
            }
            match pending.handshake() {
                Ok(ssl_stream) => return Transport::new_tls(ssl_stream),
                Err(HandshakeError::WouldBlock(mid)) => pending = mid,
                Err(_) => return Err(ErrorCode::Handshake),
            }
        }
    }

    /// TLS is not available in this build.
    #[cfg(not(feature = "tls"))]
    pub fn connect_tls(
        _stream: TcpStream,
        _host: &str,
        _ca_file: Option<&str>,
        _insecure: bool,
    ) -> Result<Self> {
        Err(ErrorCode::Unsupported)
    }

    /// TLS is not available in this build.
    #[cfg(not(feature = "tls"))]
    pub fn connect_tls_with_timeout(
        _stream: TcpStream,
        _host: &str,
        _ca_file: Option<&str>,
        _insecure: bool,
        _timeout: std::time::Duration,
    ) -> Result<Self> {
        Err(ErrorCode::Unsupported)
    }

    /// Return the underlying socket handle (used for readiness polling and
    /// as a connection identifier; I/O is performed through this struct).
    /// A file descriptor (`i32`) on Unix, a Winsock `SOCKET` (`u64`) on
    /// Windows; see [`crate::net::RawSocket`].
    pub fn fd(&self) -> RawSocket {
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
            TransportInner::Plain(fd) => match net::recv(*fd, buf) {
                SockIo::Done(n) => n as isize,
                SockIo::WouldBlock => {
                    *again = 1;
                    -1
                }
                SockIo::Failed => -1,
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
            TransportInner::Plain(fd) => match net::send(*fd, data) {
                SockIo::Done(n) => Ok(n),
                SockIo::WouldBlock => {
                    *again = 2;
                    Ok(0)
                }
                SockIo::Failed => Err(ErrorCode::Io),
            },
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

    /// Non-blocking send of `parts`, in order, in one system call on a
    /// plaintext socket, so a caller can send data spread over several
    /// buffers without first copying it together. The first
    /// `first_offset` bytes of `parts[0]` are skipped, which lets a caller
    /// resume after a partial write by passing the same slice again
    /// (advanced to the first unfinished part) instead of building a new
    /// window. Returns the number of bytes written across all parts, or 0
    /// when the socket is not ready (see [`Self::try_send`] for `again`).
    /// At most [`MAX_SEND_PARTS`] non-empty parts go out per call; the
    /// `iovec` (Unix `sendmsg`) / `WSABUF` (Windows `WSASend`) array lives on
    /// the stack, so a send never allocates or copies payload. TLS
    /// transports always return `Ok(0)` without writing: the caller
    /// buffers instead.
    pub(crate) fn try_send_vectored(
        &mut self,
        parts: &[&[u8]],
        first_offset: usize,
        again: &mut i32,
    ) -> Result<usize> {
        let fd = match &self.inner {
            TransportInner::Plain(fd) => *fd,
            #[cfg(feature = "tls")]
            TransportInner::Tls { .. } => return Ok(0),
        };
        match net::send_vectored(fd, parts, first_offset) {
            SockIo::Done(n) => Ok(n),
            SockIo::WouldBlock => {
                *again = 2;
                Ok(0)
            }
            SockIo::Failed => Err(ErrorCode::Io),
        }
    }

    /// Blocking send of the whole buffer (client-side synchronous I/O).
    ///
    /// Bounds the whole send with a 10-second deadline rather than an
    /// infinite wait, so a peer that stops reading — or that trickles out a
    /// few bytes just before each individual wait expires — cannot block the
    /// caller indefinitely. Correctly handles TLS WANT_READ during writes
    /// (e.g. renegotiation) by polling for read readiness instead of write
    /// readiness.
    pub fn send(&mut self, data: &[u8]) -> Result<()> {
        // A single fixed poll timeout only bounds each individual wait, not
        // the send as a whole; poll against one absolute deadline so the
        // wall-clock budget can't be reset by partial writes.
        let deadline = Instant::now()
            .checked_add(std::time::Duration::from_secs(10))
            .ok_or(ErrorCode::Internal)?;
        let mut sent = 0;
        while sent < data.len() {
            // The deadline has to be consulted on every iteration, not only in
            // the `n == 0` wait below: a peer that keeps freeing just enough
            // capacity to return a small positive write would otherwise reset
            // the budget on every pass and outlive it indefinitely.
            if Instant::now() >= deadline {
                return Err(ErrorCode::Timeout);
            }
            let mut again = 0i32;
            let n = self.try_send(&data[sent..], &mut again)?;
            if n == 0 {
                let interest = if again == 1 {
                    Interest::Read
                } else {
                    Interest::Write
                };
                // An exhausted budget truncates to 0 ms, which the poll reports
                // as timed out and the branch below turns into `ErrorCode::Timeout`.
                let remaining = deadline.saturating_duration_since(Instant::now());
                let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
                match net::poll_one(self.fd(), interest, timeout_ms) {
                    Wait::Ready | Wait::Interrupted => continue,
                    Wait::TimedOut => return Err(ErrorCode::Timeout),
                    Wait::Failed => return Err(ErrorCode::Io),
                }
            }
            sent += n;
        }
        Ok(())
    }

    /// Number of decrypted bytes already buffered inside the transport (always
    /// 0 for plaintext). For TLS, `SSL_pending()` reports application data
    /// OpenSSL has already decrypted but the caller hasn't read yet -- data a
    /// `poll(2)` readiness check on the raw fd cannot see, since the kernel
    /// socket buffer it was decrypted from may already be empty.
    pub fn pending(&self) -> i32 {
        match &self.inner {
            TransportInner::Plain(_) => 0,
            #[cfg(feature = "tls")]
            TransportInner::Tls { stream, .. } => stream.ssl().pending() as i32,
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        // For plain transports, explicitly close the owned socket.
        // For TLS transports, SslStream<TcpStream> closes it when it drops.
        match &self.inner {
            TransportInner::Plain(fd) => net::close(*fd),
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
        use openssl::ssl::SslMode;

        let mut builder =
            SslAcceptor::mozilla_intermediate(SslMethod::tls()).map_err(|_| ErrorCode::Internal)?;
        // Non-blocking sends (Conn::flush) may retry a WANT_READ/WANT_WRITE
        // SSL_write() with a send_buffer that grew (more queued output
        // appended) since the previous attempt; see the matching client-side
        // comment in Transport::connect_tls.
        builder.set_mode(SslMode::ACCEPT_MOVING_WRITE_BUFFER);
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
    pub(crate) fn accept_nonblocking(&self, tcp: TcpStream) -> Result<TlsAcceptOutcome> {
        let fd = net::as_raw(&tcp);
        tcp.set_nonblocking(true).map_err(|_| ErrorCode::Io)?;
        match self.acceptor.accept(tcp) {
            Ok(ssl) => Ok(TlsAcceptOutcome::Complete(Transport::new_tls(ssl)?)),
            Err(HandshakeError::WouldBlock(stream)) => {
                let interest = tls_poll_interest(&stream);
                Ok(TlsAcceptOutcome::WouldBlock(PendingTlsAccept {
                    stream,
                    fd,
                    interest,
                }))
            }
            Err(_) => Err(ErrorCode::Handshake),
        }
    }

    /// Perform a TLS server handshake on the given socket (a file descriptor
    /// on Unix, a Winsock `SOCKET` on Windows) and return a TLS
    /// [`Transport`] that owns it.
    ///
    /// This convenience helper may block for up to 10 seconds while waiting for
    /// handshake readiness. The server's accept loop uses `accept_nonblocking`
    /// instead so one stalled RTMPS peer cannot freeze other listeners.
    ///
    /// On failure the fd is closed (via the dropped `TcpStream` inside the
    /// error value) — the caller must not close it again.
    #[cfg(feature = "tls")]
    pub fn accept(&self, fd: RawSocket) -> Result<Transport> {
        // SAFETY: by this function's contract the caller hands over an open,
        // connected socket it owns; the TcpStream (or the handshake error
        // holding it) closes it exactly once.
        let tcp = unsafe { net::tcp_stream_from_raw(fd) };
        match self.accept_nonblocking(tcp)? {
            TlsAcceptOutcome::Complete(transport) => Ok(transport),
            TlsAcceptOutcome::WouldBlock(mut pending) => {
                let deadline = Instant::now() + Duration::from_secs(TLS_ACCEPT_TIMEOUT_SECS);
                loop {
                    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                        return Err(ErrorCode::Timeout);
                    };
                    let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
                    let timeout_ms = timeout_ms.max(1);
                    match net::poll_one(pending.fd(), pending.interest, timeout_ms) {
                        Wait::Ready => {}
                        Wait::TimedOut => return Err(ErrorCode::Timeout),
                        Wait::Interrupted => continue,
                        Wait::Failed => return Err(ErrorCode::Io),
                    }
                    match pending.progress()? {
                        TlsAcceptOutcome::Complete(transport) => return Ok(transport),
                        TlsAcceptOutcome::WouldBlock(next) => pending = next,
                    }
                }
            }
        }
    }

    #[cfg(not(feature = "tls"))]
    pub fn accept(&self, _fd: RawSocket) -> Result<Transport> {
        Err(ErrorCode::Unsupported)
    }
}

#[cfg(feature = "tls")]
impl PendingTlsAccept {
    pub(crate) fn fd(&self) -> RawSocket {
        self.fd
    }

    pub(crate) fn progress(self) -> Result<TlsAcceptOutcome> {
        let fd = self.fd;
        match self.stream.handshake() {
            Ok(ssl) => Ok(TlsAcceptOutcome::Complete(Transport::new_tls(ssl)?)),
            Err(HandshakeError::WouldBlock(stream)) => {
                let interest = tls_poll_interest(&stream);
                Ok(TlsAcceptOutcome::WouldBlock(PendingTlsAccept {
                    stream,
                    fd,
                    interest,
                }))
            }
            Err(_) => Err(ErrorCode::Handshake),
        }
    }
}

/// Check if TLS support is available.
pub fn tls_available() -> bool {
    cfg!(feature = "tls")
}

#[cfg(all(test, feature = "tls"))]
mod client_tls_tests {
    use super::*;
    use openssl::asn1::Asn1Time;
    use openssl::bn::{BigNum, MsbOption};
    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;
    use openssl::x509::extension::{BasicConstraints, SubjectAlternativeName};
    use openssl::x509::{X509, X509NameBuilder};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Generates a self-signed cert/key pair (PEM) for `cn`, written to
    /// unique temp files so `TlsCtx::new_server`/`connect_tls` (both of
    /// which take file paths) can consume them. Returns the paths; callers
    /// should remove them when done.
    fn self_signed_cert_files(cn: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let rsa = Rsa::generate(2048).unwrap();
        let pkey = PKey::from_rsa(rsa).unwrap();

        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", cn).unwrap();
        let name = name.build();

        let mut builder = X509::builder().unwrap();
        builder.set_version(2).unwrap();
        let mut sn = BigNum::new().unwrap();
        sn.rand(64, MsbOption::MAYBE_ZERO, false).unwrap();
        builder
            .set_serial_number(&sn.to_asn1_integer().unwrap())
            .unwrap();
        builder.set_subject_name(&name).unwrap();
        builder.set_issuer_name(&name).unwrap();
        builder.set_pubkey(&pkey).unwrap();
        builder
            .set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        builder
            .set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        builder
            .append_extension(BasicConstraints::new().critical().ca().build().unwrap())
            .unwrap();
        let san = SubjectAlternativeName::new()
            .dns(cn)
            .build(&builder.x509v3_context(None, None))
            .unwrap();
        builder.append_extension(san).unwrap();
        builder.sign(&pkey, MessageDigest::sha256()).unwrap();
        let cert = builder.build();

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("librtmp2-test-{}-{}-{}", std::process::id(), n, cn));
        let cert_path = base.with_extension("cert.pem");
        let key_path = base.with_extension("key.pem");
        std::fs::write(&cert_path, cert.to_pem().unwrap()).unwrap();
        std::fs::write(&key_path, pkey.private_key_to_pem_pkcs8().unwrap()).unwrap();
        (cert_path, key_path)
    }

    /// Starts a one-shot TLS echo-free accept-and-drop server on `cert_file`
    /// / `key_file`, returning its port. The handshake runs on a background
    /// thread; the caller is responsible for connecting exactly once.
    fn spawn_tls_server(cert_file: std::path::PathBuf, key_file: std::path::PathBuf) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let ctx = TlsCtx::new_server(cert_file.to_str().unwrap(), key_file.to_str().unwrap())
            .expect("valid self-signed cert/key");
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let fd = crate::net::into_raw(stream);
                // Blocking accept: run the handshake to completion, ignoring
                // the outcome — the test only cares whether the *client*
                // observed success/failure.
                let _ = ctx.accept(fd);
            }
        });
        port
    }

    #[test]
    fn connect_tls_insecure_accepts_self_signed_cert_without_default_verify_paths() {
        let (cert_path, key_path) = self_signed_cert_files("insecure.test");
        let port = spawn_tls_server(cert_path.clone(), key_path.clone());

        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let result = Transport::connect_tls(stream, "insecure.test", None, true);
        assert!(
            result.is_ok(),
            "insecure connect should succeed: {:?}",
            result.err()
        );

        let _ = std::fs::remove_file(cert_path);
        let _ = std::fs::remove_file(key_path);
    }

    #[test]
    fn connect_tls_with_matching_ca_file_trusts_self_signed_cert() {
        let (cert_path, key_path) = self_signed_cert_files("matching-ca.test");
        let port = spawn_tls_server(cert_path.clone(), key_path.clone());

        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let result = Transport::connect_tls(
            stream,
            "matching-ca.test",
            Some(cert_path.to_str().unwrap()),
            false,
        );
        assert!(
            result.is_ok(),
            "connect with the server's own cert as ca_file should succeed: {:?}",
            result.err()
        );

        let _ = std::fs::remove_file(cert_path);
        let _ = std::fs::remove_file(key_path);
    }

    #[test]
    fn connect_tls_with_mismatched_ca_file_rejects_self_signed_cert() {
        let (server_cert, server_key) = self_signed_cert_files("mismatched-server.test");
        let (other_cert, other_key) = self_signed_cert_files("unrelated-ca.test");
        let port = spawn_tls_server(server_cert.clone(), server_key.clone());

        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        // `other_cert` is a valid CA file, just not one that issued the
        // server's certificate: verification must fail, proving the
        // replacement trust store isn't silently falling back to trusting
        // everything (or the system store, which wouldn't contain either
        // self-signed cert anyway).
        let result = Transport::connect_tls(
            stream,
            "mismatched-server.test",
            Some(other_cert.to_str().unwrap()),
            false,
        );
        assert_eq!(result.err(), Some(ErrorCode::Handshake));

        let _ = std::fs::remove_file(server_cert);
        let _ = std::fs::remove_file(server_key);
        let _ = std::fs::remove_file(other_cert);
        let _ = std::fs::remove_file(other_key);
    }

    #[test]
    fn connect_tls_default_mode_rejects_untrusted_self_signed_cert() {
        let (cert_path, key_path) = self_signed_cert_files("default-mode.test");
        let port = spawn_tls_server(cert_path.clone(), key_path.clone());

        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        // No ca_file, not insecure: a self-signed cert absent from the
        // system trust store must be rejected, same as before this change.
        let result = Transport::connect_tls(stream, "default-mode.test", None, false);
        assert_eq!(result.err(), Some(ErrorCode::Handshake));

        let _ = std::fs::remove_file(cert_path);
        let _ = std::fs::remove_file(key_path);
    }

    #[test]
    fn connect_tls_with_timeout_times_out_against_a_stalled_peer() {
        // Accept the TCP connection but never write a byte of TLS data back:
        // the client's handshake should time out on the supplied deadline
        // rather than hang, and must not take anywhere close to the fixed
        // 10s default (proving the deadline, not the socket default, is
        // what's bounding it).
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _keep_alive = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(5));
            drop(stream);
        });

        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let start = Instant::now();
        let result = Transport::connect_tls_with_timeout(
            stream,
            "stalled.test",
            None,
            true,
            Duration::from_millis(200),
        );
        let elapsed = start.elapsed();
        assert_eq!(result.err(), Some(ErrorCode::Timeout));
        assert!(
            elapsed < Duration::from_secs(2),
            "expected the ~200ms deadline to bound the handshake, took {:?}",
            elapsed
        );
    }
}

/// Behavioural tests for the plaintext socket path, run against every
/// stream type the platform offers: a Unix socketpair (Unix only, as
/// before) and a TCP loopback connection (every platform, including
/// Windows), so both socket backends are held to the same cases.
#[cfg(test)]
mod vectored_send_tests {
    use super::*;
    use crate::net::testing::{PeerStream, small_buffer_tcp_pair};
    use std::io::{Read, Write};
    use std::time::Duration;

    fn read_exact_n<P: Read>(peer: &mut P, n: usize) -> Vec<u8> {
        let mut out = vec![0u8; n];
        peer.read_exact(&mut out).unwrap();
        out
    }

    macro_rules! plain_socket_suite {
        ($suite:ident, $pair:expr, $partial_len:expr) => {
            mod $suite {
                use super::*;

                fn pair() -> (Transport, impl Read + Write + PeerStream + Send + 'static) {
                    $pair
                }

                #[test]
                fn full_vectored_write_keeps_byte_order() {
                    let (mut t, mut peer) = pair();
                    let parts: [&[u8]; 3] = [b"abc", b"defg", b"h"];
                    let mut again = 0;
                    assert_eq!(t.try_send_vectored(&parts, 0, &mut again), Ok(8));
                    assert_eq!(read_exact_n(&mut peer, 8), b"abcdefgh");
                }

                #[test]
                fn offset_into_first_part_skips_that_prefix() {
                    let (mut t, mut peer) = pair();
                    let parts: [&[u8]; 2] = [b"abcdef", b"gh"];
                    assert_eq!(t.try_send_vectored(&parts, 4, &mut 0), Ok(4));
                    assert_eq!(read_exact_n(&mut peer, 4), b"efgh");
                }

                #[test]
                fn offset_at_or_past_the_end_of_first_part_sends_the_rest_only() {
                    let (mut t, mut peer) = pair();
                    let parts: [&[u8]; 2] = [b"abc", b"de"];
                    assert_eq!(t.try_send_vectored(&parts, 3, &mut 0), Ok(2));
                    assert_eq!(t.try_send_vectored(&parts, 99, &mut 0), Ok(2));
                    assert_eq!(read_exact_n(&mut peer, 4), b"dede");
                }

                #[test]
                fn empty_parts_are_skipped_and_all_empty_writes_nothing() {
                    let (mut t, mut peer) = pair();
                    let parts: [&[u8]; 4] = [b"", b"ab", b"", b"c"];
                    assert_eq!(t.try_send_vectored(&parts, 0, &mut 0), Ok(3));
                    assert_eq!(read_exact_n(&mut peer, 3), b"abc");
                    let empty: [&[u8]; 2] = [b"", b""];
                    assert_eq!(t.try_send_vectored(&empty, 0, &mut 0), Ok(0));
                    assert_eq!(t.try_send_vectored(&[], 0, &mut 0), Ok(0));
                }

                #[test]
                fn more_than_max_parts_go_out_in_windows_with_exact_order() {
                    let (mut t, mut peer) = pair();
                    // 2 * MAX_SEND_PARTS + 7 one-byte parts, interleaved with
                    // empties that must not count against the window.
                    let total = 2 * MAX_SEND_PARTS + 7;
                    let bytes: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
                    let mut parts: Vec<&[u8]> = Vec::new();
                    for i in 0..total {
                        parts.push(&bytes[i..i + 1]);
                        parts.push(&[]);
                    }
                    let mut next = 0;
                    let mut offset = 0;
                    let mut calls = 0;
                    let mut received = Vec::new();
                    while next < parts.len() {
                        let sent = t.try_send_vectored(&parts[next..], offset, &mut 0).unwrap();
                        assert!(sent > 0 && sent <= MAX_SEND_PARTS);
                        calls += 1;
                        received.extend(read_exact_n(&mut peer, sent));
                        let mut left = sent;
                        while left > 0 || (next < parts.len() && parts[next].len() == offset) {
                            let rest = parts[next].len() - offset;
                            if left < rest {
                                offset += left;
                                break;
                            }
                            left -= rest;
                            next += 1;
                            offset = 0;
                        }
                    }
                    assert_eq!(calls, 3);
                    assert_eq!(received, bytes);
                }

                #[test]
                fn partial_write_reports_short_count_and_resume_continues_exactly() {
                    let (mut t, mut peer) = pair();
                    // Far more than the socket buffers hold: a write must
                    // come back short, and resuming from the byte count must
                    // lose nothing. A Unix socket takes part of the first
                    // write; Winsock takes a whole send while its buffer has
                    // room and refuses the next with "would block" (a count
                    // of 0), so the payload is written as a stream of copies
                    // until a short write shows up.
                    let total: usize = $partial_len;
                    let big: Vec<u8> = (0..total).map(|i| (i % 253) as u8).collect();
                    let split = total / 4;
                    let parts: [&[u8]; 2] = [&big[..split], &big[split..]];
                    // Write from stream position `pos` (any copy of `big`).
                    let write_from = |t: &mut Transport, pos: usize| {
                        let at = pos % big.len();
                        let (next, offset) = if at < parts[0].len() {
                            (0, at)
                        } else {
                            (1, at - parts[0].len())
                        };
                        let n = t.try_send_vectored(&parts[next..], offset, &mut 0).unwrap();
                        (n, big.len() - at)
                    };
                    let mut sent = 0;
                    let copies = loop {
                        let (n, wanted) = write_from(&mut t, sent);
                        sent += n;
                        if n < wanted {
                            break sent.div_ceil(big.len());
                        }
                        assert!(sent < 8 * big.len(), "expected a partial write");
                    };
                    assert!(sent > 0, "an empty socket must take some bytes");
                    let stream_len = copies * big.len();
                    let mut got = Vec::new();
                    peer.set_nonblocking(true).unwrap();
                    let mut buf = vec![0u8; 1 << 16];
                    loop {
                        match peer.read(&mut buf) {
                            Ok(0) => panic!("peer saw EOF"),
                            Ok(n) => got.extend_from_slice(&buf[..n]),
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                if sent == stream_len && got.len() == stream_len {
                                    break;
                                }
                                if sent < stream_len {
                                    let (n, wanted) = write_from(&mut t, sent);
                                    // Never run past the end of the stream.
                                    assert!(n <= wanted);
                                    sent += n;
                                } else {
                                    // Everything was handed to the OS; let a
                                    // TCP loopback finish delivering it.
                                    std::thread::sleep(Duration::from_millis(1));
                                }
                            }
                            Err(e) => panic!("{e}"),
                        }
                    }
                    assert_eq!(got.len(), stream_len);
                    for copy in got.chunks(big.len()) {
                        assert!(copy == &big[..], "bytes lost or reordered");
                    }
                }

                #[test]
                fn full_socket_reports_again_instead_of_an_error() {
                    let (mut t, _peer) = pair();
                    let chunk = vec![7u8; 1 << 20];
                    let parts: [&[u8]; 1] = [&chunk];
                    let mut again = 0;
                    let mut guard = 0;
                    // Fill the socket until the OS refuses more.
                    loop {
                        let n = t.try_send_vectored(&parts, 0, &mut again).unwrap();
                        if n == 0 {
                            break;
                        }
                        guard += 1;
                        assert!(guard < 1000, "socket never filled");
                    }
                    assert_eq!(again, 2, "would-block must ask for write readiness");
                }

                #[test]
                fn full_socket_plain_send_reports_again_and_recv_reports_would_block() {
                    let (mut t, _peer) = pair();
                    let chunk = vec![9u8; 1 << 20];
                    let mut guard = 0;
                    loop {
                        let mut again = 0;
                        let n = t.try_send(&chunk, &mut again).unwrap();
                        if n == 0 {
                            assert_eq!(again, 2);
                            break;
                        }
                        guard += 1;
                        assert!(guard < 1000, "socket never filled");
                    }
                    // Nothing was sent our way: a read must not block.
                    let mut buf = [0u8; 16];
                    let mut again = 0;
                    assert_eq!(t.recv(&mut buf, &mut again), -1);
                    assert_eq!(again, 1, "would-block must ask for read readiness");
                }

                #[test]
                fn partial_reads_deliver_every_byte_in_order() {
                    let (mut t, mut peer) = pair();
                    let data: Vec<u8> = (0..200_000).map(|i| (i % 241) as u8).collect();
                    let writer = std::thread::spawn(move || {
                        for piece in data.chunks(7_919) {
                            peer.write_all(piece).unwrap();
                        }
                        // Closing the write side ends the stream cleanly.
                        drop(peer);
                        data
                    });
                    let mut got = Vec::new();
                    let mut buf = [0u8; 4096];
                    let deadline = Instant::now() + Duration::from_secs(10);
                    loop {
                        assert!(Instant::now() < deadline, "read stalled");
                        let mut again = 0;
                        let n = t.recv(&mut buf, &mut again);
                        if n > 0 {
                            got.extend_from_slice(&buf[..n as usize]);
                        } else if n == 0 {
                            break;
                        } else {
                            assert_eq!(again, 1, "only would-block is expected here");
                            let fd = t.fd();
                            crate::net::poll_one(fd, crate::net::Interest::Read, 100);
                        }
                    }
                    assert_eq!(got, writer.join().unwrap());
                }

                #[test]
                fn blocking_send_waits_for_a_slow_reader_and_delivers_everything() {
                    let (mut t, mut peer) = pair();
                    let data: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 239) as u8).collect();
                    let expected = data.clone();
                    let reader = std::thread::spawn(move || {
                        let mut got = vec![0u8; expected.len()];
                        let mut read = 0;
                        while read < got.len() {
                            // A slow consumer: small reads with pauses.
                            let end = (read + 32 * 1024).min(got.len());
                            peer.read_exact(&mut got[read..end]).unwrap();
                            read = end;
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        assert_eq!(got, expected);
                    });
                    assert_eq!(t.send(&data), Ok(()));
                    reader.join().unwrap();
                }

                #[test]
                fn poll_one_times_out_on_an_idle_socket_and_wakes_on_data() {
                    let (t, mut peer) = pair();
                    let start = Instant::now();
                    let w = crate::net::poll_one(t.fd(), crate::net::Interest::Read, 50);
                    assert_eq!(w, crate::net::Wait::TimedOut);
                    assert!(start.elapsed() >= Duration::from_millis(40));
                    peer.write_all(b"x").unwrap();
                    let w = crate::net::poll_one(t.fd(), crate::net::Interest::Read, 5_000);
                    assert_eq!(w, crate::net::Wait::Ready);
                    let w = crate::net::poll_one(t.fd(), crate::net::Interest::Write, 5_000);
                    assert_eq!(w, crate::net::Wait::Ready);
                }

                #[test]
                fn peer_shutdown_is_a_clean_eof() {
                    let (mut t, peer) = pair();
                    drop(peer);
                    let mut buf = [0u8; 8];
                    let deadline = Instant::now() + Duration::from_secs(5);
                    loop {
                        let mut again = 0;
                        let n = t.recv(&mut buf, &mut again);
                        if n == 0 {
                            break;
                        }
                        assert_eq!(n, -1);
                        assert_eq!(again, 1);
                        assert!(Instant::now() < deadline, "EOF never arrived");
                        crate::net::poll_one(t.fd(), crate::net::Interest::Read, 100);
                    }
                }

                #[test]
                fn closed_peer_is_an_io_error_without_sigpipe() {
                    let (mut t, peer) = pair();
                    drop(peer);
                    let parts: [&[u8]; 1] = [b"data"];
                    // A socketpair fails on the first write; over TCP the
                    // first write may still be accepted locally until the
                    // peer's reset comes back, so keep writing (bounded).
                    let deadline = Instant::now() + Duration::from_secs(5);
                    loop {
                        match t.try_send_vectored(&parts, 0, &mut 0) {
                            Err(e) => {
                                assert_eq!(e, ErrorCode::Io);
                                break;
                            }
                            Ok(_) => {
                                assert!(
                                    Instant::now() < deadline,
                                    "write to a closed peer kept succeeding"
                                );
                                std::thread::sleep(Duration::from_millis(5));
                            }
                        }
                    }
                }
            }
        };
    }

    #[cfg(unix)]
    plain_socket_suite!(
        socketpair,
        {
            let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
            a.set_nonblocking(true).unwrap();
            (Transport::new_plain(crate::net::into_raw(a)), b)
        },
        4 * 1024 * 1024
    );

    plain_socket_suite!(
        tcp_loopback,
        {
            let (a, b) = small_buffer_tcp_pair();
            (Transport::from_tcp_stream(a), b)
        },
        16 * 1024 * 1024
    );

    #[test]
    fn invalid_socket_operations_fail_cleanly() {
        // A transport over the "no socket" sentinel must report errors, not
        // crash or block, and dropping it must not close anything.
        let mut t = Transport::new_plain(crate::net::INVALID_SOCKET);
        let mut buf = [0u8; 4];
        let mut again = 0;
        assert_eq!(t.recv(&mut buf, &mut again), -1);
        assert_eq!(again, 0, "an invalid handle is fatal, not would-block");
        assert_eq!(t.try_send(b"x", &mut 0), Err(ErrorCode::Io));
        assert_eq!(t.try_send_vectored(&[b"x"], 0, &mut 0), Err(ErrorCode::Io));
    }
}
