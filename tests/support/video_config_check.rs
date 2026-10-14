// Shared by the `video_config` fuzz target and
// `tests/video_config_robustness.rs` (via `include!`; kept under `tests/` so it ships in the published package).

fn check(payload: &[u8]) {
    let kind = classify_cache_frame(FrameType::Video, payload);
    if kind == CacheFrameKind::VideoSequenceHeader
        && payload.len() >= 2
        && payload[0] & 0x80 == 0
        && payload[0] & 0x0F == 7
    {
        assert!(
            validate_avc_record(payload.get(5..).unwrap_or(&[])),
            "{payload:?}"
        );
    }
    if kind == CacheFrameKind::VideoSequenceHeader
        && payload.len() >= 5
        && payload[0] & 0x8F == 0x80
        && &payload[1..5] == b"hvc1"
    {
        assert!(validate_hevc_record(&payload[5..]), "{payload:?}");
    }
    let _ = classify_cache_frame(FrameType::Audio, payload);
    let _ = DeliveryHint::classify(FrameType::Video, payload, true);
    let _ = parse_video_metadata_hdr(payload);
    let mut tag = VideoTag::default();
    let _ = video_tag::parse(payload, &mut tag);
    let mut frame = Frame {
        frame_type: FrameType::Video,
        ..Frame::default()
    };
    populate_av_frame(&mut frame, payload);
    let _ = validate_avc_record(payload);
    let _ = validate_hevc_record(payload);
}
