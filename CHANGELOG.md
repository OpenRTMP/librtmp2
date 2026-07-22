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

### Security
- The TLS pending-handshake queue's per-address cap is now keyed on the peer
  IP instead of the full `ip:port` peer address, so a single host can no
  longer bypass it by opening incomplete RTMPS handshakes from distinct
  ephemeral source ports.
- `on_media_cb` codec authorization now checks every track's codec inside a
  multitrack (`ManyTracks`/`ManyTracksManyCodecs`) container instead of only
  the first, closing a path where a disallowed codec could ride along
  behind an allowed first track.

### Added
- `ServerConfig::max_pending_tls_per_addr` — configures the per-peer-IP cap
  on incomplete TLS handshakes (default `4` when `0`/unset). Deployments
  where many clients share one source IP (NAT, load balancer, proxy) can
  raise this to avoid spurious RTMPS handshake evictions under bursty
  connect patterns.

## [0.4.2] — 2026-07-21

### Security
- `Client::publish()` and `Client::play()` now bound their blocking AMF
  exchange with the configured connect-timeout wall-clock deadline instead
  of passing `None`, closing a stall window where a malicious server could
  hold the caller for up to ~650 seconds (64 recv polls × 10s) after connect
  succeeded.
- AMF route strings (connect `app`, publish/play stream names) are now
  decoded with strict UTF-8 validation and the command is rejected on
  failure; invalid UTF-8 previously collapsed to an empty string via
  `unwrap_or`, letting distinct wire-level names collide onto the same
  `(app, stream)` relay route.
- `read_string_checked()` now rejects embedded NUL bytes instead of copying
  them verbatim and letting `decode_route_amf_string()`'s NUL-sentinel scan
  truncate the value later, which let distinct invalid route values collapse
  onto the same app/stream route — undermining the UTF-8 collision guard
  above.
- The server session layer now rejects empty connect `app` names and empty
  publish/play stream names, and gates `onMetaData`/script relay to players
  and the stream cache on `on_media_cb` being registered.

### Fixed
- `bytes_received` is now tracked as `u64` (was `u32`), so WindowAckSize
  pacing stays correct once a connection passes 4 GiB of inbound data
  instead of wrapping.
- The client's Aggregate-message play path now passes sub-tag slices
  directly to `on_frame_cb` instead of cloning each sub-tag into a new
  `Vec`.

## [0.4.1] — 2026-07-18

### Changed
- `Client::connect()` now enforces a single wall-clock deadline across DNS
  resolution, TCP/TLS connect, the RTMP handshake, and the AMF
  `connect`/`createStream` exchange (previously only DNS/TCP connect were
  bounded). The default budget is unchanged (`TCP_CONNECT_TIMEOUT_SECS`, 10s);
  callers on slower or more loaded hosts can raise it with the new
  `Client::set_connect_timeout()`.

### Fixed
- Capped chained ModEx extension unwrapping at 32 layers to prevent CPU
  amplification from deeply nested wrappers on media frames.
- `Conn` now tracks the exact `PublishRouteRegistry` key it claimed and
  releases that same key on `FCUnpublish`/`closeStream`/play takeover,
  instead of re-deriving a route key from the current `relay_key` at release
  time. Fixes stale route ownership (blocking other publishers) and stale
  `stream_cache` entries when a host integrator pins `relay_key` after the
  initial publish claim.
- `publish()` now clears `Stream.is_playing` (mirroring what `play()` already
  does for `is_publishing`), so a play→publish switch on the same connection
  is correctly observable by hosts polling stream role flags.

## [0.4.0] — 2026-07-15

### Added
- Server-side E-RTMP v2 connect negotiation on the live session path:
  `fourCcList`, numeric `capsEx` capability bitmask, `videoFourCcInfoMap`,
  `reconnect`, and NetConnection `_error` responses when capability negotiation
  fails.
