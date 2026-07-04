use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use librtmp2::client::Client;
use librtmp2::server::Server;
use librtmp2::types::*;

const FRAME_LEN: usize = 256;
const RTMP_PORT: u16 = 19680;

static RECEIVED: AtomicUsize = AtomicUsize::new(0);

fn on_relay_frame(frame: &Frame) {
    if frame.size as usize >= FRAME_LEN {
        RECEIVED.fetch_add(1, Ordering::SeqCst);
    }
}

fn allow(_: u64, _: &str, _: &str) -> bool {
    true
}

fn relay_n_frames(frame_count: u32) {
    RECEIVED.store(0, Ordering::SeqCst);

    let mut server = Server::new(ServerConfig {
        max_connections: 8,
        chunk_size: 128,
        tls_enabled: 0,
        tls_cert_file: std::ptr::null(),
        tls_key_file: std::ptr::null(),
        tls_ca_file: std::ptr::null(),
        tls_insecure: 0,
    })
    .unwrap();
    server
        .listen(&format!("127.0.0.1:{RTMP_PORT}"))
        .unwrap();
    server.on_publish_cb = Some(allow);
    server.on_play_cb = Some(allow);

    let port = RTMP_PORT;
    let client = thread::spawn(move || {
        let mut publisher = Client::new();
        publisher
            .connect(&format!("rtmp://127.0.0.1:{port}/live/bench"))
            .unwrap();
        publisher.publish().unwrap();

        let mut player = Client::new();
        player.on_frame_cb = Some(on_relay_frame);
        player
            .connect(&format!("rtmp://127.0.0.1:{port}/live/bench"))
            .unwrap();
        player.play().unwrap();
        thread::sleep(Duration::from_millis(50));

        let data = [0x17u8; FRAME_LEN];
        for ts in 0..frame_count {
            let frame = Frame {
                frame_type: FrameType::Video,
                timestamp: ts * 40,
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
            };
            publisher.send_frame(&frame).unwrap();
        }

        let deadline = Instant::now() + Duration::from_secs(10);
        while RECEIVED.load(Ordering::SeqCst) < frame_count as usize
            && Instant::now() < deadline
        {
            player.poll(10).unwrap();
        }
    });

    let deadline = Instant::now() + Duration::from_secs(12);
    while !client.is_finished() && Instant::now() < deadline {
        server.poll(5).unwrap();
    }
    client.join().unwrap();
}

fn bench_relay_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("relay");
    for frames in [100u32, 500] {
        group.throughput(Throughput::Elements(frames as u64));
        group.bench_with_input(
            BenchmarkId::new("publish_to_player", frames),
            &frames,
            |b, &count| {
                b.iter(|| relay_n_frames(black_box(count)));
            },
        );
    }
    group.finish();
}

criterion_group!(relay, bench_relay_throughput);
criterion_main!(relay);
