//! Byte-swapping and endian helpers
//!
//! Mirrors `src/core/bytes.h` and `src/core/bytes.c`.

/// Swap bytes of a u16 (big-endian ↔ little-endian).
pub fn byteswap16(val: u16) -> u16 {
    val.swap_bytes()
}

/// Swap bytes of a u32 (big-endian ↔ little-endian).
pub fn byteswap32(val: u32) -> u32 {
    val.swap_bytes()
}

/// Swap bytes of a u64 (big-endian ↔ little-endian).
pub fn byteswap64(val: u64) -> u64 {
    val.swap_bytes()
}

/// Read a 24-bit big-endian value from a byte buffer.
///
/// This is the 24-bit length field used by RTMP chunk headers. The top byte of
/// the result is always zero.
///
/// # Panics
///
/// Panics if `buf` is shorter than 3 bytes. Callers working with
/// network-provided data must bounds-check the slice first (the chunk decoder
/// does, before calling this).
///
/// # Examples
///
/// ```
/// use librtmp2::bytes::{hton24, ntoh24};
///
/// let mut raw = [0u8; 3];
/// hton24(&mut raw, 0x12_3456);
/// assert_eq!(raw, [0x12, 0x34, 0x56]);
/// assert_eq!(ntoh24(&raw), 0x12_3456);
/// ```
pub fn ntoh24(buf: &[u8]) -> u32 {
    ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32)
}

/// Write a 24-bit big-endian value to a byte buffer.
///
/// Only the low 24 bits of `val` are used.
///
/// # Panics
///
/// Panics if `buf` is shorter than 3 bytes.
///
/// # Examples
///
/// ```
/// use librtmp2::bytes::hton24;
///
/// let mut raw = [0u8; 3];
/// hton24(&mut raw, 0xAB_CDEF);
/// assert_eq!(raw, [0xAB, 0xCD, 0xEF]);
///
/// // Bits above 24 are discarded.
/// hton24(&mut raw, 0xFF12_3456);
/// assert_eq!(raw, [0x12, 0x34, 0x56]);
/// ```
pub fn hton24(buf: &mut [u8], val: u32) {
    buf[0] = ((val >> 16) & 0xFF) as u8;
    buf[1] = ((val >> 8) & 0xFF) as u8;
    buf[2] = (val & 0xFF) as u8;
}

/// Convert a u32 to big-endian (network byte order).
///
/// Returns the value already byte-swapped on a little-endian host, so writing it
/// with [`u32::to_ne_bytes`] produces network order. Pair it with [`ntoh32`].
///
/// # Examples
///
/// ```
/// use librtmp2::bytes::{hton32, ntoh32};
///
/// // hton32 already converts, so serialize with `to_ne_bytes` (not `to_be_bytes`,
/// // which would convert a second time).
/// let wire = hton32(0x0102_0304).to_ne_bytes();
/// assert_eq!(wire, [0x01, 0x02, 0x03, 0x04]);
/// assert_eq!(ntoh32(&wire), 0x0102_0304);
/// ```
pub fn hton32(val: u32) -> u32 {
    val.to_be()
}

/// Read a big-endian u32 from a byte buffer.
///
/// # Panics
///
/// Panics if `buf` is shorter than 4 bytes; bounds-check network data first.
///
/// # Examples
///
/// ```
/// use librtmp2::bytes::ntoh32;
///
/// assert_eq!(ntoh32(&[0xDE, 0xAD, 0xBE, 0xEF]), 0xDEAD_BEEF);
/// ```
pub fn ntoh32(buf: &[u8]) -> u32 {
    u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]])
}

/// Read a big-endian u16 from a byte buffer.
///
/// # Panics
///
/// Panics if `buf` is shorter than 2 bytes; bounds-check network data first.
///
/// # Examples
///
/// ```
/// use librtmp2::bytes::ntoh16;
///
/// assert_eq!(ntoh16(&[0x12, 0x34]), 0x1234);
/// ```
pub fn ntoh16(buf: &[u8]) -> u16 {
    u16::from_be_bytes([buf[0], buf[1]])
}

/// Convert a u16 to big-endian.
///
/// Returns the value already byte-swapped on a little-endian host, so writing it
/// with [`u16::to_ne_bytes`] produces network order. Pair it with [`ntoh16`].
///
/// # Examples
///
/// ```
/// use librtmp2::bytes::{hton16, ntoh16};
///
/// // Serialize with `to_ne_bytes`: hton16 already did the conversion.
/// let wire = hton16(0x0102).to_ne_bytes();
/// assert_eq!(wire, [0x01, 0x02]);
/// assert_eq!(ntoh16(&wire), 0x0102);
/// ```
pub fn hton16(val: u16) -> u16 {
    val.to_be()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byteswap_round_trips() {
        assert_eq!(byteswap16(byteswap16(0x1234)), 0x1234);
        assert_eq!(byteswap32(byteswap32(0x1234_5678)), 0x1234_5678);
        assert_eq!(
            byteswap64(byteswap64(0x1234_5678_9ABC_DEF0)),
            0x1234_5678_9ABC_DEF0
        );
    }

    #[test]
    fn ntoh24_reads_big_endian_24_bit() {
        assert_eq!(ntoh24(&[0x01, 0x02, 0x03]), 0x0001_0203);
    }

    #[test]
    fn hton24_writes_big_endian_24_bit() {
        let mut buf = [0u8; 3];
        hton24(&mut buf, 0x0001_0203);
        assert_eq!(buf, [0x01, 0x02, 0x03]);
    }

    #[test]
    fn ntoh32_reads_big_endian_32_bit() {
        assert_eq!(ntoh32(&[0xDE, 0xAD, 0xBE, 0xEF]), 0xDEAD_BEEF);
    }

    #[test]
    fn ntoh16_reads_big_endian_16_bit() {
        assert_eq!(ntoh16(&[0x12, 0x34]), 0x1234);
    }

    #[test]
    fn hton32_then_to_ne_bytes_matches_ntoh32() {
        // hton32 already performs the host->network swap; writing its
        // result with to_ne_bytes (not to_be_bytes) yields the correct
        // wire bytes. Mixing the two would double-swap.
        assert_eq!(ntoh32(&hton32(0xDEAD_BEEF).to_ne_bytes()), 0xDEAD_BEEF);
    }
}
