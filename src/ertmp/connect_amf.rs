//! AMF0 bridge for E-RTMP connect-object capability fields.

use crate::amf::amf0::{self, Amf0Type};
use crate::buffer::Buffer;
use crate::types::{
    CapsExit, ErrorCode, FourCcList, NegotiatedCaps, Reconnect, Result, VideoFourCcInfoMap,
    CAPS_EX_MASK_MULTITRACK, CAPS_EX_MASK_MODEX, CAPS_EX_MASK_RECONNECT,
    CAPS_EX_MASK_SERVER_DEFAULT, CAPS_EX_MASK_TIMESTAMP_NANO,
};

use super::connect_caps::{
    caps_exit_parse, caps_exit_write, fourcc_list_add, fourcc_list_init, fourcc_list_parse,
    fourcc_list_write, video_fourcc_info_map_parse, video_fourcc_info_map_write,
};
use super::reconnect::{reconnect_parse, reconnect_write};

const MAX_CAPS_BLOB_BYTES: usize = 4096;

/// Read a `fourCcList` AMF value into a [`FourCcList`].
pub fn read_four_cc_list_amf(buf: &mut Buffer, list: &mut FourCcList) -> Result<()> {
    fourcc_list_init(list);
    let ty = amf0::read_type(buf)?;
    match ty {
        Amf0Type::StrictArray => read_four_cc_strict_array(buf, list),
        Amf0Type::LongString => {
            let data = read_amf_binary_blob(buf, ty)?;
            fourcc_list_parse(list, &data).map(|_| ())
        }
        _ => Err(ErrorCode::Amf),
    }
}

/// Read a `videoFourCcInfoMap` AMF value.
pub fn read_video_fourcc_info_map_amf(buf: &mut Buffer, map: &mut VideoFourCcInfoMap) -> Result<()> {
    map.count = 0;
    let ty = amf0::read_type(buf)?;
    match ty {
        Amf0Type::StrictArray => {
            let count = read_u32(buf)? as usize;
            if count > crate::types::MAX_FOURCCS {
                return Err(ErrorCode::Amf);
            }
            for _ in 0..count {
                let mut cc = [0u8; 8];
                let n = amf0::read_string(buf, &mut cc)?;
                if n >= 4 {
                    map.entries[map.count].cc[..4].copy_from_slice(&cc[..4]);
                    map.count += 1;
                }
            }
            Ok(())
        }
        Amf0Type::LongString => {
            let data = read_amf_binary_blob(buf, ty)?;
            video_fourcc_info_map_parse(map, &data).map(|_| ())
        }
        _ => Err(ErrorCode::Amf),
    }
}

/// Read a `capsEx` AMF value (numeric bitmask per E-RTMP v2 spec).
pub fn read_caps_ex_amf(buf: &mut Buffer, caps: &mut CapsExit, mask: &mut u32) -> Result<()> {
    let ty = amf0::read_type(buf)?;
    match ty {
        Amf0Type::Number => {
            *mask = amf0::read_number(buf)? as u32;
            Ok(())
        }
        Amf0Type::LongString => {
            let data = read_amf_binary_blob(buf, ty)?;
            caps_exit_parse(caps, &data)?;
            *mask = 0;
            Ok(())
        }
        Amf0Type::Object => {
            caps.version = 1;
            amf0::read_object_begin(buf)?;
            let mut keys = 0usize;
            while !amf0::is_object_end(buf) {
                keys += 1;
                if keys > amf0::MAX_OBJECT_KEYS {
                    return Err(ErrorCode::Amf);
                }
                let mut key = [0u8; 256];
                let key_len = amf0::read_object_key(buf, &mut key)?;
                let key_str = std::str::from_utf8(&key[..key_len]).unwrap_or("");
                match key_str {
                    "videoCodecId" | "videoCodecFourCC" => {
                        caps.video_codec_32 = read_fourcc_number(buf)?;
                    }
                    "audioCodecId" | "audioCodecFourCC" => {
                        caps.audio_codec_32 = read_fourcc_number(buf)?;
                    }
                    _ => {
                        amf0::skip_value(buf)?;
                    }
                }
            }
            let mut end = [0u8; 3];
            buf.read(&mut end).map_err(|_| ErrorCode::Amf)?;
            Ok(())
        }
        _ => Err(ErrorCode::Amf),
    }
}

/// Read a v2 `reconnect` AMF value.
pub fn read_reconnect_amf(buf: &mut Buffer, rc: &mut Reconnect) -> Result<()> {
    let ty = amf0::read_type(buf)?;
    let data = read_amf_binary_blob(buf, ty)?;
    reconnect_parse(rc, &data)
}

