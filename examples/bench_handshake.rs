//! Benchmark helper: RTMP connect + publish (or connect + play) handshake
//! latency.
//!
//! Measures wall-clock time from `connect()` through a successful `publish()`
//! (i.e. up to `NetStream.Publish.Start`) against any RTMP server -- this
//! server, or a third-party one (nginx-rtmp, MediaMTX, ...) -- since the
//! handshake is pure RTMP protocol and carries no media payload. With
//! `--play` it measures `connect()` through `play()` (up to
//! `NetStream.Play.Start`) instead; point it at a stream that is already
//! live, since some servers refuse to play a stream nobody publishes.
//!
//! Runs `--concurrency` clients at a time, `--count` total, and reports
//! success rate plus latency percentiles.
//!
//! Two ways to supply the per-attempt URLs:
//!   - `<rtmp_url_prefix> --count N`: each attempt appends `-<n>` to the
//!     prefix (`<prefix>-0`, `<prefix>-1`, ...) so it needs an app that
//!     accepts arbitrary stream names (e.g. nginx-rtmp, MediaMTX). With
//!     `--play` every attempt uses the URL as given, so all players join
//!     the same live stream.
//!   - `--url-list <path> --count N`: reads one full RTMP URL per line and
//!     round-robins the pool over them -- for servers (like librtmp2-server)
//!     that validate the stream key against a pre-provisioned list, generate
//!     the file from that server's own API first.
//!
//! Usage:
//!   bench_handshake <rtmp_url_prefix> [--count N] [--concurrency C] [--play]
//!   bench_handshake --url-list urls.txt [--count N] [--concurrency C] [--play]
//!
//! Example:
//!   bench_handshake rtmp://127.0.0.1:1936/live/bench --count 200 --concurrency 50
//!   bench_handshake rtmp://127.0.0.1:1936/live/source --count 200 --concurrency 50 --play

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
    play: bool,
}

fn parse_args() -> Option<Args> {
    let raw: Vec<String> = env::args().skip(1).collect();
    parse_args_from(&raw)
}

fn parse_args_from(raw: &[String]) -> Option<Args> {
    let mut url_prefix = None;
    let mut url_list_path = None;
    let mut count = 200usize;
    let mut concurrency = 50usize;
    let mut play = false;
    let mut seen_count = false;
    let mut seen_concurrency = false;
    let mut seen_play = false;
    let mut seen_url_list = false;
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--count" => {
                seen_count = take_flag_once(seen_count)?;
                count = take_numeric_value(raw, i)?;
                i += 2;
            }
            "--concurrency" => {
                seen_concurrency = take_flag_once(seen_concurrency)?;
                concurrency = take_numeric_value(raw, i)?;
                i += 2;
            }
            "--play" => {
                seen_play = take_flag_once(seen_play)?;
                play = true;
                i += 1;
            }
            "--url-list" => {
                seen_url_list = take_flag_once(seen_url_list)?;
                url_list_path = Some(take_text_value(raw, i)?.to_string());
                i += 2;
            }
            other => {
                // Only one positional argument (the RTMP URL prefix) is
                // valid; an unknown flag must not silently become the URL,
                // which made `--duration 5` benchmark the literal host "5".
                if other.starts_with('-') || url_prefix.is_some() {
                    eprintln!("unknown or unexpected argument: {other}");
                    return None;
                }
                url_prefix = Some(other.to_string());
                i += 1;
            }
        }
    }

    if url_prefix.is_some() && url_list_path.is_some() {
        eprintln!("a positional URL cannot be combined with --url-list");
        return None;
    }

    // A degenerate `--count`/`--concurrency` runs zero handshakes, which made
    // the success-rate line divide by zero (`NaN%`) and the exit gate report
    // success for a benchmark that measured nothing.
    if count == 0 || concurrency == 0 {
        return None;
    }

    let source = match url_list_path {
        Some(path) => UrlSource::List(bench_common::read_url_list(&path)?),
        None => UrlSource::Prefix(url_prefix?),
    };

    Some(Args {
        source,
        count,
        concurrency,
        play,
    })
}

/// Mark a flag as supplied; `None` means it was already seen.
fn take_flag_once(seen: bool) -> Option<bool> {
    (!seen).then_some(true)
}

/// Return the token after `i` when it is a value rather than another flag.
fn take_text_value(raw: &[String], i: usize) -> Option<&str> {
    let value = raw.get(i + 1)?;
    (!value.starts_with('-')).then_some(value.as_str())
}

/// Parse the numeric token after `i`, rejecting a missing or flag-like value.
fn take_numeric_value(raw: &[String], i: usize) -> Option<usize> {
    take_text_value(raw, i)?.parse().ok()
}

/// URL for attempt `n`: prefix mode appends `-<n>` for publishes and reuses
/// the prefix as-is for plays; list mode round-robins over the list.
fn attempt_url(source: &UrlSource, n: u64, play: bool) -> String {
    match source {
        UrlSource::Prefix(prefix) if play => prefix.clone(),
        UrlSource::Prefix(prefix) => format!("{prefix}-{n}"),
        UrlSource::List(list) => list[n as usize % list.len()].clone(),
    }
}

