from pathlib import Path
import re

ROOT = Path(".")

def read(path: str) -> str:
    return (ROOT / path).read_text()

def write(path: str, text: str) -> None:
    (ROOT / path).write_text(text)

def replace_once(path: str, old: str, new: str) -> None:
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected exactly one match, found {count}\n--- OLD ---\n{old[:500]}")
    write(path, text.replace(old, new, 1))

def replace_regex_once(path: str, pattern: str, repl: str) -> None:
    text = read(path)
    new_text, count = re.subn(pattern, repl, text, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"{path}: regex expected one match, found {count}: {pattern}")
    write(path, new_text)

replace_once(
    "src/types.rs",
    '''/// Video FourCC info map.
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct VideoFourCcInfoMap {
    pub entries: [FourCc; MAX_FOURCCS],
    pub count: usize,
}
''',
    '''/// E-RTMP v2 per-codec capability flags.
pub const FOUR_CC_INFO_CAN_DECODE: u32 = 0x01;
pub const FOUR_CC_INFO_CAN_ENCODE: u32 = 0x02;
pub const FOUR_CC_INFO_CAN_FORWARD: u32 = 0x04;
pub const FOUR_CC_INFO_ALL: u32 =
    FOUR_CC_INFO_CAN_DECODE | FOUR_CC_INFO_CAN_ENCODE | FOUR_CC_INFO_CAN_FORWARD;

/// Video FourCC info map. Each entry's mask uses `FOUR_CC_INFO_*`.
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct VideoFourCcInfoMap {
    pub entries: [FourCc; MAX_FOURCCS],
    pub masks: [u32; MAX_FOURCCS],
    pub count: usize,
}
'''
)

replace_once(
    "src/ertmp/connect_caps.rs",
    'use crate::types::{CapsExit, ErrorCode, FourCcList, Result, VideoFourCcInfoMap};',
    '''use crate::types::{
    CapsExit, ErrorCode, FourCcList, Result, VideoFourCcInfoMap, FOUR_CC_INFO_ALL,
};'''
)
replace_once(
    "src/ertmp/connect_caps.rs",
    '''pub fn fourcc_list_add(list: &mut FourCcList, cc: &[u8]) -> Result<()> {
    if list.count >= crate::types::MAX_FOURCCS {
        return Err(ErrorCode::Io);
    }
    if cc.len() < 4 {
        return Err(ErrorCode::Io);
    }
    list.entries[list.count].cc[..4].copy_from_slice(&cc[..4]);
    list.count += 1;
    Ok(())
}
''',
    '''pub fn fourcc_list_add(list: &mut FourCcList, cc: &[u8]) -> Result<()> {
    if list.count >= crate::types::MAX_FOURCCS {
        return Err(ErrorCode::Io);
    }
    let entry = &mut list.entries[list.count];
    *entry = Default::default();
    if cc == b"*" {
        entry.cc[0] = b'*';
    } else if cc.len() >= 4 {
        entry.cc[..4].copy_from_slice(&cc[..4]);
    } else {
        return Err(ErrorCode::Io);
    }
    list.count += 1;
    Ok(())
}
'''
)
replace_once(
    "src/ertmp/connect_caps.rs",
    '''    let mut offset = 4;
    for _ in 0..count {
        if offset + 6 > data.len() {
            break;
        }
        let slen = ((data[offset] as u16) << 8) | (data[offset + 1] as u16);
        offset += 2;
        if slen != 4 || offset + 4 > data.len() {
            break;
        }
        list.entries[list.count].cc[..4].copy_from_slice(&data[offset..offset + 4]);
        list.count += 1;
        offset += 4;
    }
    Ok(list.count)
''',
    '''    let mut offset = 4;
    for _ in 0..count {
        if offset + 2 > data.len() {
            break;
        }
        let slen = (((data[offset] as u16) << 8) | (data[offset + 1] as u16)) as usize;
        offset += 2;
        if !matches!(slen, 1 | 4) || offset + slen > data.len() {
            break;
        }
        fourcc_list_add(list, &data[offset..offset + slen])?;
        offset += slen;
    }
    Ok(list.count)
'''
)
replace_once(
    "src/ertmp/connect_caps.rs",
    '''pub fn fourcc_list_write(list: &FourCcList, buf: &mut [u8]) -> usize {
    let needed = 4 + list.count * 6;
    if buf.len() < needed {
        return 0;
    }

    // Write count as big-endian u32 to match fourcc_list_parse's big-endian read.
    let cnt = list.count as u32;
    buf[0] = (cnt >> 24) as u8;
    buf[1] = (cnt >> 16) as u8;
    buf[2] = (cnt >> 8) as u8;
    buf[3] = cnt as u8;

    let mut offset = 4;
    for i in 0..list.count {
        buf[offset] = 0;
        buf[offset + 1] = 4;
        offset += 2;
        buf[offset..offset + 4].copy_from_slice(&list.entries[i].cc[..4]);
        offset += 4;
    }
    offset
}
''',
    '''pub fn fourcc_list_write(list: &FourCcList, buf: &mut [u8]) -> usize {
    let needed = 4
        + (0..list.count)
            .map(|i| if list.entries[i].cc[0] == b'*' { 3 } else { 6 })
            .sum::<usize>();
    if buf.len() < needed {
        return 0;
    }

    let cnt = list.count as u32;
    buf[..4].copy_from_slice(&cnt.to_be_bytes());

    let mut offset = 4;
    for i in 0..list.count {
        let wildcard = list.entries[i].cc[0] == b'*';
        let len = if wildcard { 1 } else { 4 };
        buf[offset..offset + 2].copy_from_slice(&(len as u16).to_be_bytes());
        offset += 2;
        if wildcard {
            buf[offset] = b'*';
        } else {
            buf[offset..offset + 4].copy_from_slice(&list.entries[i].cc[..4]);
        }
        offset += len;
    }
    offset
}
'''
)
replace_regex_once(
    "src/ertmp/connect_caps.rs",
    r'''/\* ── E-RTMP v2 videoFourCcInfoMap ── \*/.*\Z''',
    r'''/* ── E-RTMP v2 videoFourCcInfoMap ── */

/// Parse a video FourCC info map from the legacy binary helper representation.
/// New encodings append a UI32 capability mask to each key; the old key-only
/// representation remains accepted and is interpreted as supporting all flags.
pub fn video_fourcc_info_map_parse(map: &mut VideoFourCcInfoMap, data: &[u8]) -> Result<usize> {
    if data.len() < 4 {
        return Err(ErrorCode::Io);
    }
    *map = VideoFourCcInfoMap::default();

    let count = u32::from_be_bytes([data[0], data[1], data[2], data[3]])
        .min(crate::types::MAX_FOURCCS as u32) as usize;
    let legacy_key_only = data.len() == 4 + count * 6;
    let mut offset = 4;

    for _ in 0..count {
        if offset + 2 > data.len() {
            return Err(ErrorCode::Io);
        }
        let slen = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;
        if !matches!(slen, 1 | 4) || offset + slen > data.len() {
            return Err(ErrorCode::Io);
        }

        let entry = &mut map.entries[map.count];
        if slen == 1 && data[offset] == b'*' {
            entry.cc[0] = b'*';
        } else if slen == 4 {
            entry.cc[..4].copy_from_slice(&data[offset..offset + 4]);
        } else {
            return Err(ErrorCode::Io);
        }
        offset += slen;

        map.masks[map.count] = if legacy_key_only {
            FOUR_CC_INFO_ALL
        } else {
            if offset + 4 > data.len() {
                return Err(ErrorCode::Io);
            }
            let mask = u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            offset += 4;
            mask
        };
        map.count += 1;
    }
    Ok(map.count)
}

/// Write the binary helper representation with a UI32 capability mask per key.
pub fn video_fourcc_info_map_write(map: &VideoFourCcInfoMap, buf: &mut [u8]) -> usize {
    let needed = 4
        + (0..map.count)
            .map(|i| if map.entries[i].cc[0] == b'*' { 7 } else { 10 })
            .sum::<usize>();
    if buf.len() < needed {
        return 0;
    }

    buf[..4].copy_from_slice(&(map.count as u32).to_be_bytes());
    let mut offset = 4;
    for i in 0..map.count {
        let wildcard = map.entries[i].cc[0] == b'*';
        let len = if wildcard { 1 } else { 4 };
        buf[offset..offset + 2].copy_from_slice(&(len as u16).to_be_bytes());
        offset += 2;
        if wildcard {
            buf[offset] = b'*';
        } else {
            buf[offset..offset + 4].copy_from_slice(&map.entries[i].cc[..4]);
        }
        offset += len;
        buf[offset..offset + 4].copy_from_slice(&map.masks[i].to_be_bytes());
        offset += 4;
    }
    offset
}

#[cfg(test)]
mod video_map_tests {
    use super::*;
    use crate::types::{FOUR_CC_INFO_CAN_DECODE, FOUR_CC_INFO_CAN_FORWARD};

    #[test]
    fn fourcc_list_round_trips_wildcard() {
        let mut list = FourCcList::default();
        fourcc_list_add(&mut list, b"*").unwrap();
        let mut wire = [0u8; 32];
        let len = fourcc_list_write(&list, &mut wire);
        let mut parsed = FourCcList::default();
        fourcc_list_parse(&mut parsed, &wire[..len]).unwrap();
        assert_eq!(parsed.count, 1);
        assert_eq!(parsed.entries[0].cc[0], b'*');
    }

    #[test]
    fn video_info_map_round_trips_flags() {
        let mut map = VideoFourCcInfoMap::default();
        map.entries[0].cc[..4].copy_from_slice(b"vp09");
        map.masks[0] = FOUR_CC_INFO_CAN_DECODE | FOUR_CC_INFO_CAN_FORWARD;
        map.count = 1;

        let mut wire = [0u8; 64];
        let len = video_fourcc_info_map_write(&map, &mut wire);
        let mut parsed = VideoFourCcInfoMap::default();
        video_fourcc_info_map_parse(&mut parsed, &wire[..len]).unwrap();

        assert_eq!(parsed.count, 1);
        assert_eq!(&parsed.entries[0].cc[..4], b"vp09");
        assert_eq!(parsed.masks[0], map.masks[0]);
    }
}
'''
)

