# librtmp2 Server — Project Concept

## Purpose

`librtmp2` is a modern open-source C library for **Legacy RTMP** and **Enhanced RTMP v1/v2**, designed as a reusable protocol foundation for custom servers, clients, relay software, OBS/FFmpeg integrations, and future products. The library is deliberately **not** a media server itself, but rather the lowest layer: handshake, chunking, AMF, commands, audio/video tags, E-RTMP extensions, state machine, and callback API. [1][2]

The project will be published under the GitHub account **`AlexanderWagnerDev`**, in a target repository such as `AlexanderWagnerDev/librtmp2`. [3]

***

## Main Goals

- Implement a complete **Legacy RTMP** foundation: handshake, chunk streams, message reassembly, commands, control messages. [2]
- Support **E-RTMP v1**: ExVideoTagHeader, FourCC codecs, metadata extensions, HDR fields, audio extensions. [4]
- Support **E-RTMP v2**: capability negotiation, `videoFourCcInfoMap`, reconnect mechanism, multitrack, ModEx. [5][6]
- Provide a **clean C API** so that future servers or tools written in C, C++, Rust, Go, Python, PHP, or other FFI-capable languages can build on top of it.
- Strict **separation between core and product logic**: no HTTP API, no stats page, no database, no auth policy in the core.

***

## Non-Goals

The following do **not** belong in `librtmp2`:

- No HTTP server
- No web UI
- No stats web page
- No REST API
- No persistence / database
- No Docker-specific product logic
- No push targets to third-party platforms
- No FFmpeg wrapper
- No full media server with business logic

`librtmp2` is intentionally the **protocol library**, not the finished product.

***

## Architecture Principles

### 1. C as the Core Language

The core library shall be written in **C** to make it as broadly usable as possible. This enables direct use in OBS, FFmpeg, GStreamer, nginx modules, Rust FFI, Go CGo, or Python FFI. A native C library is the most universal form of distribution for infrastructure projects.

### 2. Small, Stable ABI

The public API shall be kept small, versionable, and stable over the long term. Internal structures may change; public headers must break as rarely as possible.

### 3. Strict Core / Thin Host

The library processes bytes, frames, commands, and protocol states. What a host program does with them is decided by the host application via callbacks and configuration structures.

### 4. Deterministic Parsers

All parsers must be deterministic, bounds-checked, and fuzzable. No undefined behavior, no implicit assumptions about incoming packets.

### 5. Graceful Degradation

Unknown E-RTMP v2 extensions — such as unknown ModEx types or unknown capability fields — must **not** cause a hard abort immediately, but must be ignored in a protocol-compliant manner or marked as "unsupported". [5][6]

***

## Repository Structure

```text
librtmp2/
├── README.md
├── LICENSE
├── CHANGELOG.md
├── CONTRIBUTING.md
├── Makefile
├── meson.build
├── include/
│   └── librtmp2/
│       ├── librtmp2.h
│       ├── version.h
│       ├── types.h
│       ├── errors.h
│       ├── callbacks.h
│       ├── server.h
│       ├── client.h
│       ├── frame.h
│       ├── audio.h
│       ├── video.h
│       ├── amf.h
│       └── ertmp.h
├── src/
│   ├── core/
│   │   ├── alloc.c
│   │   ├── buffer.c
│   │   ├── bytes.c
│   │   ├── log.c
│   │   └── errors.c
│   ├── handshake/
│   │   ├── handshake.c
│   │   └── handshake.h
│   ├── chunk/
│   │   ├── chunk_reader.c
│   │   ├── chunk_writer.c
│   │   ├── chunk_state.c
│   │   └── chunk_internal.h
│   ├── message/
│   │   ├── message.c
│   │   ├── control.c
│   │   ├── command.c
│   │   └── user_control.c
│   ├── amf/
│   │   ├── amf0.c
│   │   ├── amf3.c
│   │   ├── amf_common.c
│   │   └── amf_internal.h
│   ├── flv/
│   │   ├── audio_tag.c
│   │   ├── video_tag.c
│   │   └── script_tag.c
│   ├── ertmp/
│   │   ├── exvideo.c
│   │   ├── fourcc.c
│   │   ├── metadata.c
│   │   ├── connect_caps.c
│   │   ├── reconnect.c
│   │   ├── multitrack.c
│   │   └── modex.c
│   ├── session/
│   │   ├── conn.c
│   │   ├── stream.c
│   │   ├── publish.c
│   │   ├── play.c
│   │   └── state_machine.c
│   ├── server/
│   │   ├── server.c
│   │   └── server_internal.h
│   └── client/
│       ├── client.c
│       └── client_internal.h
├── tests/
│   ├── unit/
│   ├── integration/
│   ├── fuzz/
│   └── fixtures/
├── examples/
│   ├── minimal_server/
│   ├── minimal_client/
│   └── dump_frames/
├── docs/
│   ├── architecture.md
│   ├── protocol-mapping-legacy.md
│   ├── protocol-mapping-ertmp-v1.md
│   ├── protocol-mapping-ertmp-v2.md
│   ├── abi-policy.md
│   └── roadmap.md
└── .github/
    └── workflows/
        ├── ci.yml
        ├── interop.yml
        └── release.yml
```

