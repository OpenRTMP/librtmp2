//! AMF0 encoder/decoder
//!
//! Mirrors `src/amf/amf.h` and `src/amf/amf0.c`.

use crate::buffer::Buffer;
use crate::types::ErrorCode;
use crate::types::Result;

/// AMF0 type markers.
///
/// The discriminant of each variant is the marker byte written on the wire in
/// front of a value, as defined by the AMF0 specification. Values only carry
/// meaning through their marker; see [`read_type`] for decoding and
/// [`write_number`]/[`write_string`] and friends for encoding.
///
/// ```
/// use librtmp2::amf::amf0::Amf0Type;
///
/// assert_eq!(Amf0Type::Number as u8, 0x00);
/// assert_eq!(Amf0Type::EcmaArray as u8, 0x08);
/// assert_eq!(Amf0Type::Avmplus as u8, 0x11);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Amf0Type {
    /// IEEE-754 double (marker `0x00`).
    Number = 0x00,
    /// Boolean (marker `0x01`).
    Boolean = 0x01,
    /// UTF-8 string with a 16-bit length (marker `0x02`).
    String = 0x02,
    /// Anonymous object (marker `0x03`).
    Object = 0x03,
    /// Reserved movieclip marker (marker `0x04`); not encodable.
    Movieclip = 0x04,
    /// Null (marker `0x05`).
    Null = 0x05,
    /// Undefined (marker `0x06`).
    Undefined = 0x06,
    /// Reference to a previously sent object (marker `0x07`).
    Reference = 0x07,
    /// Associative array (marker `0x08`).
    EcmaArray = 0x08,
    /// Object-end sentinel (`0x00 0x00 0x09`).
    ObjectEnd = 0x09,
    /// Dense array (marker `0x0A`).
    StrictArray = 0x0A,
    /// Date with a millisecond double and a 16-bit timezone (marker `0x0B`).
    Date = 0x0B,
    /// UTF-8 string with a 32-bit length (marker `0x0C`).
    LongString = 0x0C,
    /// Unsupported marker (marker `0x0D`).
    Unsupported = 0x0D,
    /// Reserved recordset marker (marker `0x0E`).
    Recordset = 0x0E,
    /// XML document (marker `0x0F`).
    XmlDoc = 0x0F,
    /// Typed object with a class name (marker `0x10`).
    TypedObject = 0x10,
    /// Marker for a following AMF3 value (marker `0x11`).
    Avmplus = 0x11,
}

impl Amf0Type {
    /// Short lowercase name of the marker, for diagnostics and log output.
    ///
    /// Markers without a dedicated name here -- and ones this crate does not
    /// encode or decode -- are reported as `"unknown"`.
    ///
    /// ```
    /// use librtmp2::amf::amf0::Amf0Type;
    ///
    /// assert_eq!(Amf0Type::Number.as_str(), "number");
    /// assert_eq!(Amf0Type::EcmaArray.as_str(), "ecma_array");
    /// assert_eq!(Amf0Type::Date.as_str(), "unknown");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            Amf0Type::Number => "number",
            Amf0Type::Boolean => "boolean",
            Amf0Type::String => "string",
            Amf0Type::Object => "object",
            Amf0Type::Null => "null",
            Amf0Type::Undefined => "undefined",
            Amf0Type::EcmaArray => "ecma_array",
            Amf0Type::LongString => "long_string",
            _ => "unknown",
        }
    }
}

/* ── HELPERS ── */

fn read_u8(buf: &mut Buffer) -> Result<u8> {
    let mut val = [0u8; 1];
    buf.read(&mut val).map_err(|_| ErrorCode::Io)?;
    Ok(val[0])
}

fn read_u16(buf: &mut Buffer) -> Result<u16> {
    let mut b = [0u8; 2];
    buf.read(&mut b).map_err(|_| ErrorCode::Io)?;
    Ok(((b[0] as u16) << 8) | (b[1] as u16))
}

