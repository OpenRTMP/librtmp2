//! Unix socket backend: `recv(2)`/`send(2)`/`sendmsg(2)`/`poll(2)`.
//!
//! Sockets may be in blocking mode: every call passes `MSG_DONTWAIT`, so a
//! single operation never blocks regardless of the descriptor's `O_NONBLOCK`
//! flag. Writes also pass `MSG_NOSIGNAL` so a reset peer surfaces as `EPIPE`
//! instead of a process-killing `SIGPIPE`.

use super::{Interest, MAX_SEND_PARTS, RawSocket, SockIo, Wait};
use std::mem::MaybeUninit;

#[inline]
fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Classify a failed (`-1`) socket call by `errno`.
#[inline]
fn classify_error() -> SockIo {
    let err = last_errno();
    if err == libc::EINTR || err == libc::EAGAIN || err == libc::EWOULDBLOCK {
        SockIo::WouldBlock
    } else {
        SockIo::Failed
    }
}

/// Nothing to do on Unix: every call passes `MSG_DONTWAIT`, so the
/// descriptor's blocking mode is left exactly as the caller set it.
#[inline]
pub(crate) fn prepare_transport_socket(_fd: RawSocket) -> bool {
    true
}

/// Non-blocking `recv(2)` into `buf`.
#[inline]
pub(crate) fn recv(fd: RawSocket, buf: &mut [u8]) -> SockIo {
    // SAFETY: `buf` is a valid, writable region of `buf.len()` bytes for the
    // duration of the call. An invalid `fd` is reported as EBADF, not UB.
    let n = unsafe {
        libc::recv(
            fd,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
            libc::MSG_DONTWAIT,
        )
    };
    if n < 0 {
        classify_error()
    } else {
        SockIo::Done(n as usize)
    }
}

/// Non-blocking `send(2)` of `data`.
#[inline]
pub(crate) fn send(fd: RawSocket, data: &[u8]) -> SockIo {
    // SAFETY: `data` is a valid, readable region of `data.len()` bytes for
    // the duration of the call.
    let n = unsafe {
        libc::send(
            fd,
            data.as_ptr() as *const libc::c_void,
            data.len(),
            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
        )
    };
    if n < 0 {
        classify_error()
    } else {
        SockIo::Done(n as usize)
    }
}

/// Fill `iov` with one entry per non-empty part (the first part minus its
/// first `first_offset` bytes), up to `iov.len()` entries; returns how many
/// were filled. Entries past the returned count stay uninitialised.
fn fill_iovecs(
    parts: &[&[u8]],
    first_offset: usize,
    iov: &mut [MaybeUninit<libc::iovec>],
) -> usize {
    let mut len = 0;
    for (i, part) in parts.iter().enumerate() {
        if len == iov.len() {
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
        iov[len].write(libc::iovec {
            iov_base: part.as_ptr() as *mut libc::c_void,
            iov_len: part.len(),
        });
        len += 1;
    }
    len
}

/// Non-blocking vectored write of `parts` (skipping the first
/// `first_offset` bytes of `parts[0]`) in one `sendmsg(2)`. At most
/// [`MAX_SEND_PARTS`] non-empty parts go out per call; the `iovec` array
/// lives on the stack, so this never allocates. Returns `Done(0)` without a
/// system call when there is nothing to send.
#[inline]
pub(crate) fn send_vectored(fd: RawSocket, parts: &[&[u8]], first_offset: usize) -> SockIo {
    let mut iov = [MaybeUninit::<libc::iovec>::uninit(); MAX_SEND_PARTS];
    let len = fill_iovecs(parts, first_offset, &mut iov);
    if len == 0 {
        return SockIo::Done(0);
    }
    // SAFETY: an all-zero msghdr is valid; `iov[..len]` was initialised by
    // `fill_iovecs` and points into `parts`, which outlives the call and is
    // only read by sendmsg.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = iov.as_mut_ptr().cast::<libc::iovec>();
    msg.msg_iovlen = len as _;
    let n = unsafe { libc::sendmsg(fd, &msg, libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL) };
    if n < 0 {
        classify_error()
    } else {
        SockIo::Done(n as usize)
    }
}

/// Wait up to `timeout_ms` (negative: forever) for `fd` to become ready in
/// direction `interest`, with one `poll(2)` call.
#[inline]
pub(crate) fn poll_one(fd: RawSocket, interest: Interest, timeout_ms: i32) -> Wait {
    let mut pfd = libc::pollfd {
        fd,
        events: match interest {
            Interest::Read => libc::POLLIN,
            Interest::Write => libc::POLLOUT,
        },
        revents: 0,
    };
    // SAFETY: `pfd` is one valid pollfd for the duration of the call.
    let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    if rc > 0 {
        Wait::Ready
    } else if rc == 0 {
        Wait::TimedOut
    } else if last_errno() == libc::EINTR {
        Wait::Interrupted
    } else {
        Wait::Failed
    }
}

/// Close an owned descriptor. Negative values are ignored.
#[inline]
pub(crate) fn close(fd: RawSocket) {
    if fd >= 0 {
        // SAFETY: the caller owns `fd` and never uses it again.
        unsafe { libc::close(fd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_iovecs_caps_at_capacity_and_skips_empties() {
        let data = [1u8; 4];
        let parts: Vec<&[u8]> = vec![&data[..0], &data[..2], &data[..0], &data[2..], &data];
        let mut iov = [MaybeUninit::<libc::iovec>::uninit(); 2];
        assert_eq!(fill_iovecs(&parts, 0, &mut iov), 2);
        // SAFETY: both entries were just filled.
        let first = unsafe { iov[0].assume_init() };
        assert_eq!(first.iov_len, 2);
        let second = unsafe { iov[1].assume_init() };
        assert_eq!(second.iov_len, 2);
    }
}
