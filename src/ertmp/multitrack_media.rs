//! E-RTMP v2 multitrack audio/video message parsing for the session hot path.
//!
//! Multitrack containers are relayed opaque; this module extracts per-track
//! slices for callbacks and init-cache classification.

use crate::types::{FrameType, VideoHeader};

/// Enhanced audio `AudioPacketType::Multitrack`.
pub const ERTMP_AUDIO_PACKET_TYPE_MULTITRACK: u8 = 5;
/// Enhanced video `VideoPacketType::Multitrack`.
pub const ERTMP_VIDEO_PACKET_TYPE_MULTITRACK: u8 = 6;

/// `AvMultitrackType` from the E-RTMP v2 spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AvMultitrackType {
    OneTrack = 0,
    ManyTracks = 1,
    ManyTracksManyCodecs = 2,
}

/// One logical track inside a multitrack RTMP message.
#[derive(Debug, Clone, Copy)]
pub struct MediaTrackSlice<'a> {
    pub track_id: u8,
    pub packet_type: u8,
    pub payload: &'a [u8],
}

fn read_u24(data: &[u8]) -> u32 {
    ((data[0] as u32) << 16) | ((data[1] as u32) << 8) | (data[2] as u32)
}

/// Returns true when `payload` is an enhanced multitrack container.
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

/// Iterate sub-tracks in a multitrack message. Returns true when at least one
/// track was delivered; false when `payload` is not a multitrack container.
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
    let mut pos = 2usize;
    let mut delivered = false;

    if multitrack_type != AvMultitrackType::ManyTracksManyCodecs as u8 && pos + 4 <= payload.len() {
        pos += 4;
    }

    loop {
        if multitrack_type == AvMultitrackType::ManyTracksManyCodecs as u8 {
            if pos + 4 > payload.len() {
                break;
            }
            pos += 4;
        }

        if pos >= payload.len() {
            break;
        }

        let track_id = payload[pos];
        pos += 1;

        let track_size = if multitrack_type != AvMultitrackType::OneTrack as u8 {
            if pos + 3 > payload.len() {
                break;
            }
            read_u24(&payload[pos..pos + 3]) as usize
        } else {
            payload.len().saturating_sub(pos)
        };
        if multitrack_type != AvMultitrackType::OneTrack as u8 {
            pos += 3;
        }

        if pos + track_size > payload.len() {
            break;
        }

        visit(&MediaTrackSlice {
            track_id,
            packet_type: inner_packet_type,
            payload: &payload[pos..pos + track_size],
        });
        delivered = true;
        pos += track_size;

        if multitrack_type == AvMultitrackType::OneTrack as u8 || pos >= payload.len() {
            break;
        }
    }

    delivered
}

/// True when a multitrack container carries at least one sequence-start track.
pub fn multitrack_has_sequence_start(frame_type: FrameType, payload: &[u8]) -> bool {
    let mut found = false;
    let _ = foreach_track(frame_type, payload, |track| {
        if track.packet_type == 0 {
            found = true;
        }
    });
    found
}

/// True when a multitrack container carries at least one video keyframe track.
pub fn multitrack_has_keyframe(payload: &[u8]) -> bool {
    if !is_multitrack_container(FrameType::Video, payload) {
        return false;
    }
    // Coded multitrack video carries the keyframe flag on the outer ExVideo header.
    if (payload[0] >> 4) & 0x07 == 1 {
        return true;
    }
    let mut found = false;
    let _ = foreach_track(FrameType::Video, payload, |track| {
        if track.packet_type != 1 {
            return;
        }
        if track.payload.is_empty() {
            return;
        }
        if track.payload[0] & 0x80 != 0 {
            let mut hdr = VideoHeader::default();
            if crate::ertmp::exvideo::exvideo_parse(track.payload, &mut hdr).is_ok()
                && hdr.frame_type == 1
            {
                found = true;
            }
        } else if (track.payload[0] >> 4) & 0x0F == 1 {
            found = true;
        }
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_many_tracks_video_message() -> Vec<u8> {
        // Enhanced multitrack header + shared avc1 fourCC + two tracks.
        let mut msg = vec![
            0x86, // ex header, multitrack packet type 6
            0x10, // ManyTracks + inner SequenceStart (0)
            b'a', b'v', b'c', b'1',
            0x00, // track 0
            0x00, 0x00, 0x03, // size
            0xAA, 0xBB, 0xCC,
            0x01, // track 1
            0x00, 0x00, 0x02, // size
            0xDD, 0xEE,
        ];
        msg
    }

    #[test]
    fn multitrack_keyframe_detected_from_outer_header() {
        let payload = vec![
            0x96, 0x11, b'a', b'v', b'c', b'1', 0x00, 0x00, 0x00, 0x02, 0xDE, 0xAD,
        ];
        assert!(multitrack_has_keyframe(&payload));
    }

    #[test]
    fn detects_multitrack_container() {
        let payload = build_many_tracks_video_message();
        assert!(is_multitrack_container(FrameType::Video, &payload));
    }

    #[test]
    fn iterates_subtracks() {
        let payload = build_many_tracks_video_message();
        let mut ids = Vec::new();
        assert!(foreach_track(FrameType::Video, &payload, |track| {
            ids.push(track.track_id);
        }));
        assert_eq!(ids, vec![0, 1]);
    }
}
