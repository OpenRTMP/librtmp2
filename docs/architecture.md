# Architecture

`librtmp2` is the lowest protocol layer of RTMP: it turns a raw byte stream into
semantic RTMP/FLV frames and protocol events, and back again. It deliberately
contains no media-server business logic — routing, auth policy, persistence, and
HTTP/stats all belong to the host application.

## Layered Overview

```text
            Host application (server / client / relay)
                          │  callbacks + config
┌─────────────────────────▼──────────────────────────────┐
│                       librtmp2                           │
│                                                         │
│   server/         client/                               │
│      │                │                                  │
│      └──────┬─────────┘                                  │
│             ▼                                            │
│          session/        state machine, publish/play    │
│             │                                            │
│             ▼                                            │
│          message/        reassembly, control, command   │
│             │                                            │
│      ┌──────┼───────┐                                    │
│      ▼      ▼       ▼                                    │
│    amf/   flv/    ertmp/   AMF0/3, FLV tags, E-RTMP v1/2 │
│      │      │       │                                    │
│      └──────┼───────┘                                    │
│             ▼                                            │
│          chunk/          chunk reader/writer + csid state│
│             │                                            │
│             ▼                                            │
│        handshake.rs      C0/C1/C2 ↔ S0/S1/S2             │
│             │                                            │
│             ▼                                            │
│   alloc/buffer/bytes/log  buffers, bytes, alloc, logging │
└─────────────────────────────────────────────────────────┘
                          │  bytes (TCP)
                          ▼
                     OS socket
```

## Module Responsibilities

| Module (`src/`) | Responsibility |
|-----------------|----------------|
| `alloc.rs`, `buffer.rs`, `bytes.rs`, `log.rs` | Allocation hook, growable buffers, big-endian byte helpers, logging. No protocol logic. |
| `handshake.rs` | Legacy C0/C1/C2 ↔ S0/S1/S2 handshake; version detection; partial-read buffering. |
| `chunk/`     | Chunk reader/writer (`reader.rs`, `writer.rs`) for header types 0–3, extended timestamps, per-`csid` state carry-forward (`state.rs`), `SetChunkSize`/`Abort`. |
| `message/`   | Reassembled-message dispatch (`message.rs`): control messages (`control.rs`), user-control messages, and AMF command encode/decode (`command.rs`). |
| `amf/`       | AMF0 (mandatory, `amf0.rs`) and AMF3 (optional, `amf3.rs`) readers/writers for primitives, strings, objects, arrays. |
| `flv/`       | FLV audio/video/script tag parsing (`audio_tag.rs`, `video_tag.rs`, `script_tag.rs`) — the payload format carried inside RTMP audio/video messages. |
| `ertmp/`     | Enhanced-RTMP v1/v2: ExVideo/ExAudio headers (`exvideo.rs`, `exaudio.rs`), FourCC registry (`fourcc.rs`), HDR metadata (`metadata.rs`), capability negotiation (`connect_caps.rs`), reconnect (`reconnect.rs`), multitrack (`multitrack.rs`), ModEx (`modex.rs`). |
| `session/`   | The connection object (`conn.rs`, `struct Conn`), per-connection state machine (`state_machine.rs`), stream bookkeeping (`stream.rs`), publish/play flows (`publish.rs`, `play.rs`). |
| `server/`    | Listening socket, accept loop, per-connection driving via `Server::poll()` (`src/server/mod.rs`; exposed over FFI as `lrtmp2_server_poll()`). |
| `client/`    | Outbound connect → createStream → publish/play, frame send, and receive polling (`src/client/mod.rs`, `struct Client`). |

## Data Flow (ingest)

1. The host calls `Server::poll()` (FFI: `lrtmp2_server_poll()`). The server
   reads available bytes per connection.
2. `handshake.rs` consumes bytes until the handshake completes, then hands the
   connection to the chunk layer.
3. `chunk/` reassembles complete messages per `csid`, applying chunk-size and
   abort control as it goes.
4. `message/message.rs::decode()` classifies each message. Command messages
   are decoded via `amf/` and dispatched by `Conn::handle_command()`
   (`src/session/conn.rs`); audio/video messages are parsed by `flv/` +
   `ertmp/` into a `Frame` (`src/types.rs`).
5. `session/` advances the state machine (`connect` → `createStream` →
   `publish`/`play`) and emits responses (`_result`, `onStatus`) via
   `Conn::send_connect_response()`, `Conn::send_create_stream_response()`,
   and `Conn::send_onstatus()`.
6. The host's `on_connect` / `on_publish` / `on_play` / `on_frame` / `on_close`
   callbacks fire with semantic events — never raw AMF arrays.

The client path mirrors this in reverse for outbound connections.

## Design Principles

- **Strict core / thin host** — the library moves bytes and state; policy is the
  host's job (see `concept/librtmp2-core.md`).
- **Deterministic, bounds-checked parsers** — every parser is bounds-checked
  and never trusts a length field from the network.
- **Graceful degradation** — unknown E-RTMP v2 ModEx types and capability fields
  are ignored protocol-compliantly rather than aborting the connection.
- **Small, stable ABI** — only the `#[no_mangle] pub extern "C"` functions and
  `#[repr(C)]` types exported from `src/lib.rs` are public; everything else
  may change. See [`abi-policy.md`](abi-policy.md).

## Threading Model

The library does no threading of its own. Per-connection chunk state
(`ChunkRegistry` in `src/chunk/state.rs`) is kept per connection rather than
process-global, so a host may run a client and a server in the same process,
or one connection per thread, without shared mutable state between
connections. Each `Conn` must, however, be driven from a single thread at a
time.
