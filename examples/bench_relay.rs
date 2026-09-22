//! Benchmark helper: concurrent-viewer relay throughput and join latency.
//!
//! Spins up `--players` concurrent RTMP `play()` clients against a single
//! already-live stream (fed by a real publisher such as ffmpeg) and measures,
//! per player: time-to-first-frame ("join latency") and frames/bytes
//! received in a steady-state window (after `--warmup-ms`, to exclude the
//! initial GOP/burst). Works against any RTMP server -- this crate's own
//! `librtmp2-server`, nginx-rtmp, MediaMTX, etc. -- since it only uses the
//! public RTMP wire protocol via `librtmp2::client::Client`.
//!
//! All players can share one URL (`<rtmp_url>`), or, for a server that caps
//! concurrent connections per stream key (e.g. `librtmp2-server` allows at
//! most 5 live connections per `play_key` by design), round-robin across
//! several distinct viewer URLs to the *same* underlying stream via
//! `--url-list <path>` (one URL per line; provision as many keys as you need
//! `players` connections for through that server's own API first).
//!
//! Usage:
//!   bench_relay <rtmp_url> [--players N] [--run-secs S] [--warmup-ms MS]
//!   bench_relay --url-list <path> [--players N] [--run-secs S] [--warmup-ms MS]
//!
//! Example (stream must already be publishing before this runs):
//!   bench_relay rtmp://127.0.0.1:1935/live/bench --players 100 --run-secs 20

#[path = "bench_common/mod.rs"]
mod bench_common;

use std::cell::RefCell;
use std::env;
use std::process::ExitCode;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use librtmp2::client::Client;
use librtmp2::types::Frame;

enum UrlSource {
    Single(String),
    List(Vec<String>),
}

impl UrlSource {
    fn pick(&self, idx: usize) -> &str {
        match self {
            UrlSource::Single(u) => u,
            UrlSource::List(list) => &list[idx % list.len()],
        }
    }

    fn label(&self) -> String {
        match self {
            UrlSource::Single(u) => u.clone(),
            UrlSource::List(list) => format!("--url-list ({} urls)", list.len()),
        }
    }
}

struct Args {
    source: UrlSource,
    players: usize,
    run_secs: u64,
    warmup_ms: u64,
}

fn parse_args() -> Option<Args> {
    let raw: Vec<String> = env::args().skip(1).collect();
    let mut url = None;
    let mut url_list_path = None;
    let mut players = 50usize;
    let mut run_secs = 20u64;
    let mut warmup_ms = 2000u64;
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--players" => {
                players = raw.get(i + 1)?.parse().ok()?;
                i += 2;
            }
            "--run-secs" => {
                run_secs = raw.get(i + 1)?.parse().ok()?;
                i += 2;
            }
            "--warmup-ms" => {
                warmup_ms = raw.get(i + 1)?.parse().ok()?;
                i += 2;
            }
            "--url-list" => {
                url_list_path = Some(raw.get(i + 1)?.clone());
                i += 2;
            }
            other => {
                url = Some(other.to_string());
                i += 1;
            }
        }
    }

    let source = match url_list_path {
        Some(path) => UrlSource::List(bench_common::read_url_list(&path)?),
        None => UrlSource::Single(url?),
    };

    Some(Args {
        source,
        players,
        run_secs,
        warmup_ms,
    })
}

#[derive(Default, Clone, Copy)]
struct ThreadState {
    connect_start_secs: f64,
    first_frame_at_secs: Option<f64>,
    steady_start_secs: Option<f64>,
    frames_total: u64,
    bytes_total: u64,
    frames_steady: u64,
    bytes_steady: u64,
}

thread_local! {
    static STATE: RefCell<ThreadState> = RefCell::new(ThreadState::default());
    static EPOCH: RefCell<Option<Instant>> = const { RefCell::new(None) };
    static WARMUP: RefCell<Duration> = const { RefCell::new(Duration::ZERO) };
}

fn now_secs() -> f64 {
    EPOCH.with(|e| e.borrow().unwrap().elapsed().as_secs_f64())
}

fn on_frame(frame: &Frame) {
    let now = now_secs();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.first_frame_at_secs.is_none() {
            s.first_frame_at_secs = Some(now);
            let warmup = WARMUP.with(|w| *w.borrow()).as_secs_f64();
            s.steady_start_secs = Some(now + warmup);
        }
        s.frames_total += 1;
        s.bytes_total += frame.size as u64;
        if let Some(steady_start) = s.steady_start_secs
            && now >= steady_start
        {
            s.frames_steady += 1;
            s.bytes_steady += frame.size as u64;
        }
    });
}

struct PlayerResult {
    connected: bool,
    played: bool,
    join_latency_ms: Option<f64>,
    frames_total: u64,
    bytes_total: u64,
    frames_steady: u64,
    bytes_steady: u64,
    steady_window_secs: f64,
}

