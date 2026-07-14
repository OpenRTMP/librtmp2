# CONTRIBUTING

This project follows the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) specification for commit messages. All commits must use English language for messages, even though the repository requires UTF-8 encoding.

## Code Contribution Guidelines
- **API Compatibility**: No breaking changes to the `extern "C"` FFI surface exported from `src/lib.rs` unless accompanied by a major version bump. For non-breaking changes that modify internals, everything outside `src/lib.rs`'s FFI exports is free to change.
- **Testing**: All new features must be covered by unit tests (inline `#[cfg(test)] mod tests` next to the code they cover) and, where applicable, an end-to-end test in `tests/`
- **ABI Stability**: See `docs/abi-policy.md`. Only change the `#[no_mangle] pub extern "C"` functions and `#[repr(C)]` types in `src/lib.rs` with a major version bump.
- **Documentation**: Update `CLAUDE.md`, `docs/abi-policy.md`, and `docs/protocol-mapping-*` files when modifying API or protocol behavior
- **Parser safety**: Never trust network-provided length fields; all parsers must be bounds-checked

## Build Guidelines
```bash
cargo build                       # debug build
cargo build --release             # release build
cargo test                        # unit tests + tests/server_client_loopback.rs
cargo clippy --all-features --all-targets
cargo fmt
```

TLS/RTMPS is enabled by default via the `tls` feature (OpenSSL). Use
`cargo build --no-default-features` for a zero-dependency, plaintext-only
build. Before any release, run `scripts/abi-baseline.sh compare` against the
previous release tag — see `docs/abi-policy.md` for the full checklist.

### Parser fuzzing (libFuzzer)

Requires nightly Rust and `cargo-fuzz`:

```bash
rustup toolchain install nightly
cargo install cargo-fuzz --locked
cargo fuzz run chunk_read -- -max_total_time=30
```

Targets: `chunk_read`, `handshake_server`, `amf0_skip`, `ertmp_parse`,
`control_decode`.

### Sanitizer builds (Linux)

AddressSanitizer (nightly + `-Zbuild-std`):

```bash
RUSTFLAGS="-Zsanitizer=address" cargo +nightly test --lib -Zbuild-std \
  --target x86_64-unknown-linux-gnu --all-features
```

Integer overflow checks (stable):

```bash
RUSTFLAGS="-C overflow-checks=on" cargo test --lib --all-features
```

CI runs ASan via `.github/workflows/sanitizers.yml`. Full UBSan
(`-Zsanitizer=undefined`) is no longer supported by current `rustc` on Linux;
overflow checks cover the most common arithmetic UB in safe Rust tests.

## License
Source code is under the [MIT license](LICENSE). Additional assets have their own license information.
