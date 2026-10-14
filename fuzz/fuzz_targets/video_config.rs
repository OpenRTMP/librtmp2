#![no_main]

//! Video header / codec-configuration classification.
//!
//! Feeds arbitrary bytes, both as one payload and as a length-prefixed
//! sequence of payloads (to exercise repeated and conflicting configuration
//! messages), through the cache classifier, delivery hints, header parsers and
//! the AVC/HEVC record validators.

use libfuzzer_sys::fuzz_target;
use librtmp2::flv::video_tag;
use librtmp2::media::codec_config::{validate_avc_record, validate_hevc_record};
use librtmp2::media::{
    CacheFrameKind, DeliveryHint, classify_cache_frame, parse_video_metadata_hdr, populate_av_frame,
};
use librtmp2::types::{Frame, FrameType, VideoTag};

fn check(payload: &[u8]) {
    let kind = classify_cache_frame(FrameType::Video, payload);

    // A legacy AVC sequence header is only ever cacheable with a valid record.
    if kind == CacheFrameKind::VideoSequenceHeader
        && payload.len() >= 2
        && payload[0] & 0x80 == 0
        && payload[0] & 0x0F == 7
    {
        assert!(validate_avc_record(payload.get(5..).unwrap_or(&[])));
    }
    // An ExHeader hvc1 SequenceStart likewise.
    if kind == CacheFrameKind::VideoSequenceHeader
        && payload.len() >= 5
        && payload[0] & 0x8F == 0x80
        && &payload[1..5] == b"hvc1"
    {
        assert!(validate_hevc_record(&payload[5..]));
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

fuzz_target!(|data: &[u8]| {
    check(data);
    let mut rest = data;
    while let Some((&n, tail)) = rest.split_first() {
        let n = (n as usize).min(tail.len());
        check(&tail[..n]);
        rest = &tail[n..];
    }
});