***

## Public API Concept

The API must be low-level enough to remain flexible, but high-level enough that host programs do not have to implement chunk reassembly themselves.

### Core Types

```c
typedef struct lrtmp2_server lrtmp2_server_t;
typedef struct lrtmp2_client lrtmp2_client_t;
typedef struct lrtmp2_conn lrtmp2_conn_t;
typedef struct lrtmp2_stream lrtmp2_stream_t;
typedef struct lrtmp2_frame lrtmp2_frame_t;
typedef struct lrtmp2_error lrtmp2_error_t;
```

### Core Constructors

```c
lrtmp2_server_t *lrtmp2_server_create(const lrtmp2_server_config_t *config);
void lrtmp2_server_destroy(lrtmp2_server_t *server);

int lrtmp2_server_listen(lrtmp2_server_t *server, const char *bind_addr);
int lrtmp2_server_poll(lrtmp2_server_t *server, int timeout_ms);

lrtmp2_client_t *lrtmp2_client_create(const lrtmp2_client_config_t *config);
void lrtmp2_client_destroy(lrtmp2_client_t *client);
int lrtmp2_client_connect(lrtmp2_client_t *client, const char *url);
```

### Callback Model

```c
typedef int (*lrtmp2_on_connect_cb)(lrtmp2_conn_t *conn, void *userdata);
typedef int (*lrtmp2_on_publish_cb)(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata);
typedef int (*lrtmp2_on_play_cb)(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata);
typedef int (*lrtmp2_on_frame_cb)(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *userdata);
typedef void (*lrtmp2_on_close_cb)(lrtmp2_conn_t *conn, void *userdata);
```

The host registers these hooks and decides whether a publish is permitted, where frames are routed, and how logging and auth work.

***

## Protocol Modules

### 1. Handshake Module

Legacy RTMP uses the classic C0/C1/C2 ↔ S0/S1/S2 handshake. The library must fully support at least the standard handshake; more complex Adobe variants can be added later. [2]

Responsibilities:
- Detect version
- Robustly read/write the fixed-length handshake
- Handle timeouts
- Correctly buffer partial reads

### 2. Chunk Module

RTMP fragments messages into chunks with a basic header, message header, optional extended timestamp, and payload. The library requires complete reassembly per chunk stream ID, including header types 0–3 and state carry-forward. [2]

Responsibilities:
- Chunk reader
- Chunk writer
- State per `csid`
- Apply `SetChunkSize` immediately
- Handle `Abort` correctly

### 3. Message Module

Message reassembly produces semantic messages such as `SetChunkSize`, `Acknowledgement`, `WindowAcknowledgementSize`, `UserControlMessage`, audio, video, and command messages. [2]

### 4. AMF Module

AMF0 is mandatory; AMF3 is optional but useful for completeness. Connect, CreateStream, Publish, and Play flows require clean encoding and decoding of nested objects, arrays, and strings.

### 5. Session and Command Module

The library requires an internal state machine for:
- `connect`
- `createStream`
- `publish`
- `play`
- `deleteStream`
- `FCPublish`
- `FCUnpublish`

The goal is to avoid leaving host applications to deal with raw AMF arrays on their own.

***

## Enhanced RTMP v1

E-RTMP v1 extends RTMP/FLV primarily with modern codecs, FourCC signaling, and metadata. [4]

### Key Points

- Detect the `IsExHeader` bit in the VideoTagHeader
- Switch from legacy `CodecID` to `PacketType + FourCC`
- Support FourCC-based codecs such as `hvc1`, `av01`, `vp09` [4]
- Extended `PacketTypeMetadata` frames for things like `colorInfo` and HDR metadata [4]
- Support `fourCcList` in the connect object [4]

### Internal Structures

```c
typedef struct {
    uint8_t is_ex_header;
    uint8_t packet_type;
    char fourcc[5];
    uint8_t frame_type;
    uint32_t composition_time;
} lrtmp2_video_header_t;
```

***

## Enhanced RTMP v2

According to the specification, E-RTMP v2 adds in particular capability negotiation, multitrack, reconnect, and ModEx. [5][6]

### Key Points

- `capsEx` and `videoFourCcInfoMap` in the connect/response exchange [5]
- Reconnect mechanism for controlled redirection or maintenance [6]
- Multiple tracks per session / stream [6]
- ModEx as an extension mechanism without hard protocol breaks [5]

### Internal Tasks

- Parse and serialize capability objects
- Manage track descriptors
- Receive and send reconnect frames
- Log and ignore unknown ModEx types

