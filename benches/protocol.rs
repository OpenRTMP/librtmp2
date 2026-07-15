use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use librtmp2::buffer::Buffer;
use librtmp2::chunk::reader::{ChunkMessage, chunk_read};
use librtmp2::chunk::state::ChunkRegistry;
use librtmp2::chunk::writer::chunk_write;
use librtmp2::ertmp::fourcc;
use librtmp2::flv::{audio_tag, video_tag};
use librtmp2::handshake::{self, Handshake};
use librtmp2::message::command;
use librtmp2::types::VideoTag;

fn bench_chunk_roundtrip(c: &mut Criterion) {
    let payload: Vec<u8> = (0..4096u16).map(|i| (i % 256) as u8).collect();
    let mut group = c.benchmark_group("chunk");
    group.throughput(Throughput::Bytes(payload.len() as u64));

    group.bench_function("write_read_roundtrip", |b| {
        b.iter(|| {
            let mut out = Buffer::new();
            let msg = ChunkMessage {
                csid: 3,
                fmt: 0,
                timestamp: 40,
                msg_length: payload.len() as u32,
                msg_type_id: 0x09,
                msg_stream_id: 1,
                is_complete: false,
            };
            chunk_write(&mut out, &msg, &payload, payload.len(), 128).unwrap();

            let mut reg = ChunkRegistry::new();
            let mut read_msg = ChunkMessage::default();
            let mut payload_ptr: *const u8 = std::ptr::null();
            let mut payload_len = 0usize;
            let mut buf = Buffer::from_slice(out.as_slice());
            loop {
                match chunk_read(
                    &mut buf,
                    &mut reg,
                    None,
                    &mut read_msg,
                    &mut payload_ptr,
                    &mut payload_len,
                ) {
                    Ok(1) if read_msg.is_complete => break,
                    Ok(0) => panic!("chunk_read needs more data"),
                    Ok(_) => {}
                    Err(e) => panic!("chunk_read failed: {e:?}"),
                }
            }
            black_box(payload_len)
        });
    });
    group.finish();
}

fn bench_amf_connect(c: &mut Criterion) {
    c.bench_function("amf0_build_connect", |b| {
        b.iter(|| {
            let mut buf = Buffer::with_capacity(512);
            command::build_connect(
                &mut buf,
                "live",
                "rtmp://127.0.0.1/live",
                "",
                "",
                "FMLE/3.0",
                3191,
                252,
                None,
            )
            .unwrap();
            black_box(buf.write_pos())
        });
    });
}

fn bench_flv_parsers(c: &mut Criterion) {
    let video_payload = [0x17u8, 0x01, 0x00, 0x01, 0x2C, 0xDE, 0xAD, 0xBE, 0xEF];
    let audio_payload = [0xAF, 0x00, 0x12, 0x10];

    let mut group = c.benchmark_group("flv");
    group.bench_function("video_tag_h264", |b| {
        b.iter(|| {
            let mut tag = VideoTag::default();
            video_tag::parse(black_box(&video_payload), &mut tag).unwrap();
            black_box(tag.codec)
        });
    });
    group.bench_function("audio_tag_aac", |b| {
        b.iter(|| {
            let mut tag = librtmp2::types::AudioTag::default();
            audio_tag::parse(black_box(&audio_payload), &mut tag).unwrap();
            black_box(tag.codec)
        });
    });
    group.finish();
}

fn bench_fourcc(c: &mut Criterion) {
    c.bench_function("fourcc_to_video_codec_avc1", |b| {
        b.iter(|| black_box(fourcc::fourcc_to_video_codec(b"avc1").unwrap()));
    });
}

fn bench_handshake_server(c: &mut Criterion) {
    let c1 = vec![0u8; 1536];
    c.bench_function("server_read_c1", |b| {
        b.iter(|| {
            let mut hs = Handshake::default();
            handshake::server_init(&mut hs);
            let mut buf = Buffer::from_slice(black_box(&c1));
            handshake::server_read_c1(&mut hs, &mut buf).unwrap();
            black_box(hs.out.write_pos())
        });
    });
}

criterion_group!(
    protocol,
    bench_chunk_roundtrip,
    bench_amf_connect,
    bench_flv_parsers,
    bench_fourcc,
    bench_handshake_server
);
criterion_main!(protocol);
