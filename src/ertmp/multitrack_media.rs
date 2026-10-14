//! E-RTMP v2 multitrack audio/video message parsing for the session hot path.
//!
//! Multitrack containers are relayed opaque; this module extracts per-track
//! slices and codec metadata for callbacks, authorization, and init caching.

use crate::types::FrameType;

pub const ERTMP_AUDIO_PACKET_TYPE_MULTITRACK: u8 = 5;
pub const ERTMP_VIDEO_PACKET_TYPE_MULTITRACK: u8 = 6;
/// Cap sub-tracks unpacked from a single multitrack container (mirrors
/// `message::message::MAX_AGGREGATE_SUBTAGS` and `session::conn::MAX_AGGREGATE_SUBTAGS`).
pub const MAX_MULTITRACK_SUBTRACKS: usize = 4096;

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
        FrameType::Audio => {
            // Genuine E-RTMP enhanced audio uses the reserved ExHeader nibble 9
            // (see `ertmp::exaudio`). Legacy SoundFormat 8/10/11/14 also set
            // bit 7 and can carry a low nibble of 5 (e.g. 0x85 G.711U), which
            // must not be misread as an audio multitrack container.
            if (payload[0] >> 4) & 0x0F != 0x09 {
                return false;
            }
            ERTMP_AUDIO_PACKET_TYPE_MULTITRACK
        }
        _ => return false,
    };
    (payload[0] & 0x0F) == expected
}

/// Read the 4-byte FourCC at `pos`.
fn read_fourcc(payload: &[u8], pos: usize) -> Option<[u8; 4]> {
    payload.get(pos..pos.checked_add(4)?)?.try_into().ok()
}

/// Parse one track header at `pos`: the track id and, for ManyTracks forms,
/// the u24 size. Returns `(track_id, body_start, body_len)` with the body
/// bounds-checked against `payload`.
fn read_track_header(payload: &[u8], pos: usize, one_track: bool) -> Option<(u8, usize, usize)> {
    let track_id = *payload.get(pos)?;
    let mut body = pos + 1;
    let len = if one_track {
        payload.len() - body
    } else {
        let size = read_u24(payload.get(body..body + 3)?);
        body += 3;
        // ManyTracks / ManyTracksManyCodecs allow up to 4096 sub-tracks in a
        // single ~16 KiB message. Zero-length sub-tracks pack the maximum
        // track count into the smallest wire footprint and force per-track
        // authorization and on_frame callbacks with no payload work to
        // amortize the cost -- reject them as malformed.
        if size == 0 {
            return None;
        }
        size
    };
    (body + len <= payload.len()).then_some((track_id, body, len))
}

