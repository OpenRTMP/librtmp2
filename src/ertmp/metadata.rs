//! E-RTMP v1 metadata / HDR / colorInfo support
//!
//! Mirrors `src/ertmp/metadata.h` and `src/ertmp/metadata.c`.

use crate::types::{ErrorCode, HdrInfo, Result};

/// Initialize HDR info with defaults.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::metadata::hdr_init;
/// use librtmp2::types::HdrInfo;
///
/// let mut hdr = HdrInfo::default();
/// hdr_init(&mut hdr);
/// // Defaults are "unspecified" (1) for all three CTA-861 fields.
/// assert_eq!(hdr.color_primaries, 1);
/// assert_eq!(hdr.transfer_chars, 1);
/// assert_eq!(hdr.matrix_coeffs, 1);
/// ```
pub fn hdr_init(hdr: &mut HdrInfo) {
    hdr.color_primaries = 1;
    hdr.transfer_chars = 1;
    hdr.matrix_coeffs = 1;
}

/// Parse HDR color info from a raw blob.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::metadata::hdr_parse;
/// use librtmp2::types::HdrInfo;
///
/// // Three big-endian u16 fields: colorPrimaries, transferCharacteristics,
/// // matrixCoefficients.
/// let data = [0x00, 0x09, 0x00, 0x10, 0x00, 0x09];
/// let mut hdr = HdrInfo::default();
/// hdr_parse(&data, &mut hdr).unwrap();
///
/// assert_eq!(hdr.color_primaries, 9);   // BT.2020
/// assert_eq!(hdr.transfer_chars, 16);   // PQ
///
/// // A truncated blob is rejected.
/// assert!(hdr_parse(&data[..4], &mut hdr).is_err());
/// ```
pub fn hdr_parse(data: &[u8], hdr: &mut HdrInfo) -> Result<()> {
    hdr_init(hdr);
    if data.len() < 6 {
        return Err(ErrorCode::Io);
    }
    hdr.color_primaries = ((data[0] as u16) << 8) | (data[1] as u16);
    hdr.transfer_chars = ((data[2] as u16) << 8) | (data[3] as u16);
    hdr.matrix_coeffs = ((data[4] as u16) << 8) | (data[5] as u16);
    Ok(())
}

/// Write HDR color info to a buffer. Returns bytes written.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::metadata::{hdr_parse, hdr_write};
/// use librtmp2::types::HdrInfo;
///
/// let hdr = HdrInfo { color_primaries: 9, transfer_chars: 16, matrix_coeffs: 9 };
/// let mut buf = [0u8; 6];
/// assert_eq!(hdr_write(&hdr, &mut buf), 6);
///
/// let mut back = HdrInfo::default();
/// hdr_parse(&buf, &mut back).unwrap();
/// assert_eq!(back.transfer_chars, 16);
///
/// // A short buffer writes nothing.
/// assert_eq!(hdr_write(&hdr, &mut [0u8; 4]), 0);
/// ```
pub fn hdr_write(hdr: &HdrInfo, buf: &mut [u8]) -> usize {
    if buf.len() < 6 {
        return 0;
    }
    buf[0] = (hdr.color_primaries >> 8) as u8;
    buf[1] = hdr.color_primaries as u8;
    buf[2] = (hdr.transfer_chars >> 8) as u8;
    buf[3] = hdr.transfer_chars as u8;
    buf[4] = (hdr.matrix_coeffs >> 8) as u8;
    buf[5] = hdr.matrix_coeffs as u8;
    6
}

/// Parse colorInfo from metadata.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::metadata::metadata_colorinfo_parse;
/// use librtmp2::types::HdrInfo;
///
/// // The same three u16 fields, as carried in an Enhanced RTMP metadata packet.
/// let data = [0x00, 0x09, 0x00, 0x10, 0x00, 0x09];
/// let mut hdr = HdrInfo::default();
/// metadata_colorinfo_parse(&data, &mut hdr).unwrap();
/// assert_eq!(hdr.color_primaries, 9);
/// ```
pub fn metadata_colorinfo_parse(data: &[u8], hdr: &mut HdrInfo) -> Result<()> {
    hdr_init(hdr);
    if data.len() < 6 {
        return Err(ErrorCode::Io);
    }
    hdr_parse(data, hdr)
}

/// Convert a FourCC to a videocodecid UI32.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::metadata::videocodecid_from_fourcc;
///
/// // A full FourCC is packed into the UI32 form used by `videocodecid`.
/// assert_eq!(videocodecid_from_fourcc(b"avc1"), u32::from_be_bytes(*b"avc1"));
///
/// // Anything shorter than four bytes falls back to the AVC codec id.
/// assert_eq!(videocodecid_from_fourcc(b"av"), 7);
/// ```
pub fn videocodecid_from_fourcc(fourcc: &[u8]) -> u32 {
    if fourcc.len() < 4 {
        return 7; // default AVC
    }
    ((fourcc[0] as u32) << 24)
        | ((fourcc[1] as u32) << 16)
        | ((fourcc[2] as u32) << 8)
        | (fourcc[3] as u32)
}

/// Stub for E-RTMP v2 capability negotiation.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::metadata::caps_negotiate;
/// use librtmp2::types::ErrorCode;
///
/// // Not implemented: the session performs real negotiation in
/// // `ertmp::connect_amf::negotiate_caps`.
/// assert_eq!(caps_negotiate(), Err(ErrorCode::Unsupported));
/// ```
pub fn caps_negotiate() -> Result<()> {
    Err(ErrorCode::Unsupported)
}