replace_once(
    "src/ertmp/connect_amf.rs",
    '''    CapsExit, ErrorCode, FourCcList, NegotiatedCaps, Reconnect, Result, VideoFourCcInfoMap,
    CAPS_EX_MASK_MULTITRACK, CAPS_EX_MASK_MODEX, CAPS_EX_MASK_RECONNECT,
    CAPS_EX_MASK_SERVER_DEFAULT, CAPS_EX_MASK_TIMESTAMP_NANO,
''',
    '''    CapsExit, ErrorCode, FourCcList, NegotiatedCaps, Reconnect, Result, VideoFourCcInfoMap,
    CAPS_EX_MASK_MULTITRACK, CAPS_EX_MASK_MODEX, CAPS_EX_MASK_RECONNECT,
    CAPS_EX_MASK_SERVER_DEFAULT, CAPS_EX_MASK_TIMESTAMP_NANO, FOUR_CC_INFO_ALL,
    FOUR_CC_INFO_CAN_FORWARD,
'''
)
replace_regex_once(
    "src/ertmp/connect_amf.rs",
    r'''/// Read a `videoFourCcInfoMap` AMF value\.\npub fn read_video_fourcc_info_map_amf.*?\n}\n\n/// Read a `capsEx`''',
    r'''/// Read a `videoFourCcInfoMap` AMF value.
pub fn read_video_fourcc_info_map_amf(buf: &mut Buffer, map: &mut VideoFourCcInfoMap) -> Result<()> {
    *map = VideoFourCcInfoMap::default();
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
                add_video_map_entry(map, &cc[..n], FOUR_CC_INFO_ALL)?;
            }
            Ok(())
        }
        Amf0Type::LongString => {
            let data = read_amf_binary_blob(buf, ty)?;
            video_fourcc_info_map_parse(map, &data).map(|_| ())
        }
        Amf0Type::Object | Amf0Type::EcmaArray => {
            if ty == Amf0Type::EcmaArray {
                let _declared_count = read_u32(buf)?;
            }
            let mut keys = 0usize;
            while !amf0::is_object_end(buf) {
                keys += 1;
                if keys > amf0::MAX_OBJECT_KEYS {
                    return Err(ErrorCode::Amf);
                }
                let mut key = [0u8; 256];
                let key_len = amf0::read_object_key(buf, &mut key)?;
                let value_type = amf0::read_type(buf)?;
                if value_type != Amf0Type::Number {
                    return Err(ErrorCode::Amf);
                }
                let value = amf0::read_number(buf)?;
                if !value.is_finite() || value < 0.0 || value > u32::MAX as f64 {
                    return Err(ErrorCode::Amf);
                }
                add_video_map_entry(map, &key[..key_len], value as u32)?;
            }
            let mut end = [0u8; 3];
            buf.read(&mut end).map_err(|_| ErrorCode::Amf)?;
            Ok(())
        }
        _ => Err(ErrorCode::Amf),
    }
}

fn add_video_map_entry(
    map: &mut VideoFourCcInfoMap,
    key: &[u8],
    mask: u32,
) -> Result<()> {
    if map.count >= crate::types::MAX_FOURCCS {
        return Err(ErrorCode::Amf);
    }
    let entry = &mut map.entries[map.count];
    if key == b"*" {
        entry.cc[0] = b'*';
    } else if key.len() >= 4 {
        entry.cc[..4].copy_from_slice(&key[..4]);
    } else {
        return Err(ErrorCode::Amf);
    }
    map.masks[map.count] = mask;
    map.count += 1;
    Ok(())
}

/// Read a `capsEx`'''
)
replace_once(
    "src/ertmp/connect_amf.rs",
    '''pub fn write_video_fourcc_info_map_amf(buf: &mut Buffer, map: &VideoFourCcInfoMap) -> Result<()> {
    write_four_cc_list_amf(buf, &FourCcList {
        entries: map.entries,
        count: map.count,
    })
}
''',
    '''pub fn write_video_fourcc_info_map_amf(buf: &mut Buffer, map: &VideoFourCcInfoMap) -> Result<()> {
    amf0::write_object_begin(buf)?;
    for i in 0..map.count {
        let key = if map.entries[i].cc[0] == b'*' {
            "*"
        } else {
            std::str::from_utf8(&map.entries[i].cc[..4]).map_err(|_| ErrorCode::Amf)?
        };
        amf0::write_object_key(buf, key)?;
        amf0::write_number(buf, map.masks[i] as f64)?;
    }
    amf0::write_object_end(buf)
}
'''
)
replace_once(
    "src/ertmp/connect_amf.rs",
    '''    for i in 0..list.count {
        let cc = std::str::from_utf8(&list.entries[i].cc[..4]).unwrap_or("????");
        amf0::write_string(buf, cc)?;
    }
''',
    '''    for i in 0..list.count {
        let cc = if list.entries[i].cc[0] == b'*' {
            "*"
        } else {
            std::str::from_utf8(&list.entries[i].cc[..4]).unwrap_or("????")
        };
        amf0::write_string(buf, cc)?;
    }
'''
)
replace_once(
    "src/ertmp/connect_amf.rs",
    '''    } else if ty == Amf0Type::String {
        let mut cc = [0u8; 8];
        let n = amf0::read_string(buf, &mut cc)?;
        if n >= 4 {
            Ok(i32::from_be_bytes([cc[0], cc[1], cc[2], cc[3]]))
        } else {
            Err(ErrorCode::Amf)
        }
''',
    '''    } else if ty == Amf0Type::String {
        let len = read_u16(buf)? as usize;
        if len < 4 || buf.available() < len {
            return Err(ErrorCode::Amf);
        }
        let mut cc = vec![0u8; len];
        buf.read(&mut cc).map_err(|_| ErrorCode::Amf)?;
        Ok(i32::from_be_bytes([cc[0], cc[1], cc[2], cc[3]]))
'''
)
replace_once(
    "src/ertmp/connect_amf.rs",
    '''fn four_cc_supported(cc: &[u8]) -> bool {
    matches!(
        cc,
        b"avc1" | b"hvc1" | b"av01" | b"vp09" | b"mp4a" | b"Opus"
    )
}
''',
    '''fn four_cc_supported(cc: &[u8]) -> bool {
    cc == b"*"
        || matches!(
            cc,
            b"avc1"
                | b"hvc1"
                | b"av01"
                | b"vp08"
                | b"vp09"
                | b"vvc1"
                | b"mp4a"
                | b"Opus"
                | b"ac-3"
                | b"ec-3"
                | b".mp3"
                | b"fLaC"
        )
}

fn fourcc_entry_bytes(entry: &crate::types::FourCc) -> &[u8] {
    if entry.cc[0] == b'*' {
        b"*"
    } else {
        &entry.cc[..4]
    }
}
'''
)
replace_once(
    "src/ertmp/connect_amf.rs",
    '''        for i in 0..client.four_cc_list.count {
            let cc = &client.four_cc_list.entries[i].cc[..4];
            if four_cc_supported(cc) {
                let _ = fourcc_list_add(&mut out.four_cc_list, cc);
            }
        }
''',
    '''        for i in 0..client.four_cc_list.count {
            let cc = fourcc_entry_bytes(&client.four_cc_list.entries[i]);
            if four_cc_supported(cc) {
                let _ = fourcc_list_add(&mut out.four_cc_list, cc);
            }
        }
'''
)
replace_once(
    "src/ertmp/connect_amf.rs",
    '''    if client.has_video_four_cc_info_map {
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
''',
    '''    if client.has_video_four_cc_info_map {
        for i in 0..client.video_four_cc_info_map.count {
            let cc = fourcc_entry_bytes(&client.video_four_cc_info_map.entries[i]);
            let negotiated_mask =
                client.video_four_cc_info_map.masks[i] & FOUR_CC_INFO_CAN_FORWARD;
            if negotiated_mask == 0 || !four_cc_supported(cc) {
                continue;
            }
            let idx = out.video_four_cc_info_map.count;
            if cc == b"*" {
                out.video_four_cc_info_map.entries[idx].cc[0] = b'*';
            } else {
                out.video_four_cc_info_map.entries[idx].cc[..4].copy_from_slice(&cc[..4]);
            }
            out.video_four_cc_info_map.masks[idx] = negotiated_mask;
            out.video_four_cc_info_map.count += 1;
        }
        out.has_video_four_cc_info_map = out.video_four_cc_info_map.count > 0;
    }
'''
)
replace_once(
    "src/ertmp/connect_amf.rs",
    '''        assert_eq!(map.count, 2);
        assert_eq!(&map.entries[0].cc[..4], b"av01");
        assert_eq!(&map.entries[1].cc[..4], b"hvc1");
''',
    '''        assert_eq!(map.count, 2);
        assert_eq!(&map.entries[0].cc[..4], b"av01");
        assert_eq!(map.masks[0], 1);
        assert_eq!(&map.entries[1].cc[..4], b"hvc1");
        assert_eq!(map.masks[1], 2);
'''
)
replace_once(
    "src/ertmp/connect_amf.rs",
    '''    #[test]
    fn negotiate_caps_legacy_object_caps_ex_uses_defaults() {
        use crate::types::ConnectInfo;

        let mut client = ConnectInfo::default();
        client.has_caps_ex = true;
        client.caps_ex_mask = CAPS_EX_MASK_SERVER_DEFAULT;
        client.caps_ex.version = 1;
        client.caps_ex.video_codec_32 = i32::from_be_bytes(*b"av01");

        let caps = negotiate_caps(&client);
        assert!(caps.has_caps_ex);
        assert_eq!(caps.caps_ex_mask, CAPS_EX_MASK_SERVER_DEFAULT);
        assert!(caps.multitrack_enabled);
    }
}
''',
    '''    #[test]
    fn negotiate_caps_legacy_object_caps_ex_uses_defaults() {
        use crate::types::ConnectInfo;

        let mut client = ConnectInfo::default();
        client.has_caps_ex = true;
        client.caps_ex_mask = CAPS_EX_MASK_SERVER_DEFAULT;
        client.caps_ex.version = 1;
        client.caps_ex.video_codec_32 = i32::from_be_bytes(*b"av01");

        let caps = negotiate_caps(&client);
        assert!(caps.has_caps_ex);
        assert_eq!(caps.caps_ex_mask, CAPS_EX_MASK_SERVER_DEFAULT);
        assert!(caps.multitrack_enabled);
    }

    #[test]
    fn video_info_map_writes_object_with_numeric_masks() {
        let mut map = VideoFourCcInfoMap::default();
        map.entries[0].cc[..4].copy_from_slice(b"vp09");
        map.masks[0] = FOUR_CC_INFO_CAN_FORWARD;
        map.count = 1;

        let mut buf = Buffer::new();
        write_video_fourcc_info_map_amf(&mut buf, &map).unwrap();
        assert_eq!(buf.peek()[0], Amf0Type::Object as u8);

        let mut parsed = VideoFourCcInfoMap::default();
        read_video_fourcc_info_map_amf(&mut buf, &mut parsed).unwrap();
        assert_eq!(parsed.count, 1);
        assert_eq!(&parsed.entries[0].cc[..4], b"vp09");
        assert_eq!(parsed.masks[0], FOUR_CC_INFO_CAN_FORWARD);
    }

    #[test]
    fn four_cc_list_preserves_wildcard() {
        let mut buf = Buffer::new();
        buf.write(&[Amf0Type::StrictArray as u8]).unwrap();
        buf.write(&1u32.to_be_bytes()).unwrap();
        amf0::write_string(&mut buf, "*").unwrap();

        let mut parsed = FourCcList::default();
        read_four_cc_list_amf(&mut buf, &mut parsed).unwrap();
        assert_eq!(parsed.count, 1);
        assert_eq!(parsed.entries[0].cc[0], b'*');
    }

    #[test]
    fn string_fourcc_value_is_read_after_consumed_marker() {
        let mut buf = Buffer::new();
        amf0::write_string(&mut buf, "av01").unwrap();
        assert_eq!(read_fourcc_number(&mut buf).unwrap(), i32::from_be_bytes(*b"av01"));
    }
}
'''
)

