# Roadmap

This roadmap tracks the phased plan from
[`concept/librtmp2-core.md`](../concept/librtmp2-core.md) against the actual
state of the code. The concept document holds the authoritative,
continuously-updated implementation-status tracker; this file is the
release-oriented summary.

## Version Milestones (SemVer)

While the API/ABI is still evolving the project stays in the `0.x` range; `1.0.0`
is cut once the public headers in `include/librtmp2/` are considered stable (see
[`abi-policy.md`](abi-policy.md)).

| Version | Milestone | Status |
|---------|-----------|--------|
| `0.1.0` | Legacy RTMP server — minimal | Implemented; end-to-end tested in-tree |
| `0.2.0` | Legacy RTMP client — minimal | Implemented; tested against own server |
| `0.3.0` | E-RTMP v1 receive (HEVC/AV1 detection) | Implemented + tested |
| `0.4.0` | E-RTMP v1 send | Implemented + tested |
| `0.5.0` | E-RTMP v2 capability layer | Implemented + tested |
| `0.6.0` | Multitrack / reconnect / ModEx | Implemented + tested |

## Phase Status

### Phase 1 — Legacy Core MVP ✅
Handshake, chunk reader/writer, message reassembly, AMF0, `connect` /
`createStream` / `publish` / `play`, minimal server example. Covered by
`tests/integration/test_server_ingest.c`.

### Phase 2 — Client MVP ✅ (in-tree)
Full outbound flow in `src/client/client.c`, covered by
`tests/integration/test_client_publish.c`.

### Phase 3 — E-RTMP v1 ✅
ExVideo/ExAudio headers, FourCC registry, HDR/colorInfo, `fourCcList`. Covered by
`tests/unit/test_ertmp.c` and `tests/integration/test_server_ertmp_v1.c`.

### Phase 4 — E-RTMP v2 ✅
`capsEx`, `videoFourCcInfoMap`, reconnect, multitrack, ModEx. Covered by
`tests/integration/test_server_ertmp_v2.c`.

### Phase 5 — Hardening ✅ (mostly)
ASan/UBSan builds clean; fuzz harnesses for all critical parsers; meson +
pkg-config; `abi-policy.md`. CI runs unit/ASan/UBSan + ffmpeg/play interop.

## Open Items / Before 1.0.0

These are the remaining gaps tracked for future releases:

- **Real-peer interop verification** — the original acceptance criteria call for
  verification against external software. ffmpeg interop is wired
  (`tests/interop/`), but **OBS → librtmp2** and **librtmp2 → SRS** have not yet
  been confirmed against real instances. HaishinKit is a later target.
- **Release automation** — `.github/workflows/release.yml` builds and packages
  tagged releases (added). ABI checks via `abi-compliance-checker` are still
  manual (see `abi-policy.md`).
- **`dump_frames` example** — listed in the concept; `minimal_server` and
  `minimal_client` exist, `dump_frames` is not yet added.
- **Project docs** — `CHANGELOG.md` and `CONTRIBUTING.md` are not yet present.
- **ABI freeze** — once the headers are stable, cut `1.0.0` and begin enforcing
  the ABI policy with automated checks.

## Out of Scope (Non-Goals)

Per the concept, the following will **not** be added to `librtmp2`: HTTP server,
web UI, stats page, REST API, persistence/database, Docker product logic,
third-party push targets, FFmpeg wrapper, or full media-server business logic.
Those belong in a separate downstream product.
