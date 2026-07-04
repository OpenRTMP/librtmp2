# Protocol Mapping — Enhanced RTMP v1

This maps the Veovera *Enhanced RTMP v1* specification onto the `librtmp2`
source tree. Enhanced RTMP v1 extends legacy RTMP/FLV mainly with modern codecs,
FourCC signaling, and richer metadata.

Source spec: [`veovera/enhanced-rtmp` — enhanced-rtmp-v1.md](https://github.com/veovera/enhanced-rtmp/blob/main/docs/enhanced/enhanced-rtmp-v1.md).

## ExVideoTagHeader

| Spec element | Implementation |
|--------------|----------------|
| `IsExHeader` bit detection in VideoTagHeader | `exvideo_parse()` in `src/ertmp/exvideo.rs` |
| `PacketType` (SequenceStart, CodedFrames, SequenceEnd, CodedFramesX, Metadata, MPEG2TSSequenceStart) | `exvideo_parse()` in `src/ertmp/exvideo.rs` |
| FourCC codec id (replaces legacy `CodecID`) | `exvideo_parse()` / `fourcc_parse()` in `src/ertmp/exvideo.rs` + `src/ertmp/fourcc.rs` |
| `frame_type` | `exvideo_parse()` in `src/ertmp/exvideo.rs` |
| Composition time (CodedFrames) | `exvideo_parse()` in `src/ertmp/exvideo.rs` |

Internal struct: `VideoHeader` (`is_ex_header`, `packet_type`, `fourcc: [u8; 5]`,
`frame_type`, `composition_time`), declared in `src/types.rs`.

## FourCC Codecs

| FourCC | Maps to | Implementation |
|--------|---------|----------------|
| `avc1` | H.264 | `src/ertmp/fourcc.rs` (`fourcc_to_video_codec()`) |
| `hvc1` | H.265 / HEVC | `src/ertmp/fourcc.rs` (`fourcc_to_video_codec()`) |
| `av01` | AV1 | `src/ertmp/fourcc.rs` (`fourcc_to_video_codec()`) |
| `vp09` | VP9 | `src/ertmp/fourcc.rs` (`fourcc_to_video_codec()`) |
| `Opus`, `mp4a` (audio) | Opus / AAC | `src/ertmp/fourcc.rs` (`fourcc_to_audio_codec()`) |

The registry (`src/ertmp/fourcc.rs`) provides forward (`fourcc_to_video_codec()`,
`fourcc_to_audio_codec()`), reverse (`video_codec_to_fourcc()`,
`audio_codec_to_fourcc()`), and human-readable name lookups
(`fourcc_video_name()`, `fourcc_audio_name()`).

## ExAudioTagHeader

| Spec element | Implementation |
|--------------|----------------|
| `IsExHeader` detection (bit7 set + len ≥ 5 + recognized FourCC distinguishes enhanced from legacy SoundFormat 8-15, incl. AAC) | `exaudio_parse()` in `src/ertmp/exaudio.rs` |
| Audio FourCC (`Opus`, `mp4a`, `mp3 `, `ec-3`) | `exaudio_parse()` in `src/ertmp/exaudio.rs` + `src/ertmp/fourcc.rs` |
| Wiring into the audio frame path | `src/message/message.rs` (calls `exaudio::exaudio_parse()` for legacy + enhanced) |

The enhanced audio FourCC is surfaced on `Frame::audio_fourcc` (`src/types.rs`).

## Metadata / HDR (`PacketTypeMetadata`, `colorInfo`)

| Spec element | Implementation |
|--------------|----------------|
| `colorInfo` parse/write | `hdr_parse()` / `hdr_write()` in `src/ertmp/metadata.rs` |
| Color primaries (e.g. BT.2020) | `hdr_parse()` in `src/ertmp/metadata.rs` |
| Transfer characteristics (PQ / HLG) | `hdr_parse()` in `src/ertmp/metadata.rs` |
| Matrix coefficients | `hdr_parse()` in `src/ertmp/metadata.rs` |
| `videocodecid_from_fourcc()` utility | `src/ertmp/metadata.rs` |

Internal struct: `HdrInfo` (`color_primaries`, `transfer_chars`, `matrix_coeffs`),
declared in `src/types.rs`.

## connect-object `fourCcList`

| Spec element | Implementation |
|--------------|----------------|
| `fourCcList` ECMAArray parse/write (E-RTMP v1 §6) | `fourcc_list_parse()` / `fourcc_list_write()` in `src/ertmp/connect_caps.rs` |

Internal struct: `FourCcList` (`src/types.rs`).

## Tests

- `connect_caps.rs`'s `fourCcList` parse/write is covered by the inline
  `#[cfg(test)] mod tests` in `src/ertmp/connect_caps.rs`.
- `exvideo.rs`, `exaudio.rs`, `fourcc.rs`, and `metadata.rs` have no dedicated
  inline unit tests; they are exercised end-to-end (HEVC `hvc1`→H.265, AV1
  `av01`→AV1, Opus, and legacy H.264/AAC) by
  `tests/interop/enhanced_rtmp_interop.sh`, which publishes real encoder
  output with ffmpeg against a librtmp2 server.
- `tests/server_client_loopback.rs` covers the legacy (non-enhanced) audio/video
  frame path end-to-end.
