//! Debug helper: connect to an RTMP stream, play, and print each received frame.
//!
//! Prints one line per frame with type, timestamp, size, and codec details.
//! Useful when inspecting a live publisher without writing a custom client.
//!
//! Usage:
//!   dump_frames rtmp://host:port/app/stream [timeout_seconds] [max_frames]
//!
//! Exit codes: 0 = received at least one frame (or max_frames reached),
//!             1 = connect/play error, 2 = timed out with zero frames.

use std::env;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use librtmp2::client::Client;
use librtmp2::types::*;

static FRAME_COUNT: AtomicUsize = AtomicUsize::new(0);
static MAX_FRAMES: AtomicUsize = AtomicUsize::new(usize::MAX);

fn fourcc_str(fourcc: &FourCc) -> String {
    let end = fourcc
        .cc
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(fourcc.cc.len());
    String::from_utf8_lossy(&fourcc.cc[..end]).into_owned()
}

fn frame_type_label(frame_type: FrameType) -> &'static str {
    match frame_type {
        FrameType::Audio => "audio",
        FrameType::Video => "video",
        FrameType::Script => "script",
        FrameType::Metadata => "metadata",
    }
}

fn video_codec_label(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::Jpeg => "jpeg",
        VideoCodec::Sorenson => "sorenson",
        VideoCodec::Screen => "screen",
        VideoCodec::Vp6 => "vp6",
        VideoCodec::Vp6a => "vp6a",
        VideoCodec::Screen2 => "screen2",
        VideoCodec::H264 => "h264",
        VideoCodec::H265 => "h265",
        VideoCodec::Av1 => "av1",
        VideoCodec::Vp9 => "vp9",
    }
}

fn audio_codec_label(codec: AudioCodec) -> &'static str {
    match codec {
        AudioCodec::Pcm => "pcm",
        AudioCodec::Adpcm => "adpcm",
        AudioCodec::Mp3 => "mp3",
        AudioCodec::PcmLe => "pcm_le",
        AudioCodec::Nelly16k => "nelly16k",
        AudioCodec::Nelly8k => "nelly8k",
        AudioCodec::Nelly => "nelly",
        AudioCodec::G711A => "g711a",
        AudioCodec::G711U => "g711u",
        AudioCodec::Aac => "aac",
        AudioCodec::Speex => "speex",
        AudioCodec::Opus => "opus",
    }
}

fn dump_frame(frame: &Frame) {
    let n = FRAME_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
    let mut line = format!(
        "[{n:04}] {} ts={} size={}",
        frame_type_label(frame.frame_type),
        frame.timestamp,
        frame.size,
    );
    if frame.composition_time != 0 {
        line.push_str(&format!(" cts={}", frame.composition_time));
    }
    match frame.frame_type {
        FrameType::Video => {
            line.push_str(&format!(
                " codec={} frame_type={}",
                video_codec_label(frame.video_codec),
                frame.video_frame_type
            ));
            let fourcc = fourcc_str(&frame.video_fourcc);
            if !fourcc.is_empty() {
                line.push_str(&format!(" fourcc={fourcc}"));
            }
        }
        FrameType::Audio => {
            line.push_str(&format!(
                " codec={} rate={} ch={}",
                audio_codec_label(frame.audio_codec),
                frame.audio_sample_rate,
                frame.audio_channels
            ));
            let fourcc = fourcc_str(&frame.audio_fourcc);
            if !fourcc.is_empty() {
                line.push_str(&format!(" fourcc={fourcc}"));
            }
        }
        FrameType::Script | FrameType::Metadata => {
            if frame.is_metadata != 0 {
                line.push_str(" metadata=1");
            }
        }
    }
    println!("{line}");
}

fn on_frame(frame: &Frame) {
    dump_frame(frame);
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "rtmp://127.0.0.1:1935/live/test".to_string());
    let timeout_s: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let max_frames: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    if max_frames > 0 {
        MAX_FRAMES.store(max_frames, Ordering::SeqCst);
    }

    let mut client = Client::new();
    client.on_frame_cb = Some(on_frame);

    eprintln!("[dump_frames] connecting to {url}");
    if client.connect(&url).is_err() {
        eprintln!("[dump_frames] connect failed");
        return ExitCode::from(1);
    }
    if client.play().is_err() {
        eprintln!("[dump_frames] play failed");
        return ExitCode::from(1);
    }
    eprintln!(
        "[dump_frames] playing (timeout={timeout_s}s, max_frames={})",
        {
            let cap = MAX_FRAMES.load(Ordering::SeqCst);
            if cap == usize::MAX {
                "unlimited".to_string()
            } else {
                cap.to_string()
            }
        }
    );

    let deadline = Instant::now() + Duration::from_secs(timeout_s);
    while Instant::now() < deadline {
        if let Err(e) = client.poll(200) {
            eprintln!("[dump_frames] poll error {e:?}");
            break;
        }
        let count = FRAME_COUNT.load(Ordering::SeqCst);
        let cap = MAX_FRAMES.load(Ordering::SeqCst);
        if cap != usize::MAX && count >= cap {
            eprintln!("[dump_frames] reached max_frames={cap}");
            return ExitCode::SUCCESS;
        }
    }

    let count = FRAME_COUNT.load(Ordering::SeqCst);
    if count > 0 {
        eprintln!("[dump_frames] done, {count} frame(s)");
        ExitCode::SUCCESS
    } else {
        eprintln!("[dump_frames] timed out with zero frames");
        ExitCode::from(2)
    }
}
