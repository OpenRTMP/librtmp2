//! Windows socket backend: Winsock `recv`/`send`/`WSASend`/`WSAPoll`.
//!
//! Winsock has no per-call `MSG_DONTWAIT`, so every socket handed to a
//! [`Transport`](crate::transport::Transport) is switched to non-blocking
//! mode once ([`prepare_transport_socket`]); after that each call below
//! returns `WSAEWOULDBLOCK` instead of blocking, matching the Unix backend.
//! Windows has no `SIGPIPE`, so no `MSG_NOSIGNAL` equivalent is needed: a
//! write to a reset peer fails with `WSAECONNRESET`/`WSAECONNABORTED`.
//!
//! Errors are read with `WSAGetLastError()`, never `errno`.
//!
//! Winsock itself is initialised (`WSAStartup`) by the Rust standard
//! library the first time a `TcpStream`/`TcpListener` is created, which is
//! where every socket this crate uses comes from.

use super::{INVALID_SOCKET, Interest, MAX_SEND_PARTS, RawSocket, SockIo, Wait};
use std::mem::MaybeUninit;
use windows_sys::Win32::Networking::WinSock::{
    self as ws, FIONBIO, POLLRDNORM, POLLWRNORM, SOCKET, SOCKET_ERROR, WSABUF, WSAEINTR,
    WSAEWOULDBLOCK, WSAPOLLFD,
};

/// `RawSocket` (`u64`) to Winsock's `SOCKET` (`usize`). Lossless: every
/// `RawSocket` value originates from a `SOCKET` of the same platform.
#[inline]
fn sock(s: RawSocket) -> SOCKET {
    s as SOCKET
}

/// Classify a failed (`SOCKET_ERROR`) Winsock call.
#[inline]
fn classify_error() -> SockIo {
    // SAFETY: no preconditions; reads this thread's last Winsock error.
    let err = unsafe { ws::WSAGetLastError() };
    if err == WSAEWOULDBLOCK || err == WSAEINTR {
        SockIo::WouldBlock
    } else {
        SockIo::Failed
    }
}

/// Put `s` into non-blocking mode (`FIONBIO`). Returns `false` if Winsock
/// refuses, so the caller never drives a blocking socket from a poll loop.
/// An invalid handle is left alone and reported as ready: every later
/// operation on it fails anyway.
pub(crate) fn prepare_transport_socket(s: RawSocket) -> bool {
    if s == INVALID_SOCKET {
        return true;
    }
    let mut nonblocking: u32 = 1;
    // SAFETY: `nonblocking` is a valid u32 for the duration of the call.
    unsafe { ws::ioctlsocket(sock(s), FIONBIO, &mut nonblocking) != SOCKET_ERROR }
}

/// Non-blocking `recv` into `buf`. A buffer longer than `i32::MAX` bytes
/// is read into partially (Winsock lengths are `i32`).
#[inline]
pub(crate) fn recv(s: RawSocket, buf: &mut [u8]) -> SockIo {
    let len = buf.len().min(i32::MAX as usize) as i32;
    // SAFETY: `buf` is valid and writable for at least `len` bytes for the
    // duration of the call. An invalid handle yields WSAENOTSOCK, not UB.
    let n = unsafe { ws::recv(sock(s), buf.as_mut_ptr(), len, 0) };
    if n == SOCKET_ERROR {
        classify_error()
    } else {
        SockIo::Done(n as usize)
    }
}

/// Non-blocking `send` of `data`. Longer than `i32::MAX` bytes reports a
/// short write, which callers already resume from.
#[inline]
pub(crate) fn send(s: RawSocket, data: &[u8]) -> SockIo {
    let len = data.len().min(i32::MAX as usize) as i32;
    // SAFETY: `data` is valid and readable for at least `len` bytes.
    let n = unsafe { ws::send(sock(s), data.as_ptr(), len, 0) };
    if n == SOCKET_ERROR {
        classify_error()
    } else {
        SockIo::Done(n as usize)
    }
}

