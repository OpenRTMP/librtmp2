# Protocol Mapping — Enhanced RTMP v2

This maps the Veovera *Enhanced RTMP v2* specification onto the `librtmp2`
source tree. Enhanced RTMP v2 adds capability negotiation, multitrack,
reconnect, and the ModEx extension mechanism on top of v1.

Source spec: [`veovera/enhanced-rtmp` — enhanced-rtmp-v2.md](https://github.com/veovera/enhanced-rtmp/blob/main/docs/enhanced/enhanced-rtmp-v2.md).

## Capability Negotiation (`capsEx`, `videoFourCcInfoMap`)

| Spec element | Implementation |
|--------------|----------------|
| `capsEx` parse/write (videoCodecId + audioCodecId as FourCC-encoded UI32) | `caps_exit_parse()` / `caps_exit_write()` in `src/ertmp/connect_caps.rs` |
| `videoFourCcInfoMap` ECMAArray parse/write | `video_fourcc_info_map_parse()` / `video_fourcc_info_map_write()` in `src/ertmp/connect_caps.rs` |

Internal structs: `CapsExit`, `VideoFourCcInfoMap` (`src/types.rs`).

State machine adds a `CapsNegotiated` state (`CAPS_NEGOTIATED` in
`conn_state_str()`) between `CONNECTED` and `APP_CONNECTED`
(`src/types.rs`'s `ConnState` enum, transitions handled by
`conn_transition()` in `src/session/state_machine.rs`).

## Reconnect

| Spec element | Implementation |
|--------------|----------------|
| Reconnect payload: `replay` (UI32) + `limit` (UI32), big-endian, 8 bytes | `reconnect_parse()` / `reconnect_write()` in `src/ertmp/reconnect.rs` |

Internal struct: `Reconnect` (`src/types.rs`).

## Multitrack

| Spec element | Implementation |
|--------------|----------------|
| Track descriptor: AMF0_NUMBER track id + AMF0_STRING name | `multitrack_parse()` / `multitrack_write()` in `src/ertmp/multitrack.rs` |
| Track types: audio (0), video (1), metadata (2) | `MultitrackType` enum in `src/types.rs`, used by `src/ertmp/multitrack.rs` |

Internal struct: `Multitrack` (`src/types.rs`).

## ModEx (Modular Extension)

| Spec element | Implementation |
|--------------|----------------|
| Marker byte `0x80 \| type` + payload | `modex_parse()` / `modex_write()` in `src/ertmp/modex.rs` |
| `NOP` (type 0) — 1 byte | `ModexType::Nop` in `src/types.rs`, handled in `src/ertmp/modex.rs` |
| `TIMESTAMP` (type 1) — 9 bytes (8-byte ns offset) | `ModexType::Timestamp` in `src/types.rs`, handled in `src/ertmp/modex.rs` |
| **Graceful degradation:** unknown types fall back to NOP | `modex_parse()` in `src/ertmp/modex.rs` |

Internal struct: `Modex` (`src/types.rs`).

This is the key "graceful degradation" requirement from the concept: an unknown
ModEx type or capability field never aborts the connection — it is ignored
protocol-compliantly.

## Tests

- `connect_caps.rs`'s capability structures (`capsEx`, `videoFourCcInfoMap`,
  `fourCcList`), `multitrack.rs`, `reconnect.rs`, and `modex.rs` (including
  unknown-ModEx → NOP degradation) are all covered by their inline
  `#[cfg(test)] mod tests` blocks.

## Known Limitations

- v2 structures parse and serialize correctly, but full v2 negotiation has not
  yet been verified against an external peer (OBS / SRS / HaishinKit). See
  `tests/interop/` for the in-tree interop coverage that exists today.
