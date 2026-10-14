# Malformed publisher input: what is guaranteed where

`librtmp2` relays audio/video payloads opaquely. It is not a decoder, so the
guarantees below are deliberately split between the transport layer and the
(cheap, structural) codec-configuration checks.

## Guaranteed by the transport / protocol layers

- Every length field (chunk size, message length, AMF/ExHeader/multitrack
  sizes) is bounds-checked before use; reassembly is capped per message and in
  aggregate, and chunk-size / window-ack values outside the accepted range are
  rejected without changing connection state.
- Control messages with short, unknown or out-of-range payloads are ignored or
  rejected; `Abort` of an unknown chunk stream is a no-op and keeps the header
  context of a known one.
- Connection state is per `Conn`; a failing publisher is closed on its own and
  releases its publish route and cache entries.
- Media is forwarded without being decoded. A malformed *bitstream* inside a
  well-framed message reaches viewers unchanged; decoders are expected to cope.

## Guaranteed by codec-aware validation (once per sequence header)

`media::codec_config` checks the structure of `AVCDecoderConfigurationRecord`
and `HEVCDecoderConfigurationRecord` (and of every sequence-start track of an
E-RTMP multitrack container): version, SPS/PPS presence, the optional High-profile extension section, array/entry counts and
lengths inside the record, no empty parameter set. Other FourCCs only need a
non-empty record. It never decodes video and allocates nothing; it is not run
for coded frames.

A sequence header that fails the check is still relayed live, but it is **never
cached**: it cannot replace a previously cached valid header, and it creates no
cache entry. Late joiners therefore only ever receive a header that passed.
Enhanced RTMP command frames (FrameType 5) are never cached.

## Configuration changes

A different valid sequence header replaces the cached one and drops the cached
keyframe (it belongs to the old configuration); a repeated identical header
keeps it. Codec switches (H.264 <-> H.265, config A -> B) are handled this way
without special-casing; no switch is rejected. Because the relay is opaque,
whether a downstream decoder copes with a mid-stream switch is out of scope.

## Tests and fuzzing

- `src/media/codec_config.rs`, `src/server/mod.rs` (cache poisoning, codec
  switches), `src/session/conn.rs` (control messages) – unit tests.
- `tests/publisher_isolation.rs` – malformed publisher vs. unrelated stream.
- `tests/malformed_churn.rs` – connect/garbage/disconnect churn with a
  baseline-relative memory check (long variant: `--ignored`, weekly CI).
- `tests/video_config_robustness.rs` + `fuzz/fuzz_targets/video_config.rs`
  (seeds in `tests/seeds/video_config`) – mutation pass on stable, libFuzzer on
  nightly.
