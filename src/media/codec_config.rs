//! Structural validation of codec configuration records.
//!
//! The relay forwards media opaquely, so these checks are *not* a codec
//! decoder: they only verify that the length fields of an
//! `AVCDecoderConfigurationRecord` / `HEVCDecoderConfigurationRecord` stay
//! inside the record and that no parameter set is empty. They run once per
//! sequence header (never per coded frame) and allocate nothing. The init
//! cache uses them so that a malformed replacement header cannot overwrite a
//! previously cached valid one and be replayed to late joiners.

/// FourCC / codec families with a structural validator.
///
/// Selects which record layout [`validate_config_record`] applies. Any codec
/// without a dedicated validator maps to [`ConfigCodec::Opaque`] and is only
/// required to be non-empty.
///
/// # Examples
///
/// ```
/// use librtmp2::media::codec_config::ConfigCodec;
///
/// assert_eq!(ConfigCodec::from_fourcc(b"avc1"), ConfigCodec::Avc);
/// assert_eq!(ConfigCodec::from_fourcc(b"hvc1"), ConfigCodec::Hevc);
/// // Anything else only gets the non-empty check.
/// assert_eq!(ConfigCodec::from_fourcc(b"av01"), ConfigCodec::Opaque);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigCodec {
    /// AVC/H.264 — an ISO/IEC 14496-15 `AVCDecoderConfigurationRecord`.
    Avc,
    /// HEVC/H.265 — an ISO/IEC 14496-15 `HEVCDecoderConfigurationRecord`.
    Hevc,
    /// Any other codec: only a non-empty record is required.
    Opaque,
}

impl ConfigCodec {
    /// Map a FourCC (e.g. `b"avc1"`, `b"hvc1"`) to its validator family.
    ///
    /// Unrecognized or too-short values map to [`ConfigCodec::Opaque`].
    ///
    /// ```
    /// use librtmp2::media::codec_config::ConfigCodec;
    ///
    /// assert_eq!(ConfigCodec::from_fourcc(b"avc1"), ConfigCodec::Avc);
    /// assert_eq!(ConfigCodec::from_fourcc(b"hvc1"), ConfigCodec::Hevc);
    /// assert_eq!(ConfigCodec::from_fourcc(b"av01"), ConfigCodec::Opaque);
    /// assert_eq!(ConfigCodec::from_fourcc(b"vp"), ConfigCodec::Opaque);
    /// ```
    pub fn from_fourcc(fourcc: &[u8]) -> Self {
        match fourcc.get(..4) {
            Some(b"avc1") => Self::Avc,
            Some(b"hvc1") => Self::Hevc,
            _ => Self::Opaque,
        }
    }
}

/// Validate a codec configuration record (the bytes after the video tag
/// header, i.e. after the FourCC or the legacy AVCPacketType + CompositionTime).
///
/// # Examples
///
/// ```
/// use librtmp2::media::codec_config::{validate_config_record, ConfigCodec};
///
/// // A valid AVC record: version 1, one SPS, one PPS, all lengths in bounds.
/// let avc = [
///     0x01, 0x64, 0x00, 0x1F, 0xFF, 0xE1,
///     0x00, 0x02, 0x67, 0x64,
///     0x01, 0x00, 0x02, 0x68, 0xEE,
/// ];
/// assert!(validate_config_record(ConfigCodec::Avc, &avc));
///
/// // Truncating the record invalidates it.
/// assert!(!validate_config_record(ConfigCodec::Avc, &avc[..6]));
///
/// // Opaque codecs only require a non-empty record.
/// assert!(validate_config_record(ConfigCodec::Opaque, &[0xAA]));
/// assert!(!validate_config_record(ConfigCodec::Opaque, &[]));
/// ```
pub fn validate_config_record(codec: ConfigCodec, record: &[u8]) -> bool {
    match codec {
        ConfigCodec::Avc => validate_avc_record(record),
        ConfigCodec::Hevc => validate_hevc_record(record),
        ConfigCodec::Opaque => !record.is_empty(),
    }
}

