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
| `modExDataSize` — `UI8 + 1`, with `0xFF` switching to the extended `UI16 + 1` form | `modex_parse()` / `modex_parse_prefix()` / `modex_write()` in `src/ertmp/modex.rs`; the framing is shared with the live relay path via `media::modex::parse_modex_layer` |
| `modExData` — at least one byte | parsed as a slice; `TimestampOffsetNano` reads its first three bytes |
| Option byte `modExType:UB[4] \| nextPacketType:UB[4]` | `Modex::modex_type` + `Modex::next_packet_type` in `src/types.rs` |
| `TimestampOffsetNano` (`modExType` 0) — UI24 nanosecond offset | `ModexType::TimestampOffsetNano`, `Modex::offset` in `src/types.rs` |
| **Graceful degradation:** unknown/reserved `modExType` values are ignored | `modex_parse()` surfaces them as `ModexType::Nop` with a zero offset |

Internal struct: `Modex` (`src/types.rs`).

`modex_parse()` takes the wrapper body (it starts at `modExDataSize`, i.e.
immediately after the enhanced packet-type nibble) and parses one layer;
`modex_parse_prefix()` additionally returns the consumed length, which is how a
caller follows chained wrappers (`nextPacketType == 7`).
`ModexType::Nop` has no wire representation, so `modex_write()` emits nothing
for it.

This is the key "graceful degradation" requirement from the concept: an unknown
ModEx type or capability field never aborts the connection — it is ignored
protocol-compliantly.

> **Rust API note:** `ModexType::Timestamp` was renamed to
> `ModexType::TimestampOffsetNano` (the v2 wire type 0, a UI24 rather than an
> eight-byte offset), and `Modex` gained `next_packet_type`. A deprecated
> `ModexType::Timestamp` alias is kept. Neither type is part of the
> `extern "C"` surface.

## Tests

- `connect_caps.rs`'s capability structures (`capsEx`, `videoFourCcInfoMap`,
  `fourCcList`), `multitrack.rs`, `reconnect.rs`, and `modex.rs` (including
  unknown-ModEx → NOP degradation) are all covered by their inline
  `#[cfg(test)] mod tests` blocks.

## Known Limitations

- v2 structures parse and serialize correctly, but full v2 negotiation has not
  yet been verified against an external peer (OBS / SRS / HaishinKit). See
  `tests/interop/` for the in-tree interop coverage that exists today.
