# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-06-27

### Added
- **Legacy RTMP server** — minimal listening socket, accept loop, per-connection driving via `lrtmp2_server_poll()`
- **Legacy RTMP client** — outbound connect → createStream → publish/play, frame send, and receive polling
- **Handshake** — C0/C1/C2 ↔ S0/S1/S2 with version detection and partial-read buffering
- **Chunk layer** — chunk reader/writer for header types 0–3, extended timestamps, per-csid state carry-forward
- **AMF** — AMF0 and AMF3 readers/writers for primitives, strings, objects, arrays
- **Message reassembly** — control messages, user-control messages, AMF command encode/decode
- **FLV tags** — audio/video/script tag parsing
- **E-RTMP v1** — ExVideoTagHeader, ExAudioTagHeader, FourCC registry (`hvc1`, `av01`, `vp09`, `Opus`, `mp4a`), HDR color info, `fourCcList`
- **E-RTMP v2** — capability negotiation (`capsEx`, `videoFourCcInfoMap`), reconnect, multitrack, ModEx
- **State machine** — TCP_ACCEPTED → HANDSHAKE → CONNECTED → CAPS_NEGOTIATED → APP_CONNECTED → STREAM_CREATED → PUBLISHING/PLAYING → CLOSING → CLOSED
- **Public C API** — stable headers in `include/librtmp2/` with FFI compatibility
- **pkg-config** — `librtmp2.pc` with correct version, prefix, and lib flags
- **Meson build** — `meson.build` with optional tests and examples
- **Unit tests** — handshake, buffer, AMF0, chunk, E-RTMP v1+v2 (5 suites, all passing)
- **Integration tests** — server ingest, client publish, E-RTMP v1, E-RTMP v2
- **Fuzz harnesses** — for all critical parsers (AMF0, chunk, handshake, ertmp audio/video, modex)
- **Examples** — `minimal_server` and `minimal_client`
- **CI** — tests workflow (gcc/clang, make/meson, ASan/UBSan), release workflow (tagged releases with binary + source tarballs)
- **ABI policy** — documented in `docs/abi-policy.md`
- **Roadmap** — documented in `docs/roadmap.md`

### Build Artifacts
- `librtmp2.so` — shared library (SONAME `librtmp2.so.0`)
- `librtmp2.a` — static library
- `librtmp2.pc` — pkg-config file

### Known Limitations
- Real-peer interop (OBS → librtmp2, librtmp2 → SRS) not yet verified against live instances
- `dump_frames` example not yet added
- ABI not yet frozen (pre-1.0.0)
