//! FLV video tag parser
//!
//! Mirrors `src/flv/video_tag.h` and `src/flv/video_tag.c`.

use crate::types::{VideoTag, VideoCodec, Result, ErrorCode};

/// Parse an FLV video tag.
pub fn parse(data: &[u8], tag: &mut VideoTag) -> Result<()> {
    if data.is_empty() {
        return Err(ErrorCode::Internal);
    }

    tag.frame_type = (data[0] >> 4) & 0x0F;
    tag.codec = match data[0] & 0x0F {
        1 => VideoCodec::Jpeg,
        2 => VideoCodec::Sorenson,
        3 => VideoCodec::Screen,
        4 => VideoCodec::Vp6,
        5 => VideoCodec::Vp6a,
        6 => VideoCodec::Screen2,
        7 => VideoCodec::H264,
        12 => VideoCodec::H265,
        13 => VideoCodec::Av1,
        _ => VideoCodec::H264,
    };

    if data.len() >= 5 && (tag.codec == VideoCodec::H264 || tag.codec == VideoCodec::H265) {
        tag.avc_packet_type = data[1];
        tag.composition_time = ((data[2] as u32) << 16) | ((data[3] as u32) << 8) | (data[4] as u32);
    }

    tag.data = data.as_ptr();
    tag.size = data.len();
    Ok(())
}