write(
    "src/ertmp/multitrack_media.rs",
    r'''//! E-RTMP v2 multitrack audio/video message parsing for the session hot path.
//!
//! Multitrack containers are relayed opaque; this module extracts per-track
//! slices and codec metadata for callbacks, authorization, and init caching.

use crate::types::FrameType;

pub const ERTMP_AUDIO_PACKET_TYPE_MULTITRACK: u8 = 5;
pub const ERTMP_VIDEO_PACKET_TYPE_MULTITRACK: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AvMultitrackType {
    OneTrack = 0,
    ManyTracks = 1,
    ManyTracksManyCodecs = 2,
}

#[derive(Debug, Clone, Copy)]
pub struct MediaTrackSlice<'a> {
    pub track_id: u8,
    pub packet_type: u8,
    pub fourcc: [u8; 4],
    pub video_frame_type: u8,
    pub payload: &'a [u8],
}

fn read_u24(data: &[u8]) -> usize {
    ((data[0] as usize) << 16) | ((data[1] as usize) << 8) | data[2] as usize
}

pub fn is_multitrack_container(frame_type: FrameType, payload: &[u8]) -> bool {
    if payload.is_empty() || payload[0] & 0x80 == 0 {
        return false;
    }
    let expected = match frame_type {
        FrameType::Video => ERTMP_VIDEO_PACKET_TYPE_MULTITRACK,
        FrameType::Audio => ERTMP_AUDIO_PACKET_TYPE_MULTITRACK,
        _ => return false,
    };
    (payload[0] & 0x0F) == expected
}

pub fn foreach_track(
    frame_type: FrameType,
    payload: &[u8],
    mut visit: impl FnMut(&MediaTrackSlice<'_>),
) -> bool {
    if !is_multitrack_container(frame_type, payload) || payload.len() < 3 {
        return false;
    }

    let multitrack_type = (payload[1] >> 4) & 0x0F;
    let inner_packet_type = payload[1] & 0x0F;
    if multitrack_type > AvMultitrackType::ManyTracksManyCodecs as u8 {
        return false;
    }

    let video_frame_type = if frame_type == FrameType::Video {
        (payload[0] >> 4) & 0x07
    } else {
        0
    };
    let mut pos = 2usize;
    let mut shared_fourcc = [0u8; 4];

    if multitrack_type != AvMultitrackType::ManyTracksManyCodecs as u8 {
        if pos + 4 > payload.len() {
            return false;
        }
        shared_fourcc.copy_from_slice(&payload[pos..pos + 4]);
        pos += 4;
    }

    let mut tracks = Vec::new();
    loop {
        let fourcc = if multitrack_type == AvMultitrackType::ManyTracksManyCodecs as u8 {
            if pos + 4 > payload.len() {
                return false;
            }
            let mut cc = [0u8; 4];
            cc.copy_from_slice(&payload[pos..pos + 4]);
            pos += 4;
            cc
        } else {
            shared_fourcc
        };

        if pos >= payload.len() {
            return false;
        }
        let track_id = payload[pos];
        pos += 1;

        let track_size = if multitrack_type != AvMultitrackType::OneTrack as u8 {
            if pos + 3 > payload.len() {
                return false;
            }
            let size = read_u24(&payload[pos..pos + 3]);
            pos += 3;
            size
        } else {
            payload.len() - pos
        };

        if pos + track_size > payload.len() {
            return false;
        }
        tracks.push(MediaTrackSlice {
            track_id,
            packet_type: inner_packet_type,
            fourcc,
            video_frame_type,
            payload: &payload[pos..pos + track_size],
        });
        pos += track_size;

        if multitrack_type == AvMultitrackType::OneTrack as u8 {
            break;
        }
        if pos == payload.len() {
            break;
        }
    }

    if pos != payload.len() || tracks.is_empty() {
        return false;
    }
    for track in &tracks {
        visit(track);
    }
    true
}

pub fn first_track_fourcc(frame_type: FrameType, payload: &[u8]) -> Option<[u8; 4]> {
    let mut result = None;
    if foreach_track(frame_type, payload, |track| {
        if result.is_none() {
            result = Some(track.fourcc);
        }
    }) {
        result
    } else {
        None
    }
}

pub fn multitrack_has_sequence_start(frame_type: FrameType, payload: &[u8]) -> bool {
    let mut found = false;
    let valid = foreach_track(frame_type, payload, |track| {
        found |= track.packet_type == 0;
    });
    valid && found
}

pub fn multitrack_has_keyframe(payload: &[u8]) -> bool {
    if !is_multitrack_container(FrameType::Video, payload) {
        return false;
    }
    let packet_type = payload.get(1).map(|b| b & 0x0F);
    let coded = matches!(packet_type, Some(1 | 3));
    coded && ((payload[0] >> 4) & 0x07) == 1 && foreach_track(FrameType::Video, payload, |_| {})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_many_tracks_video_message() -> Vec<u8> {
        vec![
            0x86, 0x10, b'a', b'v', b'c', b'1',
            0x00, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC,
            0x01, 0x00, 0x00, 0x02, 0xDD, 0xEE,
        ]
    }

    #[test]
    fn detects_multitrack_container() {
        let payload = build_many_tracks_video_message();
        assert!(is_multitrack_container(FrameType::Video, &payload));
    }

    #[test]
    fn iterates_subtracks_with_shared_codec() {
        let payload = build_many_tracks_video_message();
        let mut seen = Vec::new();
        assert!(foreach_track(FrameType::Video, &payload, |track| {
            seen.push((track.track_id, track.fourcc, track.payload.to_vec()));
        }));
        assert_eq!(seen[0], (0, *b"avc1", vec![0xAA, 0xBB, 0xCC]));
        assert_eq!(seen[1], (1, *b"avc1", vec![0xDD, 0xEE]));
    }

    #[test]
    fn iterates_many_tracks_many_codecs() {
        let payload = vec![
            0x86, 0x20,
            b'a', b'v', b'c', b'1', 0, 0, 0, 1, 0xAA,
            b'h', b'v', b'c', b'1', 1, 0, 0, 0, 1, 0xBB,
        ];
        let mut codecs = Vec::new();
        assert!(foreach_track(FrameType::Video, &payload, |track| {
            codecs.push(track.fourcc);
        }));
        assert_eq!(codecs, vec![*b"avc1", *b"hvc1"]);
    }

    #[test]
    fn malformed_message_delivers_no_partial_tracks() {
        let mut payload = build_many_tracks_video_message();
        payload.pop();
        let mut calls = 0;
        assert!(!foreach_track(FrameType::Video, &payload, |_| calls += 1));
        assert_eq!(calls, 0);
    }

    #[test]
    fn coded_frames_x_keyframe_is_detected() {
        let payload = vec![
            0x96, 0x13, b'a', b'v', b'c', b'1',
            0, 0, 0, 1, 0xAA,
        ];
        assert!(multitrack_has_keyframe(&payload));
    }
}
'''
)