/// Minimal structural check for an ISO/IEC 14496-3 AudioSpecificConfig: the
/// audio object type, sampling-frequency index (or explicit frequency) and
/// channel configuration must all be present. A one-byte `0x12` is rejected.
///
/// # Examples
///
/// ```
/// use librtmp2::media::codec_config::validate_aac_config;
///
/// // AAC-LC, 44.1 kHz, stereo: audio object type, frequency index and channel
/// // configuration are all present.
/// assert!(validate_aac_config(&[0x12, 0x10]));
///
/// // A single byte cannot carry all three fields.
/// assert!(!validate_aac_config(&[0x12]));
/// assert!(!validate_aac_config(&[]));
/// ```
pub fn validate_aac_config(record: &[u8]) -> bool {
    let mut bit_pos = 0usize;
    let mut read_bits = |count: usize| -> Option<u32> {
        if bit_pos + count > record.len() * 8 {
            return None;
        }
        let mut value = 0u32;
        for _ in 0..count {
            let byte = record[bit_pos / 8];
            let bit = (byte >> (7 - (bit_pos % 8))) & 1;
            value = (value << 1) | bit as u32;
            bit_pos += 1;
        }
        Some(value)
    };
    let Some(audio_object_type) = read_bits(5) else {
        return false;
    };
    if audio_object_type == 0 {
        return false;
    }
    if audio_object_type == 31 && read_bits(6).is_none() {
        return false;
    }
    let Some(frequency_index) = read_bits(4) else {
        return false;
    };
    match frequency_index {
        13 | 14 => return false,
        15 => {
            if read_bits(24).is_none() {
                return false;
            }
        }
        _ => {}
    }
    // `channelConfiguration` 0 (explicit program config) is legal but needs
    // more data; require the 4 bits themselves to be present.
    read_bits(4).is_some()
}

/// Read a `u16`-prefixed, non-empty parameter set at `pos`; returns the new
/// position.
fn skip_param_set(record: &[u8], pos: usize) -> Option<usize> {
    let len_end = pos.checked_add(2)?;
    let len = u16::from_be_bytes([*record.get(pos)?, *record.get(pos + 1)?]) as usize;
    if len == 0 {
        return None;
    }
    let end = len_end.checked_add(len)?;
    (end <= record.len()).then_some(end)
}

/// ISO/IEC 14496-15 AVCDecoderConfigurationRecord: version 1, at least one
/// SPS and one PPS, every parameter set non-empty and fully contained.
///
/// # Examples
///
/// ```
/// use librtmp2::media::codec_config::validate_avc_record;
///
/// let valid = [
///     0x01, 0x64, 0x00, 0x1F, 0xFF, 0xE1,
///     0x00, 0x02, 0x67, 0x64,
///     0x01, 0x00, 0x02, 0x68, 0xEE,
/// ];
/// assert!(validate_avc_record(&valid));
///
/// // The version byte must be 1.
/// let mut wrong_version = valid;
/// wrong_version[0] = 2;
/// assert!(!validate_avc_record(&wrong_version));
///
/// // At least one SPS is required.
/// let mut no_sps = valid;
/// no_sps[5] = 0xE0; // num_sps = 0
/// assert!(!validate_avc_record(&no_sps));
/// ```
pub fn validate_avc_record(record: &[u8]) -> bool {
    if record.len() < 7 || record[0] != 1 {
        return false;
    }
    let num_sps = (record[5] & 0x1F) as usize;
    if num_sps == 0 {
        return false;
    }
    let mut pos = 6usize;
    for _ in 0..num_sps {
        match skip_param_set(record, pos) {
            Some(next) => pos = next,
            None => return false,
        }
    }
    let Some(&num_pps) = record.get(pos) else {
        return false;
    };
    if num_pps == 0 {
        return false;
    }
    pos += 1;
    for _ in 0..num_pps {
        match skip_param_set(record, pos) {
            Some(next) => pos = next,
            None => return false,
        }
    }
    validate_avc_extension(&record[pos..])
}

