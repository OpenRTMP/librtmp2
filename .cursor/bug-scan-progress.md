# Bug scan progress

Last scanned: core (2026-08-26)

## Modules

- [x] core — Memory, logging, errors, buffer
- [ ] handshake — C0/C1/C2 ↔ S0/S1/S2
- [ ] chunk — Chunk reader/writer/state
- [ ] message — Message reassembly, control, commands
- [ ] amf — AMF0 + AMF3
- [ ] flv — Audio/video/script tags
- [ ] ertmp — E-RTMP v1/v2 extensions
- [ ] session — State machine, publish/play flows
- [ ] server — Server listener
- [ ] client — Outbound client

## Findings (2026-08-26 core pass)

- Reviewed `src/alloc.rs`, `src/bytes.rs`, `src/buffer.rs`, `src/log.rs`,
  `src/types.rs`, and `src/net.rs` in full. Traced callers for
  `Buffer::write`/`read`/`reset` caps (`BUFFER_MAX_SIZE` 64 MiB),
  `lrtmp2_calloc` overflow guard, unowned-buffer growth refusal,
  `ntoh32`/`ntoh24` indexing (all production callers pre-check slice
  length; `ntoh24` is test-only today), `split_host_port` IPv4/IPv6
  bracket forms and downstream `parse_rtmp_url`/`resolve_bind_addr`
  usage, log-callback deadlock avoidance, and FFI enum validation in
  `types.rs`/`lib.rs`. No new critical or high-severity issue found.

## Findings (2026-07-11 session/server/client/transport pass)

