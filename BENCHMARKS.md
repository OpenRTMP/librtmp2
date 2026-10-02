# Benchmarks

This document covers two things:

1. **Component microbenchmarks** for `librtmp2` itself (chunk codec, AMF0,
   FLV tag parsing, handshake, in-process publish→player relay), run with
   [Criterion](https://github.com/bheisler/criterion.rs) via `cargo bench`.
2. **`examples/bench_handshake.rs`** and **`examples/bench_relay.rs`**: two
   small standalone tools built on `librtmp2::client::Client` that speak
   plain RTMP over the wire, so they work against *any* RTMP server, not
   just this crate's own. They're what
   [`librtmp2-server`'s `BENCHMARKS.md`](https://github.com/OpenRTMP/librtmp2-server/blob/main/BENCHMARKS.md)
   uses to compare `librtmp2-server` against nginx-rtmp, MediaMTX, SRS and
   LiveForge on equal terms (same client, same wire protocol, same load).

All numbers below are from one run on one machine and are meant to be
**reproduced**, not quoted as guarantees — see "Environment" for the exact
hardware/software and rerun the commands yourself before relying on them.

## Reproducing the microbenchmarks

```bash
cargo bench --bench protocol
cargo bench --bench relay
```

Criterion writes full HTML reports to `target/criterion/report/index.html`.

## Environment for the numbers below

- CPU: Intel Xeon @ 2.10GHz, 4 vCPUs (a shared VM, not bare metal — treat
  absolute numbers as illustrative and re-run on your own target hardware
  for capacity planning)
- RAM: 15 GiB
- Kernel: Linux 6.18 x86_64
- rustc 1.95.0, `cargo build --release` (`tls` feature enabled, default)
- librtmp2 0.10.2
- Date: 2026-09-28

## `protocol` benchmarks (`benches/protocol.rs`)

Pure in-memory codec work — no sockets, no allocator warm-up beyond the
first iteration.

| Benchmark | Time | Throughput |
|---|---|---|
| `chunk/write_read_roundtrip` (4096-byte payload, chunk size 128) | 3.76 µs | ~1040 MiB/s |
| `amf0_build_connect` | 229 ns | — |
| `flv/video_tag_h264` (parse) | 1.10 ns | — |
| `flv/audio_tag_aac` (parse) | 1.15 ns | — |
| `fourcc_to_video_codec_avc1` | 2.21 ns | — |
| `server_read_c1` (handshake C1 parse + S0/S1/S2 build) | 1.08 µs | — |

## `relay` benchmark (`benches/relay.rs`)

End-to-end, in-process: one `librtmp2::server::Server` and two
`librtmp2::client::Client`s (publisher + player) on loopback TCP in the same
process, publishing N 256-byte video frames and waiting for all of them to
arrive at the player. This exercises the full stack (handshake → chunking →
message dispatch → relay → chunking → handshake) but is bottlenecked by
`std::thread::sleep`-based polling intervals in the harness itself, not by
the library — read the *shape*, not the absolute latency, and see
`bench_relay` below for a throughput-oriented, non-blocking version of the
same idea against a real server.

| Benchmark | Time | Throughput |
|---|---|---|
| `relay/publish_to_player/100` (100 frames) | 99.0 ms | ~1010 elem/s |
| `relay/publish_to_player/500` (500 frames) | 101.0 ms | ~4950 elem/s |

Both sizes land at roughly the same wall-clock time regardless of frame
count, which is the harness's own fixed polling-interval overhead
dominating (see above), not a per-frame cost — the throughput column is
the number worth comparing across frame counts here, not the time column.

Because of that harness overhead this benchmark does not show the relay
changes since 0.10.0 (frames chunked once per fan-out and sent to every
player without a per-player copy, compact chunk headers, 4096-byte client
publish chunks, per-player flow control). For end-to-end numbers see the
cross-server comparison in `librtmp2-server`'s `BENCHMARKS.md`: with
`librtmp2-server` 0.6.0 built on librtmp2 0.10.2, 100 concurrent viewers
joined in 2.9 ms on average (p95 6.5 ms), ahead of MediaMTX (5.5 /
10.9 ms) and LiveForge (13.0 / 26.2 ms), with every viewer receiving the
full frame rate; SRS 8.0 and nginx-rtmp took 73 and 90 ms on average for
the same join.
Players connect faster too (connect + play in 1.95 ms on average, against
3.0 ms for LiveForge and 4.6 ms for MediaMTX), and 1000 concurrent viewers
of one stream all received the full frame rate while the server used 60%
of one core and 32 MiB of memory.
That is a whole-system result: it includes that server's own changes
(multi-core sharding, auth wake-ups, publishes and plays answered from an
in-memory key cache) and does not isolate the effect of
any single library change; the full-frame-rate run also never congests a
player, so it does not exercise flow control.

## `examples/bench_handshake.rs` and `examples/bench_relay.rs`

These are not Criterion benches; they're small CLI tools meant to be pointed
at a *running* RTMP server (this crate's own test server, `librtmp2-server`,
or a third-party server like nginx-rtmp or MediaMTX) to measure real
network-facing behavior:

- **`bench_handshake`** — connect + publish handshake latency (time to
  `NetStream.Publish.Start`) under configurable concurrency. Pure protocol,
  no media, so it's directly comparable across implementations. With
  `--play` it measures connect + play instead (time to
  `NetStream.Play.Start`) against a stream that is already live.
- **`bench_relay`** — points N concurrent `play()` clients at one already-
  live stream and reports join latency (connect → first frame) and
  steady-state relay throughput/frame rate per player. Besides the
  human-readable lines it prints one machine-readable
  `delivered: viewers=.. aggregate_gbps=.. steady_frames_per_viewer=..` line
  that `librtmp2-server`'s `scripts/run_rtmp_benchmarks.sh` uses to report
  CPU per delivered Gbit/s.

```bash
cargo build --release --example bench_handshake --example bench_relay

# Handshake latency against a permissive server (arbitrary stream names):
./target/release/examples/bench_handshake rtmp://127.0.0.1:1935/live/bench --count 200 --concurrency 50

# Same, against a server that validates stream keys from a fixed list:
./target/release/examples/bench_handshake --url-list urls.txt --count 200 --concurrency 50

# Connect + play handshake latency (stream must already be publishing):
./target/release/examples/bench_handshake rtmp://127.0.0.1:1935/live/bench --count 200 --concurrency 50 --play

# Concurrent-viewer relay throughput (stream must already be publishing):
./target/release/examples/bench_relay rtmp://127.0.0.1:1935/live/bench --players 100 --run-secs 20
```

Each `bench_relay` viewer is a thread on the benchmark host, so very high
viewer counts (thousands) compete with the server for CPU unless the viewers
run on another machine. `librtmp2-server`'s `BENCHMARKS.md` has the measured
effect of the 0.10.x fan-out allocation/sort changes (no regression, CPU
within run-to-run noise: the cost is dominated by the kernel TCP path) and a
`perf` profile.

See `librtmp2-server`'s `BENCHMARKS.md` for full cross-server results
produced with these two tools, including the exact server configs used and
a caveat found along the way (nginx-rtmp's live relay state is per
worker-process, so multi-worker nginx-rtmp needs sticky publisher/viewer
routing to work at all).
