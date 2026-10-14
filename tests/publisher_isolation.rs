//! E2E: a malformed publisher must not disturb unrelated streams.
//!
//! Publisher A feeds malformed codec configurations and protocol garbage while
//! publisher B streams numbered frames to viewer B. The server is polled on
//! the test thread, so a panic in the server aborts the test; B's viewer must
//! still see every frame in order, and new clients must still be served.

mod common;

use std::io::{Read, Write};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use librtmp2::client::Client;
use librtmp2::server::Server;
use librtmp2::types::{ErrorCode, Frame, FrameType, ServerConfig};

use common::{allow_play, allow_publish, plain_config};

macro_rules! t {
    ($e:expr) => {
        $e.map_err(|e| {
            eprintln!("isolation test failed at line {}: {:?}", line!(), e);
            e
        })?
    };
}

const ADDR: &str = "127.0.0.1:19721";
const URL: &str = "rtmp://127.0.0.1:19721/live";

static VIEWER_B: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());
static LATE_A: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());
static LATE_C: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

fn capture(frame: &Frame, sink: &Mutex<Vec<Vec<u8>>>) {
    // SAFETY: `data`/`size` describe the payload for the duration of the callback.
    let bytes = unsafe { std::slice::from_raw_parts(frame.data, frame.size as usize) };
    sink.lock().unwrap().push(bytes.to_vec());
}

// A valid AVCDecoderConfigurationRecord inside a legacy FLV sequence header.
fn avc_header(tag: u8) -> Vec<u8> {
    let mut v = vec![
        0x17, 0x00, 0, 0, 0, 1, 0x64, 0, 0x1F, 0xFF, 0xE1, 0, 2, 0x67, tag, 1, 0, 2,
    ];
    v.extend_from_slice(&[0x68, tag]);
    v
}

fn counted_frame(seq: u32) -> Vec<u8> {
    let mut v = vec![0x27, 0x01, 0, 0, 0];
    v.extend_from_slice(&seq.to_be_bytes());
    v
}

/// A container that violates the multitrack framing itself; the server may
/// drop the offending publisher (and only that publisher) over it.
fn framing_violation() -> Vec<u8> {
    vec![0x86, 0x10, b'a', b'v', b'c', b'1', 0, 0xFF, 0xFF, 0xFF]
}

fn malformed_configs() -> Vec<Vec<u8>> {
    let hvc = [0x90, b'h', b'v', b'c', b'1'];
    vec![
        vec![0x17, 0x00],
        vec![0x17, 0x00, 0, 0, 0],
        vec![
            0x17, 0x00, 0, 0, 0, 1, 0x64, 0, 0x1F, 0xFF, 0xE1, 0, 0, 1, 0, 0,
        ],
        vec![
            0x17, 0x00, 0, 0, 0, 1, 0x64, 0, 0x1F, 0xFF, 0xE1, 0xFF, 0xFF, 0x67,
        ],
        hvc.to_vec(),
        [&hvc[..], &[1u8; 22], &[3, 0x80 | 32, 0, 1, 0, 0]].concat(),
        [&hvc[..], &[1u8; 22], &[0xFF, 0xA1, 0xFF, 0xFF]].concat(),
        vec![0xD0, b'h', b'v', b'c', b'1', 0xFF], // command frame, not a header
        vec![0x9F, b'h', b'v'],                   // truncated ExHeader
        vec![],
    ]
}

/// Send protocol garbage over a raw socket after a completed handshake and
/// verify the server closes the connection.
fn raw_attacker(payload: &[u8]) -> bool {
    let mut s = common::raw_handshake(ADDR);
    let _ = s.write_all(payload);
    let mut sink = [0u8; 4096];
    loop {
        match s.read(&mut sink) {
            Ok(0) => return true,
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return true,
            Err(_) => return false,
        }
    }
}

fn wait_for(deadline: Instant, mut done: impl FnMut() -> bool) {
    while !done() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
}

const PER_PHASE: u32 = 40;

fn is_counted_frame(p: &[u8]) -> bool {
    p.len() == 9 && p[0] == 0x27
}

fn frames_seen_by_viewer_b() -> usize {
    VIEWER_B
        .lock()
        .unwrap()
        .iter()
        .filter(|p| is_counted_frame(p))
        .count()
}

/// The unrelated stream: publisher B, numbered frames, viewer B.
struct StreamB {
    publisher: Client,
    viewer: Client,
    sent: u32,
}

impl StreamB {
    fn start() -> Result<Self, ErrorCode> {
        let mut publisher = Client::new();
        t!(publisher.connect(&format!("{URL}/b")));
        t!(publisher.publish());
        t!(publisher.send_frame_payload(FrameType::Video, 0, &avc_header(1)));
        let mut viewer = Client::new();
        viewer.on_frame_cb = Some(|f| capture(f, &VIEWER_B));
        t!(viewer.connect(&format!("{URL}/b")));
        t!(viewer.play());
        Ok(Self {
            publisher,
            viewer,
            sent: 0,
        })
    }

    fn send(&mut self, n: u32) -> Result<(), ErrorCode> {
        for _ in 0..n {
            let seq = self.sent;
            t!(self
                .publisher
                .send_frame_payload(FrameType::Video, seq, &counted_frame(seq)));
            self.sent += 1;
            t!(self.viewer.poll(0));
        }
        Ok(())
    }

    /// Wait until every frame sent so far reached viewer B.
    fn drain(&mut self) -> Result<(), ErrorCode> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && (frames_seen_by_viewer_b() as u32) < self.sent {
            t!(self.viewer.poll(20));
        }
        Ok(())
    }
}