***

## State Machine

```text
TCP_ACCEPTED
  -> HANDSHAKE
  -> CONNECTED
  -> APP_CONNECTED
  -> STREAM_CREATED
  -> PUBLISHING | PLAYING
  -> CLOSING
  -> CLOSED
```

With E-RTMP v2, an additional state for capability negotiation is logically added:

```text
CONNECTED
  -> CAPS_NEGOTIATED
  -> STREAM_CREATED
```

This state machine shall be fully implemented in the core so that host applications can react to semantically meaningful events.

***

## Memory and Security Rules

- Validate all input lengths before every read / copy
- No direct `malloc(len)` without upper bounds
- No trust in payload lengths from the network
- Optional custom allocator hook for host integration
- Fuzzing for handshake, chunk reader, AMF, and ExVideoTagHeader
- CI with AddressSanitizer and UndefinedBehaviorSanitizer

***

## Error Classes

```c
typedef enum {
    LRTMP2_OK = 0,
    LRTMP2_ERR_IO,
    LRTMP2_ERR_TIMEOUT,
    LRTMP2_ERR_PROTOCOL,
    LRTMP2_ERR_HANDSHAKE,
    LRTMP2_ERR_CHUNK,
    LRTMP2_ERR_AMF,
    LRTMP2_ERR_UNSUPPORTED,
    LRTMP2_ERR_AUTH,
    LRTMP2_ERR_INTERNAL
} lrtmp2_error_code_t;
```

Errors must be available in both machine-readable and human-readable form.

***

## Testing Strategy

### Unit Tests

- Handshake with golden bytes
- Chunk types 0–3
- Extended timestamp
- AMF0 primitive and complex objects
- ExVideoTagHeader parsing
- FourCC parsing
- Capability negotiation parsing

### Integration Tests

- OBS → `librtmp2` minimal server
- FFmpeg → `librtmp2` minimal server
- `librtmp2` minimal client → SRS
- Later HaishinKit → `librtmp2`

### Fuzzing

- Handshake parser
- Chunk reader
- AMF decoder
- E-RTMP header parser

***

## Build System

Recommendation:
- **Meson** or **CMake** for platform portability
- Additionally a simple `Makefile` for Linux development

Example targets:
- `make debug`
- `make release`
- `make test`
- `make fuzz`
- `make asan`
- `make install`

Artifacts:
- `librtmp2.so`
- `librtmp2.a`
- `librtmp2.dll`
- `librtmp2.lib`
- pkg-config file `librtmp2.pc`

***

## Releases and Versioning

SemVer:
- `0.x` while the API/ABI is still evolving
- `1.0.0` once the header/API is stable

Planned initial versions:
- `0.1.0` Legacy RTMP server minimal
- `0.2.0` Legacy RTMP client minimal
- `0.3.0` E-RTMP v1 receive
- `0.4.0` E-RTMP v1 send
- `0.5.0` E-RTMP v2 capability layer
- `0.6.0` Multitrack / reconnect / ModEx

***

## Phase Plan

### Phase 1 — Legacy Core MVP

- Handshake
- Chunk reader/writer
- Message reassembly
- AMF0
- `connect` / `createStream` / `publish`
- Minimal server example

**Acceptance criterion:** OBS can send H.264 to `librtmp2`.

### Phase 2 — Client MVP

- Outbound connect
- Publish flow
- Play flow (rudimentary)

**Acceptance criterion:** `librtmp2` client can publish to SRS.

### Phase 3 — E-RTMP v1

- ExVideoTagHeader
- FourCC
- HDR / metadata
- `fourCcList`

**Acceptance criterion:** HEVC/AV1 streams are detected and correctly parsed. [4]

### Phase 4 — E-RTMP v2

- `capsEx`
- `videoFourCcInfoMap`
- Reconnect
- Multitrack
- ModEx

**Acceptance criterion:** v2 negotiation without hard failures against known peers. [5]

### Phase 5 — Hardening

- ASan/UBSan
- Fuzzing
- ABI stability
- Packaging
- Docs

***

## Success Criteria

`librtmp2` is successful when:

- it is buildable as a standalone, small C library,
- OBS and FFmpeg can be tested against it,
- it provides Legacy RTMP and E-RTMP v1/v2 as a reusable foundation,
- other projects can build their own servers or clients on top of it,
- and a separate product with an API and stats page can emerge from it later.

***

## GitHub and Organization Concept

Recommended initial repository:

- `https://github.com/AlexanderWagnerDev/librtmp2`

Recommended branches:
- `main`
- `develop`

Recommended labels:
- `protocol`
- `legacy-rtmp`
- `e-rtmp-v1`
- `e-rtmp-v2`
- `amf`
- `chunking`
- `client`
- `server`
- `fuzzing`
- `interop`
- `good first issue`
