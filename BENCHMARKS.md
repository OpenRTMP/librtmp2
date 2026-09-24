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
   uses to compare `librtmp2-server` against nginx-rtmp and MediaMTX on
   equal terms (same client, same wire protocol, same load).

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

- CPU: Intel Xeon @ 2.80GHz, 4 vCPUs (a shared VM, not bare metal — treat
  absolute numbers as illustrative and re-run on your own target hardware
  for capacity planning)
- RAM: 15 GiB
- Kernel: Linux 6.18 x86_64
- rustc 1.95.0, `cargo build --release` (`tls` feature enabled, default)
- librtmp2 0.10.0
- Date: 2026-09-25

## `protocol` benchmarks (`benches/protocol.rs`)

Pure in-memory codec work — no sockets, no allocator warm-up beyond the
first iteration.

| Benchmark | Time | Throughput |
|---|---|---|
| `chunk/write_read_roundtrip` (4096-byte payload, chunk size 128) | 5.33 µs | ~733 MiB/s |
| `amf0_build_connect` | 318 ns | — |
| `flv/video_tag_h264` (parse) | 1.97 ns | — |
| `flv/audio_tag_aac` (parse) | 1.59 ns | — |
| `fourcc_to_video_codec_avc1` | 3.77 ns | — |
| `server_read_c1` (handshake C1 parse + S0/S1/S2 build) | 1.45 µs | — |

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
| `relay/publish_to_player/100` (100 frames) | 98.8 ms | ~1013 elem/s |
| `relay/publish_to_player/500` (500 frames) | 100.0 ms | ~5002 elem/s |

Both sizes land at roughly the same wall-clock time regardless of frame
count, which is the harness's own fixed polling-interval overhead
dominating (see above), not a per-frame cost — the throughput column is
the number worth comparing across frame counts here, not the time column.

Because of that harness overhead this benchmark does not show the 0.10.0
relay changes (frames chunked once per fan-out, compact chunk headers,
4096-byte client publish chunks, per-player flow control). For end-to-end
numbers see the cross-server comparison in `librtmp2-server`'s
`BENCHMARKS.md` as of
[OpenRTMP/librtmp2-server#243](https://github.com/OpenRTMP/librtmp2-server/pull/243)
(its `main` still has older numbers): with `librtmp2-server` 0.5.0 built on
librtmp2 0.10.0, 100 concurrent viewers joined in 11.9 ms on average (p95
23.2 ms), ahead of MediaMTX (14.5 / 26.1 ms) and LiveForge (24.8 /
43.4 ms) on the same box, with every viewer receiving the full frame rate.
That is a whole-system result: it includes that server's own changes
(multi-core sharding, auth wake-ups) and does not isolate the effect of
any single library change; the full-frame-rate run also never congests a
player, so it does not exercise flow control.

## `examples/bench_handshake.rs` and `examples/bench_relay.rs`

These are not Criterion benches; they're small CLI tools meant to be pointed
at a *running* RTMP server (this crate's own test server, `librtmp2-server`,
or a third-party server like nginx-rtmp or MediaMTX) to measure real
network-facing behavior:

- **`bench_handshake`** — connect + publish handshake latency (time to
  `NetStream.Publish.Start`) under configurable concurrency. Pure protocol,
  no media, so it's directly comparable across implementations.
- **`bench_relay`** — points N concurrent `play()` clients at one already-
  live stream and reports join latency (connect → first frame) and
  steady-state relay throughput/frame rate per player.

```bash
cargo build --release --example bench_handshake --example bench_relay

# Handshake latency against a permissive server (arbitrary stream names):
./target/release/examples/bench_handshake rtmp://127.0.0.1:1935/live/bench --count 200 --concurrency 50

# Same, against a server that validates stream keys from a fixed list:
./target/release/examples/bench_handshake --url-list urls.txt --count 200 --concurrency 50

# Concurrent-viewer relay throughput (stream must already be publishing):
./target/release/examples/bench_relay rtmp://127.0.0.1:1935/live/bench --players 100 --run-secs 20
```

See `librtmp2-server`'s `BENCHMARKS.md` for full cross-server results
produced with these two tools, including the exact server configs used and
a caveat found along the way (nginx-rtmp's live relay state is per
worker-process, so multi-worker nginx-rtmp needs sticky publisher/viewer
routing to work at all).