/// High-profile records may append `chroma_format`, the two bit depths and
/// `numOfSequenceParameterSetExt` followed by that many u16-prefixed,
/// non-empty parameter sets. Some encoders omit the section entirely (accepted);
/// if any bytes follow the PPS list they must form a complete, bounded section.
fn validate_avc_extension(ext: &[u8]) -> bool {
    if ext.is_empty() {
        return true;
    }
    let Some(&num_sps_ext) = ext.get(3) else {
        return false;
    };
    let mut pos = 4usize;
    for _ in 0..num_sps_ext {
        match skip_param_set(ext, pos) {
            Some(next) => pos = next,
            None => return false,
        }
    }
    true
}

const HEVC_NAL_SPS: u8 = 33;
const HEVC_NAL_PPS: u8 = 34;

/// ISO/IEC 14496-15 HEVCDecoderConfigurationRecord: version 1, every NAL unit
/// array fully contained with non-empty entries, and at least one SPS and one
/// PPS overall (a missing VPS is tolerated, as some encoders omit it).
///
/// # Examples
///
/// ```
/// use librtmp2::media::codec_config::validate_hevc_record;
///
/// // An empty or version-0 record is rejected.
/// assert!(!validate_hevc_record(&[]));
/// assert!(!validate_hevc_record(&[0x00; 23]));
///
/// // A truncated header is not enough to contain a NAL unit array.
/// assert!(!validate_hevc_record(&[0x01; 8]));
/// ```
pub fn validate_hevc_record(record: &[u8]) -> bool {
    const FIXED_HEADER: usize = 22;
    if record.len() <= FIXED_HEADER || record[0] != 1 {
        return false;
    }
    let num_arrays = record[FIXED_HEADER] as usize;
    let mut pos = FIXED_HEADER + 1;
    let (mut sps, mut pps) = (0usize, 0usize);
    for _ in 0..num_arrays {
        let (Some(&hdr), Some(&hi), Some(&lo)) =
            (record.get(pos), record.get(pos + 1), record.get(pos + 2))
        else {
            return false;
        };
        let nal_type = hdr & 0x3F;
        let num_nalus = u16::from_be_bytes([hi, lo]) as usize;
        pos += 3;
        for _ in 0..num_nalus {
            match skip_param_set(record, pos) {
                Some(next) => pos = next,
                None => return false,
            }
            match nal_type {
                HEVC_NAL_SPS => sps += 1,
                HEVC_NAL_PPS => pps += 1,
                _ => {}
            }
        }
    }
    sps > 0 && pps > 0
}

#[cfg(test)]
pub(crate) mod test_vectors {
    /// Minimal valid AVC record: 1 SPS (4 bytes), 1 PPS (2 bytes).
    pub fn avc_record(sps: &[&[u8]], pps: &[&[u8]]) -> Vec<u8> {
        let mut r = vec![1, 0x64, 0x00, 0x1F, 0xFF, 0xE0 | sps.len() as u8];
        for s in sps {
            r.extend_from_slice(&(s.len() as u16).to_be_bytes());
            r.extend_from_slice(s);
        }
        r.push(pps.len() as u8);
        for p in pps {
            r.extend_from_slice(&(p.len() as u16).to_be_bytes());
            r.extend_from_slice(p);
        }
        r
    }

    /// HEVC record from `(nal_type, entries)` arrays.
    pub fn hevc_record(arrays: &[(u8, &[&[u8]])]) -> Vec<u8> {
        let mut r = vec![0u8; 22];
        r[0] = 1;
        r[21] = 0x03;
        r.push(arrays.len() as u8);
        for (ty, entries) in arrays {
            r.push(0x80 | ty);
            r.extend_from_slice(&(entries.len() as u16).to_be_bytes());
            for e in *entries {
                r.extend_from_slice(&(e.len() as u16).to_be_bytes());
                r.extend_from_slice(e);
            }
        }
        r
    }

    /// Legacy FLV AVC sequence-header tag (`0x17 0x00 CT=0` + valid record),
    /// made unique by `tag`.
    pub fn avc_seq(tag: u8) -> Vec<u8> {
        let mut v = vec![0x17, 0x00, 0, 0, 0];
        v.extend(avc_record(&[&[0x67, tag]], &[&[0x68, tag]]));
        v
    }

