//! Small networking helpers shared by the server (bind) and client (connect)
//! entry points, plus the platform socket layer every transport sits on.
//!
//! Mirrors `src/core/net.h` and `src/core/net.c`.
//!
//! # Socket backend
//!
//! The protocol layers above never touch an OS socket API directly. They go
//! through the handful of free functions re-exported here (`recv`, `send`,
//! `send_vectored`, `poll_one`, `close`), which are implemented once per
//! platform and selected at compile time:
//!
//! - Unix (`unix.rs`): `recv(2)`/`send(2)` on an `O_NONBLOCK` socket with
//!   `MSG_DONTWAIT` (plus `MSG_NOSIGNAL` on writes), `sendmsg(2)` with a stack `iovec` array for
//!   vectored writes, and `poll(2)` for readiness.
//! - Windows (`windows.rs`): Winsock `recv`/`send` on a socket put into
//!   non-blocking mode (Winsock has no per-call `MSG_DONTWAIT`), `WSASend`
//!   with a stack `WSABUF` array for vectored writes, and `WSAPoll` for
//!   readiness.
//!
//! Every function is `#[inline]` and takes the raw handle by value, so the
//! abstraction adds no allocation, dispatch or extra system call to the
//! hot paths; both backends issue exactly one system call per operation.
//!
//! [`RawSocket`] is the platform's native handle type: `RawFd` (`i32`) on
//! Unix -- unchanged from earlier releases -- and `RawSocket` (`u64`, a
//! Winsock `SOCKET`) on Windows, which must never be truncated to `i32`.
//! [`INVALID_SOCKET`] is the matching "no socket" sentinel (`-1` on Unix).

use crate::types::ErrorCode;
use crate::types::Result;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

#[cfg(not(any(unix, windows)))]
compile_error!("librtmp2 supports Unix-like platforms and Windows only");

/// The platform's native socket handle: `RawFd` (`i32`) on Unix,
/// `RawSocket` (`u64`, a Winsock `SOCKET`) on Windows.
#[cfg(unix)]
pub type RawSocket = std::os::fd::RawFd;
/// The platform's native socket handle: `RawFd` (`i32`) on Unix,
/// `RawSocket` (`u64`, a Winsock `SOCKET`) on Windows.
#[cfg(windows)]
pub type RawSocket = std::os::windows::io::RawSocket;

/// Sentinel for "no socket": `-1` on Unix, `INVALID_SOCKET` (`!0`) on
/// Windows. Public `*_fd` fields hold this value when no socket is open.
#[cfg(unix)]
pub const INVALID_SOCKET: RawSocket = -1;
/// Sentinel for "no socket": `-1` on Unix, `INVALID_SOCKET` (`!0`) on
/// Windows. Public `*_fd` fields hold this value when no socket is open.
#[cfg(windows)]
pub const INVALID_SOCKET: RawSocket = !0;

/// Readiness direction for [`poll_one`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Interest {
    Read,
    Write,
}

impl Interest {
    /// The `again` convention used by [`crate::transport::Transport`]:
    /// `2` asks for write readiness, anything else for read readiness.
    #[inline]
    pub(crate) fn from_again(again: i32) -> Self {
        if again == 2 { Self::Write } else { Self::Read }
    }
}

/// Result of one non-blocking socket operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SockIo {
    /// Bytes transferred. For a receive, `0` means the peer closed the
    /// connection cleanly.
    Done(usize),
    /// The socket is not ready (`EAGAIN`/`EWOULDBLOCK`/`WSAEWOULDBLOCK`) or
    /// the call was interrupted (`EINTR`/`WSAEINTR`); retry once ready.
    WouldBlock,
    /// Any other error (reset, aborted, not connected, invalid handle, ...).
    Failed,
}

/// Outcome of a single readiness wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wait {
    /// The socket reported the requested readiness, or an error/hang-up
    /// condition that the next I/O call will surface.
    Ready,
    /// The timeout elapsed first.
    TimedOut,
    /// The wait was interrupted by a signal (Unix `EINTR`); retry it.
    Interrupted,
    /// The wait itself failed.
    Failed,
}

