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

## [0.2.0] — 2026-07-10

### Fixed
- `flv::audio_tag` / `video_tag` / `script_tag` parsers now reset the
  caller-owned tag struct at the start of every `parse()` call, so switching
  between codecs mid-stream (or a shorter value following a longer one, e.g.
  a script tag name) no longer leaves stale fields from a previous parse
- The RTMP client's inbound recv budget accounting no longer discards an
  already-read chunk once it slightly exceeds the remaining budget; the read
  itself is now capped at the remaining budget so bytes belonging to the
  response being waited for are never dropped
- `Client::poll()` no longer risks blocking in `poll(2)` for the full
  timeout when TLS already has decrypted plaintext buffered internally from
  a previous budget-limited drain

### Security
- Add per-poll (256 KiB) and per-command-wait (256 KiB) byte budgets to the
  RTMP client's recv path, mirroring the server's existing fairness cap, so
  a malicious server can no longer monopolize the embedder's event-loop
  thread or force hundreds of megabytes through the AMF connect handshake
- `lrtmp2_server_create` now substitutes a default `max_connections` (256)
  when the FFI caller passes a zero-initialized `ServerConfig` (e.g. via
  `calloc`/`{0}`), which previously disabled all connection limiting; an
  explicit negative value continues to mean "unlimited", matching
  `Server::new`

## [0.1.1] — 2026-07-08

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

[Unreleased]: https://github.com/OpenRTMP/librtmp2/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/OpenRTMP/librtmp2/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/OpenRTMP/librtmp2/releases/tag/v0.1.0
