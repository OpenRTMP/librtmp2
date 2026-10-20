//! E-RTMP v2 ModEx extension mechanism
//!
//! Mirrors `src/ertmp/modex.c`.
//!
//! On the wire a ModEx wrapper is (see the `ExVideoTagHeader` /
//! `ExAudioTagHeader` pseudocode in the E-RTMP v2 spec):
//!
//! ```text
//! sizeMinusOne : UI8                 // 0xFF selects the extended UI16 form
//! if sizeMinusOne == 0xFF:
//!     sizeMinusOne : UI16
//! modExData    : UI8[sizeMinusOne + 1]
//! optionByte   : modExType:UB[4] | nextPacketType:UB[4]
//! ```
//!
//! `modExType` 0 is `TimestampOffsetNano`, whose `modExData` carries a UI24
//! nanosecond offset. Every other `modExType` is reserved and is surfaced as
//! [`ModexType::Nop`] so callers degrade gracefully instead of dropping the
//! frame.
//!
//! Parsing starts at `sizeMinusOne`, i.e. immediately after the enhanced
//! packet-type nibble (for example the byte after `0x97` in a video payload).
//! The framing logic itself lives in `media::modex::parse_modex_layer` and is
//! shared with the live relay path, so there is only one ModEx parser in the
//! crate.

use crate::media::modex::parse_modex_layer;
use crate::types::{ErrorCode, Modex, ModexType, Result};

/// `modExType` for `TimestampOffsetNano` (`VideoPacketModExType` /
/// `AudioPacketModExType` value 0).
const MODEX_TYPE_TIMESTAMP_OFFSET_NANO: u8 = 0;

/// Number of bytes in a `TimestampOffsetNano` UI24.
const TIMESTAMP_OFFSET_NANO_LEN: usize = 3;

/// Parse a ModEx extension.
///
/// `data` must start at `sizeMinusOne` (the first byte after the enhanced
/// packet-type nibble) and contain one complete wrapper layer. Trailing bytes
/// after the wrapper — further ModEx layers when `next_packet_type` is
/// [`crate::media::modex::ERTMP_PACKET_TYPE_MODEX`], or the packet body — are
/// ignored; use [`modex_parse_prefix`] when the consumed length is needed.
///
/// Unknown/reserved `modExType` values succeed with [`ModexType::Nop`] and a
/// zero offset, per the spec's graceful-degradation rule. Truncated input
/// returns [`ErrorCode::Io`]; a `TimestampOffsetNano` wrapper whose `modExData`
/// is shorter than three bytes returns [`ErrorCode::Protocol`].
pub fn modex_parse(modex: &mut Modex, data: &[u8]) -> Result<()> {
    modex_parse_prefix(modex, data).map(|_| ())
}

/// Parse one ModEx extension layer and return the number of bytes consumed
/// (size field + `modExData` + option byte).
///
/// See [`modex_parse`] for the input contract. When the parsed
/// [`Modex::next_packet_type`] is
/// [`crate::media::modex::ERTMP_PACKET_TYPE_MODEX`] another layer starts at the
/// returned offset.
pub fn modex_parse_prefix(modex: &mut Modex, data: &[u8]) -> Result<usize> {
    let layer = parse_modex_layer(data, 0).ok_or(ErrorCode::Io)?;

    let modex_type = if layer.modex_type == MODEX_TYPE_TIMESTAMP_OFFSET_NANO {
        ModexType::TimestampOffsetNano
    } else {
        ModexType::Nop
    };

    let offset = if modex_type == ModexType::TimestampOffsetNano {
        let modex_data = &data[layer.data_start..layer.data_end];
        if modex_data.len() < TIMESTAMP_OFFSET_NANO_LEN {
            return Err(ErrorCode::Protocol);
        }
        u32::from_be_bytes([0, modex_data[0], modex_data[1], modex_data[2]]) as u64
    } else {
        0
    };

    *modex = Modex {
        modex_type,
        offset,
        next_packet_type: layer.next_packet_type,
    };
    Ok(layer.next_pos)
}

