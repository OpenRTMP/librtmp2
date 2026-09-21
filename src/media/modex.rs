//! Helpers for parsing E-RTMP v2 ModEx wrappers without changing relay bytes.

use std::borrow::Cow;

use crate::types::{CAPS_EX_MASK_MODEX, FrameType};

/// Enhanced RTMP packet type for ModEx wrappers (E-RTMP v2 §16).
pub const ERTMP_PACKET_TYPE_MODEX: u8 = 7;

/// Cap chained ModEx extension layers peeled per media frame. Without this a
/// peer can nest millions of single-byte ModEx wrappers in one max-size message
/// and force O(payload) normalization work on every frame.
const MAX_MODEX_CHAIN_LAYERS: usize = 32;

/// Peels chained ModEx wrappers so codec/multitrack detection sees the
/// underlying packet type. When a chain exceeds `MAX_MODEX_CHAIN_LAYERS` the
/// payload is intentionally left wrapped and opaque: codec/multitrack
/// detection derived from this payload (e.g. `Conn::detected_video_codec` /
/// `detected_audio_codec`) will not fire for that frame. This is a deliberate
/// CPU-amplification tradeoff, not a parsing bug — callers that surface
/// detected-codec stats should expect them to be absent for such frames.
pub fn normalize_modex_payload<'a>(
    payload: &'a [u8],
    caps_ex_mask: u32,
    frame_type: FrameType,
) -> Cow<'a, [u8]> {
    if payload.is_empty()
        || (caps_ex_mask & CAPS_EX_MASK_MODEX) == 0
        || payload[0] & 0x80 == 0
        || payload[0] & 0x0F != ERTMP_PACKET_TYPE_MODEX
    {
        return Cow::Borrowed(payload);
    }

    // Legacy audio SoundFormat 8/10/11/14 also set bit 7 and can carry a low
    // nibble of 7 (e.g. 0x87 G.711U), which must not be peeled as a ModEx
    // wrapper. Genuine E-RTMP enhanced audio uses the reserved ExHeader nibble
    // 9 (see `ertmp::exaudio`), so only peel audio when that nibble is present.
    // Video is unambiguous: legacy video never sets bit 7 (frame type 1-5).
    if frame_type == FrameType::Audio && (payload[0] >> 4) & 0x0F != 0x09 {
        return Cow::Borrowed(payload);
    }

    let mut pos = 1usize;
    let mut reconstructed_header = payload[0];
    let mut layers = 0usize;
    loop {
        if layers >= MAX_MODEX_CHAIN_LAYERS {
            return Cow::Borrowed(payload);
        }
        layers += 1;
        if pos >= payload.len() {
            return Cow::Borrowed(payload);
        }
        let size_minus_one = payload[pos];
        pos += 1;
        let modex_size = if size_minus_one == u8::MAX {
            if pos + 2 > payload.len() {
                return Cow::Borrowed(payload);
            }
            let size = u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize + 1;
            pos += 2;
            size
        } else {
            size_minus_one as usize + 1
        };

        if pos + modex_size + 1 > payload.len() {
            return Cow::Borrowed(payload);
        }
        pos += modex_size;
        let option_and_next_packet = payload[pos];
        pos += 1;
        let next_packet_type = option_and_next_packet & 0x0F;
        reconstructed_header = (reconstructed_header & 0xF0) | next_packet_type;
        if next_packet_type != ERTMP_PACKET_TYPE_MODEX {
            break;
        }
    }

    let mut normalized = Vec::with_capacity(1 + payload.len().saturating_sub(pos));
    normalized.push(reconstructed_header);
    normalized.extend_from_slice(&payload[pos..]);
    Cow::Owned(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_timestamp_modex_video_wrapper() {
        let payload = [
            0x97, 0x02, 0x00, 0x01, 0x02, 0x01, b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA,
        ];
        assert_eq!(
            normalize_modex_payload(&payload, CAPS_EX_MASK_MODEX, FrameType::Video).as_ref(),
            &[0x91, b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA]
        );
    }

    #[test]
    fn legacy_audio_g711u_with_modex_shaped_tail_is_unchanged() {
        // 0x87 is legacy G.711U (SoundFormat 8), not a genuine enhanced tag.
        let payload = [0x87, 0x02, 0x00, 0x01, 0x02, 0x01, b'a', b'v', b'c', b'1'];
        assert!(matches!(
            normalize_modex_payload(&payload, CAPS_EX_MASK_MODEX, FrameType::Audio),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn enhanced_audio_modex_wrapper_is_peeled() {
        // 0x97 = E-RTMP ExHeader nibble 9 + ModEx packet type 7.
        let payload = [
            0x97, 0x02, 0x00, 0x01, 0x02, 0x01, b'O', b'p', b'u', b's', 0xAA,
        ];
        assert_eq!(
            normalize_modex_payload(&payload, CAPS_EX_MASK_MODEX, FrameType::Audio).as_ref(),
            &[0x91, b'O', b'p', b'u', b's', 0xAA]
        );
    }

    #[test]
    fn legacy_aac_is_unchanged() {
        let payload = [0xAF, 0x00, 0x12, 0x10];
        assert!(matches!(
            normalize_modex_payload(&payload, CAPS_EX_MASK_MODEX, FrameType::Audio),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn malformed_modex_is_left_opaque() {
        let payload = [0x97, 0x05, 0x00];
        assert_eq!(
            normalize_modex_payload(&payload, CAPS_EX_MASK_MODEX, FrameType::Video).as_ref(),
            payload
        );
    }

    #[test]
    fn excessive_modex_chain_is_left_opaque() {
        let mut payload = vec![0x97];
        for _ in 0..MAX_MODEX_CHAIN_LAYERS + 4 {
            payload.extend_from_slice(&[0x00, 0x00, 0x07]);
        }
        payload.extend_from_slice(&[0x91, b'a', b'v', b'c', b'1', 0xAA]);
        assert!(matches!(
            normalize_modex_payload(&payload, CAPS_EX_MASK_MODEX, FrameType::Video),
            Cow::Borrowed(_)
        ));
    }
}
