//! Codec-neutral delivery hints for live media.
//!
//! A relay that forwards media to a slow receiver has to decide what it may
//! drop, and it must not need to understand H.264/H.265/AV1 to do so. The
//! hint is computed once, where librtmp2 already classifies media for its
//! init cache ([`classify_cache_frame`]), and travels with the frame.

use super::init_cache::{CacheFrameKind, classify_cache_frame};
use crate::ertmp::multitrack_media::{
    foreach_track, is_multitrack_container, multitrack_has_metadata,
};
use crate::ertmp::{exaudio, exvideo};
use crate::media::modex::ERTMP_PACKET_TYPE_MODEX;
use crate::types::{
    AudioHeader, ERTMP_AUDIO_PACKET_TYPE_METADATA, ERTMP_AUDIO_PACKET_TYPE_MULTICHANNEL,
    ERTMP_PACKET_TYPE_METADATA, FrameType, VideoHeader,
};

/// How a live media frame may be treated under congestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeliveryHint {
    /// Codec headers (sequence headers, init data) and script/metadata:
    /// everything after depends on them, so a congested receiver still
    /// gets them.
    Critical,
    /// A point a receiver can resume from after skipped frames without
    /// decode errors: a video keyframe / random access point, or any audio
    /// frame on a route without video.
    ResyncPoint,
    /// Everything else (dependent video frames, audio next to video):
    /// skipped while the receiver is behind.
    Droppable,
}

impl DeliveryHint {
    /// Classify one live frame. `payload` is the codec-parsing view of the
    /// frame (ModEx-normalized, see `RelayFrame::cache_payload`);
    /// `route_has_video` says whether the frame's stream carries video, so
    /// audio on an audio-only stream can serve as the resync point instead
    /// of waiting for a keyframe that never comes.
    pub fn classify(frame_type: FrameType, payload: &[u8], route_has_video: bool) -> Self {
        match frame_type {
            FrameType::Script | FrameType::Metadata => return Self::Critical,
            FrameType::Audio | FrameType::Video => {}
        }
        // Enhanced RTMP Metadata (colorInfo, …) rides as FrameType::Video with
        // ExHeader packet_type 4 — still Critical under congestion, matching
        // script onMetaData. Keep classify_cache_frame's LiveOnly so init-cache
        // does not treat metadata as a sequence header.
        if frame_type == FrameType::Video && is_enhanced_video_metadata(payload) {
            return Self::Critical;
        }
        // Enhanced RTMP Audio Metadata (ExHeader packet type 3) is Critical
        // for the same reason: nothing replays it later, so a congested
        // receiver would lose it permanently. VideoCommand (StartSeek /
        // EndSeek, enhanced or legacy frame type 5) and MultichannelConfig are
        // equally one-shot decoder state: dropping them leaves a receiver
        // stuck in seek or on the wrong channel map.
        if frame_type == FrameType::Audio && is_enhanced_audio_metadata(payload) {
            return Self::Critical;
        }
        if frame_type == FrameType::Video && is_video_command(payload) {
            return Self::Critical;
        }
        if frame_type == FrameType::Audio && is_enhanced_audio_multichannel(payload) {
            return Self::Critical;
        }
        // The RTMP "audio silence" message is a zero-length audio payload.
        // Dropping it under congestion loses the flush/clock transition.
        if frame_type == FrameType::Audio && payload.is_empty() {
            return Self::Critical;
        }
        match classify_cache_frame(frame_type, payload) {
            CacheFrameKind::VideoSequenceHeader | CacheFrameKind::AudioSequenceHeader => {
                Self::Critical
            }
            CacheFrameKind::VideoKeyframe => Self::ResyncPoint,
            CacheFrameKind::LiveOnly if frame_type == FrameType::Audio && !route_has_video => {
                Self::ResyncPoint
            }
            CacheFrameKind::LiveOnly => Self::Droppable,
        }
    }

    /// Stable one-byte wire code (cluster media protocol).
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::ResyncPoint => 1,
            Self::Droppable => 2,
        }
    }

    /// Inverse of [`Self::to_u8`]; `None` for an unknown code.
    pub fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Critical),
            1 => Some(Self::ResyncPoint),
            2 => Some(Self::Droppable),
            _ => None,
        }
    }
}

