from pathlib import Path

path = Path("src/server/mod.rs")
text = path.read_text()

def replace_once(old: str, new: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"expected one match, found {count}: {old[:160]!r}")
    text = text.replace(old, new, 1)

replace_once(
    "use crate::media::{CacheFrameKind, classify_cache_frame};",
    "use crate::media::{CacheFrameKind, classify_cache_frame, normalize_modex_payload};",
)
replace_once(
    """                        if !is_multitrack_container(FrameType::Video, hdr)
                            || conn.accepts_multitrack()
""",
    """                        if !Self::cached_payload_is_multitrack(FrameType::Video, hdr)
                            || conn.accepts_multitrack()
""",
)
replace_once(
    """                        if !is_multitrack_container(FrameType::Audio, hdr)
                            || conn.accepts_multitrack()
""",
    """                        if !Self::cached_payload_is_multitrack(FrameType::Audio, hdr)
                            || conn.accepts_multitrack()
""",
)
replace_once(
    """                        if !is_multitrack_container(FrameType::Video, kf)
                            || conn.accepts_multitrack()
""",
    """                        if !Self::cached_payload_is_multitrack(FrameType::Video, kf)
                            || conn.accepts_multitrack()
""",
)
replace_once(
    """                if matches!(frame.frame_type, FrameType::Audio | FrameType::Video)
                    && is_multitrack_container(frame.frame_type, &frame.payload)
                    && !conn.accepts_multitrack()
""",
    """                if matches!(frame.frame_type, FrameType::Audio | FrameType::Video)
                    && is_multitrack_container(frame.frame_type, &frame.cache_payload)
                    && !conn.accepts_multitrack()
""",
)
replace_once(
    """    fn multitrack_sequence_track_ids(frame_type: FrameType, payload: &[u8]) -> Vec<u8> {
""",
    """    fn cached_payload_is_multitrack(frame_type: FrameType, payload: &[u8]) -> bool {
        let normalized = normalize_modex_payload(payload, CAPS_EX_MASK_MODEX);
        is_multitrack_container(frame_type, normalized.as_ref())
    }

    fn multitrack_sequence_track_ids(frame_type: FrameType, payload: &[u8]) -> Vec<u8> {
""",
)
replace_once(
    """    #[test]
    fn multitrack_video_sequence_header_is_cached() {
""",
    """    #[test]
    fn modex_wrapped_multitrack_is_detected_for_player_gating() {
        let payload = [
            0x87, 0x02, 0x00, 0x01, 0x02, 0x06, 0x10, b'a', b'v', b'c', b'1',
            0x00, 0x00, 0x00, 0x01, 0xAA,
        ];
        assert!(Server::cached_payload_is_multitrack(
            FrameType::Video,
            &payload
        ));
    }

    #[test]
    fn multitrack_video_sequence_header_is_cached() {
""",
)
path.write_text(text)
print("Applied ModEx multitrack capability-gate fix")