write(
    "src/media/modex.rs",
    r'''//! Helpers for parsing E-RTMP v2 ModEx wrappers without changing relay bytes.

use std::borrow::Cow;

use crate::types::CAPS_EX_MASK_MODEX;

pub fn normalize_modex_payload<'a>(payload: &'a [u8], caps_ex_mask: u32) -> Cow<'a, [u8]> {
    if payload.is_empty()
        || (caps_ex_mask & CAPS_EX_MASK_MODEX) == 0
        || payload[0] & 0x80 == 0
        || payload[0] & 0x0F != 7
    {
        return Cow::Borrowed(payload);
    }

    let mut pos = 1usize;
    let mut reconstructed_header = payload[0];
    loop {
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
        if next_packet_type != 7 {
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
            0x97, 0x02, 0x00, 0x01, 0x02, 0x01,
            b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA,
        ];
        assert_eq!(
            normalize_modex_payload(&payload, CAPS_EX_MASK_MODEX).as_ref(),
            &[0x91, b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA]
        );
    }

    #[test]
    fn legacy_aac_is_unchanged() {
        let payload = [0xAF, 0x00, 0x12, 0x10];
        assert!(matches!(
            normalize_modex_payload(&payload, CAPS_EX_MASK_MODEX),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn malformed_modex_is_left_opaque() {
        let payload = [0x97, 0x05, 0x00];
        assert_eq!(
            normalize_modex_payload(&payload, CAPS_EX_MASK_MODEX).as_ref(),
            payload
        );
    }
}
'''
)
replace_once(
    "src/media/mod.rs",
    '''pub mod init_cache;

pub use init_cache::*;
''',
    '''pub mod init_cache;
pub mod modex;

pub use init_cache::*;
pub use modex::*;
'''
)
replace_once(
    "src/media/init_cache.rs",
    '''        return match hdr.packet_type {
            0 => CacheFrameKind::VideoSequenceHeader,
            1 if hdr.frame_type == 1 => CacheFrameKind::VideoKeyframe,
            _ => CacheFrameKind::LiveOnly,
        };
''',
    '''        return match hdr.packet_type {
            0 => CacheFrameKind::VideoSequenceHeader,
            1 | 3 if hdr.frame_type == 1 => CacheFrameKind::VideoKeyframe,
            _ => CacheFrameKind::LiveOnly,
        };
'''
)
replace_once(
    "src/media/init_cache.rs",
    '''    #[test]
    fn enhanced_av1_keyframe_is_cached() {
        let payload = [0x91, b'a', b'v', b'0', b'1', 0xDE, 0xAD];
        assert_eq!(
            classify_cache_frame(FrameType::Video, &payload),
            CacheFrameKind::VideoKeyframe
        );
    }
''',
    '''    #[test]
    fn enhanced_av1_keyframe_is_cached() {
        let payload = [0x91, b'a', b'v', b'0', b'1', 0xDE, 0xAD];
        assert_eq!(
            classify_cache_frame(FrameType::Video, &payload),
            CacheFrameKind::VideoKeyframe
        );
    }

    #[test]
    fn enhanced_coded_frames_x_keyframe_is_cached() {
        let payload = [0x93, b'a', b'v', b'c', b'1', 0xDE, 0xAD];
        assert_eq!(
            classify_cache_frame(FrameType::Video, &payload),
            CacheFrameKind::VideoKeyframe
        );
    }
'''
)

