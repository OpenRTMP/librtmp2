# librtmp2

A modern, open-source **C library** for Legacy RTMP and Enhanced RTMP v1/v2.  
`librtmp2` is a reusable protocol foundation — not a media server.

[![License](https://img.shields.io/github/license/AlexanderWagnerDev/librtmp2)](LICENSE)
[![Status](https://img.shields.io/badge/status-concept%2F%20pre--alpha-orange)]()
[![Language](https://img.shields.io/badge/language-C-blue)]()

---

## Overview

`librtmp2` implements the lowest protocol layer of RTMP: handshake, chunking, AMF, commands, audio/video tags, Enhanced RTMP extensions, state machine, and a clean callback API.

It is designed to be embedded into custom servers, clients, relay tools, OBS/FFmpeg integrations, or any future product that needs a solid RTMP foundation.

**What it is:**
- A complete Legacy RTMP implementation (handshake, chunk streams, message reassembly, commands, control messages)
- E-RTMP v1 support: ExVideoTagHeader, FourCC codecs (`hvc1`, `av01`, `vp09`), HDR metadata
- E-RTMP v2 support: capability negotiation (`capsEx`, `videoFourCcInfoMap`), reconnect, multitrack, ModEx
- A stable C API with FFI compatibility for C++, Rust, Go, Python, PHP, and others

**What it is not:**
- Not an HTTP server or web UI
- Not a media server with business logic
- Not a push-relay to third-party platforms
- Not an FFmpeg wrapper

---

## Architecture

```text
OBS / FFmpeg / App
        │
        ▼
  librtmp2-server  ←  future product built on top
        │
        ▼
      librtmp2          ← this library
      ├── Handshake
      ├── Chunking
      ├── AMF (AMF0 / AMF3)
      ├── RTMP Commands
      ├── E-RTMP v1
      └── E-RTMP v2
```

The library processes bytes, frames, commands, and protocol states. What a host program does with them is decided entirely via callbacks and configuration structures.

---

## State Machine

```text
TCP_ACCEPTED
  → HANDSHAKE
  → CONNECTED
  → CAPS_NEGOTIATED     (E-RTMP v2)
  → APP_CONNECTED
  → STREAM_CREATED
  → PUBLISHING | PLAYING
  → CLOSING
  → CLOSED
```

---

## Public API (Concept)

### Core Types

```c
typedef struct lrtmp2_server  lrtmp2_server_t;
typedef struct lrtmp2_client  lrtmp2_client_t;
typedef struct lrtmp2_conn    lrtmp2_conn_t;
typedef struct lrtmp2_stream  lrtmp2_stream_t;
typedef struct lrtmp2_frame   lrtmp2_frame_t;
typedef struct lrtmp2_error   lrtmp2_error_t;
```

### Server / Client

```c
lrtmp2_server_t *lrtmp2_server_create(const lrtmp2_server_config_t *config);
void             lrtmp2_server_destroy(lrtmp2_server_t *server);
int              lrtmp2_server_listen(lrtmp2_server_t *server, const char *bind_addr);
int              lrtmp2_server_poll(lrtmp2_server_t *server, int timeout_ms);

lrtmp2_client_t *lrtmp2_client_create(const lrtmp2_client_config_t *config);
void             lrtmp2_client_destroy(lrtmp2_client_t *client);
int              lrtmp2_client_connect(lrtmp2_client_t *client, const char *url);
```

### Callbacks

```c
typedef int  (*lrtmp2_on_connect_cb)(lrtmp2_conn_t *conn, void *userdata);
typedef int  (*lrtmp2_on_publish_cb)(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata);
typedef int  (*lrtmp2_on_play_cb)   (lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata);
typedef int  (*lrtmp2_on_frame_cb)  (lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *userdata);
typedef void (*lrtmp2_on_close_cb)  (lrtmp2_conn_t *conn, void *userdata);
```

---

## Error Codes

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

---

## Build

```bash
# Debug build
make debug

# Release build
make release

# Run tests
make test

# AddressSanitizer
make asan

# Fuzzing targets
make fuzz

# Install
make install
```

Build artifacts: `librtmp2.so`, `librtmp2.a`, `librtmp2.dll`, `librtmp2.lib`, `librtmp2.pc`

---

## Repository Structure

```text
librtmp2/
├── include/librtmp2/       Public headers
├── src/
│   ├── core/               Memory, logging, errors
│   ├── handshake/          C0/C1/C2 ↔ S0/S1/S2
│   ├── chunk/              Chunk reader/writer/state
│   ├── message/            Message reassembly, control, commands
│   ├── amf/                AMF0 + AMF3
│   ├── flv/                Audio/video/script tags
│   ├── ertmp/              E-RTMP v1/v2 extensions
│   ├── session/            State machine, publish/play flows
│   ├── server/             Server listener
│   └── client/             Outbound client
├── tests/
│   ├── unit/
│   ├── integration/
│   └── fuzz/
├── examples/
│   ├── minimal_server/
│   ├── minimal_client/
│   └── dump_frames/
├── docs/
└── concept/                Project concept documents
```

---

## Roadmap

| Version | Milestone |
|---------|-----------|
| `0.1.0` | Legacy RTMP server — minimal (OBS → librtmp2 working) |
| `0.2.0` | Legacy RTMP client — minimal (librtmp2 → SRS working) |
| `0.3.0` | E-RTMP v1 receive (HEVC/AV1 detection) |
| `0.4.0` | E-RTMP v1 send |
| `0.5.0` | E-RTMP v2 capability layer |
| `0.6.0` | Multitrack / reconnect / ModEx |

---

## Concept Documents

Detailed design documents are located in [`concept/`](concept/):

- [`concept/librtmp2-core.md`](concept/librtmp2-core.md) — Core library architecture, API design, protocol modules, security rules
- [`concept/librtmp2-server.md`](concept/librtmp2-server.md) — Future server product built on top of this library

---

## Related Projects

| Repository | Description |
|---|---|
| `AlexanderWagnerDev/librtmp2-server` | RTMP/E-RTMP server with HTTP API and stats UI (planned) |

---

## License

To be determined. See [LICENSE](LICENSE) once added.