fn is_enhanced_video_metadata(payload: &[u8]) -> bool {
    // Multitrack outer packet type is 6; metadata lives in the shared inner
    // track descriptor — check that before a single-track ExHeader parse.
    if is_multitrack_container(FrameType::Video, payload) {
        return multitrack_has_metadata(FrameType::Video, payload);
    }
    // Legacy AVC/AAC tags are not ExHeader; skip parse so classify_cache_frame
    // does not re-parse the same ordinary payload on the common path.
    let Some(&b0) = payload.first() else {
        return false;
    };
    if b0 & 0x80 == 0 {
        return false;
    }
    let mut hdr = VideoHeader::default();
    if exvideo::exvideo_parse(payload, &mut hdr).is_err() {
        return false;
    }
    hdr.is_ex_header != 0 && hdr.packet_type == ERTMP_PACKET_TYPE_METADATA
}

fn is_enhanced_audio_metadata(payload: &[u8]) -> bool {
    // Multitrack outer packet type is 5; metadata lives in the shared inner
    // track descriptor — check that before a single-track ExHeader parse.
    if is_multitrack_container(FrameType::Audio, payload) {
        return multitrack_has_metadata(FrameType::Audio, payload);
    }
    // Legacy audio tags are not ExHeader; skip parse so classify_cache_frame
    // does not re-parse the same ordinary payload on the common path.
    let Some(&b0) = payload.first() else {
        return false;
    };
    if (b0 >> 4) & 0x0F != 0x09 {
        return false;
    }
    let mut hdr = AudioHeader::default();
    if exaudio::exaudio_parse(payload, &mut hdr).is_err() {
        return false;
    }
    hdr.is_ex_header != 0 && hdr.packet_type == ERTMP_AUDIO_PACKET_TYPE_METADATA
}

fn is_enhanced_audio_multichannel(payload: &[u8]) -> bool {
    // Multitrack containers carry the inner packet type in their track
    // descriptors; the outer ExHeader only says "multitrack".
    if is_multitrack_container(FrameType::Audio, payload) {
        let mut found = false;
        let valid = foreach_track(FrameType::Audio, payload, |track| {
            found |= track.packet_type == ERTMP_AUDIO_PACKET_TYPE_MULTICHANNEL;
        });
        return valid && found;
    }
    let Some(&b0) = payload.first() else {
        return false;
    };
    if (b0 >> 4) & 0x0F != 0x09 {
        return false;
    }
    let mut hdr = AudioHeader::default();
    if exaudio::exaudio_parse(payload, &mut hdr).is_err() {
        return false;
    }
    hdr.is_ex_header != 0 && hdr.packet_type == ERTMP_AUDIO_PACKET_TYPE_MULTICHANNEL
}

