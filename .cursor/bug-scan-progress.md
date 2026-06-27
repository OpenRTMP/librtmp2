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
