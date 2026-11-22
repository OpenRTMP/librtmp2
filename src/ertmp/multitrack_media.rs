//! E-RTMP v2 multitrack audio/video message parsing for the session hot path.
//!
//! Multitrack containers are relayed opaque; this module extracts per-track
//! slices and codec metadata for callbacks, authorization, and init caching.

use crate::ertmp::exvideo::VIDEO_FRAME_TYPE_COMMAND;
use crate::types::FrameType;

/// E-RTMP v2 `AudioPacketType` value identifying a multitrack audio container.
///
/// Same value as [`crate::types::ERTMP_AUDIO_PACKET_TYPE_MULTITRACK`], repeated
/// here so this module is self-contained.
pub const ERTMP_AUDIO_PACKET_TYPE_MULTITRACK: u8 = 5;
/// E-RTMP v2 `VideoPacketType` value identifying a multitrack video container.
///
/// Same value as [`crate::types::ERTMP_VIDEO_PACKET_TYPE_MULTITRACK`].
pub const ERTMP_VIDEO_PACKET_TYPE_MULTITRACK: u8 = 6;
/// Cap sub-tracks unpacked from a single multitrack container (mirrors
/// `message::message::MAX_AGGREGATE_SUBTAGS` and `session::conn::MAX_AGGREGATE_SUBTAGS`).
pub const MAX_MULTITRACK_SUBTRACKS: usize = 4096;

/// How the sub-tracks inside a multitrack container are laid out.
///
/// Encoded in the high nibble of the container's second byte; see
/// [`crate::ertmp::multitrack`] for the full wire format.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::multitrack_media::AvMultitrackType;
///
/// // Encoded in the high nibble of the container's second byte.
/// assert_eq!(AvMultitrackType::OneTrack as u8, 0);
/// assert_eq!(AvMultitrackType::ManyTracks as u8, 1);
/// assert_eq!(AvMultitrackType::ManyTracksManyCodecs as u8, 2);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AvMultitrackType {
    /// Exactly one track; the container carries no per-track size field.
    OneTrack = 0,
    /// Several tracks, all sharing one FourCC given once after the header.
    ManyTracks = 1,
    /// Several tracks, each preceded by its own FourCC.
    ManyTracksManyCodecs = 2,
}

/// One sub-track extracted from a multitrack container.
///
/// Borrowed from the container payload: [`MediaTrackSlice::payload`] points
/// into the caller's `&[u8]` and is valid for exactly as long as that slice.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::multitrack_media::MediaTrackSlice;
///
/// // A slice borrows from the container payload, so it costs no allocation.
/// let body = [0xAAu8, 0xBB];
/// let track = MediaTrackSlice {
///     track_id: 0,
///     packet_type: 1,
///     fourcc: *b"avc1",
///     video_frame_type: 1,
///     payload: &body,
/// };
///
/// assert_eq!(track.track_id, 0);
/// assert_eq!(track.payload, &[0xAA, 0xBB]);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct MediaTrackSlice<'a> {
    /// Track id (`0` is the default track).
    pub track_id: u8,
    /// Inner Enhanced RTMP packet type shared by every track in the container.
    pub packet_type: u8,
    /// Codec FourCC for this track.
    pub fourcc: [u8; 4],
    /// FLV video frame type nibble (`1` = keyframe); `0` for audio containers.
    pub video_frame_type: u8,
    /// Track body, borrowed from the container payload.
    pub payload: &'a [u8],
}

fn read_u24(data: &[u8]) -> usize {
    ((data[0] as usize) << 16) | ((data[1] as usize) << 8) | data[2] as usize
}

