# Changelog

All notable changes to this project will be documented in this file.

> ⚠️ **Alpha software.** `librtmp2` is in active early development. It has **no
> fixed, stable release version yet** — everything below is pre-release (alpha)
> and the API/ABI may change at any time without notice. Pin to a specific git
> commit if you depend on it.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
While in alpha the project stays on `0.x`; semantic-versioning guarantees only
begin at `1.0.0`.

## [Unreleased]

### Fixed
- Cap per-connection recv drain in `process_connections()` to 256 KiB per
  poll pass, preventing a peer that keeps its kernel recv buffer full from
  starving other sessions in the single-threaded poll loop

### Documentation
- Update docs.rs badge to track the latest published version

## [0.1.0] — 2026-07-08

First tagged pre-release. `librtmp2` is a Rust crate (built via Cargo as
`cdylib`/`staticlib`/`lib`) exposing both an idiomatic Rust API and an
FFI-compatible `extern "C"` layer for consumption from C, Go, Python, PHP,
and others.

### Added
- TLS / RTMPS support via OpenSSL, enabled by default through the `tls`
  Cargo feature (`cargo build --no-default-features` for a zero-dependency,
  plaintext-only build)
- Transport abstraction shared by plaintext RTMP and TLS so the layers above
  never branch on the wire type
- Server-side TLS termination and client-side `rtmps://` connect with SNI
  and certificate verification
- `lrtmp2_tls_supported()` runtime capability check
- Legacy RTMP protocol support (handshake, chunk, message, AMF0)
- Enhanced RTMP v1 support (ExVideo/ExAudio headers, FourCC registry, HDR/colorInfo)
- Enhanced RTMP v2 support (capsEx, reconnect, multitrack, ModEx)
- Full server API with callbacks (`on_connect`, `on_publish`, `on_play`, `on_frame`, `on_close`)
- Full client API with publish/play flows
- Frame API supporting audio, video, script, and metadata types
- H.264, H.265, AV1, and legacy video codec support
- AAC, Opus, MP3, G.711 audio codec support
- Example programs: `minimal_server`, `minimal_client`, `play_pull`, `ffmpeg_ingest`
- Inline unit tests throughout `src/`, an end-to-end loopback integration
  test (`tests/server_client_loopback.rs`), and interop shell scripts
  (`tests/interop/`)
- ABI baseline tooling (`scripts/abi-baseline.sh`) for `0.x` compatibility checks
- Automated ABI compliance checks in CI (`cargo-semver-checks` via `.github/workflows/abi-check.yml`)

### Security
- Bounds-checked parsers for all network-provided length fields
- Constant-time RNG for handshake
- Safe handling of unknown E-RTMP v2 ModEx types (degrades to NOP, not panic)

### Documentation
- `CLAUDE.md` with build commands and architecture guide
- `docs/abi-policy.md` with ABI compliance checklist
- Protocol mapping documents for legacy, E-RTMP v1, and E-RTMP v2
- `CONTRIBUTING.md` guidelines

[Unreleased]: https://github.com/OpenRTMP/librtmp2/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OpenRTMP/librtmp2/releases/tag/v0.1.0
