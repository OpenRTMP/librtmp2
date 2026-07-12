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

## [0.2.2] — 2026-07-12

### Fixed
- `lrtmp2_client_create()` silently ignored `ServerConfig.tls_ca_file` and
  `ServerConfig.tls_insecure` — the client always verified `rtmps://` peers
  against only the system trust store regardless of what those fields were
  set to, even though the ABI documented them as controlling client TLS
  verification. `Client`/`Transport::connect_tls` now honor a caller-supplied
  CA bundle or an explicit opt-out of verification.
- `Transport::connect_tls()`'s `ca_file` option now *replaces* the trust
  store instead of adding the caller's CA bundle to the system default
  trust store — a custom CA is meant to restrict which peers are trusted,
  not merely extend the existing set. Previously a publicly trusted
  certificate for the same hostname would still pass even with a private CA
  configured.
- `insecure = true` no longer loads the system's default CA verify paths at
  all, so `rtmps://` connections with verification intentionally disabled no
  longer fail on hosts without a usable default CA store.
- `lrtmp2_client_create()` now rejects (returns NULL for) a non-UTF-8
  `tls_ca_file` path instead of silently discarding it and falling back to
  default verification.

### Changed
- `Transport::connect_tls()` (Rust-only API, not part of the FFI ABI) gained
  two new parameters (`ca_file: Option<&str>`, `insecure: bool`) to support
  the fix above.

## [0.2.1] — 2026-07-10

### Added
- `lrtmp2_tls_supported()` runtime capability check exported to FFI
- Parse `onMetaData` from RTMP data messages into `Conn` fields (duration, width, height, framerate, videodatarate, audiodatarate, codec info)

### Fixed
- `flv::audio_tag` / `video_tag` / `script_tag` parsers now reset the
  caller-owned tag struct at the start of every `parse()` call, so switching
  between codecs mid-stream (or a shorter value following a longer one, e.g.
  a script tag name) no longer leaves stale fields from a previous parse
- CodeQL invalid-pointer alert in FFI `server_create` test resolved
- Borrow checker errors in AMF0 `skip_value_depth`
- `onMetaData` parsing scope and lifecycle per review feedback

### Security
- Cap and copy FFI frame payloads before sending and reject an oversized
  `frame.size`; ignore inbound media whose `msg_stream_id` doesn't match
  the current stream; retain `on_frame_cb` payloads in connection-scoped
  scratch buffers instead of a shared one
- Add per-poll (256 KiB) and per-command-wait (256 KiB) byte budgets to the
  RTMP client's recv path, mirroring the server's existing fairness cap, so
  a malicious server can no longer monopolize the embedder's event-loop
  thread or force hundreds of megabytes through the AMF connect handshake
- `lrtmp2_server_create` now substitutes a default `max_connections` (256)
  when the FFI caller passes a zero-initialized `ServerConfig` (e.g. via
  `calloc`/`{0}`), which previously disabled all connection limiting; an
  explicit negative value continues to mean "unlimited", matching
  `Server::new`

### Documentation
- Improved `onMetaData` parsing robustness per CodeRabbit/Codex reviews

## [0.2.0] — 2026-07-10

### Added
- RTMP Aggregate message (`0x16`) handling: aggregate-framed audio/video
  from a publisher is now unpacked and relayed through the normal
  media-frame path instead of being silently dropped

### Fixed
- `Client::connect()` now actually accepts `rtmps://` URLs via the new
  `Transport::connect_tls()` (dialing over TLS and verifying the server
  certificate against the system trust store) — previously only the
  server side implemented RTMPS, despite the 0.1.0 notes describing
  client-side `rtmps://` support; `parse_rtmp_url()` now recognizes the
  scheme and defaults to port 443
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
- FFI `lrtmp2_client_send_frame` and `Client::send_frame_payload` now
  enforce the max client frame-size cap consistently on every call path,
  and reject a stale/non-owned frame pointer when the client isn't in the
  `Publishing` state, instead of only checking on one of two paths
- The server poll loop keeps draining a connection's already-buffered
  messages (up to 3 extra passes) when a batch exceeds the per-recv message
  budget, instead of stalling until the peer happens to send more bytes
- The RTMPS client now checks TLS support before dialing, instead of
  opening a plaintext TCP connection first even in a
  `--no-default-features` build that given an `rtmps://` URL would only
  reject after connecting
- The client's blocking read helpers now poll for write-readiness (not
  just read-readiness) when a TLS read reports `WANT_WRITE` during
  renegotiation, instead of stalling until timeout on the wrong direction
- The client's TLS handshake is now bounded by the same 10s timeout used
  elsewhere in the transport, instead of blocking indefinitely if a peer
  stalls mid-handshake
- Retry `poll(2)` on `EINTR` in the client's transport-readiness wait
  helper instead of surfacing it as a hard I/O error and aborting the
  caller's read/handshake

### Security
- Cap and copy FFI frame payloads before sending and reject an oversized
  `frame.size`; ignore inbound media whose `msg_stream_id` doesn't match
  the current stream; retain `on_frame_cb` payloads in connection-scoped
  scratch buffers instead of a shared one
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

[Unreleased]: https://github.com/OpenRTMP/librtmp2/compare/v0.2.2...HEAD
[0.2.2]: https://github.com/OpenRTMP/librtmp2/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/OpenRTMP/librtmp2/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/OpenRTMP/librtmp2/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/OpenRTMP/librtmp2/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/OpenRTMP/librtmp2/releases/tag/v0.1.0
