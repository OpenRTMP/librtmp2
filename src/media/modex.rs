//! Helpers for parsing E-RTMP v2 ModEx wrappers without changing relay bytes.

use std::borrow::Cow;

use crate::types::{CAPS_EX_MASK_MODEX, FrameType};

/// Enhanced RTMP packet type for ModEx wrappers (E-RTMP v2 §16).
pub const ERTMP_PACKET_TYPE_MODEX: u8 = 7;

/// Cap chained ModEx extension layers peeled per media frame. Without this a
/// peer can nest millions of single-byte ModEx wrappers in one max-size message
/// and force O(payload) normalization work on every frame.
const MAX_MODEX_CHAIN_LAYERS: usize = 32;

/// One `modExDataSize` / `modExData` / option-byte triple parsed from an
/// E-RTMP v2 ModEx wrapper (spec: the `ExAudioTagHeader` / `ExVideoTagHeader`
/// pseudocode).
///
/// This is the single source of truth for ModEx framing in the crate: the live
/// relay path ([`normalize_modex_payload_with_frame_type`]) and the public
/// [`crate::ertmp::modex`] parse/write API both go through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModexLayer {
    /// `modExType` — the high nibble of the byte that follows `modExData`.
    /// 0 is `TimestampOffsetNano`; every other value is reserved.
    pub(crate) modex_type: u8,
    /// `nextPacketType` — the low nibble of the same byte. 7 means another
    /// ModEx layer follows immediately.
    pub(crate) next_packet_type: u8,
    /// Start of `modExData` in the parsed payload.
    pub(crate) data_start: usize,
    /// End (exclusive) of `modExData` in the parsed payload.
    pub(crate) data_end: usize,
    /// Offset of the byte after the option byte (next layer or packet body).
    pub(crate) next_pos: usize,
}

/// Parse one ModEx wrapper layer starting at `pos`, which must point at the
/// `modExDataSize` UI8.
///
/// Returns `None` when the layer is truncated. The size field is
/// `UI8 + 1`, with `0xFF` selecting the extended `UI16 + 1` form; `modExData`
/// is therefore always at least one byte long.
pub(crate) fn parse_modex_layer(payload: &[u8], pos: usize) -> Option<ModexLayer> {
    let size_minus_one = *payload.get(pos)?;
    let mut cursor = pos + 1;
    let modex_size = if size_minus_one == u8::MAX {
        let hi = *payload.get(cursor)?;
        let lo = *payload.get(cursor + 1)?;
        cursor += 2;
        (u16::from_be_bytes([hi, lo]) as usize).checked_add(1)?
    } else {
        size_minus_one as usize + 1
    };

    let data_end = cursor.checked_add(modex_size)?;
    let option = *payload.get(data_end)?;
    Some(ModexLayer {
        modex_type: option >> 4,
        next_packet_type: option & 0x0F,
        data_start: cursor,
        data_end,
        next_pos: data_end.checked_add(1)?,
    })
}

/// Peels chained ModEx wrappers so codec/multitrack detection sees the
/// underlying packet type. When a chain exceeds `MAX_MODEX_CHAIN_LAYERS` the
/// payload is intentionally left wrapped and opaque: codec/multitrack
/// detection derived from this payload (e.g. `Conn::detected_video_codec` /
/// `detected_audio_codec`) will not fire for that frame. This is a deliberate
/// CPU-amplification tradeoff, not a parsing bug — callers that surface
/// detected-codec stats should expect them to be absent for such frames.
///
/// This is the legacy two-argument entry point: it treats the payload as a
/// video tag, so legacy audio tags that also set bit 7 are peeled as before.
/// Prefer [`normalize_modex_payload_with_frame_type`] when the frame type is
/// known so legacy audio is not peeled.
pub fn normalize_modex_payload<'a>(payload: &'a [u8], caps_ex_mask: u32) -> Cow<'a, [u8]> {
    normalize_modex_payload_with_frame_type(payload, caps_ex_mask, FrameType::Video)
}

