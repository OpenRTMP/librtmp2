//! Enhanced RTMP v1 VideoTagHeader / FourCC parsing
//!
//! Mirrors `src/ertmp/exvideo.c`.

use crate::media::modex::ERTMP_PACKET_TYPE_MODEX;
use crate::types::{ERTMP_PACKET_TYPE_METADATA, ErrorCode, Result, VideoHeader};

/// E-RTMP `VideoFrameType.Command` (frame type 5): the tag body is a single
/// `UI8` videoCommand, not a FourCC header.
pub(crate) const VIDEO_FRAME_TYPE_COMMAND: u8 = 5;

/// Parse a FourCC from raw bytes.
pub fn fourcc_parse(data: &[u8]) -> Result<[u8; 5]> {
    if data.len() < 4 {
        return Err(ErrorCode::Io);
    }
    let mut fourcc = [0u8; 5];
    fourcc[..4].copy_from_slice(&data[..4]);
    Ok(fourcc)
}

fn is_composition_time_codec(fourcc: &[u8]) -> bool {
    // E-RTMP / oxideav: SI24 composition time on CodedFrames for avc1/hvc1/vvc1.
    // Parsing VVC CT does not re-advertise vvc1 in connect caps.
    fourcc[..4] == *b"avc1" || fourcc[..4] == *b"hvc1" || fourcc[..4] == *b"vvc1"
}

/// Parse an Enhanced RTMP v1 video tag header.
pub fn exvideo_parse(data: &[u8], hdr: &mut VideoHeader) -> Result<()> {
    *hdr = VideoHeader::default();

    if data.is_empty() {
        return Err(ErrorCode::Io);
    }

    let b0 = data[0];
    hdr.is_ex_header = if b0 & 0x80 != 0 { 1 } else { 0 };

    if hdr.is_ex_header == 0 {
        hdr.frame_type = (b0 >> 4) & 0x0F;
        hdr.header_size = 1;
        // Legacy FLV AVCVIDEOPACKET: AVCPacketType (1) + CompositionTime SI24.
        if (b0 & 0x0F) == 7 && data.len() >= 5 {
            let ct = ((data[2] as i32) << 16) | ((data[3] as i32) << 8) | (data[4] as i32);
            let ct = if ct & 0x00800000 != 0 {
                ct | 0xFF000000u32 as i32
            } else {
                ct
            };
            hdr.composition_time = ct as u32;
            // Bytes 1..5 were consumed into VideoHeader; match the enhanced-path
            // contract that header_size covers every interpreted header byte.
            hdr.header_size = 5;
        }
        return Ok(());
    }

    hdr.frame_type = (b0 >> 4) & 0x07;
    hdr.packet_type = b0 & 0x0F;

    // E-RTMP v2 ExVideoTagBody: a command frame (frame type 5) whose packet
    // type is not Metadata carries a single UI8 videoCommand and no FourCC.
    // A ModEx wrapper (packet type 7) is excluded: its low nibble selects the
    // wrapper, and the frame type describes the *inner* packet only after the
    // chain is peeled. Treating an unpeeled ModEx wrapper as a VideoCommand
    // would hand `detect_video_codec` a non-`None` label for a payload that may
    // wrap a disallowed codec, defeating codec deny lists.
    if hdr.frame_type == VIDEO_FRAME_TYPE_COMMAND
        && hdr.packet_type != ERTMP_PACKET_TYPE_METADATA
        && hdr.packet_type != ERTMP_PACKET_TYPE_MODEX
    {
        if data.len() < 2 {
            return Err(ErrorCode::Io);
        }
        // The UI8 videoCommand is body data (`processVideoBody = false`); it
        // is not retained in `VideoHeader`, which only reports the frame
        // shape.
        hdr.header_size = 2;
        return Ok(());
    }

    if data.len() < 5 {
        return Err(ErrorCode::Io);
    }

    hdr.fourcc[..4].copy_from_slice(&data[1..5]);
    hdr.header_size = 5;

    if hdr.packet_type == 1 && is_composition_time_codec(&hdr.fourcc) {
        if data.len() < 8 {
            return Err(ErrorCode::Io);
        }
        let ct = ((data[5] as i32) << 16) | ((data[6] as i32) << 8) | (data[7] as i32);
        let ct = if ct & 0x00800000 != 0 {
            ct | 0xFF000000u32 as i32
        } else {
            ct
        };
        hdr.composition_time = ct as u32;
        hdr.header_size = 8;
    }

    Ok(())
}

