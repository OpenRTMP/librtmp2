# Protocol Mapping — Legacy RTMP

This document maps the Adobe *RTMP Specification 1.0* onto the `librtmp2`
source tree, so contributors can find the code that implements each part of the
spec. Section numbers refer to the Adobe RTMP 1.0 spec.

## Handshake (spec §5.2)

| Spec element | Implementation |
|--------------|----------------|
| C0/S0 version byte | `server_read_c0()` / `client_read_s0()` in `src/handshake.rs` |
| C1/S1 (time + zero + random) | `server_read_c1()` / `client_read_s1()` in `src/handshake.rs` |
| C2/S2 echo | `server_read_c2()` / `client_read_s2()` in `src/handshake.rs` |
| Partial-read buffering, timeout | `src/handshake.rs` (`struct Handshake`), driven by `Conn::do_handshake()` in `src/session/conn.rs` |

Only the standard (non-encrypted, non-Adobe-digest) handshake is implemented;
more complex Adobe variants are out of scope for now.

## Chunk Stream (spec §5.3)

| Spec element | Implementation |
|--------------|----------------|
| Basic header (fmt + csid, 1/2/3-byte forms) | `chunk_read()` in `src/chunk/reader.rs`, `chunk_write()` in `src/chunk/writer.rs` |
| Message header types 0–3 | `chunk_read()` in `src/chunk/reader.rs` |
| Extended timestamp | `src/chunk/reader.rs`, `chunk_write_extended_timestamp()` in `src/chunk/writer.rs` |
| Per-`csid` state carry-forward | `ChunkRegistry` / `ChunkStream` in `src/chunk/state.rs` (per-connection, not global) |
| Reassembly to full messages | `src/chunk/reader.rs` → `message::message::decode()` in `src/message/message.rs` |

## Protocol Control Messages (spec §5.4)

| Message | Type ID | Implementation |
|---------|---------|----------------|
| Set Chunk Size | 1 | `read_set_chunk_size()` / `write_set_chunk_size()` in `src/message/control.rs` (applied immediately to chunk state via `Conn::apply_chunk_size()` in `src/session/conn.rs`) |
| Abort Message | 2 | `read_abort_message()` / `write_abort_message()` in `src/message/control.rs` |
| Acknowledgement | 3 | `read_acknowledgement_size()` / `write_acknowledgement()` in `src/message/control.rs` |
| Window Acknowledgement Size | 5 | `read_window_ack_size()` / `write_window_ack_size()` in `src/message/control.rs` |
| Set Peer Bandwidth | 6 | `read_set_peer_bandwidth()` / `write_set_peer_bandwidth()` in `src/message/control.rs` |

## User Control Messages (spec §6.2 / 7.1.7)

| Element | Implementation |
|---------|----------------|
| StreamBegin / StreamEOF / SetBufferLength / PingRequest / PingResponse | `read_user_control()` and the `write_user_control_*()` functions in `src/message/control.rs` |

## Command Messages / NetConnection & NetStream (spec §7.2)

| Command | Direction | Implementation |
|---------|-----------|----------------|
| `connect` | C→S | decode `read_connect()`, encode `build_connect()` in `src/message/command.rs`; dispatch `Conn::handle_command()` in `src/session/conn.rs` |
| `createStream` | C→S | `read_create_stream()` / `build_create_stream()` in `src/message/command.rs` + `Conn::handle_command()` in `src/session/conn.rs` |
| `publish` | C→S | `read_publish()` / `build_publish()` in `src/message/command.rs` + `publish_begin()` in `src/session/publish.rs` |
| `play` | C→S | `read_play()` / `build_play()` in `src/message/command.rs` + `play_begin()` in `src/session/play.rs` |
| `deleteStream` | C→S | accepted as no-op in `Conn::handle_command()` (`src/session/conn.rs`) |
| `FCPublish` / `FCUnpublish` / `releaseStream` | C→S | accepted as no-ops in `Conn::handle_command()` (`src/session/conn.rs`) |
| `_result` / `_error` | S→C | `Conn::send_connect_response()`, `Conn::send_create_stream_response()` (`src/session/conn.rs`), built via `build_create_stream_result()` in `src/message/command.rs` |
| `onStatus` | S→C | `Conn::send_onstatus()` (`src/session/conn.rs`), built via `build_onstatus()` in `src/message/command.rs` |

## AMF0 (spec §A; AMF0 spec)

| Type | Implementation |
|------|----------------|
| Number, Boolean, String, Object, Null, ECMA Array, Strict Array, Long String | `src/amf/amf0.rs` (`read_number`, `read_string`, `read_object_begin`, `write_ecma_array_begin`, etc.) |
| Nesting-depth bound (DoS guard) | `skip_value_depth()` / `MAX_SKIP_DEPTH` in `src/amf/amf0.rs` |

AMF3 (`src/amf/amf3.rs`) is implemented for completeness but not required by the
legacy connect/publish/play flows.

## Audio / Video / Script Data Messages (spec §7.1)

| Message | Type ID | Implementation |
|---------|---------|----------------|
| Audio | 8 | `parse()` in `src/flv/audio_tag.rs` → `Frame` (`src/types.rs`) |
| Video | 9 | `parse()` in `src/flv/video_tag.rs` → `Frame` (`src/types.rs`) |
| Data / Script (`@setDataFrame`, `onMetaData`) | 18/15 | `parse()` in `src/flv/script_tag.rs` (hardened against malformed input) |

## State Machine (spec flow)

```text
TCP_ACCEPTED → HANDSHAKE → CONNECTED → APP_CONNECTED
            → STREAM_CREATED → PUBLISHING | PLAYING → CLOSING → CLOSED
```

Implemented in `src/session/state_machine.rs` (`conn_transition()`, `ConnState`), driven by `src/session/conn.rs` (`struct Conn`).

## Tests

- Handshake: inline `#[cfg(test)] mod tests` in `src/handshake.rs`
- Chunk types 0–3, extended timestamp: inline `#[cfg(test)] mod tests` in
  `src/chunk/reader.rs`, `src/chunk/writer.rs`, `src/chunk/state.rs`
- AMF0 primitives/objects: inline `#[cfg(test)] mod tests` in `src/amf/amf0.rs`
- Command encode/decode and control messages: inline `#[cfg(test)] mod tests`
  in `src/message/command.rs` and `src/message/control.rs`
- Connection dispatch (`handle_command`, control handling): inline
  `#[cfg(test)] mod tests` in `src/session/conn.rs`
- End-to-end ingest and client publish over loopback (handshake → connect →
  createStream → publish/play → frame): `tests/server_client_loopback.rs`