- Reviewed `src/session/conn.rs`, `src/server/mod.rs`, `src/client/mod.rs`,
  `src/transport.rs` in full. The recv-buffer cap, bounded TCP connect, and
  stream-cache byte/frame caps landed by earlier passes (see git log:
  "cap recv_buffer, bound client TCP connect, limit stream cache init
  frames") are present and correct in the current code. Traced network-length
  indexing, caller-owned struct reuse, integer over/underflow, reachable
  panics (`.unwrap()`/`.expect()`/indexing) from network input, fd/socket
  leaks on error paths, unbounded growth, TLS certificate verification, and
  the connection state machine. No new critical or high-severity issue found.
- **Bug (fixed):** `lrtmp2_client_create()` accepted a `*const ServerConfig`
  parameter but discarded it entirely (`_config: *const ServerConfig`) —
  `ServerConfig.tls_ca_file` ("Client: CA bundle for verification") and
  `ServerConfig.tls_insecure` ("Client: skip certificate verification") were
  documented ABI fields that did nothing. `Client::connect()` had no TLS
  config path at all; `Transport::connect_tls()` always built a bare
  `SslConnector` with the default verify mode. An integrator setting
  `tls_insecure=1` to reach a self-signed RTMPS deployment, or `tls_ca_file`
  to pin a private CA, would get silent full verification against the system
  trust store instead (fails closed, but the documented control was a no-op).
  Fixed: `Client` now stores `tls_ca_file`/`tls_insecure` (set via
  `Client::set_tls_client_config`, wired from `ServerConfig` in
  `lrtmp2_client_create`), and `Transport::connect_tls()` takes `ca_file`/
  `insecure` and applies `set_ca_file` / `SslVerifyMode::NONE` accordingly.
  Added regression tests in `client/mod.rs`.
- Two low-confidence items noted, not actioned (design/scope decisions):
  - `Client::connect()` resolves DNS synchronously via `to_socket_addrs()`
    before the bounded TCP-connect loop starts; `TCP_CONNECT_TIMEOUT_SECS`
    does not cover the resolver call itself. Only relevant if a deployment
    passes attacker-influenced hostnames to `connect()`.
  - No other unbounded-blocking or resource issues found in the reviewed
    files; existing regression tests already cover ping rate limiting,
    stream-cache eviction bookkeeping, and TLS pending-handshake accounting.

## Findings (2026-07-11 ertmp pass)

- Reviewed all nine files under `src/ertmp/` (`exvideo.rs`, `exaudio.rs`,
  `fourcc.rs`, `metadata.rs`, `connect_caps.rs`, `reconnect.rs`,
  `multitrack.rs`, `modex.rs`, `mod.rs`). Traced parse/write paths for OOB
  reads, stale-state reuse on caller-owned output structs, integer overflow,
  and graceful degradation of unknown ModEx types.
- **Bug:** `exvideo_parse()` and `exaudio_parse()` reused caller-owned
  `VideoHeader`/`AudioHeader` without clearing prior fields. Re-parsing H264
  CodedFrames (with composition time) then AV1 SequenceStart left a stale
  `composition_time`; legacy AAC→enhanced Opus left stale `aac_packet_type`;
  enhanced→legacy left stale `fourcc`/`packet_type`. Same class as the flv
  reuse fix. Fixed by resetting `*hdr = Header::default()` at parse entry.
  Added regression tests in `exvideo.rs` and `exaudio.rs`.
- **Bug:** `multitrack_parse()` did not reset `Multitrack` on entry; a failed
  parse after a prior descriptor left stale `track_type`/`track_name`. Fixed
  with `*mt = Multitrack::default()` at entry plus regression test.
- `modex.rs`, `reconnect.rs`, `connect_caps.rs`, `metadata.rs`, and
  `fourcc.rs` already guard lengths, cap counts, or re-init output structs;
  no further critical issues found.

## Findings (2026-07-10 flv pass)

- Reviewed `src/flv/audio_tag.rs`, `src/flv/video_tag.rs`, `src/flv/script_tag.rs`,
  and `mod.rs`. Traced callers (benches, embedder Rust API; session ingest uses
  lightweight `detect_*_codec` instead). AMF paths in `script_tag` inherit amf0
  bounds checks (`MAX_OBJECT_KEYS`, `MAX_SKIP_DEPTH`, string length caps).
- **Bug:** All three `parse()` functions reused caller-owned `AudioTag`/`VideoTag`/
  `ScriptTag` structs without clearing prior fields. Re-parsing a shorter script
  name left a stale suffix (`"hi"` after `"onMetaData"` → `"hiataData"` as C
  string). Re-parsing H264 then VP6 left stale `avc_packet_type`/`composition_time`;
  AAC→MP3 left stale `aac_packet_type`. Failed parses (empty payload, unknown
  codec) could also leave a mix of old and new fields. Fixed by resetting
  `*tag = Tag::default()` at the start of each `parse()`. Added regression tests
  for reuse and error paths.

## Findings (2026-07-09 amf pass)

- Reviewed `src/amf/amf0.rs`, `src/amf/amf3.rs`, and `mod.rs` plus all
  callers (`message/command.rs`, `flv/script_tag.rs`, `session/conn.rs`).
  Traced encoder/decoder paths for OOB reads, unbounded recursion, integer
  overflow, and partial-parse desync on adversarial/truncated payloads.
  `skip_value` enforces `MAX_SKIP_DEPTH` (32) and `MAX_OBJECT_KEYS` (256);
  string/long-string/date/reference skips all bounds-check before drain.
  AMF3 U29/string readers reject truncated input and string references.
  No critical bugs found. Added adversarial regression tests in amf0/amf3.

## Findings (2026-07-02 message pass)

- message/control.rs: `read_abort_message()` and `read_acknowledgement_size()`
  called `ntoh32(data)` (which indexes `data[0..4]` directly) with no length
  check, unlike every sibling decoder in the file (`read_set_chunk_size`,
  `read_window_ack_size`, `read_set_peer_bandwidth`, `read_user_control` all
  guard their own input). Both call sites (message/message.rs `decode()` and
  session/conn.rs `handle_control()`) currently pre-check `payload.len() >= 4`
  before calling, so this was not reachable today, but the functions
  themselves had no defense — a future caller (or a fuzz/API user calling
  them directly) forwarding a 0-3 byte Abort/Ack control payload would panic
  on the network-controlled length field, a DoS on that connection/thread.
  Added the same `data.len() < 4 -> Err(ErrorCode::Protocol)` guard used by
  the other decoders in this module, plus regression tests
  (`abort_message_rejects_short_input`, `acknowledgement_size_rejects_short_input`).
  Rest of src/message/ (command.rs AMF command decode, control.rs remaining
  decoders, message.rs aggregate/dispatch) reviewed for OOB slicing, integer
  overflow, and stale-state reuse; all length fields there are already
  bounds-checked before use and no further issues were found.

## Findings (2026-07-01 chunk pass)

- chunk/reader.rs: After a message completed, `type0_*` metadata lingered while
  `reassembly_bytes_read` was reset to zero. A peer could send fmt=2/3 on the
  same CSID and have attacker-controlled bytes reassembled under the prior
  message's type/length, delivering a forged AMF/control message to the session
  layer. Reject fmt=2/3 when no message is in progress
  (`reassembly_bytes_read == 0`).

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
