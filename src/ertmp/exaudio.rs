//! Enhanced RTMP v1 ExAudioTagHeader parsing
//!
//! Mirrors `src/ertmp/exaudio.c`.

use super::fourcc;
use crate::types::{AudioHeader, ErrorCode, Result};

/// Parse an Enhanced RTMP v1 audio tag header.
pub fn exaudio_parse(data: &[u8], hdr: &mut AudioHeader) -> Result<()> {
    *hdr = AudioHeader::default();

    if data.is_empty() {
        return Err(ErrorCode::Io);
    }

    let b0 = data[0];

    // Disambiguate legacy SoundFormat from IsExHeader
    let is_ex =
        (b0 & 0x80 != 0) && data.len() >= 5 && fourcc::fourcc_to_audio_codec(&data[1..5]).is_ok();

    hdr.is_ex_header = if is_ex { 1 } else { 0 };

    if hdr.is_ex_header == 0 {
        // Legacy layout
        hdr.audio_codec = match (b0 >> 4) & 0x0F {
            0 => crate::types::AudioCodec::Pcm,
            1 => crate::types::AudioCodec::Adpcm,
            2 => crate::types::AudioCodec::Mp3,
            3 => crate::types::AudioCodec::PcmLe,
            4 => crate::types::AudioCodec::Nelly16k,
            5 => crate::types::AudioCodec::Nelly8k,
            6 => crate::types::AudioCodec::Nelly,
            7 => crate::types::AudioCodec::G711A,
            8 => crate::types::AudioCodec::G711U,
            10 => crate::types::AudioCodec::Aac,
            11 => crate::types::AudioCodec::Speex,
            14 => crate::types::AudioCodec::Opus,
            _ => crate::types::AudioCodec::Aac,
        };
        hdr.sample_rate = (b0 >> 2) & 0x03;
        hdr.sample_size = (b0 >> 1) & 0x01;
        hdr.channels = b0 & 0x01;
        hdr.header_size = 1;

        if hdr.audio_codec == crate::types::AudioCodec::Aac && data.len() >= 2 {
            hdr.aac_packet_type = data[1];
            hdr.header_size = 2;
        }
        return Ok(());
    }

    // Enhanced layout
    hdr.packet_type = b0 & 0x0F;
    hdr.fourcc[..4].copy_from_slice(&data[1..5]);
    hdr.header_size = 5;
    hdr.audio_codec =
        fourcc::fourcc_to_audio_codec(&data[1..5]).unwrap_or(crate::types::AudioCodec::Aac);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AudioCodec, AudioHeader};

    #[test]
    fn parse_clears_aac_packet_type_on_enhanced_reuse() {
        let legacy_aac = [0xAF, 0x01];
        let enhanced_opus = [0x90, b'O', b'p', b'u', b's'];

        let mut hdr = AudioHeader::default();
        exaudio_parse(&legacy_aac, &mut hdr).unwrap();
        assert_eq!(hdr.is_ex_header, 0);
        assert_eq!(hdr.aac_packet_type, 1);

        exaudio_parse(&enhanced_opus, &mut hdr).unwrap();
        assert_eq!(hdr.is_ex_header, 1);
        assert_eq!(hdr.aac_packet_type, 0);
        assert_eq!(hdr.audio_codec, AudioCodec::Opus);
    }

    #[test]
    fn parse_error_leaves_header_cleared() {
        let legacy_aac = [0xAF, 0x01];
        let mut hdr = AudioHeader::default();
        exaudio_parse(&legacy_aac, &mut hdr).unwrap();

        assert!(exaudio_parse(&[], &mut hdr).is_err());
        assert_eq!(hdr.aac_packet_type, 0);
        assert_eq!(hdr.is_ex_header, 0);
    }
}