    /// Like [`avc_seq`], padded with SPS bytes up to exactly `total` bytes
    /// (`total` must be at least the unpadded size and at most 65 553).
    pub fn avc_seq_len(tag: u8, total: usize) -> Vec<u8> {
        let overhead = 5 + 6 + 2 + 1 + 2 + 1;
        let sps = vec![tag; total - overhead];
        let mut v = vec![0x17, 0x00, 0, 0, 0];
        v.extend(avc_record(&[&sps], &[&[0x68]]));
        assert_eq!(v.len(), total);
        v
    }

    /// Enhanced RTMP `hvc1` SequenceStart tag with a valid record.
    pub fn hvc_seq(tag: u8) -> Vec<u8> {
        let mut v = vec![0x90, b'h', b'v', b'c', b'1'];
        v.extend(hevc_record(&[
            (32, &[&[0x40, tag]]),
            (33, &[&[0x42, tag]]),
            (34, &[&[0x44, tag]]),
        ]));
        v
    }

    /// Multitrack (`ManyTracks`, packet type SequenceStart) `avc1` video
    /// container; each entry is `(track_id, config_record)`.
    pub fn mt_avc_seq(tracks: &[(u8, &[u8])]) -> Vec<u8> {
        let mut v = vec![0x86, 0x10, b'a', b'v', b'c', b'1'];
        for (id, rec) in tracks {
            v.push(*id);
            v.extend_from_slice(&(rec.len() as u32).to_be_bytes()[1..]);
            v.extend_from_slice(rec);
        }
        v
    }

    pub fn valid_avc() -> Vec<u8> {
        avc_record(&[&[0x67, 0x64, 0x00, 0x1F]], &[&[0x68, 0xEE]])
    }