- Multitrack media support (E-RTMP v2 `AudioPacketType::Multitrack` /
  `VideoPacketType::Multitrack`): opaque relay of full multitrack messages,
  per-track `on_frame_cb` delivery with `Frame.track_id`, and init-cache replay
  of multitrack sequence-start headers to late-joining players.
- Enhanced init-frame classification via `exvideo`/`exaudio` parsers (HEVC, AV1,
  Opus, AAC) in `media/init_cache.rs`, replacing legacy nibble-only detection.
- `onMetaData` script caching in `StreamCache` and replay to players that join
  after the publisher has already sent metadata.
- Client receive path for AMF0/AMF3 `onMetaData` and RTMP Aggregate messages
  (`0x16`), unpacking sub-tags into the normal A/V and script frame callbacks.
- Legacy RTMP commands on the server session path: `pause`, `seek`,
  `receiveAudio`, `receiveVideo`, and `closeStream`.
- User Control message handling for `StreamBegin`, `StreamEOF`, and
  `SetBufferLength`; AMF3 Shared Object messages are accepted as no-ops.
- `examples/dump_frames.rs` — play a stream and print one line per received frame
  (type, timestamp, size, codec details) for debugging live publishers.
- libFuzzer harnesses under `fuzz/` for chunk reading, handshake parsing, AMF0
  skipping, E-RTMP parsers, and RTMP control-message decoders.
- CI jobs: `.github/workflows/sanitizers.yml` (ASan unit tests, overflow-check
  unit tests, ASan example builds) and `.github/workflows/fuzz.yml` (scheduled
  libFuzzer smoke runs).

### Changed
- The built-in relay fan-out budget is configurable through
  `Server::max_relay_sends_per_poll` (default: 4096 sends per poll). The first
  eligible frame in a poll is always processed even when its audience exceeds
  the budget, preventing an oversized fan-out frame from being re-queued forever.
- Connect AMF helpers now parse and write E-RTMP v2 capability representations,
  including wildcard FourCC entries, numeric `capsEx`, and per-codec
  `videoFourCcInfoMap` masks. The built-in client continues to connect without
  advertising capabilities; negotiation is active on the server session path.
- `Frame` now carries populated codec/header fields (`video_fourcc`, `audio_fourcc`,
  composition time, etc.) and optional `track_id` for multitrack callbacks.
- Corrected E-RTMP audio/video packet-type constants for sequence-end,
  multichannel, and multitrack values.
- Removed `docs/roadmap.md`; release status lives in `README.md` and
  `CHANGELOG.md`.

### Fixed
- ModEx prefix bytes are normalized only when the ModEx capability was negotiated
  and the leading bytes form an unambiguous ModEx wrapper; enhanced
  ExVideo/ExAudio, legacy AAC, and multitrack tags are preserved. Relay always
  forwards the original payload.
- Aggregate processing preserves relative sub-tag timestamps for A/V and metadata,
  rejects malformed or truncated payloads, and routes script tags through the
  normal metadata path.
- Re-enabling `receiveAudio` or `receiveVideo` schedules init-cache replay;
  paused players and disabled media types are filtered during relay.
- Enhanced `CodedFramesX` keyframes and multitrack sequence headers are classified
  correctly for late-join cache replay.
- Complex RTMP handshake (digest/HMAC) is not implemented; peers requesting it
  still receive a legacy simple S1/S2 response so ffmpeg and similar clients connect.

### Security
- Bound E-RTMP capability blobs to 4 KiB and Aggregate messages to 4096 sub-tags
  on both server and client receive paths; stream-cache resource accounting now
  includes metadata and per-track headers.

## [0.3.1] — 2026-07-13

### Fixed
- `Client::connect()` DNS resolution now respects the TCP connect deadline
  instead of blocking indefinitely in the system resolver; lookups run on a
  single shared worker thread with a bounded job queue, and worker startup
  failures return `ErrorCode::Internal` instead of panicking.
- `send_frame_payload()` and `lrtmp2_client_send_frame` now service inbound
  RTMP UserControl ping requests before sending media, so in-tree clients
  stay connected when the server enforces ping timeouts.