/// Write an Enhanced RTMP v1 video tag header. Returns bytes written, or 0 if
/// `buf` is too small, or when the header cannot be represented (see below).
/// Mirrors [`exvideo_parse`] in reverse.
///
/// `VideoHeader` has no field for a legacy (non-ex-header) video codec ID --
/// `exvideo_parse` never populates one either, since the legacy tag byte's
/// low nibble is a codec ID, not part of the ex-header layout this type
/// models. Writing `is_ex_header == 0` therefore cannot produce a valid
/// legacy tag byte (there is no codec ID to encode; a codec ID of 0 is not a
/// defined legacy video codec) and returns 0 instead. Callers that need a
/// legacy FLV video tag header should build one directly (see
/// `src/flv/video_tag.rs`) rather than through this enhanced-only writer.
///
/// `VideoHeader` does not retain the UI8 videoCommand either (`exvideo_parse`
/// treats it as body data), so a command frame (frame type 5, non-Metadata
/// packet type) cannot be written through this helper: the writer would have
/// to fabricate a command byte, silently turning e.g. an EndSeek into a
/// StartSeek. The command-frame branch returns 0 instead.
pub fn exvideo_write(hdr: &VideoHeader, buf: &mut [u8]) -> usize {
    if hdr.is_ex_header == 0 {
        return 0;
    }

    // E-RTMP v2 ExVideoTagBody: a command frame (frame type 5, non-Metadata
    // packet type) is the tag byte followed by the UI8 videoCommand, which
    // `VideoHeader` does not retain (see the doc comment above).
    if hdr.frame_type == VIDEO_FRAME_TYPE_COMMAND && hdr.packet_type != ERTMP_PACKET_TYPE_METADATA {
        return 0;
    }

    let needs_ct = hdr.packet_type == 1 && is_composition_time_codec(&hdr.fourcc);
    let len = if needs_ct { 8 } else { 5 };
    if buf.len() < len {
        return 0;
    }

    buf[0] = 0x80 | ((hdr.frame_type & 0x07) << 4) | (hdr.packet_type & 0x0F);
    buf[1..5].copy_from_slice(&hdr.fourcc[..4]);

    if needs_ct {
        let ct = hdr.composition_time as i32;
        buf[5] = (ct >> 16) as u8;
        buf[6] = (ct >> 8) as u8;
        buf[7] = ct as u8;
    }

    len
}

