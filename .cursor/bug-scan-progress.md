# Bug scan progress

Last scanned: chunk (2026-06-30)

## Modules

- [x] core — Memory, logging, errors, buffer
- [x] handshake — C0/C1/C2 ↔ S0/S1/S2
- [x] chunk — Chunk reader/writer/state
- [ ] message — Message reassembly, control, commands
- [ ] amf — AMF0 + AMF3
- [ ] flv — Audio/video/script tags
- [ ] ertmp — E-RTMP v1/v2 extensions
- [ ] session — State machine, publish/play flows
- [ ] server — Server listener
- [ ] client — Outbound client

## Findings (2026-06-30 chunk pass)

- reader.rs: extended-timestamp check used the initial `available` byte count instead
  of `buf.available()` after consuming headers. A partial read with only the
  message header present passed the stale check and returned `Io` error, tearing
  down the connection instead of waiting for more data (`Ok(0)`).
- reader.rs: fmt=3 continuation chunks never consumed the 4-byte extended
  timestamp that `chunk_write` emits when `timestamp >= 0xFFFFFF`. Fragmented
  large-timestamp messages were mis-parsed (payload shifted by 4 bytes).
- reader.rs: fmt=0/fmt=1 did not reset partial reassembly state. A new message
  on the same csid after an incomplete prior message could false-complete with
  corrupted payload (stale `reassembly_bytes_read` vs new `msg_length`).
- reader.rs: `can_grow_reassembly` / `MAX_REASSEMBLY_BYTES_PER_CONN` was never
  enforced during reassembly; added per-chunk growth check before writing.

## Findings (2026-06-29 handshake pass)

- client.c: `lrtmp2_client_connect()` tore down transport/socket on reconnect
  but left `recv_buffer` and handshake state from the prior attempt. A second
  `connect()` on the same client object could consume stale S0/S1/S2 bytes and
  complete a handshake against the wrong peer data. Reset recv/send buffers and
  re-init handshake at the start of every connect.
- handshake.c: `hs->out` was heap-allocated via `LRTMP2_MALLOC` but left
  `owned=0`, so `lrtmp2_buffer_write()` would refuse growth (latent footgun).
  Set `owned=1` on malloc; check `buffer_write` return codes; allocate the
  output buffer in `client_read_s1` when C2 is queued before `generate_c0c1`
  ran (unit-test order and robustness).

## Findings (2026-06-28 core pass)

- buffer.c: `lrtmp2_buffer_write()` called `realloc()` on any buffer whose
  `capacity` was exceeded. External view buffers (stack/static storage with
  `memset` + `.data`/`.capacity` set, used throughout session/client for AMF
  encoding) are not heap-owned; growth past capacity invoked `realloc()` on a
  stack pointer → heap corruption / crash (e.g. long `description` in
  `lrtmp2_conn_send_onstatus`). Added `owned` flag set by `buffer_create`;
  non-owned buffers now return `LRTMP2_ERR_INTERNAL` instead of reallocating.
  Also hardened `buffer_compact()` when `read_pos >= size`.

## Findings (2026-06-27 full pass)

- server.c: connection teardown leaked the socket fd on graceful close and
  leaked the whole connection (fd + struct, no on_close) on a socket error;
  unparseable input was not torn down. Unified the CLOSING path.
- message/control.c + message.c + client.c: RTMP control-message decoders
  read fixed offsets with no length check; a short message read past the
  logical payload. Added payload_len guards; UserControl now only reads the
  optional second param when present.
- control.c: UserControl event type was read via an unaligned uint16_t* cast
  (UB on strict-alignment targets) — switched to byte-wise read.
- ertmp/multitrack.c: parser guarded len<7 but then read data[1..8]
  (needs >=9) — out-of-bounds read on 7/8-byte input. Fixed the guard.

## Findings (2026-06-27 review/optimization pass)

- Makefile: build silenced two of the most dangerous C diagnostics via
  `-Wno-implicit-function-declaration -Wno-return-type`. These hide
  calls to undeclared functions and missing `return` statements — both
  common sources of memory-corruption and UB. Verified the whole tree
  (library + unit tests) compiles cleanly with them removed, so dropped
  the suppressions to keep those bugs from ever building silently.
  ASAN unit suite remains green.

### Recommendations (not yet actioned — design decisions for maintainer)

- chunk_reader: msg_length (24-bit, up to 16 MB) is accepted without an
  application-level cap. Each chunk-stream id keeps its own reassembly
  buffer (capped at 64 MB), so a peer opening many csids with large
  in-progress messages can grow memory substantially. A configurable
  max-message-size and max-csid limit would bound this. The incoming
  recv buffer is already capped at 64 MB, limiting per-recv exposure.
- handshake.c uses rand() for the handshake random block. The data is
  never validated by either peer here, so this is cosmetic, but rand()
  is a weak PRNG; a library should not call srand() (that is the host
  app's responsibility), so leaving the RNG choice to the integrator.
- amf3.c integer/string length uses a fixed 4-byte field rather than the
  AMF3 U29 variable-length encoding (module is documented as "minimal"
  and reader/writer agree). Full U29 support is a feature, not a bugfix.