replace_once("src/session/conn.rs", 'use crate::ertmp::multitrack_media::foreach_track;', 'use crate::ertmp::multitrack_media::{first_track_fourcc, foreach_track};')
replace_once("src/session/conn.rs", 'use crate::media::{is_on_metadata_payload, populate_av_frame};', 'use crate::media::{is_on_metadata_payload, normalize_modex_payload, populate_av_frame};')
replace_once(
    "src/session/conn.rs",
    '''pub struct RelayFrame {
    pub frame_type: FrameType,
    pub timestamp: u32,
    pub payload: Vec<u8>,
    pub app: String,
''',
    '''pub struct RelayFrame {
    pub frame_type: FrameType,
    pub timestamp: u32,
    pub payload: Vec<u8>,
    /// ModEx-normalized bytes used only for codec parsing and cache classification.
    pub cache_payload: Vec<u8>,
    pub app: String,
'''
)
replace_once(
    "src/session/conn.rs",
    '''    pub fn relay_route_key(&self) -> String {
        if !self.relay_key.is_empty() {
            return self.relay_key.clone();
        }
        self.current_stream
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }
''',
    '''    pub fn relay_route_key(&self) -> String {
        if !self.relay_key.is_empty() {
            return self.relay_key.clone();
        }
        self.current_stream
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }

    pub fn accepts_multitrack(&self) -> bool {
        !self.negotiated_caps.has_caps_ex || self.negotiated_caps.multitrack_enabled
    }
'''
)
replace_once("src/session/conn.rs", '            self.queue_relay_frame(FrameType::Script, timestamp, payload)?;', '            self.queue_relay_frame(FrameType::Script, timestamp, payload, payload)?;')
replace_once(
    "src/session/conn.rs",
    '''    fn queue_relay_frame(
        &mut self,
        frame_type: FrameType,
        timestamp: u32,
        payload: &[u8],
    ) -> Result<()> {
''',
    '''    fn queue_relay_frame(
        &mut self,
        frame_type: FrameType,
        timestamp: u32,
        payload: &[u8],
        cache_payload: &[u8],
    ) -> Result<()> {
'''
)
replace_once(
    "src/session/conn.rs",
    '''            timestamp,
            payload: payload.to_vec(),
            app: self.app.clone(),
''',
    '''            timestamp,
            payload: payload.to_vec(),
            cache_payload: cache_payload.to_vec(),
            app: self.app.clone(),
'''
)
replace_once(
    "src/session/conn.rs",
    '''        let parse_payload = strip_leading_modex(payload, self.negotiated_caps.caps_ex_mask);

        match frame_type {
''',
    '''        let normalized_payload =
            normalize_modex_payload(payload, self.negotiated_caps.caps_ex_mask);
        let parse_payload = normalized_payload.as_ref();

        match frame_type {
'''
)
replace_once(
    "src/session/conn.rs",
    '''        if self
            .queue_relay_frame(frame_type, timestamp, payload)
            .is_err()
''',
    '''        if self
            .queue_relay_frame(frame_type, timestamp, payload, parse_payload)
            .is_err()
'''
)
replace_regex_once("src/session/conn.rs", r'''fn looks_like_fourcc\(data: &\[u8\]\) -> bool \{.*?\n\}\n\nfn positive_f64_to_u32''', 'fn positive_f64_to_u32')
replace_once(
    "src/session/conn.rs",
    '''fn detect_video_codec(payload: &[u8]) -> Option<String> {
    let mut hdr = VideoHeader::default();
''',
    '''fn detect_video_codec(payload: &[u8]) -> Option<String> {
    if let Some(cc) = first_track_fourcc(FrameType::Video, payload) {
        return std::str::from_utf8(&cc).ok().map(str::to_owned);
    }
    let mut hdr = VideoHeader::default();
'''
)
replace_once(
    "src/session/conn.rs",
    '''fn detect_audio_codec(payload: &[u8]) -> Option<String> {
    let mut hdr = AudioHeader::default();
''',
    '''fn detect_audio_codec(payload: &[u8]) -> Option<String> {
    if let Some(cc) = first_track_fourcc(FrameType::Audio, payload) {
        return std::str::from_utf8(&cc).ok().map(str::to_owned);
    }
    let mut hdr = AudioHeader::default();
'''
)
replace_once(
    "src/session/conn.rs",
    '''            "receiveAudio" => {
                if let Ok(flag) = command::read_bool_command(&mut buf) {
                    if let Some(ref mut stream) = self.current_stream {
                        stream.receive_audio = flag;
                    }
                }
            }
            "receiveVideo" => {
                if let Ok(flag) = command::read_bool_command(&mut buf) {
                    if let Some(ref mut stream) = self.current_stream {
                        stream.receive_video = flag;
                    }
                }
            }
''',
    '''            "receiveAudio" => {
                if let Ok(flag) = command::read_bool_command(&mut buf) {
                    let was_enabled = self
                        .current_stream
                        .as_ref()
                        .map(|stream| stream.receive_audio)
                        .unwrap_or(true);
                    if let Some(ref mut stream) = self.current_stream {
                        stream.receive_audio = flag;
                    }
                    if flag && !was_enabled {
                        self.needs_init_frames = true;
                    }
                }
            }
            "receiveVideo" => {
                if let Ok(flag) = command::read_bool_command(&mut buf) {
                    let was_enabled = self
                        .current_stream
                        .as_ref()
                        .map(|stream| stream.receive_video)
                        .unwrap_or(true);
                    if let Some(ref mut stream) = self.current_stream {
                        stream.receive_video = flag;
                    }
                    if flag && !was_enabled {
                        self.needs_init_frames = true;
                    }
                }
            }
'''
)
replace_regex_once(
    "src/session/conn.rs",
    r'''    #\[test\]\n    fn strip_leading_modex_preserves_enhanced_video_tag\(\) \{.*?    #\[test\]\n    fn on_frame_cb_scratch_retains_payload_after_delivery''',
    '''    #[test]
    fn modex_is_normalized_for_callbacks_and_cache_but_relayed_opaque() {
        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.negotiated_caps.has_caps_ex = true;
        conn.negotiated_caps.caps_ex_mask = CAPS_EX_MASK_MODEX;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        conn.current_stream.as_mut().unwrap().is_publishing = true;
        conn.on_frame_cb = Some(|_| {});

        let payload = vec![
            0x97, 0x02, 0, 1, 2, 0x01,
            b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA,
        ];
        conn.handle_media_frame(1, FrameType::Video, 0, &payload).unwrap();

        assert_eq!(conn.pending_relay[0].payload, payload);
        assert_eq!(conn.pending_relay[0].cache_payload, vec![0x91, b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA]);
        assert_eq!(conn.frame_cb_scratch, vec![0x91, b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA]);
    }

    #[test]
    fn on_frame_cb_scratch_retains_payload_after_delivery'''
)
replace_once(
    "src/session/conn.rs",
    '''    #[test]
    fn multitrack_on_frame_cb_scratch_retains_last_track_payload() {
''',
    '''    #[test]
    fn multitrack_codec_is_detected_before_authorization() {
        use std::sync::{LazyLock, Mutex};

        static SEEN_CODEC: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));
        fn allow_media(_: u64, _: FrameType, codec: Option<&str>) -> bool {
            *SEEN_CODEC.lock().unwrap() = codec.map(str::to_owned);
            true
        }

        let mut conn = Conn::new();
        conn.relay_enabled = true;
        conn.current_stream = Some(Box::new(Stream::new(1)));
        conn.current_stream.as_mut().unwrap().is_publishing = true;
        conn.on_media_cb = Some(allow_media);
        let payload = vec![0x86, 0x10, b'a', b'v', b'c', b'1', 0, 0, 0, 1, 0xAA];
        conn.handle_media_frame(1, FrameType::Video, 0, &payload).unwrap();
        assert_eq!(SEEN_CODEC.lock().unwrap().as_deref(), Some("avc1"));
    }

    #[test]
    fn multitrack_on_frame_cb_scratch_retains_last_track_payload() {
'''
)
replace_once(
    "src/session/conn.rs",
    '''    #[test]
    fn malformed_pause_command_leaves_stream_unpaused() {
''',
    '''    #[test]
    fn receive_video_reenable_requests_cached_replay() {
        let mut conn = Conn::new();
        conn.current_stream = Some(Box::new(Stream::new(1)));
        {
            let stream = conn.current_stream.as_mut().unwrap();
            stream.is_playing = true;
            stream.receive_video = false;
        }
        conn.needs_init_frames = false;

        let mut buf = Buffer::new();
        crate::amf::amf0::write_string(&mut buf, "receiveVideo").unwrap();
        crate::amf::amf0::write_number(&mut buf, 1.0).unwrap();
        crate::amf::amf0::write_null(&mut buf).unwrap();
        crate::amf::amf0::write_boolean(&mut buf, true).unwrap();
        conn.handle_command(buf.as_slice()).unwrap();

        assert!(conn.current_stream.as_ref().unwrap().receive_video);
        assert!(conn.needs_init_frames);
    }

    #[test]
    fn malformed_pause_command_leaves_stream_unpaused() {
'''
)