/// Frame-type-aware variant of [`normalize_modex_payload`].
///
/// Audio is peeled only when the tag carries the reserved E-RTMP ExHeader
/// nibble 9 (see `ertmp::exaudio`): legacy SoundFormat 8/10/11/14 also set
/// bit 7 and can carry a low nibble of 7 (e.g. `0x87` G.711U), which must not
/// be treated as a ModEx wrapper. Video is unambiguous (legacy video never
/// sets bit 7), so it keeps the previous behavior.
pub fn normalize_modex_payload_with_frame_type<'a>(
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

    // Audio: only genuine E-RTMP enhanced tags (ExHeader nibble 9) may be
    // peeled; see the function docs. Video keeps the previous behavior.
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
        let layer = match parse_modex_layer(payload, pos) {
            Some(layer) => layer,
            None => return Cow::Borrowed(payload),
        };
        pos = layer.next_pos;
        reconstructed_header = (reconstructed_header & 0xF0) | layer.next_packet_type;
        if layer.next_packet_type != ERTMP_PACKET_TYPE_MODEX {
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
            normalize_modex_payload_with_frame_type(&payload, CAPS_EX_MASK_MODEX, FrameType::Video)
                .as_ref(),
            &[0x91, b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA]
        );
    }

    #[test]
    fn legacy_audio_g711u_with_modex_shaped_tail_is_unchanged() {
        // 0x87 is legacy G.711U (SoundFormat 8), not a genuine enhanced tag.
        let payload = [0x87, 0x02, 0x00, 0x01, 0x02, 0x01, b'a', b'v', b'c', b'1'];
        assert!(matches!(
            normalize_modex_payload_with_frame_type(&payload, CAPS_EX_MASK_MODEX, FrameType::Audio),
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
            normalize_modex_payload_with_frame_type(&payload, CAPS_EX_MASK_MODEX, FrameType::Audio)
                .as_ref(),
            &[0x91, b'O', b'p', b'u', b's', 0xAA]
        );
    }

    #[test]
    fn legacy_aac_is_unchanged() {
        let payload = [0xAF, 0x00, 0x12, 0x10];
        assert!(matches!(
            normalize_modex_payload_with_frame_type(&payload, CAPS_EX_MASK_MODEX, FrameType::Audio),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn legacy_two_argument_entry_point_keeps_video_behavior() {
        // The compatibility wrapper treats the payload as video, preserving the
        // pre-existing behavior for external callers.
        let payload = [
            0x97, 0x02, 0x00, 0x01, 0x02, 0x01, b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA,
        ];
        assert_eq!(
            normalize_modex_payload(&payload, CAPS_EX_MASK_MODEX).as_ref(),
            &[0x91, b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA]
        );
    }

    #[test]
    fn malformed_modex_is_left_opaque() {
        let payload = [0x97, 0x05, 0x00];
        assert_eq!(
            normalize_modex_payload_with_frame_type(&payload, CAPS_EX_MASK_MODEX, FrameType::Video)
                .as_ref(),
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
            normalize_modex_payload_with_frame_type(&payload, CAPS_EX_MASK_MODEX, FrameType::Video),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn parses_short_size_mod_ex_layer() {
        // sizeMinusOne = 0x02 => 3-byte modExData, option byte 0x15 => type 1, next 5.
        let payload = [0x02, 0xAA, 0xBB, 0xCC, 0x15];
        let layer = parse_modex_layer(&payload, 0).unwrap();
        assert_eq!(layer.modex_type, 1);
        assert_eq!(layer.next_packet_type, 5);
        assert_eq!(
            &payload[layer.data_start..layer.data_end],
            &[0xAA, 0xBB, 0xCC]
        );
        assert_eq!(
            (layer.data_start, layer.data_end, layer.next_pos),
            (1, 4, 5)
        );
    }

    #[test]
    fn parses_extended_size_mod_ex_layer() {
        // sizeMinusOne = 0xFF, UI16 = 0x0002 => 3-byte modExData.
        let layer = parse_modex_layer(&[0xFF, 0x00, 0x02, 0x0A, 0x0B, 0x0C, 0x03], 0).unwrap();
        assert_eq!(layer.modex_type, 0);
        assert_eq!(layer.next_packet_type, 3);
        assert_eq!(
            (layer.data_start, layer.data_end, layer.next_pos),
            (3, 6, 7)
        );

        // UI16 = 0x00FF => 256-byte modExData, the largest extended form the
        // spec's "1..=256 bytes" note describes before another size prefix.
        let mut payload = vec![0xFF, 0x00, 0xFF];
        payload.extend(std::iter::repeat_n(0x5A, 256));
        payload.push(0x07);
        let layer = parse_modex_layer(&payload, 0).unwrap();
        assert_eq!(
            (layer.data_start, layer.data_end, layer.next_pos),
            (3, 259, 260)
        );
    }

    #[test]
    fn rejects_truncated_mod_ex_layers() {
        assert!(parse_modex_layer(&[], 0).is_none());
        assert!(parse_modex_layer(&[0x02, 0xAA], 0).is_none());
        assert!(parse_modex_layer(&[0x02, 0xAA, 0xBB, 0xCC], 0).is_none());
        assert!(parse_modex_layer(&[0xFF], 0).is_none());
        assert!(parse_modex_layer(&[0xFF, 0x00], 0).is_none());
        assert!(parse_modex_layer(&[0xFF, 0x00, 0x02, 0xAA], 0).is_none());
    }
}
