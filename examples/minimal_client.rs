//! Minimal RTMP client: connects, publishes a stream, and sends a single
//! synthetic video frame. Rust port of the old
//! `examples/minimal_client/minimal_client.c`.
//!
//! Usage: minimal_client rtmp://host:port/app/stream

use std::env;
use std::process::ExitCode;

use librtmp2::client::Client;
use librtmp2::types::*;

fn main() -> ExitCode {
    let Some(url) = env::args().nth(1) else {
        eprintln!("Usage: minimal_client rtmp://host:port/app/stream");
        return ExitCode::FAILURE;
    };

    println!("[minimal_client] librtmp2 v{VERSION_STRING}");
    println!("[minimal_client] Connecting to {url}");

    let mut client = Client::new();
    if let Err(e) = client.connect(&url) {
        eprintln!("Failed to connect: {e:?}");
        return ExitCode::FAILURE;
    }
    println!("[minimal_client] Connected");

    if let Err(e) = client.publish() {
        eprintln!("Failed to publish: {e:?}");
        return ExitCode::FAILURE;
    }
    println!("[minimal_client] Publishing");

    // `send_frame` writes `data`/`size` verbatim, so this body is what the
    // peer sees: 0x17 = FLV frame type 1 (keyframe) + codec id 7 (AVC),
    // AVCPacketType 1 (single NALU), zero composition time, 55-byte NALU.
    let mut payload = [0u8; 64];
    payload[0] = 0x17;
    payload[1] = 1;
    payload[5..9].copy_from_slice(&55u32.to_be_bytes());
    let frame = Frame {
        frame_type: FrameType::Video,
        timestamp: 0,
        size: payload.len() as u32,
        data: payload.as_ptr(),
        video_codec: VideoCodec::H264,
        video_frame_type: 1, // keyframe
        ..Default::default()
    };

    if let Err(e) = client.send_frame(&frame) {
        eprintln!("Failed to send frame: {e:?}");
        return ExitCode::FAILURE;
    }
    println!("[minimal_client] Sent {}-byte video frame", frame.size);

    ExitCode::SUCCESS
}
