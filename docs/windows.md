# Windows

librtmp2 builds and runs natively on Windows for **x86_64**
(`x86_64-pc-windows-msvc`) and **ARM64** (`aarch64-pc-windows-msvc`), with
the same feature set as on Linux and macOS: RTMP and RTMPS, client and
server, Enhanced RTMP v1/v2, relay, authorization callbacks, the stream
cache, external relay export/inject and the full `extern "C"` API.

## Requirements

- Rust (stable, MSRV as in `Cargo.toml`) with the MSVC target for your
  architecture: `rustup target add x86_64-pc-windows-msvc` or
  `aarch64-pc-windows-msvc`.
- Visual Studio Build Tools (MSVC linker and Windows SDK).
- For the default `tls` feature: OpenSSL 3 headers and libraries. The
  simplest source is [vcpkg](https://vcpkg.io); the static `-static-md`
  triplets give a DLL that needs no OpenSSL DLLs at run time.

Like every Rust `cdylib` built with MSVC, `librtmp2.dll` uses the dynamic C
runtime, so the target machine needs the Microsoft Visual C++
Redistributable (2015–2022), which current Windows installations normally
already have.

## Building

```powershell
# OpenSSL, statically linked (x64; use arm64-windows-static-md on ARM64).
# VCPKG_ROOT is the vcpkg checkout, e.g. C:\vcpkg.
$env:VCPKG_ROOT = "C:\vcpkg"
& "$env:VCPKG_ROOT\vcpkg.exe" install openssl:x64-windows-static-md
$env:OPENSSL_DIR = "$env:VCPKG_ROOT\installed\x64-windows-static-md"
$env:OPENSSL_STATIC = "1"

cargo build --release --target x86_64-pc-windows-msvc
```

This produces `librtmp2.dll`, its import library `librtmp2.dll.lib` and the
static library `librtmp2.lib` under `target\<triple>\release\`. The C header
is `include\librtmp2\librtmp2.h`, identical on every platform.

A plaintext-only build needs no OpenSSL at all:

```powershell
cargo build --release --no-default-features
```

## Testing

```powershell
cargo test --all-features
cargo test --no-default-features
# Loopback integration tests, serially:
cargo test --all-features --test server_client_loopback --test cross_platform_transport -- --test-threads=1
# Check a built DLL: architecture, dependencies, C ABI, a real listener
scripts\windows-dll-smoke.ps1 -Dll target\x86_64-pc-windows-msvc\release\librtmp2.dll -Arch x86_64
```

The unit tests that use a Unix socketpair on Unix run over a TCP loopback
connection on Windows; the vectored-write and `Transport` behaviour suites
run against both stream types on Unix and against TCP on Windows, so both
socket backends are held to the same cases. `tests/cross_platform_transport.rs`
exercises RTMP and RTMPS publish/play, certificate verification, concurrent
players, frame ordering, slow consumers, resets and cleanup on every
platform. CI (`.github/workflows/cross-platform.yml`) runs all of this, plus
`rustfmt`, `clippy -D warnings`, a release build and the DLL smoke test,
natively on Windows x64 and Windows ARM64 runners.

The interop shell scripts in `tests/interop/` need a POSIX shell and are not
run on Windows.

## Packages

Signed ZIP packages for `x86_64` and `arm64` (DLL, import and static
libraries, header, README, LICENSE, OpenSSL license notice, SHA-256 checksum
and OpenPGP signature) are built by the release workflow in
[OpenRTMP/packages](https://github.com/OpenRTMP/packages) and served from
`https://packages.openrtmp.org/windows/<arch>/<version>/`.

## Platform differences

### Socket handles

The public fields and methods that expose a socket use the platform's
native handle type, `librtmp2::net::RawSocket`:

| API | Unix | Windows |
|-----|------|---------|
| `net::RawSocket` | `RawFd` (`i32`) | `RawSocket` (`u64`, a Winsock `SOCKET`) |
| "no socket" value | `-1` | `INVALID_SOCKET` (`!0`) |

This covers `Client::client_fd`, `Conn::client_fd`, `Conn::get_fd()`,
`Server::server_fd`, `Server::listener_fds()`, `Transport::fd()`,
`Transport::new_plain()` and `TlsCtx::accept()`. On Unix these keep their
previous `i32` signatures exactly. On Windows a `SOCKET` is never truncated
to `i32`. Use `net::INVALID_SOCKET` instead of comparing against `-1` or
`>= 0` to stay portable, and `Transport::from_tcp_stream()` to wrap a
`TcpStream` without touching raw handles.

`Transport::new_plain()` takes ownership of the socket on both platforms and
closes it exactly once on drop. On Windows it also switches the socket to
non-blocking mode, because Winsock has no per-call `MSG_DONTWAIT`.

The C API is unchanged: no exported function or struct carries a socket
handle (`lrtmp2_conn_get_fd()` returns `-1` on every platform, as before).

### `SO_REUSEPORT`

`Server::listen_reuseport()` and `Server::listen_tls_reuseport()` return
`ErrorCode::Unsupported` on Windows. Windows has no `SO_REUSEPORT`, and its
`SO_REUSEADDR` lets a second socket take over a bound port instead of having
the kernel spread connections across listeners, so it is not substituted.
Plain `listen()` / `listen_tls()` work normally, including several listeners
on one `Server`. Sharded multi-listener setups (for example
`LRTMP2_RTMP_SHARDS` in librtmp2-server) need Linux or macOS.

### Signals

On Unix the library ignores `SIGPIPE` once when the first TLS connection is
set up (unless the host installed its own disposition). Windows has no
`SIGPIPE`; a write to a reset peer fails with `WSAECONNRESET`, which is
reported as `ErrorCode::Io` like `EPIPE` on Unix.

## Socket backend design

All OS socket calls go through `src/net/`, selected at compile time with
`#[cfg(unix)]` / `#[cfg(windows)]`. There is no runtime dispatch and no
allocation; each operation is one system call on either platform.

| Operation | Unix (`src/net/unix.rs`) | Windows (`src/net/windows.rs`) |
|-----------|--------------------------|--------------------------------|
| receive | `recv(MSG_DONTWAIT)` | `recv` on a non-blocking socket |
| send | `send(MSG_DONTWAIT \| MSG_NOSIGNAL)` | `send` |
| vectored send | `sendmsg` with a stack `iovec[512]` | `WSASend` with a stack `WSABUF[512]` |
| readiness | `poll(2)` | `WSAPoll` |
| errors | `errno` | `WSAGetLastError()` |
| close | `close(2)` | `closesocket` |

Both backends classify results the same way: bytes transferred, "would
block" (`EAGAIN`/`EWOULDBLOCK`/`EINTR`, or `WSAEWOULDBLOCK`/`WSAEINTR`), or a
fatal error (reset, aborted, invalid handle, ...), which keeps the
`Transport` return conventions and `ErrorCode` mapping identical.

Vectored relay writes keep their zero-copy shape on Windows: shared media
payloads are passed to `WSASend` by reference, at most 512 buffers per
call, partial writes resume from the returned byte count, and a single part
larger than `WSABUF`'s 32-bit length is clamped and ends the batch so bytes
can never be reordered.

`WSAPoll` is used only to wait on one socket at a time (client reads/writes,
TLS handshake progress, blocking sends with a deadline), which is the case it
handles correctly; it is never used to detect connect failures. A
negative timeout waits indefinitely and error/hang-up states count as ready,
exactly like `poll(2)`. The server's own `poll()` loop is unchanged and
platform-independent.

TLS runs on OpenSSL over a non-blocking `TcpStream` on both platforms:
certificate and hostname/IP verification, SNI, custom CA bundles,
`tls_insecure`, non-blocking server handshakes with `WANT_READ`/`WANT_WRITE`
readiness and handshake deadlines all behave the same.

## Performance

The Unix code paths are the same system calls with the same flags and the
same stack-allocated buffers as before; the abstraction is a set of
`#[inline]` functions taking the raw handle by value. The Windows backend
has the same per-operation shape (one system call, no allocation, no payload
copy).

Linux before and after the socket layer change, with librtmp2-server
relaying one 2.5 Mbit/s stream:

| Viewers | Server CPU (% of one core) | Peak RSS | Delivered |
|---------|----------------------------|----------|-----------|
| 1000 | 71.5 → 69.0 | 32 → 32 MiB | full frame rate, 1.1 Gbit/s |
| 2000 | 89.8 → 90.5 | 51.6 → 52.9 MiB | full frame rate, 2.2 Gbit/s |

The Criterion relay benchmarks are unchanged (`relay/publish_to_player/100`
+0.7 %, `relay/publish_to_player/500` ±0 %).
