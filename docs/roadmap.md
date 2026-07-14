# Roadmap

This roadmap tracks the phased implementation plan against the actual
state of the code. It is the release-oriented summary; see also
[`architecture.md`](architecture.md) for design principles.

## Release Strategy

The first public release is `0.1.0` and ships the complete feature set below —
legacy RTMP plus E-RTMP v1/v2 — rather than staging features across a sequence
of `0.x` releases. The project stays in the `0.x` range while the API/ABI is
still evolving; `1.0.0` is cut once the `#[no_mangle] pub extern "C"` FFI
surface exported from `src/lib.rs` is considered stable (see
[`abi-policy.md`](abi-policy.md)).

## Feature Status

Everything below is implemented and targeted for the `0.1.0` release.

| Feature | Status |
|---------|--------|
| Legacy RTMP server — minimal | Implemented; end-to-end tested in-tree |
| Legacy RTMP client — minimal | Implemented; tested against own server |
| E-RTMP v1 receive (HEVC/AV1 detection) | Implemented + tested |
| E-RTMP v1 send | Implemented + tested |
| E-RTMP v2 capability layer | Implemented + tested |
| Multitrack / reconnect / ModEx | Implemented + tested |

## Phase Status

### Phase 1 — Legacy Core MVP ✅
Handshake, chunk reader/writer, message reassembly, AMF0, `connect` /
`createStream` / `publish` / `play`, `examples/minimal_server.rs`. Covered by
`tests/server_client_loopback.rs`.

### Phase 2 — Client MVP ✅ (in-tree)
Full outbound flow in `src/client/mod.rs` (`struct Client`), covered by
`tests/server_client_loopback.rs`.

### Phase 3 — E-RTMP v1 ✅
ExVideo/ExAudio headers (`src/ertmp/exvideo.rs`, `src/ertmp/exaudio.rs`),
FourCC registry (`src/ertmp/fourcc.rs`), HDR/colorInfo
(`src/ertmp/metadata.rs`), `fourCcList` (`src/ertmp/connect_caps.rs`). Covered
by `tests/interop/enhanced_rtmp_interop.sh` (HEVC/AV1/Opus against real ffmpeg
encoders) plus the inline `#[cfg(test)] mod tests` in `src/ertmp/connect_caps.rs`.

### Phase 4 — E-RTMP v2 ✅
`capsEx`, `videoFourCcInfoMap`, reconnect, multitrack, ModEx
(`src/ertmp/connect_caps.rs`, `src/ertmp/reconnect.rs`,
`src/ertmp/multitrack.rs`, `src/ertmp/modex.rs`), all covered by inline
`#[cfg(test)] mod tests` (see
[`protocol-mapping-ertmp-v2.md`](protocol-mapping-ertmp-v2.md)).

### Phase 5 — Hardening ✅ (mostly)
Build tooling is pure Cargo (no meson/pkg-config); `abi-policy.md` tracks the
FFI ABI surface via `scripts/abi-baseline.sh`. CI (`.github/workflows/tests.yml`)
runs `cargo test` (default + `--no-default-features`) and `cargo clippy`, plus
the ffmpeg/play interop scripts under `tests/interop/`
(`.github/workflows/interop-ffmpeg.yml`, `interop-play.yml`). ASan/UBSan
sanitizer runs and dedicated fuzz harnesses are not currently wired up.

## Open Items / Before 1.0.0

These are the remaining gaps tracked for future releases:

- **Real-peer interop verification** — the original acceptance criteria call for
  verification against external software. ffmpeg interop is wired
  (`tests/interop/`), but **OBS → librtmp2** and **librtmp2 → SRS** have not yet
  been confirmed against real instances. HaishinKit is a later target.
- **Release automation** — `.github/workflows/release.yml` builds and packages
  tagged releases (added). ABI checks via `abi-compliance-checker` are still
  manual (see `abi-policy.md`).
- **`dump_frames` example** — planned but not yet added; `minimal_server` and
  `minimal_client` exist, `dump_frames` is not yet added.
- **Project docs** — `CHANGELOG.md` and `CONTRIBUTING.md` are not yet present.
- **ABI freeze** — once the FFI surface is stable, cut `1.0.0` and begin
  enforcing the ABI policy with automated checks.

## Out of Scope (Non-Goals)

The following will **not** be added to `librtmp2`: HTTP server,
web UI, stats page, REST API, persistence/database, Docker product logic,
third-party push targets, FFmpeg wrapper, or full media-server business logic.
Those belong in a separate downstream product.
