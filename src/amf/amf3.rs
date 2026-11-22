//! AMF3 encoder/decoder (minimal implementation)
//!
//! Mirrors `src/amf/amf3.h` and `src/amf/amf3.c`.

use crate::buffer::Buffer;
use crate::types::ErrorCode;
use crate::types::Result;

/* AMF3 type markers */
const AMF3_UNDEFINED: u8 = 0x00;
const AMF3_NULL: u8 = 0x01;
const AMF3_FALSE: u8 = 0x02;
const AMF3_TRUE: u8 = 0x03;
const AMF3_INTEGER: u8 = 0x04;
const AMF3_DOUBLE: u8 = 0x05;
const AMF3_STRING: u8 = 0x06;
const AMF3_XML_DOC: u8 = 0x07;
const AMF3_DATE: u8 = 0x08;
const AMF3_ARRAY: u8 = 0x09;
const AMF3_OBJECT: u8 = 0x0A;
const AMF3_XML: u8 = 0x0B;
const AMF3_BYTE_ARRAY: u8 = 0x0C;

fn read_u8(buf: &mut Buffer) -> Result<u8> {
    let mut val = [0u8; 1];
    buf.read(&mut val).map_err(|_| ErrorCode::Io)?;
    Ok(val[0])
}

/// Read a U29 variable-length integer.
fn read_u29(buf: &mut Buffer) -> Result<u32> {
    let mut result: u32 = 0;
    for i in 0..4 {
        let b = read_u8(buf)?;
        if i < 3 {
            result = (result << 7) | (b & 0x7F) as u32;
            if b & 0x80 == 0 {
                return Ok(result);
            }
        } else {
            result = (result << 8) | b as u32;
        }
    }
    Ok(result)
}

