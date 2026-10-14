//! Connect / send malformed input / disconnect churn against a local server.
//!
//! The default run is short and deterministic. Memory is compared against a
//! baseline taken after a warm-up phase of the same workload (not a hardcoded
//! limit), so allocator and runner differences cancel out. The long variant
//! is `#[ignore]`d and runs from the extended workflow.

mod common;

use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

use librtmp2::client::Client;
use librtmp2::server::Server;
use librtmp2::types::{FrameType, ServerConfig};

use common::{allow_play, allow_publish, plain_config};

fn rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

fn raw_garbage_session(addr: &str, seed: u32) {
    let mut s = common::raw_handshake(addr);
    // Start a large message, never finish it, then violate the protocol.
    let mut junk = vec![0x03, 0, 0, 0, 0x00, 0x40, 0x00, 0x09, 0, 0, 0, 0];
    junk.extend_from_slice(&[0xAA; 100]);
    match seed % 3 {
        0 => junk.extend_from_slice(&[
            0x02, 0, 0, 0, 0, 0, 4, 0x01, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF,
        ]),
        1 => junk.extend_from_slice(&[0xC3; 300]),
        _ => {}
    }
    let _ = s.write_all(&junk);
    // Abrupt close (or server-initiated close, whichever comes first).
    if seed.is_multiple_of(2) {
        let _ = s.shutdown(std::net::Shutdown::Both);
    }
}

fn run_churn(addr: &'static str, warmup: u32, measured: u32) -> (u64, u64) {
    let mut server = Server::new(ServerConfig {
        max_connections: 64,
        max_connections_per_addr: 32,
        ..plain_config()
    })
    .unwrap();
    server.listen(addr).unwrap();
    server.on_publish_cb = Some(allow_publish);
    server.on_play_cb = Some(allow_play);

    let (tx, rx) = std::sync::mpsc::channel::<&'static str>();
    let client = thread::spawn(move || {
        let url = format!("rtmp://{addr}/live");
        let round = |i: u32| {
            raw_garbage_session(addr, i);
            // A publisher that sends malformed configs then vanishes.
            let mut p = Client::new();
            if p.connect(&format!("{url}/churn{}", i % 8)).is_ok() && p.publish().is_ok() {
                for bad in [&[0x17u8, 0x00][..], &[0x90, b'h', b'v', b'c', b'1', 3], &[]] {
                    let _ = p.send_frame_payload(FrameType::Video, 0, bad);
                }
            }
        };
        for i in 0..warmup {
            round(i);
        }
        tx.send("warm").unwrap();
        for i in 0..measured {
            round(warmup + i);
        }
        tx.send("done").unwrap();
    });

    let mut baseline = 0;
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        server.poll(2).unwrap();
        match rx.try_recv() {
            Ok("warm") => {
                // Let reaping settle before sampling.
                for _ in 0..50 {
                    server.poll(5).unwrap();
                }
                baseline = rss_kib().unwrap_or(0);
            }
            Ok(_) => break,
            Err(_) => {}
        }
        assert!(Instant::now() < deadline, "churn timed out");
    }
    client.join().unwrap();
    for _ in 0..100 {
        server.poll(5).unwrap();
    }
    (baseline, rss_kib().unwrap_or(0))
}

fn assert_stable(baseline: u64, after: u64, measured: u32) {
    if baseline == 0 {
        return; // no /proc on this platform
    }
    // Growth allowed for allocator noise: 16 MiB, or 5% of the baseline.
    let allowed = (16 * 1024).max(baseline / 20);
    assert!(
        after <= baseline + allowed,
        "RSS grew from {baseline} KiB to {after} KiB over {measured} churn rounds"
    );
}

#[test]
fn malformed_connection_churn_does_not_grow_memory() {
    let (before, after) = run_churn("127.0.0.1:19722", 60, 150);
    assert_stable(before, after, 150);
}

#[test]
#[ignore = "long-running stress test; run via the extended workflow"]
fn malformed_connection_churn_long() {
    let n: u32 = std::env::var("CHURN_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);
    let (before, after) = run_churn("127.0.0.1:19723", 200, n);
    assert_stable(before, after, n);
}
