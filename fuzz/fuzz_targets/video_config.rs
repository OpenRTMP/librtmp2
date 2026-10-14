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

include!("../../tests/support/video_config_check.rs");

fuzz_target!(|data: &[u8]| {
    check(data);
    let mut rest = data;
    while let Some((&n, tail)) = rest.split_first() {
        let n = (n as usize).min(tail.len());
        check(&tail[..n]);
        rest = &tail[n..];
    }
});
