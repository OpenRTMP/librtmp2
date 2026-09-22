//! Shared helpers for the `bench_handshake` and `bench_relay` example tools.
//!
//! Lives in a subdirectory (rather than `examples/bench_common.rs`) so
//! Cargo's example auto-discovery doesn't also try to build it as its own
//! binary; each tool pulls it in with `#[path = "bench_common/mod.rs"] mod
//! bench_common;`.

use std::fs;

/// Reads one URL per line from `path`, skipping blank lines. `None` if the
/// file can't be read or has no non-blank lines.
pub fn read_url_list(path: &str) -> Option<Vec<String>> {
    let text = fs::read_to_string(path).ok()?;
    let urls: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if urls.is_empty() { None } else { Some(urls) }
}

/// Nearest-rank percentile (`p` in `[0, 1]`) of an already-sorted slice.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}