fn read_u32(buf: &mut Buffer) -> Result<u32> {
    let mut b = [0u8; 4];
    buf.read(&mut b).map_err(|_| ErrorCode::Io)?;
    Ok(((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32))
}

fn read_double(buf: &mut Buffer) -> Result<f64> {
    let mut b = [0u8; 8];
    buf.read(&mut b).map_err(|_| ErrorCode::Io)?;
    let mut bits: u64 = 0;
    for i in 0..8 {
        bits |= (b[i] as u64) << ((7 - i) * 8);
    }
    Ok(f64::from_bits(bits))
}

/* ── ENCODER ── */

/// Write an AMF0 number (double, 8 bytes big-endian).
///
/// Appends the `0x00` marker byte followed by the IEEE-754 bits in network
/// byte order.
///
/// # Errors
///
/// Returns [`ErrorCode::Internal`] when the buffer cannot grow.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf0::write_number(&mut buf, 42.0).unwrap();
/// // Marker byte plus an 8-byte double.
/// assert_eq!(buf.available(), 9);
/// assert_eq!(buf.peek()[0], 0x00);
/// ```
pub fn write_number(buf: &mut Buffer, value: f64) -> Result<()> {
    buf.write(&[Amf0Type::Number as u8])
        .map_err(|_| ErrorCode::Internal)?;
    let bits = value.to_bits();
    let mut b = [0u8; 8];
    for i in 0..8 {
        b[i] = (bits >> ((7 - i) * 8)) as u8;
    }
    buf.write(&b).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an AMF0 boolean.
pub fn write_boolean(buf: &mut Buffer, value: bool) -> Result<()> {
    buf.write(&[Amf0Type::Boolean as u8])
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&[if value { 1 } else { 0 }])
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an AMF0 null.
pub fn write_null(buf: &mut Buffer) -> Result<()> {
    buf.write(&[Amf0Type::Null as u8])
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an AMF0 undefined.
pub fn write_undefined(buf: &mut Buffer) -> Result<()> {
    buf.write(&[Amf0Type::Undefined as u8])
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an AMF0 string (2-byte length + UTF-8).
///
/// # Errors
///
/// Returns [`ErrorCode::Amf`] when `s` is longer than `u16::MAX` bytes, which
/// the wire format cannot represent. Use [`write_long_string`] for longer
/// values.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf0::write_string(&mut buf, "onMetaData").unwrap();
/// assert_eq!(buf.available(), 1 + 2 + "onMetaData".len());
/// ```
pub fn write_string(buf: &mut Buffer, s: &str) -> Result<()> {
    let len = s.len();
    if len > u16::MAX as usize {
        return Err(ErrorCode::Amf);
    }
    buf.write(&[Amf0Type::String as u8])
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&(len as u16).to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(s.as_bytes()).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an AMF0 long string (4-byte length + UTF-8).
pub fn write_long_string(buf: &mut Buffer, s: &str) -> Result<()> {
    write_long_string_bytes(buf, s.as_bytes())
}

/// Write an AMF0 long string from raw bytes.
pub fn write_long_string_bytes(buf: &mut Buffer, data: &[u8]) -> Result<()> {
    let len = data.len();
    if len > u32::MAX as usize {
        return Err(ErrorCode::Amf);
    }
    buf.write(&[Amf0Type::LongString as u8])
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&(len as u32).to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(data).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write the beginning of an AMF0 object.
///
/// Must be followed by zero or more [`write_object_key`]/value pairs and then
/// [`write_object_end`]. An object missing its end marker makes the whole
/// message undecodable, so the three calls always travel together.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0;
/// use librtmp2::buffer::Buffer;
///
/// // {"app": "live"}
/// let mut buf = Buffer::new();
/// amf0::write_object_begin(&mut buf).unwrap();
/// amf0::write_object_key(&mut buf, "app").unwrap();
/// amf0::write_string(&mut buf, "live").unwrap();
/// amf0::write_object_end(&mut buf).unwrap();
///
/// assert_eq!(buf.peek()[0], 0x03); // Object marker
/// ```
pub fn write_object_begin(buf: &mut Buffer) -> Result<()> {
    buf.write(&[Amf0Type::Object as u8])
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write the end of an AMF0 object (0x00 0x00 0x09).
///
/// Closes an object opened with [`write_object_begin`]; the three-byte sequence
/// is an empty key followed by the `ObjectEnd` marker.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf0::write_object_begin(&mut buf).unwrap();
/// amf0::write_object_end(&mut buf).unwrap();
/// // Object marker followed by the end sentinel.
/// assert_eq!(buf.peek(), &[0x03, 0x00, 0x00, 0x09]);
/// ```
pub fn write_object_end(buf: &mut Buffer) -> Result<()> {
    buf.write(&[0x00, 0x00, 0x09])
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an AMF0 object key (2-byte length + UTF-8).
pub fn write_object_key(buf: &mut Buffer, key: &str) -> Result<()> {
    let len = key.len();
    if len > u16::MAX as usize {
        return Err(ErrorCode::Amf);
    }
    buf.write(&(len as u16).to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(key.as_bytes()).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write the beginning of an AMF0 ECMA array (4-byte count).
///
/// `count` announces the number of key/value pairs; the decoder still relies on
/// the object-end marker to stop, so the count is advisory. Close with
/// [`write_object_end`].
///
/// # Errors
///
/// Returns [`ErrorCode::Internal`] when the buffer cannot grow.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf0::write_ecma_array_begin(&mut buf, 1).unwrap();
/// amf0::write_object_key(&mut buf, "duration").unwrap();
/// amf0::write_number(&mut buf, 0.0).unwrap();
/// amf0::write_object_end(&mut buf).unwrap();
///
/// assert_eq!(buf.peek()[0], 0x08); // ECMA array marker
/// ```
pub fn write_ecma_array_begin(buf: &mut Buffer, count: u32) -> Result<()> {
    buf.write(&[Amf0Type::EcmaArray as u8])
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&count.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/* ── DECODER ── */

/// Read an AMF0 type marker.
///
/// Consumes exactly one byte and maps it to an [`Amf0Type`].
///
/// # Errors
///
/// Returns [`ErrorCode::Amf`] for a marker byte this crate does not know, and
/// [`ErrorCode::Io`] when no byte is available.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0::{self, Amf0Type};
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::from_slice(&[0x0C]);
/// assert_eq!(amf0::read_type(&mut buf).unwrap(), Amf0Type::LongString);
/// // Unknown markers are rejected.
/// let mut bad = Buffer::from_slice(&[0x42]);
/// assert!(amf0::read_type(&mut bad).is_err());
/// ```
pub fn read_type(buf: &mut Buffer) -> Result<Amf0Type> {
    let t = read_u8(buf)?;
    Ok(match t {
        0x00 => Amf0Type::Number,
        0x01 => Amf0Type::Boolean,
        0x02 => Amf0Type::String,
        0x03 => Amf0Type::Object,
        0x04 => Amf0Type::Movieclip,
        0x05 => Amf0Type::Null,
        0x06 => Amf0Type::Undefined,
        0x07 => Amf0Type::Reference,
        0x08 => Amf0Type::EcmaArray,
        0x09 => Amf0Type::ObjectEnd,
        0x0A => Amf0Type::StrictArray,
        0x0B => Amf0Type::Date,
        0x0C => Amf0Type::LongString,
        0x0D => Amf0Type::Unsupported,
        0x0E => Amf0Type::Recordset,
        0x0F => Amf0Type::XmlDoc,
        0x10 => Amf0Type::TypedObject,
        0x11 => Amf0Type::Avmplus,
        _ => return Err(ErrorCode::Amf),
    })
}

/// Read an AMF0 number (without type marker).
///
/// Consumes eight bytes of IEEE-754 big-endian data. The marker is *not* read;
/// call [`read_type`] first when decoding a self-describing stream, or use
/// [`read_number`]'s counterpart directly after a known marker.
///
/// # Errors
///
/// Returns [`ErrorCode::Io`] when fewer than eight bytes are available.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0;
/// use librtmp2::buffer::Buffer;
///
/// // Encode then decode, skipping the marker by hand.
/// let mut buf = Buffer::new();
/// amf0::write_number(&mut buf, 1.5).unwrap();
/// let mut reader = Buffer::from_slice(&buf.peek()[1..]);
/// assert_eq!(amf0::read_number(&mut reader).unwrap(), 1.5);
/// ```
pub fn read_number(buf: &mut Buffer) -> Result<f64> {
    read_double(buf)
}

/// Read an AMF0 boolean.
///
/// Consumes one byte (non-zero means `true`). Like [`read_number`], the type
/// marker is not consumed.
///
/// # Errors
///
/// Returns [`ErrorCode::Io`] when no byte is available.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf0::write_boolean(&mut buf, true).unwrap();
/// // Skip the 0x01 marker, then read the value byte.
/// let mut reader = Buffer::from_slice(&buf.peek()[1..]);
/// assert!(amf0::read_boolean(&mut reader).unwrap());
/// ```
pub fn read_boolean(buf: &mut Buffer) -> Result<bool> {
    let b = read_u8(buf)?;
    Ok(b != 0)
}

/// Read an AMF0 string into a byte buffer.
///
/// Consumes the marker, the 16-bit length and the UTF-8 bytes, writing them to
/// `out` plus a trailing NUL, and returns the string length without the NUL.
/// Prefer [`read_string_alloc`] when the length is not known in advance.
///
/// # Errors
///
/// Returns [`ErrorCode::Amf`] when the marker is not `0x02`, the string does not
/// fit in `out` (one byte is reserved for the NUL), or the bytes are not read.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf0::write_string(&mut buf, "live").unwrap();
///
/// let mut out = [0u8; 16];
/// let n = amf0::read_string(&mut buf, &mut out).unwrap();
/// assert_eq!(n, 4);
/// assert_eq!(&out[..n], b"live");
/// assert_eq!(out[n], 0); // NUL-terminated for C interop
/// ```
pub fn read_string(buf: &mut Buffer, out: &mut [u8]) -> Result<usize> {
    let ty = read_u8(buf)?;
    if ty != Amf0Type::String as u8 {
        return Err(ErrorCode::Amf);
    }
    let str_len = read_u16(buf)? as usize;
    if str_len >= out.len() {
        return Err(ErrorCode::Amf);
    }
    buf.read(&mut out[..str_len]).map_err(|_| ErrorCode::Amf)?;
    out[str_len] = 0;
    Ok(str_len)
}

/// Read an AMF0 string with the wire length limit (u16) and return it as a
/// freshly allocated `String`. Used where a fixed-size staging buffer would
/// otherwise impose a smaller, unrelated cap than the writer accepts.
///
/// Unlike [`read_string`], the result is not NUL-terminated and any length up to
/// `u16::MAX` is accepted.
///
/// # Errors
///
/// Returns [`ErrorCode::Amf`] when the marker is not `0x02` or the bytes are not
/// valid UTF-8, and [`ErrorCode::Io`] when the data is truncated.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0;
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::new();
/// amf0::write_string(&mut buf, "onMetaData").unwrap();
/// assert_eq!(amf0::read_string_alloc(&mut buf).unwrap(), "onMetaData");
/// ```
pub fn read_string_alloc(buf: &mut Buffer) -> Result<String> {
    let ty = read_u8(buf)?;
    if ty != Amf0Type::String as u8 {
        return Err(ErrorCode::Amf);
    }
    let len = read_u16(buf)? as usize;
    let mut bytes = vec![0u8; len];
    buf.read(&mut bytes).map_err(|_| ErrorCode::Amf)?;
    String::from_utf8(bytes).map_err(|_| ErrorCode::Amf)
}

/// Read an AMF0 long string.
pub fn read_long_string(buf: &mut Buffer, out: &mut [u8]) -> Result<usize> {
    let ty = read_u8(buf)?;
    if ty != Amf0Type::LongString as u8 {
        return Err(ErrorCode::Amf);
    }
    let str_len = read_u32(buf)? as usize;
    if out.is_empty() || str_len >= out.len() {
        return Err(ErrorCode::Amf);
    }
    buf.read(&mut out[..str_len]).map_err(|_| ErrorCode::Amf)?;
    out[str_len] = 0;
    Ok(str_len)
}

/// Read the beginning of an AMF0 object.
pub fn read_object_begin(buf: &mut Buffer) -> Result<()> {
    let ty = read_u8(buf)?;
    if ty != Amf0Type::Object as u8 {
        return Err(ErrorCode::Amf);
    }
    Ok(())
}

/// Read an AMF0 object key.
pub fn read_object_key(buf: &mut Buffer, out: &mut [u8]) -> Result<usize> {
    let klen = read_u16(buf)? as usize;
    if klen >= out.len() {
        return Err(ErrorCode::Amf);
    }
    buf.read(&mut out[..klen]).map_err(|_| ErrorCode::Amf)?;
    out[klen] = 0;
    Ok(klen)
}

/// Check if the next 3 bytes are the object end marker (0x00 0x00 0x09).
///
/// Non-consuming: the caller still has to consume the marker (e.g. by reading
/// it into a scratch slice) once this returns `true`. Returns `false` when fewer
/// than three bytes are buffered.
///
/// # Examples
///
/// ```
/// use librtmp2::amf::amf0;
/// use librtmp2::buffer::Buffer;
///
/// // After an object's last value, the end marker is pending.
/// let mut buf = Buffer::new();
/// amf0::write_object_begin(&mut buf).unwrap();
/// amf0::write_object_end(&mut buf).unwrap();
///
/// let mut reader = Buffer::from_slice(&buf.peek()[1..]);
/// assert!(amf0::is_object_end(&mut reader));
///
/// // A key follows here, so this is not the end.
/// let mut with_key = Buffer::from_slice(&[0x00, 0x03, b'a']);
/// assert!(!amf0::is_object_end(&mut with_key));
/// ```
pub fn is_object_end(buf: &mut Buffer) -> bool {
    let peek = buf.peek();
    if peek.len() < 3 {
        return false;
    }
    peek[0] == 0x00 && peek[1] == 0x00 && peek[2] == 0x09
}

const MAX_SKIP_DEPTH: i32 = 32;
/// Maximum number of object keys tolerated while decoding one AMF0 object
/// before the decoder rejects the message.
///
/// Bounds the work a hostile peer can demand from an object that claims a huge
/// number of keys, independent of how many bytes it actually sends.
pub const MAX_OBJECT_KEYS: usize = 256;

/// Skip an AMF0 value whose type marker has already been consumed.
pub fn skip_value_after_type(buf: &mut Buffer, ty: Amf0Type) -> Result<()> {
    skip_value_depth(buf, 0, ty)
}

fn skip_value_depth(buf: &mut Buffer, depth: i32, ty: Amf0Type) -> Result<()> {
    match ty {
        Amf0Type::Number => {
            read_double(buf)?;
            Ok(())
        }
        Amf0Type::Boolean => {
            read_u8(buf)?;
            Ok(())
        }
        Amf0Type::String => {
            let len = read_u16(buf)? as usize;
            if buf.available() < len {
                return Err(ErrorCode::Io);
            }
            buf.drain(len);
            Ok(())
        }
        Amf0Type::LongString => {
            let len = read_u32(buf)? as usize;
            if buf.available() < len {
                return Err(ErrorCode::Io);
            }
            buf.drain(len);
            Ok(())
        }
        Amf0Type::StrictArray => {
            if depth >= MAX_SKIP_DEPTH {
                return Err(ErrorCode::Amf);
            }
            let count = read_u32(buf)? as usize;
            if count > MAX_OBJECT_KEYS {
                return Err(ErrorCode::Amf);
            }
            for _ in 0..count {
                let child_ty = read_type(buf)?;
                skip_value_depth(buf, depth + 1, child_ty)?;
            }
            Ok(())
        }
        Amf0Type::Object | Amf0Type::EcmaArray => {
            if depth >= MAX_SKIP_DEPTH {
                return Err(ErrorCode::Amf);
            }
            if ty == Amf0Type::EcmaArray {
                read_u32(buf)?; // ecma count
            }
            let mut keys = 0;
            loop {
                if is_object_end(buf) {
                    let mut end = [0u8; 3];
                    buf.read(&mut end).map_err(|_| ErrorCode::Io)?;
                    return Ok(());
                }
                keys += 1;
                if keys > MAX_OBJECT_KEYS {
                    return Err(ErrorCode::Amf);
                }
                let klen = read_u16(buf)? as usize;
                if buf.available() < klen {
                    return Err(ErrorCode::Io);
                }
                buf.drain(klen);
                let child_ty = read_type(buf)?;
                skip_value_depth(buf, depth + 1, child_ty)?;
            }
        }
        Amf0Type::Date => {
            // 8-byte double (ms since epoch) + 2-byte timezone field.
            if buf.available() < 10 {
                return Err(ErrorCode::Io);
            }
            buf.drain(10);
            Ok(())
        }
        Amf0Type::Reference => {
            if buf.available() < 2 {
                return Err(ErrorCode::Io);
            }
            buf.drain(2);
            Ok(())
        }
        Amf0Type::Null | Amf0Type::Undefined | Amf0Type::Movieclip => Ok(()),
        Amf0Type::XmlDoc => {
            let len = read_u32(buf)? as usize;
            if buf.available() < len {
                return Err(ErrorCode::Io);
            }
            buf.drain(len);
            Ok(())
        }
        Amf0Type::TypedObject => {
            if depth >= MAX_SKIP_DEPTH {
                return Err(ErrorCode::Amf);
            }
            let class_len = read_u16(buf)? as usize;
            if buf.available() < class_len {
                return Err(ErrorCode::Io);
            }
            buf.drain(class_len);
            let mut keys = 0;
            loop {
                if is_object_end(buf) {
                    let mut end = [0u8; 3];
                    buf.read(&mut end).map_err(|_| ErrorCode::Io)?;
                    return Ok(());
                }
                keys += 1;
                if keys > MAX_OBJECT_KEYS {
                    return Err(ErrorCode::Amf);
                }
                let klen = read_u16(buf)? as usize;
                if buf.available() < klen {
                    return Err(ErrorCode::Io);
                }
                buf.drain(klen);
                let child_ty = read_type(buf)?;
                skip_value_depth(buf, depth + 1, child_ty)?;
            }
        }
        _ => Err(ErrorCode::Unsupported),
    }
}

/// Skip an AMF0 value.
pub fn skip_value(buf: &mut Buffer) -> Result<()> {
    let ty = read_type(buf)?;
    skip_value_depth(buf, 0, ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_wire_format_is_big_endian() {
        let mut buf = Buffer::new();
        write_string(&mut buf, "abc").unwrap();
        // marker(1) + len(2, big-endian) + bytes
        assert_eq!(buf.peek(), &[0x02, 0x00, 0x03, b'a', b'b', b'c']);
    }

    #[test]
    fn number_round_trips() {
        let mut buf = Buffer::new();
        write_number(&mut buf, 42.5).unwrap();
        assert_eq!(read_type(&mut buf).unwrap(), Amf0Type::Number);
        assert_eq!(read_number(&mut buf).unwrap(), 42.5);
    }

    #[test]
    fn boolean_round_trips() {
        let mut buf = Buffer::new();
        write_boolean(&mut buf, true).unwrap();
        assert_eq!(read_type(&mut buf).unwrap(), Amf0Type::Boolean);
        assert!(read_boolean(&mut buf).unwrap());
    }

    #[test]
    fn string_round_trips_through_read_string() {
        let mut buf = Buffer::new();
        write_string(&mut buf, "hello").unwrap();
        let mut out = [0u8; 16];
        let len = read_string(&mut buf, &mut out).unwrap();
        assert_eq!(&out[..len], b"hello");
    }

    #[test]
    fn long_string_wire_format_is_big_endian() {
        let mut buf = Buffer::new();
        write_long_string(&mut buf, "x").unwrap();
        assert_eq!(buf.peek(), &[0x0C, 0x00, 0x00, 0x00, 0x01, b'x']);
    }

    #[test]
    fn object_key_wire_format_is_big_endian() {
        let mut buf = Buffer::new();
        write_object_key(&mut buf, "k").unwrap();
        assert_eq!(buf.peek(), &[0x00, 0x01, b'k']);
    }

    #[test]
    fn strict_array_can_be_skipped() {
        let mut buf = Buffer::new();
        buf.write(&[
            0x0A, 0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x04, b'a', b'v', b'0', b'1',
        ])
        .unwrap();
        skip_value(&mut buf).unwrap();
        assert_eq!(buf.available(), 0);
    }

    #[test]
    fn ecma_array_begin_wire_format_is_big_endian() {
        let mut buf = Buffer::new();
        write_ecma_array_begin(&mut buf, 2).unwrap();
        assert_eq!(buf.peek(), &[0x08, 0x00, 0x00, 0x00, 0x02]);
    }

    #[test]
    fn skip_value_skips_nested_object() {
        let mut buf = Buffer::new();
        write_object_begin(&mut buf).unwrap();
        write_object_key(&mut buf, "a").unwrap();
        write_number(&mut buf, 1.0).unwrap();
        write_object_end(&mut buf).unwrap();
        write_number(&mut buf, 2.0).unwrap();

        skip_value(&mut buf).unwrap();
        assert_eq!(read_type(&mut buf).unwrap(), Amf0Type::Number);
        assert_eq!(read_number(&mut buf).unwrap(), 2.0);
    }

    #[test]
    fn is_object_end_detects_marker() {
        let mut buf = Buffer::new();
        write_object_end(&mut buf).unwrap();
        assert!(is_object_end(&mut buf));
    }

    #[test]
    fn read_string_rejects_truncated_payload() {
        let mut buf = Buffer::from_slice(&[0x02, 0x00, 0x05, b'a', b'b']);
        let mut out = [0u8; 16];
        assert_eq!(read_string(&mut buf, &mut out), Err(ErrorCode::Amf));
    }

    #[test]
    fn read_long_string_rejects_truncated_payload() {
        let mut buf = Buffer::from_slice(&[0x0C, 0x00, 0x00, 0x00, 0x04, b'x']);
        let mut out = [0u8; 16];
        assert_eq!(read_long_string(&mut buf, &mut out), Err(ErrorCode::Amf));
    }

    #[test]
    fn read_object_key_rejects_truncated_key_bytes() {
        let mut buf = Buffer::from_slice(&[0x00, 0x03, b'a']);
        let mut out = [0u8; 16];
        assert_eq!(read_object_key(&mut buf, &mut out), Err(ErrorCode::Amf));
    }

    #[test]
    fn skip_value_rejects_truncated_string_length() {
        let mut buf = Buffer::from_slice(&[0x02, 0x00, 0x10]);
        assert_eq!(skip_value(&mut buf), Err(ErrorCode::Io));
    }

    #[test]
    fn skip_value_rejects_truncated_long_string_length() {
        let mut buf = Buffer::from_slice(&[0x0C, 0x00, 0x00, 0x00, 0x08, 0xAA]);
        assert_eq!(skip_value(&mut buf), Err(ErrorCode::Io));
    }

    #[test]
    fn skip_value_rejects_truncated_date() {
        let mut buf = Buffer::from_slice(&[0x0B, 0x00, 0x00, 0x00]);
        assert_eq!(skip_value(&mut buf), Err(ErrorCode::Io));
    }

    #[test]
    fn skip_value_rejects_excessive_strict_array_count() {
        // count = 257 (> MAX_OBJECT_KEYS)
        let mut buf = Buffer::from_slice(&[0x0A, 0x00, 0x00, 0x01, 0x01]);
        assert_eq!(skip_value(&mut buf), Err(ErrorCode::Amf));
    }

    #[test]
    fn skip_value_rejects_excessive_nesting_depth() {
        // Build nested objects via empty-key chains: { "": { "": { ... } } }
        let mut payload = Vec::new();
        for _ in 0..=MAX_SKIP_DEPTH as usize {
            payload.push(Amf0Type::Object as u8);
            payload.extend_from_slice(&[0x00, 0x00]); // empty key
        }
        payload.extend_from_slice(&[Amf0Type::Null as u8]); // innermost value
        for _ in 0..=MAX_SKIP_DEPTH as usize {
            payload.extend_from_slice(&[0x00, 0x00, 0x09]);
        }
        let mut buf = Buffer::from_slice(&payload);
        assert_eq!(skip_value(&mut buf), Err(ErrorCode::Amf));
    }

    #[test]
    fn skip_value_rejects_object_with_too_many_keys() {
        let mut buf = Buffer::new();
        write_object_begin(&mut buf).unwrap();
        for i in 0..=MAX_OBJECT_KEYS {
            write_object_key(&mut buf, &format!("k{i}")).unwrap();
            write_null(&mut buf).unwrap();
        }
        write_object_end(&mut buf).unwrap();
        assert_eq!(skip_value(&mut buf), Err(ErrorCode::Amf));
    }
}