fn run_player(url: String, run_secs: u64, warmup: Duration, epoch: Instant) -> PlayerResult {
    EPOCH.with(|e| *e.borrow_mut() = Some(epoch));
    WARMUP.with(|w| *w.borrow_mut() = warmup);
    STATE.with(|s| *s.borrow_mut() = ThreadState::default());

    let connect_start = now_secs();
    STATE.with(|s| s.borrow_mut().connect_start_secs = connect_start);

    let mut client = Client::new();
    client.on_frame_cb = Some(on_frame);
    client.set_connect_timeout(Duration::from_secs(10));

    let connected = client.connect(&url).is_ok();
    let played = connected && client.play().is_ok();

    if played {
        let deadline = Instant::now() + Duration::from_secs(run_secs);
        while Instant::now() < deadline {
            let _ = client.poll(50);
        }
    }

    STATE.with(|s| {
        let s = s.borrow();
        let steady_window_secs = match s.steady_start_secs {
            Some(steady_start) => (now_secs() - steady_start).max(0.0),
            None => 0.0,
        };
        PlayerResult {
            connected,
            played,
            join_latency_ms: s
                .first_frame_at_secs
                .map(|t| (t - s.connect_start_secs) * 1000.0),
            frames_total: s.frames_total,
            bytes_total: s.bytes_total,
            frames_steady: s.frames_steady,
            bytes_steady: s.bytes_steady,
            steady_window_secs,
        }
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Some(a) => a,
        None => {
            eprintln!(
                "usage: bench_relay <rtmp_url> [--players N] [--run-secs S] [--warmup-ms MS]\n   \
                    or: bench_relay --url-list <path> [--players N] [--run-secs S] [--warmup-ms MS]\n\
                 The stream(s) must already be publishing (e.g. via ffmpeg) before this runs."
            );
            return ExitCode::from(1);
        }
    };

    let epoch = Instant::now();
    let warmup = Duration::from_millis(args.warmup_ms);
    let results: Mutex<Vec<PlayerResult>> = Mutex::new(Vec::with_capacity(args.players));
    let url_label = args.source.label();

    thread::scope(|scope| {
        for i in 0..args.players {
            let url = args.source.pick(i).to_string();
            let results = &results;
            let run_secs = args.run_secs;
            scope.spawn(move || {
                let r = run_player(url, run_secs, warmup, epoch);
                results.lock().unwrap().push(r);
            });
        }
    });

    let results = results.into_inner().unwrap();
    let total = results.len();
    let connected = results.iter().filter(|r| r.connected).count();
    let played = results.iter().filter(|r| r.played).count();
    let received_any = results.iter().filter(|r| r.frames_total > 0).count();

    let mut join_ms: Vec<f64> = results.iter().filter_map(|r| r.join_latency_ms).collect();
    join_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let sum_frames_steady: u64 = results.iter().map(|r| r.frames_steady).sum();
    let sum_bytes_steady: u64 = results.iter().map(|r| r.bytes_steady).sum();
    let sum_frames_total: u64 = results.iter().map(|r| r.frames_total).sum();
    let sum_bytes_total: u64 = results.iter().map(|r| r.bytes_total).sum();
    let avg_steady_window = if received_any > 0 {
        results.iter().map(|r| r.steady_window_secs).sum::<f64>() / received_any as f64
    } else {
        0.0
    };

    println!(
        "url={} players={} run_secs={} warmup_ms={}",
        url_label, args.players, args.run_secs, args.warmup_ms
    );
    println!(
        "connected={connected}/{total} played={played}/{total} received_frames={received_any}/{total}"
    );
    println!(
        "join latency ms (connect -> first frame): avg={:.2} p50={:.2} p95={:.2} p99={:.2} max={:.2}",
        join_ms.iter().sum::<f64>() / join_ms.len().max(1) as f64,
        bench_common::percentile(&join_ms, 0.50),
        bench_common::percentile(&join_ms, 0.95),
        bench_common::percentile(&join_ms, 0.99),
        join_ms.last().copied().unwrap_or(0.0),
    );
    println!(
        "totals: frames={sum_frames_total} bytes={sum_bytes_total} ({:.2} MiB)",
        sum_bytes_total as f64 / (1024.0 * 1024.0)
    );
    println!(
        "steady-state (post warmup, avg window {:.1}s/player): frames={sum_frames_steady} \
         aggregate_throughput={:.2} Mbps avg_fps_per_player={:.2}",
        avg_steady_window,
        (sum_bytes_steady as f64 * 8.0) / avg_steady_window.max(1e-9) / 1_000_000.0,
        sum_frames_steady as f64 / avg_steady_window.max(1e-9) / received_any.max(1) as f64,
    );

    if received_any == 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}
