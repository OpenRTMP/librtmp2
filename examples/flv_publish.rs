//! Publish an FLV file's audio/video tags to an RTMP server in real time
//! using `librtmp2::client::Client`, e.g. to check publish interop against
//! a third-party server (see `tests/interop/publish_interop.sh`).
//!
//! Usage: flv_publish <file.flv> rtmp://host:port/app/stream

use std::env;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use librtmp2::client::Client;
use librtmp2::types::FrameType;

const FLV_HEADER_LEN: usize = 9;
const FLV_TAG_HEADER_LEN: usize = 11;

/// One FLV tag: (frame type, timestamp in ms, payload).
type FlvTag<'a> = (FrameType, u32, &'a [u8]);

fn flv_tags(data: &[u8]) -> Result<Vec<FlvTag<'_>>, String> {
    if data.len() < FLV_HEADER_LEN + 4 || &data[..3] != b"FLV" {
        return Err("not an FLV file".into());
    }
    let mut pos = FLV_HEADER_LEN + 4; // header + PreviousTagSize0
    let mut tags = Vec::new();
    while pos + FLV_TAG_HEADER_LEN <= data.len() {
        let hdr = &data[pos..pos + FLV_TAG_HEADER_LEN];
        let size = u32::from_be_bytes([0, hdr[1], hdr[2], hdr[3]]) as usize;
        let ts = u32::from_be_bytes([hdr[7], hdr[4], hdr[5], hdr[6]]);
        let body_start = pos + FLV_TAG_HEADER_LEN;
        let Some(body) = data.get(body_start..body_start + size) else {
            break; // truncated trailing tag
        };
        let frame_type = match hdr[0] & 0x1F {
            8 => Some(FrameType::Audio),
            9 => Some(FrameType::Video),
            _ => None, // script tags (onMetaData) are not needed to decode
        };
        if let Some(frame_type) = frame_type {
            tags.push((frame_type, ts, body));
        }
        pos = body_start + size + 4; // + PreviousTagSize
    }
    Ok(tags)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let [_, path, url] = args.as_slice() else {
        eprintln!("Usage: flv_publish <file.flv> rtmp://host:port/app/stream");
        return ExitCode::FAILURE;
    };
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let tags = match flv_tags(&data) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut client = Client::new();
    if let Err(e) = client.connect(url) {
        eprintln!("connect failed: {e:?}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = client.publish() {
        eprintln!("publish failed: {e:?}");
        return ExitCode::FAILURE;
    }

    let start = Instant::now();
    let mut bytes = 0usize;
    for (frame_type, ts, payload) in &tags {
        // Pace to the tag's timestamp so the server sees a live stream.
        let due = Duration::from_millis(u64::from(*ts));
        while start.elapsed() < due {
            let wait = (due - start.elapsed()).as_millis().clamp(1, 20) as i32;
            if let Err(e) = client.poll(wait) {
                eprintln!("poll failed: {e:?}");
                return ExitCode::FAILURE;
            }
        }
        if let Err(e) = client.send_frame_payload(*frame_type, *ts, payload) {
            eprintln!("send failed at ts={ts}: {e:?}");
            return ExitCode::FAILURE;
        }
        bytes += payload.len();
    }
    // Let the last frames drain before disconnecting.
    let drain_until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < drain_until {
        if client.poll(20).is_err() {
            break;
        }
    }
    println!(
        "[flv_publish] sent {} tags, {bytes} payload bytes",
        tags.len()
    );
    ExitCode::SUCCESS
}
