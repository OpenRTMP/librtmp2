//! E-RTMP v1 fourCcList + E-RTMP v2 capability layer
//!
//! Mirrors `src/ertmp/connect_caps.c`.

use crate::types::{CapsExit, ErrorCode, FOUR_CC_INFO_ALL, FourCcList, Result, VideoFourCcInfoMap};

/* ── fourCcList ── */

/// Initialize a FourCC list.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::connect_caps::fourcc_list_init;
/// use librtmp2::types::FourCcList;
///
/// let mut list = FourCcList::default();
/// fourcc_list_init(&mut list);
/// assert_eq!(list.count, 0);
/// ```
pub fn fourcc_list_init(list: &mut FourCcList) {
    list.count = 0;
}

/// Add a FourCC to the list.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::connect_caps::{fourcc_list_add, fourcc_list_init};
/// use librtmp2::types::FourCcList;
///
/// let mut list = FourCcList::default();
/// fourcc_list_init(&mut list);
/// fourcc_list_add(&mut list, b"avc1").unwrap();
/// fourcc_list_add(&mut list, b"hvc1").unwrap();
///
/// assert_eq!(list.count, 2);
/// assert_eq!(&list.entries[0].cc[..4], b"avc1");
/// ```
pub fn fourcc_list_add(list: &mut FourCcList, cc: &[u8]) -> Result<()> {
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

/// Parse a FourCC list from raw data.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::connect_caps::{fourcc_list_init, fourcc_list_parse};
/// use librtmp2::types::FourCcList;
///
/// // A UI32 entry count, then per entry a UI16 length (1 for "*", 4 for a codec)
/// // followed by the bytes.
/// let mut data = Vec::new();
/// data.extend_from_slice(&1u32.to_be_bytes());
/// data.extend_from_slice(&4u16.to_be_bytes());
/// data.extend_from_slice(b"avc1");
///
/// let mut list = FourCcList::default();
/// fourcc_list_init(&mut list);
/// let n = fourcc_list_parse(&mut list, &data).unwrap();
///
/// assert_eq!(n, 1); // entries parsed
/// assert_eq!(&list.entries[0].cc[..4], b"avc1");
/// ```
pub fn fourcc_list_parse(list: &mut FourCcList, data: &[u8]) -> Result<usize> {
    if data.len() < 4 {
        return Err(ErrorCode::Io);
    }
    fourcc_list_init(list);

    let count = ((data[0] as u32) << 24)
        | ((data[1] as u32) << 16)
        | ((data[2] as u32) << 8)
        | (data[3] as u32);
    let count = count.min(crate::types::MAX_FOURCCS as u32);

    let mut offset = 4;
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
}

/// Write a FourCC list to a buffer. Returns bytes written.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::connect_caps::{
///     fourcc_list_add, fourcc_list_init, fourcc_list_parse, fourcc_list_write,
/// };
/// use librtmp2::types::FourCcList;
///
/// let mut list = FourCcList::default();
/// fourcc_list_init(&mut list);
/// fourcc_list_add(&mut list, b"hvc1").unwrap();
///
/// let mut buf = [0u8; 32];
/// let n = fourcc_list_write(&list, &mut buf);
/// assert!(n > 0);
///
/// // Round-trips through the parser.
/// let mut back = FourCcList::default();
/// fourcc_list_init(&mut back);
/// fourcc_list_parse(&mut back, &buf[..n]).unwrap();
/// assert_eq!(back.count, 1);
/// assert_eq!(&back.entries[0].cc[..4], b"hvc1");
/// ```
pub fn fourcc_list_write(list: &FourCcList, buf: &mut [u8]) -> usize {
    let count = list.count.min(list.entries.len());
    let needed = 4 + list
        .entries
        .iter()
        .take(count)
        .map(|entry| if entry.cc[0] == b'*' { 3 } else { 6 })
        .sum::<usize>();
    if buf.len() < needed {
        return 0;
    }

    let cnt = count as u32;
    buf[..4].copy_from_slice(&cnt.to_be_bytes());

    let mut offset = 4;
    for entry in list.entries.iter().take(count) {
        let wildcard = entry.cc[0] == b'*';
        let len = if wildcard { 1 } else { 4 };
        buf[offset..offset + 2].copy_from_slice(&(len as u16).to_be_bytes());
        offset += 2;
        if wildcard {
            buf[offset] = b'*';
        } else {
            buf[offset..offset + 4].copy_from_slice(&entry.cc[..4]);
        }
        offset += len;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FourCcList;

    #[test]
    fn fourcc_list_add_rejects_short_input_instead_of_panicking() {
        let mut list = FourCcList::default();
        assert!(matches!(
            fourcc_list_add(&mut list, &[0x61, 0x76]),
            Err(ErrorCode::Io)
        ));
        assert_eq!(list.count, 0);
    }

    #[test]
    fn fourcc_list_add_accepts_exactly_four_bytes() {
        let mut list = FourCcList::default();
        assert!(fourcc_list_add(&mut list, b"av01").is_ok());
        assert_eq!(list.count, 1);
        assert_eq!(&list.entries[0].cc[..4], b"av01");
    }

    #[test]
    fn oversized_fourcc_count_is_clamped_instead_of_panicking() {
        let mut list = FourCcList::default();
        for entry in &mut list.entries {
            entry.cc[..4].copy_from_slice(b"av01");
        }
        list.count = list.entries.len() + 1;
        let mut wire = [0u8; 256];
        let len = fourcc_list_write(&list, &mut wire);
        assert!(len > 0);
        let mut parsed = FourCcList::default();
        fourcc_list_parse(&mut parsed, &wire[..len]).unwrap();
        assert_eq!(parsed.count, list.entries.len());
    }
}

/* ── E-RTMP v2 capsEx ── */

/// Parse capability negotiation data.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::connect_caps::caps_exit_parse;
/// use librtmp2::types::CapsExit;
///
/// // Two UI32 legacy codec bitmasks; the version is implied, not transmitted.
/// let mut data = Vec::new();
/// data.extend_from_slice(&0x80u32.to_be_bytes());  // video bitmask
/// data.extend_from_slice(&0x400u32.to_be_bytes()); // audio bitmask
///
/// let mut caps = CapsExit::default();
/// caps_exit_parse(&mut caps, &data).unwrap();
/// assert_eq!(caps.version, 1); // set by the parser
/// assert_eq!(caps.video_codec_32, 0x80);
/// assert_eq!(caps.audio_codec_32, 0x400);
///
/// // Fewer than 8 bytes is rejected.
/// assert!(caps_exit_parse(&mut caps, &[0u8; 4]).is_err());
/// ```
pub fn caps_exit_parse(caps: &mut CapsExit, data: &[u8]) -> Result<()> {
    if data.len() < 8 {
        return Err(ErrorCode::Io);
    }
    caps.version = 1;
    caps.video_codec_32 =
        ((data[0] as u32) << 24 | (data[1] as u32) << 16 | (data[2] as u32) << 8 | data[3] as u32)
            as i32;
    caps.audio_codec_32 =
        ((data[4] as u32) << 24 | (data[5] as u32) << 16 | (data[6] as u32) << 8 | data[7] as u32)
            as i32;
    Ok(())
}

/// Write capability negotiation data. Returns bytes written.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::connect_caps::{caps_exit_parse, caps_exit_write};
/// use librtmp2::types::CapsExit;
///
/// let caps = CapsExit {
///     version: 2,
///     video_codec_32: 1,
///     audio_codec_32: 2,
/// };
///
/// // The wire form is two UI32 masks: 8 bytes, no version field.
/// let mut buf = [0u8; 8];
/// let n = caps_exit_write(&caps, &mut buf);
/// assert_eq!(n, 8);
///
/// let mut back = CapsExit::default();
/// caps_exit_parse(&mut back, &buf[..n]).unwrap();
/// assert_eq!(back.video_codec_32, 1);
/// assert_eq!(back.audio_codec_32, 2);
/// ```
pub fn caps_exit_write(caps: &CapsExit, buf: &mut [u8]) -> usize {
    if buf.len() < 8 {
        return 0;
    }
    let vc = caps.video_codec_32 as u32;
    let ac = caps.audio_codec_32 as u32;
    buf[0] = (vc >> 24) as u8;
    buf[1] = (vc >> 16) as u8;
    buf[2] = (vc >> 8) as u8;
    buf[3] = vc as u8;
    buf[4] = (ac >> 24) as u8;
    buf[5] = (ac >> 16) as u8;
    buf[6] = (ac >> 8) as u8;
    buf[7] = ac as u8;
    8
}

/* ── E-RTMP v2 videoFourCcInfoMap ── */

/// Parse a video FourCC info map from the legacy binary helper representation.
/// New encodings append a UI32 capability mask to each key; the old key-only
/// representation remains accepted and is interpreted as supporting all flags.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::connect_caps::video_fourcc_info_map_parse;
/// use librtmp2::types::VideoFourCcInfoMap;
///
/// // UI32 entry count, then per entry: UI16 length, the FourCC, and a UI32 mask.
/// let mut data = Vec::new();
/// data.extend_from_slice(&1u32.to_be_bytes());
/// data.extend_from_slice(&4u16.to_be_bytes());
/// data.extend_from_slice(b"avc1");
/// data.extend_from_slice(&0x03u32.to_be_bytes()); // decode | encode
///
/// let mut map = VideoFourCcInfoMap::default();
/// let n = video_fourcc_info_map_parse(&mut map, &data).unwrap();
/// assert_eq!(n, 1);
/// assert_eq!(&map.entries[0].cc[..4], b"avc1");
/// assert_eq!(map.masks[0], 0x03);
/// ```
pub fn video_fourcc_info_map_parse(map: &mut VideoFourCcInfoMap, data: &[u8]) -> Result<usize> {
    if data.len() < 4 {
        return Err(ErrorCode::Io);
    }
    *map = VideoFourCcInfoMap::default();

    let count = u32::from_be_bytes([data[0], data[1], data[2], data[3]])
        .min(crate::types::MAX_FOURCCS as u32) as usize;

    // The two encodings store keys at the same positions and differ only by
    // the trailing UI32 mask per entry. Probe both layouts without erroring:
    // walking a key+mask blob as if it were key-only would otherwise read a
    // mask (e.g. 0x00000003) as the next entry's string length and reject the
    // whole map. Each walk returns the total length it would consume.
    let walk = |entry_extra: usize| -> Option<usize> {
        let mut scan = 4usize;
        for _ in 0..count {
            if scan + 2 > data.len() {
                return None;
            }
            let slen = u16::from_be_bytes([data[scan], data[scan + 1]]) as usize;
            scan += 2;
            if !matches!(slen, 1 | 4) || scan + slen + entry_extra > data.len() {
                return None;
            }
            scan += slen + entry_extra;
        }
        Some(scan)
    };
    // Prefer the new key+mask layout when both happen to match: a genuine
    // legacy blob can never satisfy it (it is 4*count bytes shorter), while a
    // new blob can coincidentally satisfy the legacy walk.
    let legacy_key_only = match (walk(4), walk(0)) {
        (Some(len), _) if len == data.len() => false,
        (_, Some(len)) if len == data.len() => true,
        _ => return Err(ErrorCode::Io),
    };
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
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::connect_caps::{
///     video_fourcc_info_map_parse, video_fourcc_info_map_write,
/// };
/// use librtmp2::types::{FourCc, VideoFourCcInfoMap, FOUR_CC_INFO_ALL};
///
/// let mut map = VideoFourCcInfoMap::default();
/// map.entries[0] = FourCc { cc: *b"avc1 " };
/// map.masks[0] = FOUR_CC_INFO_ALL;
/// map.count = 1;
///
/// let mut buf = [0u8; 64];
/// let n = video_fourcc_info_map_write(&map, &mut buf);
/// assert!(n > 0);
///
/// let mut back = VideoFourCcInfoMap::default();
/// video_fourcc_info_map_parse(&mut back, &buf[..n]).unwrap();
/// assert_eq!(back.count, 1);
/// ```
pub fn video_fourcc_info_map_write(map: &VideoFourCcInfoMap, buf: &mut [u8]) -> usize {
    let count = map.count.min(map.entries.len()).min(map.masks.len());
    let needed = 4 + map
        .entries
        .iter()
        .take(count)
        .map(|entry| if entry.cc[0] == b'*' { 7 } else { 10 })
        .sum::<usize>();
    if buf.len() < needed {
        return 0;
    }

    buf[..4].copy_from_slice(&(count as u32).to_be_bytes());
    let mut offset = 4;
    for (entry, mask) in map.entries.iter().zip(map.masks.iter()).take(count) {
        let wildcard = entry.cc[0] == b'*';
        let len = if wildcard { 1 } else { 4 };
        buf[offset..offset + 2].copy_from_slice(&(len as u16).to_be_bytes());
        offset += 2;
        if wildcard {
            buf[offset] = b'*';
        } else {
            buf[offset..offset + 4].copy_from_slice(&entry.cc[..4]);
        }
        offset += len;
        buf[offset..offset + 4].copy_from_slice(&mask.to_be_bytes());
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
    fn video_info_map_parses_legacy_key_only_with_wildcard() {
        // Legacy key-only encoding (no per-entry UI32 mask) containing a
        // 3-byte wildcard entry: count=2, "*" + "vp09".
        let wire: [u8; 13] = [
            0, 0, 0, 2, // count
            0, 1, b'*', // length 1 wildcard
            0, 4, b'v', b'p', b'0', b'9', // length 4 fourcc
        ];
        let mut parsed = VideoFourCcInfoMap::default();
        video_fourcc_info_map_parse(&mut parsed, &wire).unwrap();
        assert_eq!(parsed.count, 2);
        assert_eq!(parsed.entries[0].cc[0], b'*');
        assert_eq!(&parsed.entries[1].cc[..4], b"vp09");
        assert_eq!(parsed.masks[0], crate::types::FOUR_CC_INFO_ALL);
        assert_eq!(parsed.masks[1], crate::types::FOUR_CC_INFO_ALL);
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

    #[test]
    fn video_info_map_round_trips_multiple_masked_entries() {
        // Regression: the layout probe must not read an entry's UI32 mask as
        // the next entry's string length. A 0x00000003 mask previously made
        // the parser reject any valid new-format map with two or more entries.
        let mut map = VideoFourCcInfoMap::default();
        map.entries[0].cc[..4].copy_from_slice(b"avc1");
        map.masks[0] = 3;
        map.entries[1].cc[..4].copy_from_slice(b"hvc1");
        map.masks[1] = FOUR_CC_INFO_CAN_DECODE;
        map.count = 2;

        let mut wire = [0u8; 64];
        let len = video_fourcc_info_map_write(&map, &mut wire);
        assert!(len > 0);

        let mut parsed = VideoFourCcInfoMap::default();
        video_fourcc_info_map_parse(&mut parsed, &wire[..len]).unwrap();
        assert_eq!(parsed.count, 2);
        assert_eq!(&parsed.entries[0].cc[..4], b"avc1");
        assert_eq!(&parsed.entries[1].cc[..4], b"hvc1");
        assert_eq!(parsed.masks[0], 3);
        assert_eq!(parsed.masks[1], FOUR_CC_INFO_CAN_DECODE);
    }

    #[test]
    fn oversized_video_map_count_is_clamped_instead_of_panicking() {
        let mut map = VideoFourCcInfoMap::default();
        for (entry, mask) in map.entries.iter_mut().zip(map.masks.iter_mut()) {
            entry.cc[..4].copy_from_slice(b"av01");
            *mask = FOUR_CC_INFO_CAN_DECODE;
        }
        map.count = map.entries.len() + 1;
        let mut wire = [0u8; 256];
        let len = video_fourcc_info_map_write(&map, &mut wire);
        assert!(len > 0);
        let mut parsed = VideoFourCcInfoMap::default();
        video_fourcc_info_map_parse(&mut parsed, &wire[..len]).unwrap();
        assert_eq!(parsed.count, map.entries.len());
    }
}
