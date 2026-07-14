//! Shared helpers for librtmp2 integration / E2E tests.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use librtmp2::server::Server;
use librtmp2::types::*;
pub const SENT_FRAME_BYTE: u8 = 0xAB;
pub const SENT_FRAME_LEN: usize = 32;

pub static FRAMES_RECEIVED: AtomicUsize = AtomicUsize::new(0);
pub static OBSERVED_CONN_ID: AtomicU64 = AtomicU64::new(0);

pub fn plain_config() -> ServerConfig {
    ServerConfig {
        max_connections: 8,
        chunk_size: 128,
        tls_enabled: 0,
        tls_cert_file: std::ptr::null(),
        tls_key_file: std::ptr::null(),
        tls_ca_file: std::ptr::null(),
        tls_insecure: 0,
    }
}

pub fn on_video_frame(frame: &Frame) {
    if frame.size as usize == SENT_FRAME_LEN {
        FRAMES_RECEIVED.fetch_add(1, Ordering::SeqCst);
    }
}

pub fn allow_publish(_conn_id: u64, _app: &str, _stream_name: &str) -> bool {
    true
}

pub fn allow_play(_conn_id: u64, _app: &str, _stream_name: &str) -> bool {
    true
}

pub fn deny_publish(_conn_id: u64, _app: &str, _stream_name: &str) -> bool {
    false
}

pub fn record_conn_id(conn_id: u64, _app: &str, _stream_name: &str) -> bool {
    OBSERVED_CONN_ID.store(conn_id, Ordering::SeqCst);
    true
}

pub fn make_video_frame(timestamp: u32, byte: u8) -> ([u8; SENT_FRAME_LEN], Frame) {
    let data = [byte; SENT_FRAME_LEN];
    let frame = Frame {
        frame_type: FrameType::Video,
        timestamp,
        composition_time: 0,
        size: data.len() as u32,
        data: data.as_ptr(),
        audio_codec: AudioCodec::default(),
        audio_sample_rate: 0,
        audio_channels: 0,
        audio_bit_depth: 0,
        audio_fourcc: FourCc::default(),
        video_codec: VideoCodec::H264,
        video_fourcc: FourCc::default(),
        video_frame_type: 1,
        is_metadata: 0,
        track_id: u8::MAX,
    };
    (data, frame)
}

pub fn make_audio_frame(timestamp: u32) -> ([u8; 16], Frame) {
    let data = [0xAF, 0x00, 0x12, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let frame = Frame {
        frame_type: FrameType::Audio,
        timestamp,
        composition_time: 0,
        size: data.len() as u32,
        data: data.as_ptr(),
        audio_codec: AudioCodec::Aac,
        audio_sample_rate: 44100,
        audio_channels: 2,
        audio_bit_depth: 16,
        audio_fourcc: FourCc::default(),
        video_codec: VideoCodec::default(),
        video_fourcc: FourCc::default(),
        video_frame_type: 0,
        is_metadata: 0,
        track_id: u8::MAX,
    };
    (data, frame)
}

/// Poll `server` until `predicate` returns true or the deadline expires.
pub fn poll_until<F>(server: &mut Server, deadline: Instant, mut predicate: F)
where
    F: FnMut() -> bool,
{
    while !predicate() && Instant::now() < deadline {
        server.poll(20).unwrap();
    }
}

/// Run a client closure on a background thread while polling the server.
pub fn run_client_with_server<F>(server: &mut Server, client_fn: F)
where
    F: FnOnce() -> std::result::Result<(), ErrorCode> + Send + 'static,
{
    let (setup_tx, setup_rx) = std::sync::mpsc::channel();
    let client_thread = thread::spawn(move || {
        let result = client_fn();
        let _ = setup_tx.send(result.is_ok());
        result.unwrap();
        thread::sleep(Duration::from_millis(200));
    });

    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        if let Ok(setup_ok) = setup_rx.try_recv() {
            assert!(setup_ok, "client setup failed");
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        server.poll(20).unwrap();
    }

    client_thread.join().unwrap();
}