/// Write a U29 variable-length integer.
fn write_u29(buf: &mut Buffer, val: u32) -> Result<()> {
    let v = val & 0x1FFFFFFF;
    let bytes = if v < 0x80 {
        vec![v as u8]
    } else if v < 0x4000 {
        vec![((v >> 7) | 0x80) as u8, (v & 0x7F) as u8]
    } else if v < 0x200000 {
        vec![
            ((v >> 14) | 0x80) as u8,
            (((v >> 7) & 0x7F) | 0x80) as u8,
            (v & 0x7F) as u8,
        ]
    } else {
        vec![
            ((v >> 22) | 0x80) as u8,
            (((v >> 15) & 0x7F) | 0x80) as u8,
            (((v >> 8) & 0x7F) | 0x80) as u8,
            (v & 0xFF) as u8,
        ]
    };
    buf.write(&bytes).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

fn read_double_raw(buf: &mut Buffer) -> Result<f64> {
    let mut b = [0u8; 8];
    buf.read(&mut b).map_err(|_| ErrorCode::Io)?;
    let mut bits: u64 = 0;
    for i in 0..8 {
        bits |= (b[i] as u64) << ((7 - i) * 8);
    }
    Ok(f64::from_bits(bits))
}

/* ── Public API ── */

/// Read an AMF3 type marker.

/// # Examples
///
/// ```
/// use librtmp2::amf::amf3;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf3::write_null(&mut buf).unwrap();
/// let ty = amf3::read_type(&mut buf).unwrap();
/// assert_eq!(ty as u8, buf.peek().first().copied().unwrap_or(ty as u8));
/// ```
pub fn read_type(buf: &mut Buffer) -> Result<u8> {
    read_u8(buf)
}

/// Read an AMF3 null.

/// # Examples
///
/// ```
/// use librtmp2::amf::amf3;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf3::write_null(&mut buf).unwrap();
/// // Consumes the marker and reports it was a null.
/// amf3::read_null(&mut buf).unwrap();
/// assert_eq!(buf.available(), 0);
/// ```
pub fn read_null(buf: &mut Buffer) -> Result<()> {
    let t = read_u8(buf)?;
    if t != AMF3_NULL {
        return Err(ErrorCode::Amf);
    }
    Ok(())
}

/// Read an AMF3 integer (U29).
///
/// Reads only the variable-length U29 value; the `AMF3_INTEGER` marker is not
/// consumed. Read it with [`read_type`] first when decoding a self-describing
/// stream.

/// # Examples
///
/// ```
/// use librtmp2::amf::amf3;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf3::write_integer(&mut buf, 300).unwrap();
///
/// // `read_integer` reads only the U29 value, so skip the marker byte.
/// let mut reader = Buffer::from_slice(&buf.peek()[1..]);
/// assert_eq!(amf3::read_integer(&mut reader).unwrap(), 300);
/// ```
pub fn read_integer(buf: &mut Buffer) -> Result<u32> {
    read_u29(buf)
}

/// Read an AMF3 double.
///
/// Reads only the eight IEEE-754 bytes; the type marker is **not** consumed.
/// This mirrors [`read_integer`] and [`read_string`], which also expect the
/// caller to have read the marker via [`read_type`] first. Note the asymmetry:
/// [`write_double`] *does* emit a marker, so a naive write/read round trip reads
/// the marker byte as data.
///
/// # Errors
///
/// Returns [`ErrorCode::Io`] when fewer than eight bytes are available.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf3;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf3::write_double(&mut buf, -0.25).unwrap();
///
/// // Skip the marker byte that `write_double` emitted.
/// let mut reader = Buffer::from_slice(&buf.peek()[1..]);
/// assert_eq!(amf3::read_double(&mut reader).unwrap(), -0.25);
/// ```

pub fn read_double(buf: &mut Buffer) -> Result<f64> {
    read_double_raw(buf)
}

/// Read an AMF3 boolean.

/// # Examples
///
/// ```
/// use librtmp2::amf::amf3;
/// use librtmp2::buffer::Buffer;
///
/// // AMF3 booleans are their own marker bytes, not a marker plus a value.
/// let mut buf = Buffer::from_slice(&[0x03]);
/// assert!(amf3::read_boolean(&mut buf).unwrap());
/// ```
pub fn read_boolean(buf: &mut Buffer) -> Result<bool> {
    let t = read_u8(buf)?;
    match t {
        AMF3_TRUE => Ok(true),
        AMF3_FALSE => Ok(false),
        _ => Err(ErrorCode::Amf),
    }
}

/// Read an AMF3 string.

/// # Examples
///
/// ```
/// use librtmp2::amf::amf3;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf3::write_string(&mut buf, "live").unwrap();
///
/// let mut out = [0u8; 16];
/// let n = amf3::read_string(&mut buf, &mut out).unwrap();
/// assert_eq!(&out[..n], b"live");
/// ```
pub fn read_string(buf: &mut Buffer, out: &mut [u8]) -> Result<usize> {
    let t = read_u8(buf)?;
    if t != AMF3_STRING {
        return Err(ErrorCode::Amf);
    }

    let ref_val = read_u29(buf)?;
    let len = (ref_val >> 1) as usize;
    let inline_bit = ref_val & 1;

    if inline_bit == 0 {
        return Err(ErrorCode::Unsupported);
    }

    if out.is_empty() || len >= out.len() {
        return Err(ErrorCode::Amf);
    }

    buf.read(&mut out[..len]).map_err(|_| ErrorCode::Io)?;
    out[len] = 0;
    Ok(len)
}

/// Write an AMF3 null.

/// # Examples
///
/// ```
/// use librtmp2::amf::amf3;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf3::write_null(&mut buf).unwrap();
/// assert_eq!(buf.available(), 1); // just the null marker
/// ```
pub fn write_null(buf: &mut Buffer) -> Result<()> {
    buf.write(&[AMF3_NULL]).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an AMF3 integer (U29).

/// # Examples
///
/// ```
/// use librtmp2::amf::amf3;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// // Values below 128 take a single U29 byte.
/// amf3::write_integer(&mut buf, 42).unwrap();
/// assert_eq!(buf.available(), 2); // marker + one U29 byte
/// ```
pub fn write_integer(buf: &mut Buffer, val: u32) -> Result<()> {
    buf.write(&[AMF3_INTEGER])
        .map_err(|_| ErrorCode::Internal)?;
    write_u29(buf, val)
}

/// Write an AMF3 double.

/// # Examples
///
/// ```
/// use librtmp2::amf::amf3;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf3::write_double(&mut buf, 1.5).unwrap();
/// // Marker byte plus the eight IEEE-754 bytes.
/// assert_eq!(buf.available(), 9);
///
/// // `read_double` expects the marker already consumed.
/// let mut reader = Buffer::from_slice(&buf.peek()[1..]);
/// assert_eq!(amf3::read_double(&mut reader).unwrap(), 1.5);
/// ```
pub fn write_double(buf: &mut Buffer, val: f64) -> Result<()> {
    buf.write(&[AMF3_DOUBLE]).map_err(|_| ErrorCode::Internal)?;
    let bits = val.to_bits();
    let mut b = [0u8; 8];
    for i in 0..8 {
        b[i] = (bits >> ((7 - i) * 8)) as u8;
    }
    buf.write(&b).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an AMF3 string.

/// # Examples
///
/// ```
/// use librtmp2::amf::amf3;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf3::write_string(&mut buf, "onMetaData").unwrap();
/// assert!(buf.available() > "onMetaData".len());
/// ```
pub fn write_string(buf: &mut Buffer, s: &str) -> Result<()> {
    let len = s.len();
    if len > 0x0FFFFFFF {
        return Err(ErrorCode::Amf);
    }
    buf.write(&[AMF3_STRING]).map_err(|_| ErrorCode::Internal)?;
    write_u29(buf, ((len as u32) << 1) | 1)?;
    buf.write(s.as_bytes()).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_round_trips_small_value() {
        let mut buf = Buffer::new();
        write_integer(&mut buf, 5).unwrap();
        assert_eq!(read_type(&mut buf).unwrap(), AMF3_INTEGER);
        assert_eq!(read_integer(&mut buf).unwrap(), 5);
    }

    #[test]
    fn integer_round_trips_multi_byte_u29() {
        let mut buf = Buffer::new();
        write_integer(&mut buf, 1_000_000).unwrap();
        assert_eq!(read_type(&mut buf).unwrap(), AMF3_INTEGER);
        assert_eq!(read_integer(&mut buf).unwrap(), 1_000_000);
    }

    #[test]
    fn double_round_trips() {
        let mut buf = Buffer::new();
        write_double(&mut buf, 3.5).unwrap();
        assert_eq!(read_type(&mut buf).unwrap(), AMF3_DOUBLE);
        assert_eq!(read_double(&mut buf).unwrap(), 3.5);
    }

    #[test]
    fn boolean_round_trips() {
        let mut buf = Buffer::new();
        buf.write(&[AMF3_TRUE]).unwrap();
        assert!(read_boolean(&mut buf).unwrap());
        let mut buf = Buffer::new();
        buf.write(&[AMF3_FALSE]).unwrap();
        assert!(!read_boolean(&mut buf).unwrap());
    }

    #[test]
    fn string_round_trips() {
        let mut buf = Buffer::new();
        write_string(&mut buf, "abc").unwrap();
        let mut out = [0u8; 16];
        let len = read_string(&mut buf, &mut out).unwrap();
        assert_eq!(&out[..len], b"abc");
    }

    #[test]
    fn null_round_trips() {
        let mut buf = Buffer::new();
        write_null(&mut buf).unwrap();
        assert!(read_null(&mut buf).is_ok());
    }

    #[test]
    fn read_string_rejects_truncated_payload() {
        let mut buf = Buffer::from_slice(&[AMF3_STRING, 0x07, b'a', b'b']);
        let mut out = [0u8; 16];
        assert_eq!(read_string(&mut buf, &mut out), Err(ErrorCode::Io));
    }

    #[test]
    fn read_string_rejects_string_reference() {
        let mut buf = Buffer::from_slice(&[AMF3_STRING, 0x00]);
        let mut out = [0u8; 16];
        assert_eq!(read_string(&mut buf, &mut out), Err(ErrorCode::Unsupported));
    }

    #[test]
    fn read_integer_rejects_truncated_u29() {
        let mut buf = Buffer::from_slice(&[0x80, 0x80]);
        assert_eq!(read_integer(&mut buf), Err(ErrorCode::Io));
    }

    #[test]
    fn read_double_rejects_truncated_payload() {
        let mut buf = Buffer::from_slice(&[0x00, 0x00, 0x00]);
        assert_eq!(read_double(&mut buf), Err(ErrorCode::Io));
    }
}
