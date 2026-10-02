//! Codec-neutral delivery hints for live media.
//!
//! A relay that forwards media to a slow receiver has to decide what it may
//! drop, and it must not need to understand H.264/H.265/AV1 to do so. The
//! hint is computed once, where librtmp2 already classifies media for its
//! init cache ([`classify_cache_frame`]), and travels with the frame.

use super::init_cache::{CacheFrameKind, classify_cache_frame};
use crate::types::FrameType;

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

#[cfg(test)]
mod tests {
    use super::*;

    const AVC_SEQ: [u8; 5] = [0x17, 0x00, 0, 0, 0];
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
            hint(FrameType::Video, &AVC_SEQ, true),
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
    }

    #[test]
    fn enhanced_rtmp_hevc_headers_and_keyframes() {
        // ExVideo header byte: IsExHeader (0x80) | frame type << 4 | packet
        // type (0 = sequence start, 1 = coded frames), then the FourCC.
        let seq = [0x90, b'h', b'v', b'c', b'1', 0xAA];
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
        assert_eq!(hint(FrameType::Audio, &[], true), DeliveryHint::Droppable);
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
