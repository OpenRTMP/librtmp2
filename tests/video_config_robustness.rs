//! Deterministic mutation pass over the `video_config` fuzz seeds.
//!
//! Runs on stable in normal CI: every seed in `fuzz/seeds/video_config` is
//! truncated at every length, bit-flipped, and has boundary values written at
//! every offset, and the same functions as the `video_config` fuzz target must
//! neither panic nor violate its invariants.

use std::fs;
use std::path::Path;

use librtmp2::flv::video_tag;
use librtmp2::media::codec_config::{validate_avc_record, validate_hevc_record};
use librtmp2::media::{
    CacheFrameKind, DeliveryHint, classify_cache_frame, parse_video_metadata_hdr, populate_av_frame,
};
use librtmp2::types::{Frame, FrameType, VideoTag};

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

fn seeds() -> Vec<(String, Vec<u8>)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds/video_config");
    let mut out: Vec<_> = fs::read_dir(dir)
        .expect("seed directory")
        .map(|e| e.unwrap())
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    out.sort();
    assert!(out.len() >= 10, "seed corpus missing");
    out
}

#[test]
fn seeds_are_handled_unmodified() {
    for (_, data) in seeds() {
        check(&data);
    }
}

#[test]
fn every_truncation_and_boundary_mutation_is_handled() {
    for (name, data) in seeds() {
        for n in 0..=data.len() {
            check(&data[..n]);
        }
        let mut buf = data.clone();
        for i in 0..data.len() {
            for v in [0x00, 0x01, 0x7F, 0x80, 0xFE, 0xFF] {
                buf[i] = v;
                check(&buf);
            }
            buf[i] = data[i];
        }
        assert_eq!(buf, data, "{name}");
    }
}

#[test]
fn random_bit_flips_and_splices_are_handled() {
    let all = seeds();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..20_000 {
        let (_, base) = &all[(next() % all.len() as u64) as usize];
        let mut buf = base.clone();
        for _ in 0..(next() % 4) {
            if buf.is_empty() {
                break;
            }
            let i = (next() % buf.len() as u64) as usize;
            buf[i] ^= 1 << (next() % 8);
        }
        if next().is_multiple_of(3) {
            let (_, other) = &all[(next() % all.len() as u64) as usize];
            let cut = (next() % (buf.len() as u64 + 1)) as usize;
            buf.truncate(cut);
            buf.extend_from_slice(&other[(next() % (other.len() as u64 + 1)) as usize..]);
        }
        check(&buf);
    }
}