fn is_video_command(payload: &[u8]) -> bool {
    // VideoCommand is frame type 5: its low nibble carries the command
    // (StartSeek/EndSeek), not a codec packet type. The enhanced form uses the
    // 3-bit ExHeader frame type; the legacy FLV tag uses the full high nibble.
    payload.first().is_some_and(|b| {
        if b & 0x80 != 0 {
            // An unpeeled ModEx wrapper's frame type describes the inner
            // packet only after the chain is peeled. Classifying the wrapper
            // itself as a one-shot VideoCommand would make every such frame
            // Critical under congestion, defeating the droppable policy.
            let frame_type = (b >> 4) & 0x07;
            let packet_type = b & 0x0F;
            frame_type == 5 && packet_type != ERTMP_PACKET_TYPE_MODEX
        } else {
            (b >> 4) & 0x0F == 5
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::codec_config::test_vectors::*;

    const AVC_KEY: [u8; 5] = [0x17, 0x01, 0, 0, 0];
    const AVC_INTER: [u8; 5] = [0x27, 0x01, 0, 0, 0];
    const AAC_SEQ: [u8; 4] = [0xAF, 0x00, 0x12, 0x10];
    const AAC_RAW: [u8; 3] = [0xAF, 0x01, 0x21];

    fn hint(ty: FrameType, payload: &[u8], video: bool) -> DeliveryHint {
        DeliveryHint::classify(ty, payload, video)
    }

    #[test]
    fn h264_headers_are_critical_keyframes_resync_inter_frames_droppable() {
        assert_eq!(
            hint(FrameType::Video, &avc_seq(1), true),
            DeliveryHint::Critical
        );
        assert_eq!(
            hint(FrameType::Video, &AVC_KEY, true),
            DeliveryHint::ResyncPoint
        );
        assert_eq!(
            hint(FrameType::Video, &AVC_INTER, true),
            DeliveryHint::Droppable
        );
    }

    #[test]
    fn audio_next_to_video_is_droppable_but_audio_only_resyncs_on_audio() {
        assert_eq!(
            hint(FrameType::Audio, &AAC_SEQ, true),
            DeliveryHint::Critical
        );
        assert_eq!(
            hint(FrameType::Audio, &AAC_RAW, true),
            DeliveryHint::Droppable
        );
        assert_eq!(
            hint(FrameType::Audio, &AAC_SEQ, false),
            DeliveryHint::Critical
        );
        assert_eq!(
            hint(FrameType::Audio, &AAC_RAW, false),
            DeliveryHint::ResyncPoint
        );
    }

    #[test]
    fn script_and_metadata_are_critical() {
        assert_eq!(
            hint(FrameType::Script, b"onMetaData", true),
            DeliveryHint::Critical
        );
        assert_eq!(
            hint(FrameType::Metadata, &[], false),
            DeliveryHint::Critical
        );
        // ExHeader | frame_type=1 | packet_type=4 (Metadata) + FourCC hvc1
        let enhanced_meta = [0x94, b'h', b'v', b'c', b'1'];
        assert_eq!(
            hint(FrameType::Video, &enhanced_meta, true),
            DeliveryHint::Critical
        );
        // Multitrack outer type 6 with shared inner packet_type 4 (Metadata).
        let multitrack_meta = vec![
            0x86, 0x14, b'a', b'v', b'c', b'1', 0x00, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC,
        ];
        assert_eq!(
            hint(FrameType::Video, &multitrack_meta, true),
            DeliveryHint::Critical
        );
    }

    #[test]
    fn enhanced_rtmp_hevc_headers_and_keyframes() {
        // ExVideo header byte: IsExHeader (0x80) | frame type << 4 | packet
        // type (0 = sequence start, 1 = coded frames), then the FourCC.
        let seq = hvc_seq(1);
        let key = [0x91, b'h', b'v', b'c', b'1', 0, 0, 0, 0xAA];
        let inter = [0xA1, b'h', b'v', b'c', b'1', 0, 0, 0, 0xAA];
        assert_eq!(hint(FrameType::Video, &seq, true), DeliveryHint::Critical);
        assert_eq!(
            hint(FrameType::Video, &key, true),
            DeliveryHint::ResyncPoint
        );
        assert_eq!(
            hint(FrameType::Video, &inter, true),
            DeliveryHint::Droppable
        );
    }

    #[test]
    fn truncated_or_garbage_payloads_are_droppable_never_panic() {
        assert_eq!(hint(FrameType::Video, &[], true), DeliveryHint::Droppable);
        assert_eq!(
            hint(FrameType::Video, &[0x17], true),
            DeliveryHint::Droppable
        );
        // A zero-length audio message is the spec-defined silence message and
        // stays Critical; a non-empty truncated audio tag is droppable.
        assert_eq!(hint(FrameType::Audio, &[], true), DeliveryHint::Critical);
        assert_eq!(
            hint(FrameType::Audio, &[0xAF], true),
            DeliveryHint::Droppable
        );
    }

    #[test]
    fn audio_silence_is_critical_on_every_route() {
        assert_eq!(hint(FrameType::Audio, &[], true), DeliveryHint::Critical);
        assert_eq!(hint(FrameType::Audio, &[], false), DeliveryHint::Critical);
    }

    #[test]
    fn legacy_video_command_is_critical_not_droppable() {
        // Legacy frame type 5 (VideoInfo/Command) carries one command byte.
        // Dropping it leaves a receiver stuck in a seek state, so it must be
        // treated like the enhanced command framing.
        assert_eq!(
            hint(FrameType::Video, &[0x56, 0x00], true),
            DeliveryHint::Critical
        );
        assert_eq!(
            hint(FrameType::Video, &[0x57, 0x01], true),
            DeliveryHint::Critical
        );
    }

    #[test]
    fn unpeeled_modex_wrapper_is_not_a_video_command() {
        // 0xD7 = ExHeader | frame type 5 | packet type 7 (ModEx). The frame
        // type belongs to the inner packet only after the chain is peeled; the
        // wrapper itself must not become permanently Critical.
        assert_eq!(
            hint(
                FrameType::Video,
                &[0xD7, 0x02, 0x00, 0x00, 0x07, 0x01, 0x27, 0x01],
                true
            ),
            DeliveryHint::Droppable
        );
    }

    #[test]
    fn wire_codes_round_trip_and_reject_unknown() {
        for h in [
            DeliveryHint::Critical,
            DeliveryHint::ResyncPoint,
            DeliveryHint::Droppable,
        ] {
            assert_eq!(DeliveryHint::from_u8(h.to_u8()), Some(h));
        }
        assert_eq!(DeliveryHint::from_u8(3), None);
        assert_eq!(DeliveryHint::from_u8(255), None);
    }
}