replace_once(
    "src/server/mod.rs",
    '''                if receive_video {
                    if let Some(ref hdr) = cache.avc_header.clone() {
                        send_failed |= conn.send_frame(FrameType::Video, 0, hdr).is_err();
                    }
                    for hdr in cache.video_track_headers.values() {
                        send_failed |= conn.send_frame(FrameType::Video, 0, hdr).is_err();
                    }
                }
''',
    '''                if receive_video {
                    if let Some(ref hdr) = cache.avc_header.clone() {
                        if !is_multitrack_container(FrameType::Video, hdr) || conn.accepts_multitrack() {
                            send_failed |= conn.send_frame(FrameType::Video, 0, hdr).is_err();
                        }
                    }
                    for hdr in cache.video_track_headers.values() {
                        if conn.accepts_multitrack() {
                            send_failed |= conn.send_frame(FrameType::Video, 0, hdr).is_err();
                        }
                    }
                }
'''
)
replace_once(
    "src/server/mod.rs",
    '''                if receive_audio && !send_failed {
                    if let Some(ref hdr) = cache.aac_header.clone() {
                        send_failed |= conn.send_frame(FrameType::Audio, 0, hdr).is_err();
                    }
                    for hdr in cache.audio_track_headers.values() {
                        send_failed |= conn.send_frame(FrameType::Audio, 0, hdr).is_err();
                    }
                }
''',
    '''                if receive_audio && !send_failed {
                    if let Some(ref hdr) = cache.aac_header.clone() {
                        if !is_multitrack_container(FrameType::Audio, hdr) || conn.accepts_multitrack() {
                            send_failed |= conn.send_frame(FrameType::Audio, 0, hdr).is_err();
                        }
                    }
                    for hdr in cache.audio_track_headers.values() {
                        if conn.accepts_multitrack() {
                            send_failed |= conn.send_frame(FrameType::Audio, 0, hdr).is_err();
                        }
                    }
                }
'''
)
replace_once(
    "src/server/mod.rs",
    '''                if receive_video && !send_failed {
                    if let Some((ts, ref kf)) = cache.last_keyframe.clone() {
                        send_failed |= conn.send_frame(FrameType::Video, ts, kf).is_err();
                    }
                }
''',
    '''                if receive_video && !send_failed {
                    if let Some((ts, ref kf)) = cache.last_keyframe.clone() {
                        if !is_multitrack_container(FrameType::Video, kf) || conn.accepts_multitrack() {
                            send_failed |= conn.send_frame(FrameType::Video, ts, kf).is_err();
                        }
                    }
                }
'''
)
replace_once(
    "src/server/mod.rs",
    '''                let send_result = match frame.frame_type {
''',
    '''                if matches!(frame.frame_type, FrameType::Audio | FrameType::Video)
                    && is_multitrack_container(frame.frame_type, &frame.payload)
                    && !conn.accepts_multitrack()
                {
                    continue;
                }
                let send_result = match frame.frame_type {
'''
)
replace_once("src/server/mod.rs", '        let cache_kind = classify_cache_frame(frame.frame_type, &frame.payload);', '        let cache_kind = classify_cache_frame(frame.frame_type, &frame.cache_payload);')
text = read("src/server/mod.rs")
text = text.replace('Self::multitrack_sequence_track_ids(FrameType::Video, &frame.payload)', 'Self::multitrack_sequence_track_ids(FrameType::Video, &frame.cache_payload)')
text = text.replace('Self::multitrack_sequence_track_ids(FrameType::Audio, &frame.payload)', 'Self::multitrack_sequence_track_ids(FrameType::Audio, &frame.cache_payload)')
write("src/server/mod.rs", text)
replace_once(
    "src/server/mod.rs",
    '''            if seq_tracks.len() > 1 {
                cache.avc_header = Some(frame.payload.clone());
            } else if let Some(track_id) = seq_tracks.first().copied() {
                cache
                    .video_track_headers
                    .insert(track_id, frame.payload.clone());
            } else {
                cache.avc_header = Some(frame.payload.clone());
            }
''',
    '''            if seq_tracks.len() > 1 {
                cache.video_track_headers.clear();
                cache.avc_header = Some(frame.payload.clone());
            } else if let Some(track_id) = seq_tracks.first().copied() {
                cache.avc_header = None;
                cache.video_track_headers.insert(track_id, frame.payload.clone());
            } else {
                cache.video_track_headers.clear();
                cache.avc_header = Some(frame.payload.clone());
            }
'''
)
replace_once(
    "src/server/mod.rs",
    '''            if seq_tracks.len() > 1 {
                cache.aac_header = Some(frame.payload.clone());
            } else if let Some(track_id) = seq_tracks.first().copied() {
                cache
                    .audio_track_headers
                    .insert(track_id, frame.payload.clone());
            } else {
                cache.aac_header = Some(frame.payload.clone());
            }
''',
    '''            if seq_tracks.len() > 1 {
                cache.audio_track_headers.clear();
                cache.aac_header = Some(frame.payload.clone());
            } else if let Some(track_id) = seq_tracks.first().copied() {
                cache.aac_header = None;
                cache.audio_track_headers.insert(track_id, frame.payload.clone());
            } else {
                cache.audio_track_headers.clear();
                cache.aac_header = Some(frame.payload.clone());
            }
'''
)
replace_once(
    "src/server/mod.rs",
    '''                    if seq_tracks.len() > 1 {
                        cache.avc_header = None;
                    } else if let Some(track_id) = seq_tracks.first().copied() {
                        cache.video_track_headers.remove(&track_id);
                    } else {
                        cache.avc_header = None;
                    }
''',
    '''                    if seq_tracks.len() > 1 {
                        cache.avc_header = None;
                        cache.video_track_headers.clear();
                    } else if let Some(track_id) = seq_tracks.first().copied() {
                        cache.avc_header = None;
                        cache.video_track_headers.remove(&track_id);
                    } else {
                        cache.avc_header = None;
                        cache.video_track_headers.clear();
                    }
'''
)
replace_once(
    "src/server/mod.rs",
    '''                    if seq_tracks.len() > 1 {
                        cache.aac_header = None;
                    } else if let Some(track_id) = seq_tracks.first().copied() {
                        cache.audio_track_headers.remove(&track_id);
                    } else {
                        cache.aac_header = None;
                    }
''',
    '''                    if seq_tracks.len() > 1 {
                        cache.aac_header = None;
                        cache.audio_track_headers.clear();
                    } else if let Some(track_id) = seq_tracks.first().copied() {
                        cache.aac_header = None;
                        cache.audio_track_headers.remove(&track_id);
                    } else {
                        cache.aac_header = None;
                        cache.audio_track_headers.clear();
                    }
'''
)