- Ping responses issued while draining inbound messages during `poll()` or
  publishing now use nonblocking `try_send` instead of blocking
  `Transport::send()`, so a zero-timeout poll cannot stall for up to 10
  seconds on a peer that stops reading.
- `service_inbound_nonblocking()` now honors the same per-poll byte and
  message budgets as `poll()`, and stops reading on transient EAGAIN instead
  of spinning until the socket blocks.
- Server-side ping RTT tracking starts only after the ping has fully left
  `send_buffer`; unflushed pings queued behind a slow reader no longer start
  the timeout early, and pings stuck unflushed longer than `PING_TIMEOUT`
  now close the connection. RTT timing now tracks the ping's own queued byte
  range instead of waiting for the entire `send_buffer` to drain, so prompt
  ping responses are not discarded when later media remains queued.
- `Client::poll()` now works while publishing to service inbound pings and
  retry queued ping responses after transient `EAGAIN`, and the shared DNS
  worker is re-created after a transient thread-spawn failure instead of
  permanently caching the error in a `OnceLock`.
- Publishing clients now poll for socket writability (`POLLOUT`) when queued
  pong bytes remain after a transient `EAGAIN`, so idle publishers can flush
  keepalive responses without sitting out the full read timeout.
- Publishing clients also poll `POLLOUT` when the send queue backs up during
  media upload, and preserve the correct TLS poll direction when flushing
  queued pong bytes after a transient `EAGAIN`.
- Nonblocking publish sends now propagate `try_flush_send_buffer()` errors
  instead of dropping flush failures on the floor.

### Security
- Inbound peers that open TCP but never complete the AMF connect exchange
  (including partial legacy handshake bytes) are now closed after a 10-second
  setup deadline instead of holding a connection slot until
  `max_connections` is reached.
- Server connections with unanswered or stale outbound RTMP pings are now
  closed instead of accumulating indefinitely in `pending_pings`.
- DNS lookups abandoned after the connect deadline no longer spawn unbounded
  detached resolver threads, and the shared resolver job queue is capped so
  wedged lookups cannot grow heap usage without bound.

## [0.3.0] — 2026-07-12

### Fixed
- `lrtmp2_client_create()` silently ignored `ServerConfig.tls_ca_file` and
  `ServerConfig.tls_insecure` — the client always verified `rtmps://` peers
  against only the system trust store regardless of what those fields were
  set to, even though the ABI documented them as controlling client TLS
  verification. `Client`/`Transport::connect_tls` now honor a caller-supplied
  CA bundle or an explicit opt-out of verification.
- `Transport::connect_tls()`'s `ca_file` option now *replaces* the trust
  store instead of adding the caller's CA bundle to the system default trust
  store — a custom CA is meant to restrict which peers are trusted, not
  merely extend the existing set.
- `insecure = true` no longer loads the system's default CA verify paths at
  all, so `rtmps://` connections with verification intentionally disabled no
  longer fail on hosts without a usable default CA store.
- `lrtmp2_client_create()` now rejects (returns NULL for) a non-UTF-8
  `tls_ca_file` path instead of silently discarding it and falling back to
  default verification.

### Changed
- `Transport::connect_tls()` (Rust-only API, not part of the FFI ABI) gained
  two new parameters (`ca_file: Option<&str>`, `insecure: bool`) to support
  the fix above. Bumped the minor version per this crate's pre-1.0
  versioning policy (breaking Rust API change, ABI/FFI surface unaffected).

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

[Unreleased]: https://github.com/OpenRTMP/librtmp2/compare/v0.4.2...HEAD
[0.4.2]: https://github.com/OpenRTMP/librtmp2/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/OpenRTMP/librtmp2/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/OpenRTMP/librtmp2/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/OpenRTMP/librtmp2/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/OpenRTMP/librtmp2/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/OpenRTMP/librtmp2/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/OpenRTMP/librtmp2/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/OpenRTMP/librtmp2/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/OpenRTMP/librtmp2/releases/tag/v0.1.0