/// Publisher A sends malformed codec configuration mid-stream.
fn phase_malformed_configs(pub_a: &mut Client, b: &mut StreamB) -> Result<(), ErrorCode> {
    for (i, bad) in malformed_configs().iter().enumerate() {
        // A may be disconnected by the server; that is its own business.
        let _ = pub_a.send_frame_payload(FrameType::Video, 2 + i as u32, bad);
        t!(b.send(2));
    }
    b.send(PER_PHASE)
}

/// Raw protocol garbage on fresh connections.
fn phase_raw_garbage(b: &mut StreamB) -> Result<(), ErrorCode> {
    let garbage: [&[u8]; 4] = [
        &[0x03, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0x14, 0, 0, 0, 0], // 16 MiB-1 command
        &[
            0x02, 0, 0, 0, 0, 0, 4, 0x01, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF,
        ], // huge chunk size
        &[0x42; 256],
        &[0xC3; 512], // type-3 chunks with no prior header
    ];
    for g in garbage {
        assert!(raw_attacker(g), "server must drop garbage connection");
        t!(b.send(5));
    }
    b.send(PER_PHASE)
}

/// A late joiner on A sees only A's valid header, never malformed data.
fn phase_late_joiner() -> Result<(), ErrorCode> {
    let mut late = Client::new();
    late.on_frame_cb = Some(|f| capture(f, &LATE_A));
    t!(late.connect(&format!("{URL}/a")));
    t!(late.play());
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && LATE_A.lock().unwrap().is_empty() {
        t!(late.poll(20));
    }
    Ok(())
}

/// A disconnects; its route is released and a new publisher can take it,
/// and a brand new publisher/viewer pair works after the incident.
fn phase_replacement_after_incident(pub_a: Client) -> Result<(), ErrorCode> {
    drop(pub_a);
    thread::sleep(Duration::from_millis(200));
    let mut pub_c = Client::new();
    t!(pub_c.connect(&format!("{URL}/a")));
    t!(pub_c.publish());
    let mut viewer_c = Client::new();
    viewer_c.on_frame_cb = Some(|f| capture(f, &LATE_C));
    t!(viewer_c.connect(&format!("{URL}/a")));
    t!(viewer_c.play());
    thread::sleep(Duration::from_millis(100));
    t!(pub_c.send_frame_payload(FrameType::Video, 0, &counted_frame(7777)));
    wait_for(Instant::now() + Duration::from_secs(3), || {
        let _ = viewer_c.poll(20);
        LATE_C
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.ends_with(&7777u32.to_be_bytes()))
    });
    Ok(())
}

fn run_scenario() -> Result<(), ErrorCode> {
    let mut b = StreamB::start()?;

    let mut pub_a = Client::new();
    t!(pub_a.connect(&format!("{URL}/a")));
    t!(pub_a.publish());
    t!(pub_a.send_frame_payload(FrameType::Video, 0, &avc_header(9)));
    t!(pub_a.send_frame_payload(FrameType::Video, 1, &[0x17, 0x01, 0, 0, 0, 0xEE]));

    t!(b.send(PER_PHASE));
    t!(phase_malformed_configs(&mut pub_a, &mut b));
    t!(phase_raw_garbage(&mut b));
    t!(b.drain());
    t!(phase_late_joiner());

    // A framing violation may cost A its own connection; B must not notice.
    let _ = pub_a.send_frame_payload(FrameType::Video, 99, &framing_violation());
    t!(b.send(PER_PHASE));
    t!(b.drain());

    phase_replacement_after_incident(pub_a)
}

fn assert_viewer_b_unaffected() {
    let b = VIEWER_B.lock().unwrap();
    let counters: Vec<u32> = b
        .iter()
        .filter(|p| is_counted_frame(p))
        .map(|p| u32::from_be_bytes(p[5..9].try_into().unwrap()))
        .collect();
    assert!(
        counters.len() >= (PER_PHASE * 4) as usize,
        "viewer B only received {} frames",
        counters.len()
    );
    assert_eq!(
        counters,
        (0..counters.len() as u32).collect::<Vec<_>>(),
        "viewer B must see B's frames complete and in order"
    );
    assert!(
        b.iter().any(|p| *p == avc_header(1)),
        "viewer B lost its header"
    );
}

fn assert_late_joiner_saw_only_valid_state() {
    let late = LATE_A.lock().unwrap();
    assert!(
        late.iter().any(|p| *p == avc_header(9)),
        "late joiner on A must receive A's last valid header, got {late:?}"
    );
    for p in late.iter() {
        assert!(
            !malformed_configs().contains(p) || p.is_empty(),
            "late joiner received a malformed configuration: {p:?}"
        );
    }
}

#[test]
fn malformed_publisher_does_not_disrupt_unrelated_stream() {
    let mut server = Server::new(ServerConfig {
        max_connections: 64,
        max_connections_per_addr: 32,
        ..plain_config()
    })
    .unwrap();
    server.listen(ADDR).unwrap();
    server.on_publish_cb = Some(allow_publish);
    server.on_play_cb = Some(allow_play);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let client = thread::spawn(move || {
        let result = run_scenario();
        let _ = done_tx.send(result.is_ok());
        result
    });

    // Poll the server on this thread until the client script finishes; a
    // server panic fails the test directly.
    let deadline = Instant::now() + Duration::from_secs(30);
    let ok = loop {
        if let Ok(ok) = done_rx.try_recv() {
            break ok;
        }
        assert!(Instant::now() < deadline, "isolation scenario timed out");
        server.poll(5).unwrap();
    };
    client.join().unwrap().ok();
    assert!(ok, "client scenario failed");

    assert_viewer_b_unaffected();
    assert_late_joiner_saw_only_valid_state();
    assert!(
        LATE_C
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.ends_with(&7777u32.to_be_bytes())),
        "new publisher/viewer pair must work after the incident"
    );
}