pub(crate) use sys::{close, poll_one, prepare_transport_socket, recv, send, send_vectored};

/// Raw handle of any socket-backed std type (`TcpStream`, `TcpListener`,
/// and on Unix also `UnixStream`), without transferring ownership.
#[cfg(unix)]
#[inline]
pub(crate) fn as_raw<T: std::os::fd::AsRawFd>(s: &T) -> RawSocket {
    s.as_raw_fd()
}
/// Raw handle of any socket-backed std type (`TcpStream`, `TcpListener`,
/// and on Unix also `UnixStream`), without transferring ownership.
#[cfg(windows)]
#[inline]
pub(crate) fn as_raw<T: std::os::windows::io::AsRawSocket>(s: &T) -> RawSocket {
    s.as_raw_socket()
}

/// Release ownership of a socket-backed std type and return its handle;
/// the caller becomes responsible for closing it.
#[cfg(unix)]
#[inline]
pub(crate) fn into_raw<T: std::os::fd::IntoRawFd>(s: T) -> RawSocket {
    s.into_raw_fd()
}
/// Release ownership of a socket-backed std type and return its handle;
/// the caller becomes responsible for closing it.
#[cfg(windows)]
#[inline]
pub(crate) fn into_raw<T: std::os::windows::io::IntoRawSocket>(s: T) -> RawSocket {
    s.into_raw_socket()
}

/// Take ownership of a connected TCP socket handle as a `TcpStream`.
///
/// # Safety
///
/// `s` must be an open, connected stream socket that the caller owns and
/// does not use or close afterwards: the returned `TcpStream` closes it.
#[cfg(unix)]
#[inline]
pub(crate) unsafe fn tcp_stream_from_raw(s: RawSocket) -> std::net::TcpStream {
    // SAFETY: forwarded to the caller.
    unsafe { std::os::fd::FromRawFd::from_raw_fd(s) }
}
/// Take ownership of a connected TCP socket handle as a `TcpStream`.
///
/// # Safety
///
/// `s` must be an open, connected stream socket that the caller owns and
/// does not use or close afterwards: the returned `TcpStream` closes it.
#[cfg(windows)]
#[inline]
pub(crate) unsafe fn tcp_stream_from_raw(s: RawSocket) -> std::net::TcpStream {
    // SAFETY: forwarded to the caller.
    unsafe { std::os::windows::io::FromRawSocket::from_raw_socket(s) }
}

/// Most buffers one vectored write passes to the OS (well under Linux's
/// `UIO_MAXIOV` of 1024; Winsock imposes no lower limit).
pub(crate) const MAX_SEND_PARTS: usize = 512;

/// Split a "host:port" authority into separate host and port strings.
///
/// Accepts:
/// - "host:port"        -> host, port
/// - "host"             -> host, def_port
/// - "[v6addr]:port"    -> v6addr (brackets stripped), port
/// - "[v6addr]"         -> v6addr, def_port
/// - "fe80::1" / "::"   -> the whole string as host, def_port
/// - ":port"            -> "" (empty host = wildcard), port
///
/// `def_port` is copied into `port` whenever the input carries no port of its own.
/// Returns Ok(()) on success, or an error if a destination buffer is too small
/// or the bracketed form is malformed.
pub fn split_host_port(
    input: &str,
    host: &mut String,
    port: &mut String,
    def_port: &str,
) -> Result<()> {
    port.clear();
    port.push_str(def_port);

    if input.starts_with('[') {
        // Bracketed IPv6 literal: "[addr]" or "[addr]:port"
        let end = input.find(']').ok_or(ErrorCode::Internal)?;
        let addr = &input[1..end];
        *host = addr.to_string();
        let rest = &input[end + 1..];
        if rest.starts_with(':') {
            port.clear();
            port.push_str(&rest[1..]);
        } else if !rest.is_empty() {
            return Err(ErrorCode::Internal);
        }
        return Ok(());
    }

    // Count colons to tell "host:port" apart from a bare IPv6 literal.
    let colons = input.chars().filter(|&c| c == ':').count();

    if colons > 1 {
        // Unbracketed and multi-colon -> a bare IPv6 literal with no port.
        *host = input.to_string();
        return Ok(());
    }

    if colons == 1 {
        let (h, p) = input.split_once(':').unwrap();
        *host = h.to_string();
        port.clear();
        port.push_str(p);
        return Ok(());
    }

    // No colon: host only, default port.
    *host = input.to_string();
    Ok(())
}