replace_once(
    "src/message/command.rs",
    '''    if ty == amf0::Amf0Type::Number {
        Ok(Some(read_number_value(buf)? as u32))
''',
    '''    if ty == amf0::Amf0Type::Number {
        Ok(Some(amf0::read_number(buf)? as u32))
'''
)
replace_once(
    "src/message/command.rs",
    '''    #[test]
    fn read_close_stream_accepts_three_argument_form() {
        let mut buf = Buffer::new();
        amf0::write_string(&mut buf, "closeStream").unwrap();
        amf0::write_number(&mut buf, 2.0).unwrap();
        amf0::write_null(&mut buf).unwrap();

        assert_eq!(read_close_stream(&mut buf).unwrap(), None);
    }
}
''',
    '''    #[test]
    fn read_close_stream_accepts_three_argument_form() {
        let mut buf = Buffer::new();
        amf0::write_string(&mut buf, "closeStream").unwrap();
        amf0::write_number(&mut buf, 2.0).unwrap();
        amf0::write_null(&mut buf).unwrap();

        assert_eq!(read_close_stream(&mut buf).unwrap(), None);
    }

    #[test]
    fn read_close_stream_accepts_explicit_stream_id() {
        let mut buf = Buffer::new();
        amf0::write_string(&mut buf, "closeStream").unwrap();
        amf0::write_number(&mut buf, 2.0).unwrap();
        amf0::write_null(&mut buf).unwrap();
        amf0::write_number(&mut buf, 7.0).unwrap();

        assert_eq!(read_close_stream(&mut buf).unwrap(), Some(7));
    }
}
'''
)

print("Applied PR #128 fixes")