/// Write negotiated caps into an AMF0 object (without surrounding object markers).
pub fn write_negotiated_caps(buf: &mut Buffer, caps: &NegotiatedCaps) -> Result<()> {
    if caps.has_four_cc_list && caps.four_cc_list.count > 0 {
        amf0::write_object_key(buf, "fourCcList")?;
        write_four_cc_list_amf(buf, &caps.four_cc_list)?;
    }
    if caps.has_caps_ex {
        amf0::write_object_key(buf, "capsEx")?;
        write_caps_ex_amf(buf, caps.caps_ex_mask)?;
    }
    if caps.has_video_four_cc_info_map && caps.video_four_cc_info_map.count > 0 {
        amf0::write_object_key(buf, "videoFourCcInfoMap")?;
        write_video_fourcc_info_map_amf(buf, &caps.video_four_cc_info_map)?;
    }
    if caps.has_reconnect {
        amf0::write_object_key(buf, "reconnect")?;
        write_reconnect_amf(buf, &caps.reconnect)?;
    }
    Ok(())
}

pub fn write_four_cc_list_amf(buf: &mut Buffer, list: &FourCcList) -> Result<()> {
    buf.write(&[Amf0Type::StrictArray as u8])
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&(list.count as u32).to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    for i in 0..list.count {
        let cc = std::str::from_utf8(&list.entries[i].cc[..4]).unwrap_or("????");
        amf0::write_string(buf, cc)?;
    }
    Ok(())
}

pub fn write_video_fourcc_info_map_amf(buf: &mut Buffer, map: &VideoFourCcInfoMap) -> Result<()> {
    write_four_cc_list_amf(buf, &FourCcList {
        entries: map.entries,
        count: map.count,
    })
}

pub fn write_caps_ex_amf(buf: &mut Buffer, caps_ex_mask: u32) -> Result<()> {
    amf0::write_number(buf, caps_ex_mask as f64)
}

pub fn write_reconnect_amf(buf: &mut Buffer, rc: &Reconnect) -> Result<()> {
    let mut blob = [0u8; 8];
    if reconnect_write(rc, &mut blob) != 8 {
        return Err(ErrorCode::Internal);
    }
    amf0::write_long_string_bytes(buf, &blob)
}

fn read_four_cc_strict_array(buf: &mut Buffer, list: &mut FourCcList) -> Result<()> {
    let count = read_u32(buf)? as usize;
    if count > crate::types::MAX_FOURCCS {
        return Err(ErrorCode::Amf);
    }
    for _ in 0..count {
        let mut cc = [0u8; 8];
        let n = amf0::read_string(buf, &mut cc)?;
        if n >= 4 {
            fourcc_list_add(list, &cc[..n])?;
        }
    }
    Ok(())
}

fn read_fourcc_number(buf: &mut Buffer) -> Result<i32> {
    let ty = amf0::read_type(buf)?;
    if ty == Amf0Type::Number {
        Ok(amf0::read_number(buf)? as i32)
    } else if ty == Amf0Type::String {
        let mut cc = [0u8; 8];
        let n = amf0::read_string(buf, &mut cc)?;
        if n >= 4 {
            Ok(i32::from_be_bytes([cc[0], cc[1], cc[2], cc[3]]))
        } else {
            Err(ErrorCode::Amf)
        }
    } else {
        Err(ErrorCode::Amf)
    }
}

fn read_amf_binary_blob(buf: &mut Buffer, ty: Amf0Type) -> Result<Vec<u8>> {
    let len = match ty {
        Amf0Type::LongString => read_u32(buf)? as usize,
        Amf0Type::String => read_u16(buf)? as usize,
        _ => return Err(ErrorCode::Amf),
    };
    if len > MAX_CAPS_BLOB_BYTES || buf.available() < len {
        return Err(ErrorCode::Amf);
    }
    let mut data = vec![0u8; len];
    buf.read(&mut data).map_err(|_| ErrorCode::Amf)?;
    Ok(data)
}

fn read_u16(buf: &mut Buffer) -> Result<u16> {
    let mut b = [0u8; 2];
    buf.read(&mut b).map_err(|_| ErrorCode::Amf)?;
    Ok(u16::from_be_bytes(b))
}

fn read_u32(buf: &mut Buffer) -> Result<u32> {
    let mut b = [0u8; 4];
    buf.read(&mut b).map_err(|_| ErrorCode::Amf)?;
    Ok(u32::from_be_bytes(b))
}