/// Get the E-RTMP version string.
pub fn version_string() -> &'static str {
    "E-RTMP v1 (ExVideoTagHeader/FourCC)"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::VideoHeader;

    #[test]
    fn parse_clears_composition_time_on_non_ct_packet_type_reuse() {
        let h264_ct = [0x91, b'a', b'v', b'c', b'1', 0x00, 0x01, 0x2C];
        let av1_seq = [0x90, b'a', b'v', b'0', b'1'];

        let mut hdr = VideoHeader::default();
        exvideo_parse(&h264_ct, &mut hdr).unwrap();
        assert_eq!(hdr.composition_time, 0x00012C);
        assert_eq!(hdr.header_size, 8);

        exvideo_parse(&av1_seq, &mut hdr).unwrap();
        assert_eq!(hdr.composition_time, 0);
        assert_eq!(hdr.header_size, 5);
        assert_eq!(&hdr.fourcc[..4], b"av01");
    }

    #[test]
    fn parse_error_leaves_header_cleared() {
        let h264_ct = [0x91, b'a', b'v', b'c', b'1', 0x00, 0x01, 0x2C];
        let mut hdr = VideoHeader::default();
        exvideo_parse(&h264_ct, &mut hdr).unwrap();

        assert!(exvideo_parse(&[0x91, b'a', b'v'], &mut hdr).is_err());
        assert_eq!(hdr.composition_time, 0);
        assert_eq!(hdr.is_ex_header, 1);
    }

    #[test]
    fn modex_video_command_frame_is_not_parsed_as_a_command() {
        // 0xD7 = IsExHeader | frame type 5 | packet type 7 (ModEx). The low
        // nibble selects a wrapper, not a videoCommand: taking the command fast
        // path would report a 2-byte header with no FourCC and let codec
        // authorization see an always-allowed "command" label.
        let payload = [0xD7, 0x00, 0x00, 0x07, 0x01, 0x90, b'h', b'v', b'c', b'1'];
        let mut hdr = VideoHeader::default();
        exvideo_parse(&payload, &mut hdr).unwrap();
        assert_eq!(hdr.frame_type, VIDEO_FRAME_TYPE_COMMAND);
        assert_eq!(hdr.packet_type, ERTMP_PACKET_TYPE_MODEX);
        assert_eq!(
            hdr.header_size, 5,
            "a ModEx wrapper keeps the FourCC layout instead of the command header"
        );
        assert_eq!(&hdr.fourcc[..4], &[0x00, 0x00, 0x07, 0x01]);

        // A ModEx wrapper without room for the FourCC layout is not a valid
        // 2-byte command header either.
        assert!(exvideo_parse(&[0xD7, 0x00], &mut hdr).is_err());

        // A genuine command frame (packet type 0) still takes the fast path.
        let mut cmd = VideoHeader::default();
        exvideo_parse(&[0xD0, 0x00], &mut cmd).unwrap();
        assert_eq!(cmd.header_size, 2);
        assert_eq!(cmd.packet_type, 0);
    }

    #[test]
    fn write_rejects_legacy_header() {
        // VideoHeader carries no legacy codec ID (exvideo_parse never
        // populates one either), so a legacy tag byte cannot be constructed
        // -- writing it would produce codec ID 0, which is not a valid
        // legacy video codec.
        let hdr = VideoHeader {
            is_ex_header: 0,
            frame_type: 1,
            ..Default::default()
        };
        let mut buf = [0u8; 8];
        assert_eq!(exvideo_write(&hdr, &mut buf), 0);
    }

    #[test]
    fn write_round_trips_enhanced_header_with_composition_time() {
        let mut hdr = VideoHeader {
            is_ex_header: 1,
            packet_type: 1,
            frame_type: 1,
            composition_time: 300,
            ..Default::default()
        };
        hdr.fourcc[..4].copy_from_slice(b"avc1");
        let mut buf = [0u8; 8];
        let n = exvideo_write(&hdr, &mut buf);
        assert_eq!(n, 8);

        let mut parsed = VideoHeader::default();
        exvideo_parse(&buf[..n], &mut parsed).unwrap();
        assert_eq!(parsed.composition_time, 300);
        assert_eq!(&parsed.fourcc[..4], b"avc1");
        assert_eq!(parsed.header_size, 8);
    }

    #[test]
    fn write_round_trips_enhanced_header_without_composition_time() {
        let mut hdr = VideoHeader {
            is_ex_header: 1,
            packet_type: 0,
            frame_type: 1,
            ..Default::default()
        };
        hdr.fourcc[..4].copy_from_slice(b"av01");
        let mut buf = [0u8; 8];
        let n = exvideo_write(&hdr, &mut buf);
        assert_eq!(n, 5);

        let mut parsed = VideoHeader::default();
        exvideo_parse(&buf[..n], &mut parsed).unwrap();
        assert_eq!(parsed.composition_time, 0);
        assert_eq!(parsed.header_size, 5);
        assert_eq!(&parsed.fourcc[..4], b"av01");
    }

    #[test]
    fn write_rejects_undersized_buffer() {
        let mut hdr = VideoHeader {
            is_ex_header: 1,
            packet_type: 1,
            frame_type: 1,
            ..Default::default()
        };
        hdr.fourcc[..4].copy_from_slice(b"avc1");
        let mut buf = [0u8; 4];
        assert_eq!(exvideo_write(&hdr, &mut buf), 0);
    }

    #[test]
    fn legacy_avc_sets_header_size_after_composition_time() {
        // tag=0x17 (key + AVC), AVCPacketType=1, CT=42
        let data = [0x17, 0x01, 0x00, 0x00, 0x2A, 0xDE, 0xAD];
        let mut hdr = VideoHeader::default();
        exvideo_parse(&data, &mut hdr).unwrap();
        assert_eq!(hdr.is_ex_header, 0);
        assert_eq!(hdr.composition_time, 42);
        assert_eq!(hdr.header_size, 5);
    }

    #[test]
    fn legacy_non_avc_keeps_header_size_one() {
        let data = [0x12, 0x00, 0x00, 0x00, 0x00];
        let mut hdr = VideoHeader::default();
        exvideo_parse(&data, &mut hdr).unwrap();
        assert_eq!(hdr.is_ex_header, 0);
        assert_eq!(hdr.header_size, 1);
        assert_eq!(hdr.composition_time, 0);
    }

    #[test]
    fn vvc1_coded_frames_consume_composition_time() {
        let data = [0x91, b'v', b'v', b'c', b'1', 0x00, 0x01, 0x2C, 0xAA];
        let mut hdr = VideoHeader::default();
        exvideo_parse(&data, &mut hdr).unwrap();
        assert_eq!(hdr.is_ex_header, 1);
        assert_eq!(hdr.packet_type, 1);
        assert_eq!(&hdr.fourcc[..4], b"vvc1");
        assert_eq!(hdr.composition_time, 0x00012C);
        assert_eq!(hdr.header_size, 8);

        let mut buf = [0u8; 8];
        assert_eq!(exvideo_write(&hdr, &mut buf), 8);
        assert_eq!(&buf[..], &data[..8]);
    }
}
