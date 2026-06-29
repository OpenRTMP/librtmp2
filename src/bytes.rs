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
pub fn ntoh24(buf: &[u8]) -> u32 {
    ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32)
}

/// Write a 24-bit big-endian value to a byte buffer.
pub fn hton24(buf: &mut [u8], val: u32) {
    buf[0] = ((val >> 16) & 0xFF) as u8;
    buf[1] = ((val >> 8) & 0xFF) as u8;
    buf[2] = (val & 0xFF) as u8;
}

/// Convert a u32 to big-endian (network byte order).
pub fn hton32(val: u32) -> u32 {
    val.to_be()
}

/// Read a big-endian u32 from a byte buffer.
pub fn ntoh32(buf: &[u8]) -> u32 {
    u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]])
}

/// Read a big-endian u16 from a byte buffer.
pub fn ntoh16(buf: &[u8]) -> u16 {
    u16::from_be_bytes([buf[0], buf[1]])
}

/// Convert a u16 to big-endian.
pub fn hton16(val: u16) -> u16 {
    val.to_be()
}