/// Walk the tracks of a multitrack container without collecting them.
/// `visit` is invoked as each track is parsed, so it can run before a later
/// track turns out to be malformed; callers that need all-or-nothing
/// semantics validate with a first pass (see [`foreach_track`]).
fn walk_tracks(
    frame_type: FrameType,
    payload: &[u8],
    mut visit: impl FnMut(&MediaTrackSlice<'_>),
) -> bool {
    if !is_multitrack_container(frame_type, payload) || payload.len() < 3 {
        return false;
    }
    let multitrack_type = (payload[1] >> 4) & 0x0F;
    if multitrack_type > AvMultitrackType::ManyTracksManyCodecs as u8 {
        return false;
    }
    let many_codecs = multitrack_type == AvMultitrackType::ManyTracksManyCodecs as u8;
    let one_track = multitrack_type == AvMultitrackType::OneTrack as u8;
    let packet_type = payload[1] & 0x0F;
    let video_frame_type = if frame_type == FrameType::Video {
        (payload[0] >> 4) & 0x07
    } else {
        0
    };

    let mut pos = 2usize;
    let mut shared_fourcc = [0u8; 4];
    if !many_codecs {
        let Some(cc) = read_fourcc(payload, pos) else {
            return false;
        };
        shared_fourcc = cc;
        pos += 4;
    }

    let mut track_count = 0usize;
    loop {
        let fourcc = if many_codecs {
            let Some(cc) = read_fourcc(payload, pos) else {
                return false;
            };
            pos += 4;
            cc
        } else {
            shared_fourcc
        };
        let Some((track_id, body, len)) = read_track_header(payload, pos, one_track) else {
            return false;
        };
        if track_count >= MAX_MULTITRACK_SUBTRACKS {
            return false;
        }
        track_count += 1;
        visit(&MediaTrackSlice {
            track_id,
            packet_type,
            fourcc,
            video_frame_type,
            payload: &payload[body..body + len],
        });
        pos = body + len;
        if one_track || pos == payload.len() {
            break;
        }
    }
    pos == payload.len() && track_count > 0
}

/// Visit every track of a valid multitrack container. Returns `false`
/// without visiting anything when the container is malformed. Allocation-free:
/// the container is validated in a first pass, then visited in a second.
pub fn foreach_track(
    frame_type: FrameType,
    payload: &[u8],
    visit: impl FnMut(&MediaTrackSlice<'_>),
) -> bool {
    walk_tracks(frame_type, payload, |_| {}) && walk_tracks(frame_type, payload, visit)
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

/// True when a valid multitrack container carries Enhanced Metadata as the
/// shared inner packet type (`ERTMP_PACKET_TYPE_METADATA` / audio metadata 3).
pub fn multitrack_has_metadata(frame_type: FrameType, payload: &[u8]) -> bool {
    let metadata_ty = match frame_type {
        FrameType::Video => 4, // ERTMP_PACKET_TYPE_METADATA
        FrameType::Audio => 3, // ERTMP_AUDIO_PACKET_TYPE_METADATA
        _ => return false,
    };
    let mut found = false;
    let valid = foreach_track(frame_type, payload, |track| {
        found |= track.packet_type == metadata_ty;
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
            0x86, 0x10, b'a', b'v', b'c', b'1', 0x00, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC, 0x01,
            0x00, 0x00, 0x02, 0xDD, 0xEE,
        ]
    }

    #[test]
    fn detects_multitrack_container() {
        let payload = build_many_tracks_video_message();
        assert!(is_multitrack_container(FrameType::Video, &payload));
    }

    #[test]
    fn legacy_audio_with_multitrack_shaped_nibble_is_not_a_container() {
        // 0x85 is legacy G.711U (SoundFormat 8) with low nibble 5, not an
        // E-RTMP enhanced audio multitrack container.
        let legacy = [
            0x85, 0x10, b'a', b'v', b'c', b'1', 0x00, 0x00, 0x00, 0x01, 0xAA,
        ];
        assert!(!is_multitrack_container(FrameType::Audio, &legacy));
    }

    #[test]
    fn enhanced_audio_multitrack_container_is_detected() {
        // 0x95 = E-RTMP ExHeader nibble 9 + audio multitrack packet type 5.
        let enhanced = [
            0x95, 0x10, b'O', b'p', b'u', b's', 0x00, 0x00, 0x00, 0x01, 0xAA,
        ];
        assert!(is_multitrack_container(FrameType::Audio, &enhanced));
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
            0x86, 0x20, b'a', b'v', b'c', b'1', 0, 0, 0, 1, 0xAA, b'h', b'v', b'c', b'1', 1, 0, 0,
            1, 0xBB,
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
        let payload = vec![0x96, 0x13, b'a', b'v', b'c', b'1', 0, 0, 0, 1, 0xAA];
        assert!(multitrack_has_keyframe(&payload));
    }

    fn build_many_tracks_min_payload_message(track_count: usize) -> Vec<u8> {
        let mut payload = vec![0x86, 0x10, b'a', b'v', b'c', b'1'];
        for id in 0..track_count {
            payload.push(id as u8);
            payload.extend_from_slice(&[0x00, 0x00, 0x01, 0xAA]);
        }
        payload
    }

    fn build_many_tracks_zero_payload_message(track_count: usize) -> Vec<u8> {
        let mut payload = vec![0x86, 0x10, b'a', b'v', b'c', b'1'];
        for id in 0..track_count {
            payload.push(id as u8);
            payload.extend_from_slice(&[0x00, 0x00, 0x00]);
        }
        payload
    }

    #[test]
    fn rejects_multitrack_messages_with_too_many_subtracks() {
        let at_limit = build_many_tracks_min_payload_message(MAX_MULTITRACK_SUBTRACKS);
        let mut calls = 0;
        assert!(foreach_track(FrameType::Video, &at_limit, |_| calls += 1));
        assert_eq!(calls, MAX_MULTITRACK_SUBTRACKS);

        let over_limit = build_many_tracks_min_payload_message(MAX_MULTITRACK_SUBTRACKS + 1);
        let mut over_calls = 0;
        assert!(!foreach_track(FrameType::Video, &over_limit, |_| {
            over_calls += 1
        }));
        assert_eq!(over_calls, 0);
    }

    #[test]
    fn rejects_multitrack_messages_with_zero_size_subtracks() {
        let zero_payload = build_many_tracks_zero_payload_message(2);
        let mut calls = 0;
        assert!(!foreach_track(FrameType::Video, &zero_payload, |_| {
            calls += 1
        }));
        assert_eq!(calls, 0);
    }
}
