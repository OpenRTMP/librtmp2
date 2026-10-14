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

include!("../fuzz/video_config_check.rs");

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
