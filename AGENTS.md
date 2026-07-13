# AGENTS.md

## Cursor Cloud specific instructions

`librtmp2` is a pure Rust RTMP/E-RTMP protocol library (no services to run). Standard build/test/lint commands are in `README.md` and `CLAUDE.md` — use those.

Non-obvious notes for this environment:

- **Toolchain:** requires a recent stable Rust (`edition = "2024"`, `rust-version = "1.93"` in `Cargo.toml`). The VM snapshot ships stable Rust 1.97 as the default `rustup` toolchain; the system default Rust (1.83) is too old, so do not switch back to it.
- **TLS/OpenSSL:** the default `tls` feature links system OpenSSL via `openssl-sys`, which needs `pkg-config` + `libssl-dev` (already installed in the snapshot). For a zero-system-dependency build use `cargo build --no-default-features`.
- `Cargo.lock` is intentionally git-ignored (this is a library crate).
- **Interop scripts** under `tests/interop/` are standalone (not part of `cargo test`) and need a real `ffmpeg` (present in the snapshot).
- This crate is published to crates.io; `librtmp2-server` depends on the **published** `librtmp2` version, not this working copy, so local edits here do not affect a sibling server build unless it is patched to a path dependency.