/// Socket helpers shared by unit tests on every platform.
#[cfg(test)]
pub(crate) mod testing {
    use std::net::{TcpListener, TcpStream};

    /// The test-side end of a connected stream pair.
    pub(crate) trait PeerStream {
        fn set_nonblocking(&self, nonblocking: bool) -> std::io::Result<()>;
    }

    impl PeerStream for TcpStream {
        fn set_nonblocking(&self, nonblocking: bool) -> std::io::Result<()> {
            TcpStream::set_nonblocking(self, nonblocking)
        }
    }

    #[cfg(unix)]
    impl PeerStream for std::os::unix::net::UnixStream {
        fn set_nonblocking(&self, nonblocking: bool) -> std::io::Result<()> {
            std::os::unix::net::UnixStream::set_nonblocking(self, nonblocking)
        }
    }

    /// A connected TCP loopback pair `(ours, peer)`.
    pub(crate) fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let ours = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (peer, _) = listener.accept().unwrap();
        ours.set_nodelay(true).unwrap();
        (ours, peer)
    }

    /// Like [`tcp_pair`], with small socket buffers on both ends so tests
    /// can fill the connection (backpressure, partial writes) with a few
    /// megabytes on every platform, whatever its buffer auto-tuning does.
    pub(crate) fn small_buffer_tcp_pair() -> (TcpStream, TcpStream) {
        let (ours, peer) = tcp_pair();
        for s in [&ours, &peer] {
            let sock = socket2::SockRef::from(s);
            sock.set_send_buffer_size(64 * 1024).unwrap();
            sock.set_recv_buffer_size(64 * 1024).unwrap();
        }
        (ours, peer)
    }

    /// The connected pair most tests use: a Unix socketpair on Unix (as
    /// these tests always have), TCP loopback on Windows.
    #[cfg(unix)]
    pub(crate) fn stream_pair() -> std::io::Result<(PairStream, PairStream)> {
        std::os::unix::net::UnixStream::pair()
    }

    /// The connected pair most tests use: a Unix socketpair on Unix (as
    /// these tests always have), TCP loopback on Windows. The TCP buffers
    /// are pinned to Linux's socketpair default (208 KiB) so tests that
    /// fill a connection behave the same on both.
    #[cfg(windows)]
    pub(crate) fn stream_pair() -> std::io::Result<(PairStream, PairStream)> {
        let (ours, peer) = tcp_pair();
        for s in [&ours, &peer] {
            let sock = socket2::SockRef::from(s);
            sock.set_send_buffer_size(212_992)?;
            sock.set_recv_buffer_size(212_992)?;
        }
        Ok((ours, peer))
    }

    /// The stream type [`stream_pair`] returns.
    #[cfg(unix)]
    pub(crate) type PairStream = std::os::unix::net::UnixStream;
    /// The stream type [`stream_pair`] returns.
    #[cfg(windows)]
    pub(crate) type PairStream = TcpStream;

    /// Local port of a bound listener, looked up through its raw handle
    /// (so tests also prove the exposed handle is the real socket).
    pub(crate) fn local_port(s: super::RawSocket) -> u16 {
        #[cfg(unix)]
        // SAFETY: `s` is a listener the server under test keeps open for
        // the duration of this call; it is only borrowed.
        let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(s) };
        #[cfg(windows)]
        // SAFETY: as above.
        let borrowed = unsafe { std::os::windows::io::BorrowedSocket::borrow_raw(s) };
        socket2::SockRef::from(&borrowed)
            .local_addr()
            .unwrap()
            .as_socket()
            .unwrap()
            .port()
    }

    /// Hand one end of a [`stream_pair`] to a plaintext transport.
    pub(crate) fn transport_from(stream: PairStream) -> crate::transport::Transport {
        crate::transport::Transport::new_plain(super::into_raw(stream))
    }
}