/// Fill `bufs` with one `WSABUF` per non-empty part (the first part minus
/// its first `first_offset` bytes), up to `bufs.len()` entries; returns how
/// many were filled. `WSABUF::len` is a `u32`: a part longer than
/// `u32::MAX` is clamped and ends the list, so bytes are never sent out of
/// order (the caller resumes from the short count). Entries past the
/// returned count stay uninitialised.
fn fill_wsabufs(parts: &[&[u8]], first_offset: usize, bufs: &mut [MaybeUninit<WSABUF>]) -> usize {
    let mut len = 0;
    for (i, part) in parts.iter().enumerate() {
        if len == bufs.len() {
            break;
        }
        let part = if i == 0 {
            part.get(first_offset..).unwrap_or(&[])
        } else {
            part
        };
        if part.is_empty() {
            continue;
        }
        let clamped = part.len() > u32::MAX as usize;
        bufs[len].write(WSABUF {
            len: part.len().min(u32::MAX as usize) as u32,
            // WSASend only reads through this pointer.
            buf: part.as_ptr() as *mut u8,
        });
        len += 1;
        if clamped {
            break;
        }
    }
    len
}

/// Non-blocking vectored write of `parts` (skipping the first
/// `first_offset` bytes of `parts[0]`) in one `WSASend`. At most
/// [`MAX_SEND_PARTS`] non-empty parts go out per call; the `WSABUF` array
/// lives on the stack, so this never allocates or copies payload. Returns
/// `Done(0)` without a system call when there is nothing to send.
#[inline]
pub(crate) fn send_vectored(s: RawSocket, parts: &[&[u8]], first_offset: usize) -> SockIo {
    let mut bufs = [MaybeUninit::<WSABUF>::uninit(); MAX_SEND_PARTS];
    let count = fill_wsabufs(parts, first_offset, &mut bufs);
    if count == 0 {
        return SockIo::Done(0);
    }
    let mut sent: u32 = 0;
    // SAFETY: `bufs[..count]` was initialised by `fill_wsabufs` and points
    // into `parts`, which outlives this synchronous (non-overlapped) call.
    // `sent` is a valid out-pointer.
    let rc = unsafe {
        ws::WSASend(
            sock(s),
            bufs.as_ptr().cast::<WSABUF>(),
            count as u32,
            &mut sent,
            0,
            std::ptr::null_mut(),
            None,
        )
    };
    if rc == SOCKET_ERROR {
        classify_error()
    } else {
        SockIo::Done(sent as usize)
    }
}

/// Wait up to `timeout_ms` (negative: forever) for `s` to become ready in
/// direction `interest`, with one `WSAPoll` call. Error and hang-up states
/// (`POLLERR`/`POLLHUP`/`POLLNVAL`) count as ready so the next I/O call
/// reports them, exactly like `poll(2)`.
#[inline]
pub(crate) fn poll_one(s: RawSocket, interest: Interest, timeout_ms: i32) -> Wait {
    let mut pfd = WSAPOLLFD {
        fd: sock(s),
        events: match interest {
            Interest::Read => POLLRDNORM,
            Interest::Write => POLLWRNORM,
        },
        revents: 0,
    };
    // SAFETY: `pfd` is one valid WSAPOLLFD for the duration of the call.
    let rc = unsafe { ws::WSAPoll(&mut pfd, 1, timeout_ms) };
    if rc > 0 {
        Wait::Ready
    } else if rc == 0 {
        Wait::TimedOut
    } else if unsafe { ws::WSAGetLastError() } == WSAEINTR {
        Wait::Interrupted
    } else {
        Wait::Failed
    }
}

/// Close an owned socket. [`INVALID_SOCKET`] is ignored.
#[inline]
pub(crate) fn close(s: RawSocket) {
    if s != INVALID_SOCKET {
        // SAFETY: the caller owns `s` and never uses it again.
        unsafe { ws::closesocket(sock(s)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_wsabufs_caps_at_capacity_and_skips_empties() {
        let data = [1u8; 4];
        let parts: Vec<&[u8]> = vec![&data[..0], &data[..2], &data[..0], &data[2..], &data];
        let mut bufs = [MaybeUninit::<WSABUF>::uninit(); 2];
        assert_eq!(fill_wsabufs(&parts, 0, &mut bufs), 2);
        // SAFETY: both entries were just filled.
        let first = unsafe { bufs[0].assume_init() };
        assert_eq!(first.len, 2);
        let second = unsafe { bufs[1].assume_init() };
        assert_eq!(second.len, 2);
    }

    #[test]
    fn fill_wsabufs_honours_first_offset() {
        let parts: [&[u8]; 2] = [b"abcdef", b"gh"];
        let mut bufs = [MaybeUninit::<WSABUF>::uninit(); 4];
        assert_eq!(fill_wsabufs(&parts, 4, &mut bufs), 2);
        // SAFETY: both entries were just filled.
        let first = unsafe { bufs[0].assume_init() };
        assert_eq!(first.len, 2);
        assert_eq!(first.buf as *const u8, parts[0][4..].as_ptr());
    }
}