/// Whether `payload` is an E-RTMP v2 multitrack container for `frame_type`.
///
/// Checks only the container header nibble, not whether the track bodies are
/// well formed; use [`foreach_track`] for full validation. Legacy FLV audio that
/// happens to set bit 7 and carry a `5` low nibble (e.g. `0x85` G.711 µ-law) is
/// correctly *not* treated as a container.
///
/// ```
/// use librtmp2::ertmp::multitrack_media::is_multitrack_container;
/// use librtmp2::types::FrameType;
///
/// // Enhanced video header with packet type 6 (multitrack).
/// let container = [
///     0x86, 0x10, b'a', b'v', b'c', b'1', 0, 0, 0, 1, 0xAA,
/// ];
/// assert!(is_multitrack_container(FrameType::Video, &container));
///
/// // A plain AVC keyframe (packet type 1) is not a container.
/// assert!(!is_multitrack_container(FrameType::Video, &[0x97, 0x01, 0x00]));
/// ```
pub fn is_multitrack_container(frame_type: FrameType, payload: &[u8]) -> bool {
    if payload.is_empty() || payload[0] & 0x80 == 0 {
        return false;
    }
    let expected = match frame_type {
        FrameType::Video => {
            // FrameType 5 is a VideoCommand: its low nibble is not a packet
            // type, so it can never be a multitrack container -- whatever the
            // nibble looks like (including the multitrack one).
            if (payload[0] >> 4) & 0x07 == VIDEO_FRAME_TYPE_COMMAND {
                return false;
            }
            ERTMP_VIDEO_PACKET_TYPE_MULTITRACK
        }
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
        // A zero-length sub-track is a legal empty track, not a malformed
        // one: the E-RTMP v2 spec allows an empty Opus sequence-start
        // payload (the channel mapping then comes from the
        // `MultichannelConfig` signal, or defaults to mono/stereo), and the
        // OneTrack multitrack form -- which has no size field -- already
        // accepts an empty body. ManyTracks / ManyTracksManyCodecs allow up
        // to 4096 sub-tracks in a single ~16 KiB message, so the per-container
        // sub-track cap (`MAX_MULTITRACK_SUBTRACKS`) is what bounds the work
        // a container packed with empty tracks can cost.
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
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::multitrack_media::foreach_track;
/// use librtmp2::types::FrameType;
///
/// // Two `avc1` tracks, each `u24` size-prefixed.
/// let container = [
///     0x86, 0x10, b'a', b'v', b'c', b'1',
///     0x00, 0x00, 0x00, 0x02, 0xAA, 0xBB,
///     0x01, 0x00, 0x00, 0x01, 0xCC,
/// ];
/// let mut ids = Vec::new();
/// assert!(foreach_track(FrameType::Video, &container, |track| {
///     ids.push(track.track_id);
/// }));
/// assert_eq!(ids, vec![0, 1]);
/// ```
pub fn foreach_track(
    frame_type: FrameType,
    payload: &[u8],
    visit: impl FnMut(&MediaTrackSlice<'_>),
) -> bool {
    walk_tracks(frame_type, payload, |_| {}) && walk_tracks(frame_type, payload, visit)
}

/// FourCC of the first track in a valid multitrack container, or `None` when
/// the container is malformed or empty.
///
/// ```
/// use librtmp2::ertmp::multitrack_media::first_track_fourcc;
/// use librtmp2::types::FrameType;
///
/// let container = [
///     0x86, 0x10, b'h', b'v', b'c', b'1', 0, 0, 0, 1, 0xAA,
/// ];
/// assert_eq!(first_track_fourcc(FrameType::Video, &container), Some(*b"hvc1"));
/// assert_eq!(first_track_fourcc(FrameType::Video, &[0x97, 0x01]), None);
/// ```
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

/// Whether a valid multitrack container carries at least one sequence-start
/// track (packet type `0`), i.e. whether it holds a codec configuration for a
/// track. Malformed containers report `false`.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::multitrack_media::multitrack_has_sequence_start;
/// use librtmp2::types::FrameType;
///
/// // Video container, shared packet type 0 = sequence start.
/// let seq = [0x86, 0x10, b'a', b'v', b'c', b'1', 0, 0, 0, 0];
/// assert!(multitrack_has_sequence_start(FrameType::Video, &seq));
///
/// // A coded-frames container (packet type 1) is not a sequence start.
/// let coded = [0x86, 0x11, b'a', b'v', b'c', b'1', 0, 0, 0, 1, 0xAA];
/// assert!(!multitrack_has_sequence_start(FrameType::Video, &coded));
/// ```
pub fn multitrack_has_sequence_start(frame_type: FrameType, payload: &[u8]) -> bool {
    let mut found = false;
    let valid = foreach_track(frame_type, payload, |track| {
        found |= track.packet_type == 0;
    });
    valid && found
}

/// True when a valid multitrack container carries Enhanced Metadata as the
/// shared inner packet type (`ERTMP_PACKET_TYPE_METADATA` / audio metadata 3).
///
/// ```
/// use librtmp2::ertmp::multitrack_media::multitrack_has_metadata;
/// use librtmp2::types::FrameType;
///
/// // Video container, shared packet type 4 = metadata.
/// let metadata = [
///     0x86, 0x14, b'a', b'v', b'c', b'1', 0, 0, 0, 0,
/// ];
/// assert!(multitrack_has_metadata(FrameType::Video, &metadata));
/// ```
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

/// Whether a valid video multitrack container carries coded frames
/// (`CodedFrames` or `CodedFramesX`) for a keyframe. Used by the relay to decide
/// whether a player joining mid-stream can start decoding immediately.
///
/// # Examples
///
/// ```
/// use librtmp2::ertmp::multitrack_media::multitrack_has_keyframe;
///
/// // CodedFrames keyframe: container bit 7 set, frame type 1, packet type 1.
/// let keyframe = [0x96, 0x13, b'a', b'v', b'c', b'1', 0, 0, 0, 1, 0xAA];
/// assert!(multitrack_has_keyframe(&keyframe));
///
/// // A non-keyframe container is not a resync point.
/// let inter = [0x86, 0x13, b'a', b'v', b'c', b'1', 0, 0, 0, 1, 0xAA];
/// assert!(!multitrack_has_keyframe(&inter));
/// ```
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
    fn accepts_multitrack_messages_with_zero_size_subtracks() {
        // A zero-length sub-track is a legal empty track (an Opus sequence
        // start may carry no payload), so the container is not malformed.
        let zero_payload = build_many_tracks_zero_payload_message(2);
        let mut seen = Vec::new();
        assert!(foreach_track(FrameType::Video, &zero_payload, |track| {
            seen.push((track.track_id, track.payload.len()));
        }));
        assert_eq!(seen, vec![(0, 0), (1, 0)]);

        // The sub-track cap -- not the empty body -- is what bounds the work
        // a container packed with empty tracks can cost.
        let over_limit = build_many_tracks_zero_payload_message(MAX_MULTITRACK_SUBTRACKS + 1);
        let mut over_calls = 0;
        assert!(!foreach_track(FrameType::Video, &over_limit, |_| {
            over_calls += 1
        }));
        assert_eq!(over_calls, 0);
    }
}