/// Write a ModEx extension.
///
/// Emits `sizeMinusOne` (short form), `modExData`, and the
/// `modExType | nextPacketType` option byte. Only the low 24 bits of
/// [`Modex::offset`] are written, matching the `TimestampOffsetNano` UI24, so
/// the short size form is always sufficient. [`Modex::next_packet_type`] is
/// masked to its low nibble.
///
/// Returns the number of bytes written, or 0 when `buf` is too small — or when
/// `modex` is a [`ModexType::Nop`], which has no v2 wire representation.
pub fn modex_write(modex: &Modex, buf: &mut [u8]) -> usize {
    let modex_type = match modex.modex_type {
        ModexType::TimestampOffsetNano => MODEX_TYPE_TIMESTAMP_OFFSET_NANO,
        ModexType::Nop => return 0,
    };

    let offset = (modex.offset & 0x00FF_FFFF) as u32;
    let modex_data = [(offset >> 16) as u8, (offset >> 8) as u8, offset as u8];

    let total = 1 + modex_data.len() + 1;
    if buf.len() < total {
        return 0;
    }

    buf[0] = (modex_data.len() - 1) as u8;
    buf[1..1 + modex_data.len()].copy_from_slice(&modex_data);
    buf[1 + modex_data.len()] = (modex_type << 4) | (modex.next_packet_type & 0x0F);
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modex(modex_type: ModexType, offset: u64, next_packet_type: u8) -> Modex {
        Modex {
            modex_type,
            offset,
            next_packet_type,
        }
    }

    #[test]
    fn timestamp_offset_nano_round_trips_through_write_and_parse() {
        let written = modex(ModexType::TimestampOffsetNano, 0x12_3456, 1);
        let mut buf = [0u8; 5];
        assert_eq!(modex_write(&written, &mut buf), 5);
        // sizeMinusOne=2 (3-byte modExData), UI24 0x123456, option 0x01.
        assert_eq!(buf, [0x02, 0x12, 0x34, 0x56, 0x01]);

        let mut parsed = Modex::default();
        assert_eq!(modex_parse_prefix(&mut parsed, &buf).unwrap(), 5);
        assert_eq!(parsed.modex_type, ModexType::TimestampOffsetNano);
        assert_eq!(parsed.offset, 0x12_3456);
        assert_eq!(parsed.next_packet_type, 1);
    }

    #[test]
    fn parses_valid_v2_frame_from_the_spec() {
        // sizeMinusOne=2 => modExData 00 01 02, option 0x01 => type 0, next 1.
        let mut parsed = Modex::default();
        assert_eq!(
            modex_parse_prefix(&mut parsed, &[0x02, 0x00, 0x01, 0x02, 0x01]).unwrap(),
            5
        );
        assert_eq!(parsed.modex_type, ModexType::TimestampOffsetNano);
        assert_eq!(parsed.offset, 0x00_0102);
        assert_eq!(parsed.next_packet_type, 1);
    }

    #[test]
    fn parses_timestamp_offset_nano_ui24_boundaries() {
        let mut parsed = Modex::default();
        modex_parse(&mut parsed, &[0x02, 0x00, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(parsed.offset, 0);

        // 999_999 ns is the largest value the spec allows in a UI24 offset.
        modex_parse(&mut parsed, &[0x02, 0x0F, 0x42, 0x3F, 0x00]).unwrap();
        assert_eq!(parsed.offset, 999_999);

        // The whole UI24 range still fits (even if peers should stay <= 999_999).
        modex_parse(&mut parsed, &[0x02, 0xFF, 0xFF, 0xFF, 0x00]).unwrap();
        assert_eq!(parsed.offset, 0xFF_FFFF);
    }

    #[test]
    fn parses_extended_size_form() {
        // 0xFF + UI16 0x0002 => size 3, then UI24 0x0A0B0C, option 0x03.
        let mut parsed = Modex::default();
        let payload = [0xFF, 0x00, 0x02, 0x0A, 0x0B, 0x0C, 0x03];
        assert_eq!(modex_parse_prefix(&mut parsed, &payload).unwrap(), 7);
        assert_eq!(parsed.modex_type, ModexType::TimestampOffsetNano);
        assert_eq!(parsed.offset, 0x0A_0B0C);
        assert_eq!(parsed.next_packet_type, 3);

        // 0xFF + UI16 0x00FF => 256-byte modExData; only the first three bytes
        // are the TimestampOffsetNano UI24.
        let mut payload = vec![0xFF, 0x00, 0xFF, 0xAA, 0xBB, 0xCC];
        payload.extend(std::iter::repeat_n(0u8, 253));
        payload.push(0x02);
        assert_eq!(modex_parse_prefix(&mut parsed, &payload).unwrap(), 260);
        assert_eq!(parsed.offset, 0xAA_BBCC);
        assert_eq!(parsed.next_packet_type, 2);
    }

    #[test]
    fn parses_next_packet_type_nibble() {
        for next in 0u8..=0x0F {
            let mut parsed = Modex::default();
            modex_parse(&mut parsed, &[0x02, 0, 0, 0, next]).unwrap();
            assert_eq!(parsed.next_packet_type, next);
        }
    }

    #[test]
    fn unknown_modex_types_degrade_to_nop() {
        // modExType 1 and 2 are reserved; both must parse as Nop with no offset
        // while keeping the next-packet-type nibble.
        let mut parsed = modex(ModexType::TimestampOffsetNano, 7, 0);
        modex_parse(&mut parsed, &[0x00, 0xAA, 0x1F]).unwrap();
        assert_eq!(parsed.modex_type, ModexType::Nop);
        assert_eq!(parsed.offset, 0);
        assert_eq!(parsed.next_packet_type, 0x0F);

        modex_parse(&mut parsed, &[0x01, 0xAA, 0xBB, 0x2C]).unwrap();
        assert_eq!(parsed.modex_type, ModexType::Nop);
        assert_eq!(parsed.offset, 0);
        assert_eq!(parsed.next_packet_type, 0x0C);
    }

    #[test]
    fn parse_rejects_empty_and_truncated_input() {
        let mut modex = Modex::default();
        assert_eq!(modex_parse(&mut modex, &[]).unwrap_err(), ErrorCode::Io);
        // Size byte without modExData or the option byte.
        assert_eq!(modex_parse(&mut modex, &[0x00]).unwrap_err(), ErrorCode::Io);
        // Extended size prefix cut short.
        assert_eq!(modex_parse(&mut modex, &[0xFF]).unwrap_err(), ErrorCode::Io);
        assert_eq!(
            modex_parse(&mut modex, &[0xFF, 0x00]).unwrap_err(),
            ErrorCode::Io
        );
        // Declared 3-byte payload, only two bytes present.
        assert_eq!(
            modex_parse(&mut modex, &[0x02, 0xAA, 0xBB]).unwrap_err(),
            ErrorCode::Io
        );
        // Option byte present but modExData truncated.
        assert_eq!(
            modex_parse(&mut modex, &[0x02, 0xAA, 0xBB, 0x00]).unwrap_err(),
            ErrorCode::Io
        );
    }

    #[test]
    fn parse_rejects_short_timestamp_offset_nano_payload() {
        // modExType 0 with two bytes of modExData is protocol-invalid.
        let mut modex = Modex::default();
        assert_eq!(
            modex_parse(&mut modex, &[0x01, 0x00, 0x01, 0x00]).unwrap_err(),
            ErrorCode::Protocol
        );
    }

    #[test]
    fn parse_rejects_the_legacy_fabricated_marker_layout() {
        // 0x81 is a sizeMinusOne of 129, not a 0x80|type marker: the frame is
        // truncated, so the old fabricated encoding must not be accepted.
        let mut modex = Modex::default();
        assert!(modex_parse(&mut modex, &[0x81, 0, 0, 0, 0, 0, 0, 0, 5]).is_err());
        assert!(modex_parse(&mut modex, &[0xFF]).is_err());
    }

    #[test]
    fn write_rejects_short_buffer() {
        let timestamp = modex(ModexType::TimestampOffsetNano, 1, 1);
        let mut buf = [0u8; 4];
        assert_eq!(modex_write(&timestamp, &mut buf), 0);
        assert_eq!(modex_write(&timestamp, &mut []), 0);
    }

    #[test]
    fn write_nop_emits_nothing() {
        let nop = Modex::default();
        let mut buf = [0u8; 5];
        assert_eq!(modex_write(&nop, &mut buf), 0);
        assert_eq!(buf, [0u8; 5]);
    }

    #[test]
    fn write_masks_offset_to_ui24_and_next_packet_type_to_nibble() {
        let written = modex(ModexType::TimestampOffsetNano, 0xAB_123456, 0xF7);
        let mut buf = [0u8; 5];
        assert_eq!(modex_write(&written, &mut buf), 5);
        assert_eq!(buf, [0x02, 0x12, 0x34, 0x56, 0x07]);

        let mut parsed = Modex::default();
        modex_parse(&mut parsed, &buf).unwrap();
        assert_eq!(parsed.offset, 0x12_3456);
        assert_eq!(parsed.next_packet_type, 7);
    }

    #[test]
    fn chained_layers_round_trip_through_prefix_parse() {
        // Two stacked wrappers: outer type 0 / next 7, inner type 0 / next 1.
        let outer = modex(ModexType::TimestampOffsetNano, 0x00_0001, 7);
        let inner = modex(ModexType::TimestampOffsetNano, 0x00_0002, 1);
        let mut buf = [0u8; 10];
        assert_eq!(modex_write(&outer, &mut buf[..5]), 5);
        assert_eq!(modex_write(&inner, &mut buf[5..]), 5);

        let mut parsed = Modex::default();
        let consumed = modex_parse_prefix(&mut parsed, &buf).unwrap();
        assert_eq!((parsed.offset, parsed.next_packet_type), (1, 7));

        modex_parse_prefix(&mut parsed, &buf[consumed..]).unwrap();
        assert_eq!((parsed.offset, parsed.next_packet_type), (2, 1));
    }

    #[test]
    fn public_parser_agrees_with_the_live_normalizer() {
        use crate::media::modex::normalize_modex_payload_with_frame_type;
        use crate::types::{CAPS_EX_MASK_MODEX, FrameType};

        // 0x97 = ExHeader nibble 9 + packet type 7 (ModEx).
        let written = modex(ModexType::TimestampOffsetNano, 0x00_0A0B, 1);
        let mut frame = vec![0x97];
        let mut wrapper = [0u8; 5];
        assert_eq!(modex_write(&written, &mut wrapper), 5);
        frame.extend_from_slice(&wrapper);
        frame.extend_from_slice(&[b'a', b'v', b'c', b'1']);

        let mut parsed = Modex::default();
        assert_eq!(modex_parse_prefix(&mut parsed, &frame[1..]).unwrap(), 5);
        assert_eq!(parsed.offset, 0x0A0B);
        assert_eq!(parsed.next_packet_type, 1);

        assert_eq!(
            normalize_modex_payload_with_frame_type(&frame, CAPS_EX_MASK_MODEX, FrameType::Video)
                .as_ref(),
            &[0x91, b'a', b'v', b'c', b'1']
        );
    }

    #[test]
    fn deprecated_timestamp_alias_still_names_timestamp_offset_nano() {
        #[allow(deprecated)]
        let alias = ModexType::Timestamp;
        assert_eq!(alias, ModexType::TimestampOffsetNano);
    }
}
