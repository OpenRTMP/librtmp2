//! Benchmark helper: RTMP connect + publish handshake latency.
//!
//! Measures wall-clock time from `connect()` through a successful `publish()`
//! (i.e. up to `NetStream.Publish.Start`) against any RTMP server -- this
//! server, or a third-party one (nginx-rtmp, MediaMTX, ...) -- since the
//! handshake is pure RTMP protocol and carries no media payload.
//!
//! Runs `--concurrency` publishers at a time, `--count` total, and reports
//! success rate plus latency percentiles.
//!
//! Two ways to supply the per-attempt URLs:
//!   - `<rtmp_url_prefix> --count N`: each attempt appends `-<n>` to the
//!     prefix (`<prefix>-0`, `<prefix>-1`, ...) so it needs an app that
//!     accepts arbitrary stream names (e.g. nginx-rtmp, MediaMTX).
//!   - `--url-list <path> --count N`: reads one full RTMP URL per line and
//!     round-robins the pool over them -- for servers (like librtmp2-server)
//!     that validate the stream key against a pre-provisioned list, generate
//!     the file from that server's own API first.
//!
//! Usage:
//!   bench_handshake <rtmp_url_prefix> [--count N] [--concurrency C]
//!   bench_handshake --url-list urls.txt [--count N] [--concurrency C]
//!
//! Example:
//!   bench_handshake rtmp://127.0.0.1:1936/live/bench --count 200 --concurrency 50

#[path = "bench_common/mod.rs"]
mod bench_common;

use std::env;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use librtmp2::client::Client;

enum UrlSource {
    Prefix(String),
    List(Vec<String>),
}

struct Args {
    source: UrlSource,
    count: usize,
    concurrency: usize,
}

fn parse_args() -> Option<Args> {
    let raw: Vec<String> = env::args().skip(1).collect();
    let mut url_prefix = None;
    let mut url_list_path = None;
    let mut count = 200usize;
    let mut concurrency = 50usize;
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--count" => {
                count = raw.get(i + 1)?.parse().ok()?;
                i += 2;
            }
            "--concurrency" => {
                concurrency = raw.get(i + 1)?.parse().ok()?;
                i += 2;
            }
            "--url-list" => {
                url_list_path = Some(raw.get(i + 1)?.clone());
                i += 2;
            }
            other => {
                url_prefix = Some(other.to_string());
                i += 1;
            }
        }
    }

    let source = match url_list_path {
        Some(path) => UrlSource::List(bench_common::read_url_list(&path)?),
        None => UrlSource::Prefix(url_prefix?),
    };

    Some(Args {
        source,
        count,
        concurrency,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Some(a) => a,
        None => {
            eprintln!(
                "usage: bench_handshake <rtmp_url_prefix> [--count N] [--concurrency C]\n   \
                    or: bench_handshake --url-list <path> [--count N] [--concurrency C]"
            );
            return ExitCode::from(1);
        }
    };

    let label = match &args.source {
        UrlSource::Prefix(p) => format!("{p}-*"),
        UrlSource::List(urls) => format!("--url-list ({} urls)", urls.len()),
    };
    let urls = &args.source;

    let next_seq = AtomicU64::new(0);
    let latencies_ms: Mutex<Vec<f64>> = Mutex::new(Vec::with_capacity(args.count));
    let failures = AtomicU64::new(0);

    let wall_start = Instant::now();

    // Simple worker-pool: `concurrency` threads pull from a shared counter
    // until `count` handshakes have been attempted.
    thread::scope(|scope| {
        for _ in 0..args.concurrency {
            let next_seq = &next_seq;
            let latencies_ms = &latencies_ms;
            let failures = &failures;
            let urls = &urls;
            let count = args.count;
            scope.spawn(move || {
                loop {
                    let n = next_seq.fetch_add(1, Ordering::Relaxed);
                    if n >= count as u64 {
                        break;
                    }
                    let url = match urls {
                        UrlSource::Prefix(prefix) => format!("{prefix}-{n}"),
                        UrlSource::List(list) => list[n as usize % list.len()].clone(),
                    };
                    let start = Instant::now();
                    let mut client = Client::new();
                    client.set_connect_timeout(Duration::from_secs(10));
                    let ok = client.connect(&url).is_ok() && client.publish().is_ok();
                    if ok {
                        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                        latencies_ms.lock().unwrap().push(elapsed_ms);
                    } else {
                        failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });

    let wall_elapsed = wall_start.elapsed().as_secs_f64();
    let mut latencies = latencies_ms.into_inner().unwrap();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let ok_count = latencies.len();
    let fail_count = failures.load(Ordering::Relaxed);
    let avg = if ok_count > 0 {
        latencies.iter().sum::<f64>() / ok_count as f64
    } else {
        0.0
    };

    println!(
        "url={label} count={} concurrency={}",
        args.count, args.concurrency
    );
    println!(
        "ok={ok_count} failed={fail_count} success_rate={:.1}% wall_time_s={:.2} handshakes_per_s={:.1}",
        100.0 * ok_count as f64 / args.count as f64,
        wall_elapsed,
        ok_count as f64 / wall_elapsed.max(1e-9),
    );
    println!(
        "connect+publish latency ms: avg={:.2} p50={:.2} p95={:.2} p99={:.2} max={:.2}",
        avg,
        bench_common::percentile(&latencies, 0.50),
        bench_common::percentile(&latencies, 0.95),
        bench_common::percentile(&latencies, 0.99),
        latencies.last().copied().unwrap_or(0.0),
    );

    if fail_count > 0 && ok_count == 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}