    pub fn valid_hevc() -> Vec<u8> {
        hevc_record(&[
            (32, &[&[0x40, 0x01]]),
            (33, &[&[0x42, 0x01, 0x01]]),
            (34, &[&[0x44, 0x01]]),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::test_vectors::*;
    use super::*;

    #[test]
    fn avc_accepts_valid_and_multiple_parameter_sets() {
        assert!(validate_avc_record(&valid_avc()));
        let multi = avc_record(&[&[0x67, 1], &[0x67, 2]], &[&[0x68, 1], &[0x68, 2]]);
        assert!(validate_avc_record(&multi));
        // High-profile extension bytes after the last PPS are tolerated.
        let mut ext = valid_avc();
        ext.extend_from_slice(&[0xFD, 0xF8, 0xF8, 0x00]);
        assert!(validate_avc_record(&ext));
    }

    #[test]
    fn avc_validates_the_high_profile_extension_section() {
        let base = valid_avc();
        let with = |tail: &[u8]| [&base[..], tail].concat();
        // Complete section with zero and with two SPS extensions.
        assert!(validate_avc_record(&with(&[0xFD, 0xF8, 0xF8, 0])));
        assert!(validate_avc_record(&with(&[
            0xFD, 0xF8, 0xF8, 2, 0, 2, 0x6D, 1, 0, 1, 0x6D
        ])));
        // Missing/truncated fields, extension count beyond the record, an
        // empty extension SPS and a cut-off entry are all rejected.
        assert!(!validate_avc_record(&with(&[0xFD])));
        assert!(!validate_avc_record(&with(&[0xFD, 0xF8, 0xF8])));
        assert!(!validate_avc_record(&with(&[
            0xFD, 0xF8, 0xF8, 3, 0, 1, 0x6D
        ])));
        assert!(!validate_avc_record(&with(&[0xFD, 0xF8, 0xF8, 1, 0, 0])));
        assert!(!validate_avc_record(&with(&[
            0xFD, 0xF8, 0xF8, 1, 0, 5, 0x6D
        ])));
        assert!(!validate_avc_record(&with(&[0xFD, 0xF8, 0xF8, 0xFF])));
    }

    #[test]
    fn avc_rejects_empty_and_truncated_records() {
        let good = valid_avc();
        assert!(!validate_avc_record(&[]));
        for n in 0..good.len() {
            assert!(!validate_avc_record(&good[..n]), "prefix {n} accepted");
        }
    }

    #[test]
    fn avc_rejects_zero_length_parameter_sets() {
        assert!(!validate_avc_record(&avc_record(&[&[]], &[&[0x68]])));
        assert!(!validate_avc_record(&avc_record(&[&[0x67]], &[&[]])));
    }

    #[test]
    fn avc_rejects_missing_sps_or_pps() {
        assert!(!validate_avc_record(&avc_record(&[], &[&[0x68]])));
        assert!(!validate_avc_record(&avc_record(&[&[0x67]], &[])));
    }

    #[test]
    fn avc_rejects_lengths_and_counts_beyond_record() {
        let mut r = valid_avc();
        r[6] = 0xFF; // SPS length high byte -> 0xFF04
        assert!(!validate_avc_record(&r));
        let mut r = valid_avc();
        r[5] = 0xE5; // claims 5 SPS, only 1 present
        assert!(!validate_avc_record(&r));
        let mut r = valid_avc();
        let pps_count = 6 + 2 + 4;
        r[pps_count] = 0xFF; // claims 255 PPS
        assert!(!validate_avc_record(&r));
    }

    #[test]
    fn avc_rejects_wrong_version() {
        let mut r = valid_avc();
        r[0] = 0;
        assert!(!validate_avc_record(&r));
        r[0] = 2;
        assert!(!validate_avc_record(&r));
    }

    #[test]
    fn hevc_accepts_valid_with_and_without_vps() {
        assert!(validate_hevc_record(&valid_hevc()));
        let no_vps = hevc_record(&[(33, &[&[0x42]]), (34, &[&[0x44]])]);
        assert!(validate_hevc_record(&no_vps));
    }

    #[test]
    fn hevc_rejects_empty_and_every_truncated_prefix() {
        let good = valid_hevc();
        assert!(!validate_hevc_record(&[]));
        for n in 0..good.len() {
            assert!(!validate_hevc_record(&good[..n]), "prefix {n} accepted");
        }
    }

    #[test]
    fn hevc_rejects_zero_length_vps_sps_pps() {
        let zero_vps = hevc_record(&[(32, &[&[]]), (33, &[&[0x42]]), (34, &[&[0x44]])]);
        let zero_sps = hevc_record(&[(32, &[&[0x40]]), (33, &[&[]]), (34, &[&[0x44]])]);
        let zero_pps = hevc_record(&[(32, &[&[0x40]]), (33, &[&[0x42]]), (34, &[&[]])]);
        assert!(!validate_hevc_record(&zero_vps));
        assert!(!validate_hevc_record(&zero_sps));
        assert!(!validate_hevc_record(&zero_pps));
    }

    #[test]
    fn hevc_rejects_missing_sps_pps_and_bad_counts() {
        assert!(!validate_hevc_record(&hevc_record(&[])));
        assert!(!validate_hevc_record(&hevc_record(&[(32, &[&[0x40]])])));
        assert!(!validate_hevc_record(&hevc_record(&[
            (33, &[&[0x42]]),
            (34, &[]),
        ])));
        let mut r = valid_hevc();
        r[22] = 0xFF; // claims 255 arrays
        assert!(!validate_hevc_record(&r));
        let mut r = valid_hevc();
        r[24] = 0xFF; // first array claims 255+ NAL units
        assert!(!validate_hevc_record(&r));
        let mut r = valid_hevc();
        r[0] = 0;
        assert!(!validate_hevc_record(&r));
    }

    #[test]
    fn extreme_declared_lengths_do_not_panic_or_overflow() {
        let mut r = valid_hevc();
        for b in r.iter_mut().skip(23) {
            *b = 0xFF;
        }
        assert!(!validate_hevc_record(&r));
        let r = vec![1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(!validate_avc_record(&r));
    }

    #[test]
    fn opaque_codecs_require_non_empty_record() {
        assert!(validate_config_record(ConfigCodec::Opaque, &[1]));
        assert!(!validate_config_record(ConfigCodec::Opaque, &[]));
        assert_eq!(ConfigCodec::from_fourcc(b"av01"), ConfigCodec::Opaque);
        assert_eq!(ConfigCodec::from_fourcc(b"hvc1"), ConfigCodec::Hevc);
        assert_eq!(ConfigCodec::from_fourcc(b"avc1"), ConfigCodec::Avc);
        assert_eq!(ConfigCodec::from_fourcc(b"av"), ConfigCodec::Opaque);
    }
}
