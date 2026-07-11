//! Enhanced RTMP v1 VideoTagHeader / FourCC parsing
//!
//! Mirrors `src/ertmp/exvideo.c`.

use crate::types::{ErrorCode, Result, VideoHeader};

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
    fourcc[..4] == *b"avc1" || fourcc[..4] == *b"hvc1"
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
        return Ok(());
    }

    hdr.frame_type = (b0 >> 4) & 0x07;
    hdr.packet_type = b0 & 0x0F;

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
}