/// One connect + publish (or play) handshake; returns its latency in ms, or
/// `None` if any step failed.
fn attempt(url: &str, play: bool) -> Option<f64> {
    let start = Instant::now();
    let mut client = Client::new();
    client.set_connect_timeout(Duration::from_secs(10));
    client.connect(url).ok()?;
    if play {
        client.play().ok()?;
    } else {
        client.publish().ok()?;
    }
    Some(start.elapsed().as_secs_f64() * 1000.0)
}

/// Simple worker pool: `concurrency` threads pull from a shared counter
/// until `count` handshakes have been attempted. Returns the successful
/// latencies (unsorted) and the number of failures.
fn run_handshakes(args: &Args) -> (Vec<f64>, u64) {
    let next_seq = AtomicU64::new(0);
    let latencies_ms: Mutex<Vec<f64>> = Mutex::new(Vec::with_capacity(args.count));
    let failures = AtomicU64::new(0);

    thread::scope(|scope| {
        for _ in 0..args.concurrency {
            scope.spawn(|| {
                loop {
                    let n = next_seq.fetch_add(1, Ordering::Relaxed);
                    if n >= args.count as u64 {
                        break;
                    }
                    match attempt(&attempt_url(&args.source, n, args.play), args.play) {
                        Some(ms) => latencies_ms.lock().unwrap().push(ms),
                        None => {
                            failures.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });

    (
        latencies_ms.into_inner().unwrap(),
        failures.load(Ordering::Relaxed),
    )
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Some(a) => a,
        None => {
            eprintln!(
                "usage: bench_handshake <rtmp_url_prefix> [--count N] [--concurrency C] [--play]\n   \
                    or: bench_handshake --url-list <path> [--count N] [--concurrency C] [--play]"
            );
            return ExitCode::from(1);
        }
    };

    let play = args.play;
    let label = match &args.source {
        UrlSource::Prefix(p) if play => p.clone(),
        UrlSource::Prefix(p) => format!("{p}-*"),
        UrlSource::List(urls) => format!("--url-list ({} urls)", urls.len()),
    };
    let wall_start = Instant::now();
    let (mut latencies, fail_count) = run_handshakes(&args);
    let wall_elapsed = wall_start.elapsed().as_secs_f64();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let ok_count = latencies.len();
    let avg = if ok_count > 0 {
        latencies.iter().sum::<f64>() / ok_count as f64
    } else {
        0.0
    };

    println!(
        "url={label} count={} concurrency={} mode={}",
        args.count,
        args.concurrency,
        if play { "play" } else { "publish" }
    );
    println!(
        "ok={ok_count} failed={fail_count} success_rate={:.1}% wall_time_s={:.2} handshakes_per_s={:.1}",
        100.0 * ok_count as f64 / args.count as f64,
        wall_elapsed,
        ok_count as f64 / wall_elapsed.max(1e-9),
    );
    println!(
        "connect+{} latency ms: avg={:.2} p50={:.2} p95={:.2} p99={:.2} max={:.2}",
        if play { "play" } else { "publish" },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(raw: &[&str]) -> Option<Args> {
        parse_args_from(&raw.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn unknown_flag_is_rejected_instead_of_becoming_the_url() {
        assert!(args(&["--duration", "5", "rtmp://127.0.0.1/live"]).is_none());
        assert!(args(&["--concurrency=5"]).is_none());
    }

    #[test]
    fn second_positional_is_rejected() {
        assert!(args(&["rtmp://a/live", "rtmp://b/live"]).is_none());
    }

    #[test]
    fn flag_value_must_not_be_parsed_as_another_flag() {
        assert!(args(&["--count", "--play", "rtmp://a/live"]).is_none());
    }

    #[test]
    fn repeated_options_are_rejected() {
        assert!(args(&["--count", "10", "--count", "100", "rtmp://a/live"]).is_none());
        assert!(args(&["--concurrency", "2", "--concurrency", "3", "rtmp://a/live"]).is_none());
        assert!(args(&["--play", "--play", "rtmp://a/live"]).is_none());
    }

    #[test]
    fn positional_url_and_url_list_cannot_be_combined() {
        let listed_path = std::env::temp_dir().join("bench_handshake_conflict_urls.txt");
        std::fs::write(&listed_path, "rtmp://a/live-1\n").expect("write url list");
        let path = listed_path.to_str().expect("utf-8 temp path");
        assert!(args(&["rtmp://a/live", "--url-list", path]).is_none());
        assert!(args(&["--url-list", path, "rtmp://a/live"]).is_none());
        let _ = std::fs::remove_file(&listed_path);
    }

    #[test]
    fn documented_forms_still_parse() {
        let parsed = args(&[
            "--count",
            "10",
            "--concurrency",
            "2",
            "--play",
            "rtmp://a/live",
        ])
        .expect("documented arguments must parse");
        assert_eq!(parsed.count, 10);
        assert_eq!(parsed.concurrency, 2);
        assert!(parsed.play);
        assert!(matches!(parsed.source, UrlSource::Prefix(ref p) if p == "rtmp://a/live"));

        let listed_path = std::env::temp_dir().join("bench_handshake_test_urls.txt");
        std::fs::write(&listed_path, "rtmp://a/live-1\n").expect("write url list");
        let listed = args(&["--url-list", listed_path.to_str().expect("utf-8 temp path")])
            .expect("url-list must parse");
        assert!(matches!(listed.source, UrlSource::List(_)));
        let _ = std::fs::remove_file(&listed_path);
    }
}
