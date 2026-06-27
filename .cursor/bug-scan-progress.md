# Bug scan progress

Last scanned: full pass (2026-06-27)

## Modules

- [x] core — Memory, logging, errors, buffer
- [x] handshake — C0/C1/C2 ↔ S0/S1/S2
- [x] chunk — Chunk reader/writer/state
- [x] message — Message reassembly, control, commands
- [x] amf — AMF0 + AMF3
- [x] flv — Audio/video/script tags
- [x] ertmp — E-RTMP v1/v2 extensions
- [x] session — State machine, publish/play flows
- [x] server — Server listener
- [x] client — Outbound client

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