fn four_cc_supported(cc: &[u8]) -> bool {
    matches!(
        cc,
        b"avc1" | b"hvc1" | b"av01" | b"vp09" | b"mp4a" | b"Opus"
    )
}

/// Intersect client-offered E-RTMP caps with what this server supports.
pub fn negotiate_caps(client: &crate::types::ConnectInfo) -> NegotiatedCaps {
    let mut out = NegotiatedCaps::default();

    if client.has_four_cc_list {
        for i in 0..client.four_cc_list.count {
            let cc = &client.four_cc_list.entries[i].cc[..4];
            if four_cc_supported(cc) {
                let _ = fourcc_list_add(&mut out.four_cc_list, cc);
            }
        }
        out.has_four_cc_list = out.four_cc_list.count > 0;
    }

    if client.has_caps_ex {
        let requested = client.caps_ex_mask;
        out.caps_ex_mask = requested & CAPS_EX_MASK_SERVER_DEFAULT;
        if client.has_reconnect {
            out.caps_ex_mask |= CAPS_EX_MASK_RECONNECT & requested;
        }
        out.has_caps_ex = true;
        out.multitrack_enabled = (out.caps_ex_mask & CAPS_EX_MASK_MULTITRACK) != 0;
    } else {
        // Legacy clients without capsEx still get multitrack relay when publishing
        // enhanced multitrack payloads; capability is not gated on connect.
        out.multitrack_enabled = true;
    }

    if client.has_video_four_cc_info_map {
        for i in 0..client.video_four_cc_info_map.count {
            let cc = &client.video_four_cc_info_map.entries[i].cc[..4];
            if four_cc_supported(cc) {
                out.video_four_cc_info_map.entries[out.video_four_cc_info_map.count].cc[..4]
                    .copy_from_slice(cc);
                out.video_four_cc_info_map.count += 1;
            }
        }
        out.has_video_four_cc_info_map = out.video_four_cc_info_map.count > 0;
    }

    if client.has_reconnect {
        out.reconnect = client.reconnect;
        out.has_reconnect = true;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FourCcList;

    #[test]
    fn four_cc_list_strict_array_round_trip() {
        let mut list = FourCcList::default();
        fourcc_list_add(&mut list, b"av01").unwrap();
        fourcc_list_add(&mut list, b"hvc1").unwrap();

        let mut buf = Buffer::new();
        write_four_cc_list_amf(&mut buf, &list).unwrap();

        let mut parsed = FourCcList::default();
        read_four_cc_list_amf(&mut buf, &mut parsed).unwrap();
        assert_eq!(parsed.count, 2);
        assert_eq!(&parsed.entries[0].cc[..4], b"av01");
        assert_eq!(&parsed.entries[1].cc[..4], b"hvc1");
    }

    #[test]
    fn caps_ex_mask_round_trip() {
        let mut buf = Buffer::new();
        write_caps_ex_amf(&mut buf, CAPS_EX_MASK_MULTITRACK | CAPS_EX_MASK_MODEX).unwrap();

        let mut parsed = CapsExit::default();
        let mut mask = 0u32;
        read_caps_ex_amf(&mut buf, &mut parsed, &mut mask).unwrap();
        assert_eq!(mask, CAPS_EX_MASK_MULTITRACK | CAPS_EX_MASK_MODEX);
    }

    #[test]
    fn caps_ex_binary_round_trip() {
        let caps = CapsExit {
            version: 1,
            video_codec_32: i32::from_be_bytes(*b"av01"),
            audio_codec_32: i32::from_be_bytes(*b"mp4a"),
        };
        let mut buf = Buffer::new();
        let mut blob = [0u8; 8];
        caps_exit_write(&caps, &mut blob);
        amf0::write_long_string_bytes(&mut buf, &blob).unwrap();

        let mut parsed = CapsExit::default();
        let mut mask = 0u32;
        read_caps_ex_amf(&mut buf, &mut parsed, &mut mask).unwrap();
        assert_eq!(parsed.video_codec_32, caps.video_codec_32);
        assert_eq!(parsed.audio_codec_32, caps.audio_codec_32);
    }

    #[test]
    fn negotiate_caps_honors_explicit_zero_caps_ex() {
        use crate::types::ConnectInfo;

        let mut client = ConnectInfo::default();
        client.has_caps_ex = true;
        client.caps_ex_mask = 0;

        let caps = negotiate_caps(&client);
        assert!(caps.has_caps_ex);
        assert_eq!(caps.caps_ex_mask, 0);
        assert!(!caps.multitrack_enabled);
    }
}
