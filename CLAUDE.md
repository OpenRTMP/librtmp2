# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`librtmp2` is a Rust port of the original C library — a 1:1 rewrite, not a
wrapper. There is no C source, Makefile, or Meson build left in this repo;
everything below is Cargo.

## Build Commands

```bash
cargo build                       # debug build
cargo build --release             # release build
cargo test                        # unit tests (inline `#[cfg(test)]` modules) + tests/server_client_loopback.rs
cargo clippy --all-features --all-targets
cargo fmt
```

**TLS / RTMPS** is enabled by default via the `tls` feature (OpenSSL). Build a
zero-dependency, plaintext-only library with `cargo build --no-default-features`.
The transport abstraction lives in `src/` under the client/server modules;
plaintext and TLS share one send/recv path so the layers above never branch on
the wire type.

**Interop scripts** — standalone, not part of `cargo test`:
```bash
tests/interop/ffmpeg_interop.sh
tests/interop/enhanced_rtmp_interop.sh
tests/interop/play_interop.sh
```

**ABI baseline** (see `docs/abi-policy.md`):
```bash
scripts/abi-baseline.sh dump           # generate baseline ABI dump
scripts/abi-baseline.sh compare HEAD   # compare current vs HEAD/tag
```

## Architecture

`librtmp2` is a pure protocol library — no media server logic, no HTTP, no
auth policy. The host application registers callbacks and the library
delivers semantic events. It exposes both an idiomatic Rust API and an
FFI-compatible `extern "C"` layer (built as `cdylib`/`staticlib`/`lib`, see
`src/lib.rs`) for consumption from C, Go, Python, PHP, and others.

**Module layout (bottom → top):**

```
core (alloc.rs, bytes.rs, buffer.rs, log.rs, error.rs)
                    growable buffers, big-endian byte helpers, logging, errors
handshake.rs        C0/C1/C2 <-> S0/S1/S2; partial-read buffering; version detection
chunk/              chunk_reader, chunk_writer, chunk_state (per-csid); SetChunkSize/Abort
message/            reassembled message dispatch: control, user-control, AMF command decode/encode
amf/                AMF0 (mandatory) + AMF3 (optional)
flv/                FLV audio/video/script tag parsing
ertmp/              E-RTMP v1 (ExVideo/ExAudio, FourCC, HDR) + v2 (capsEx, reconnect, multitrack, ModEx)
session/            connection object (conn.rs), state machine, stream bookkeeping, publish/play flows
server/             listening socket, accept loop, per-connection poll
client/             outbound connect -> createStream -> publish/play, frame send, receive poll
```

**Ingest data flow:** `lrtmp2_server_poll()` -> `handshake` -> `chunk` (reassemble per csid) -> `message` (classify) -> `amf` (decode commands) or `flv` + `ertmp` (decode frames) -> `session` (advance state machine, emit `_result`/`onStatus`) -> host callbacks (`on_connect`, `on_publish`, `on_play`, `on_frame`, `on_close`).

**Connection state machine:**
```
TCP_ACCEPTED -> HANDSHAKE -> CONNECTED -> [CAPS_NEGOTIATED] -> APP_CONNECTED -> STREAM_CREATED -> PUBLISHING | PLAYING -> CLOSING -> CLOSED
```
`CAPS_NEGOTIATED` is the E-RTMP v2 capability exchange state between CONNECTED and APP_CONNECTED.

## Key Design Rules

**ABI boundary:** Only the `extern "C"` functions exported from `src/lib.rs`
(and the types they take/return) are the public FFI surface. Everything else
in `src/` is free to change between minor versions. See `docs/abi-policy.md`
for the full compatibility policy.

**Threading:** The library is single-threaded per connection. Per-connection
state (chunk state, session state) is not global, so a client and server can
coexist in the same process or on separate threads without shared mutable
state. Each connection object must be driven from one thread at a time.

**Parser safety:** Never trust network-provided length fields. All parsers
are bounds-checked. Unknown E-RTMP v2 ModEx types must degrade gracefully to
NOP, not panic.

## Test Structure

- Unit tests live inline next to the code they cover (`#[cfg(test)] mod tests`
  in each module) — run with `cargo test`
- `tests/server_client_loopback.rs` — integration tests that spin up real
  server/client pairs over loopback TCP
- `tests/interop/` — shell scripts for interop testing against ffmpeg and
  other real-world clients (not run by `cargo test`)

## Versioning

SemVer: `0.x` while API/ABI is evolving; `1.0.0` once stable. Before any
release, run `scripts/abi-baseline.sh compare` against the previous release —
see `docs/abi-policy.md` for the full checklist. The crate version in
`Cargo.toml` is the single source of truth; `version_string()` /
`lrtmp2_version_string()` read it via `CARGO_PKG_VERSION` rather than
duplicating it.
