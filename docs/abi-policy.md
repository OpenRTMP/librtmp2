# ABI Stability Policy

## Versioning Rules

librtmp2 follows [Semantic Versioning](https://semver.org/):

- **Patch** (`0.x.Z`): Bug fixes only. No API/ABI changes. Always safe to upgrade.
- **Minor** (`0.X.0`): New functions, types, or enum values. Existing symbols unchanged. Safe to re-link.
- **Major** (`X.0.0`): Breaking API/ABI changes. Not expected before `1.0.0`.

`librtmp2` is a Rust crate (`cdylib` + `staticlib` + `rlib`, see `Cargo.toml`).
There is no `include/*.h` in this repo — the ABI boundary is the set of
`#[no_mangle] pub extern "C"` functions and `#[repr(C)]` types exported from
`src/lib.rs`, for consumption from C, Go, Python, PHP, and other FFI callers.

## ABI Guarantees

### Stable (guaranteed ABI-compatible across minor/patch releases)

- Signatures of `#[no_mangle] pub extern "C"` functions in `src/lib.rs`
  (parameter types, return types)
- Layout of `#[repr(C)]` structs used across the FFI boundary (e.g. `Frame`)
- Values of `#[repr(C)]` enums used across the FFI boundary (codec IDs, error
  codes)

### May Change

- Any Rust-only (non-`extern "C"`) function signature or struct layout
- Internal module structure under `src/`
- Opaque connection/server/client types passed only as pointers across FFI
- Configuration struct field order (new fields may be appended)

### Symbol Visibility

- Only functions marked `#[no_mangle] pub extern "C"` are exported from the
  `cdylib`/`staticlib` build artifacts.
- Everything else in `src/` is a private Rust item, not a stable exported
  symbol.

## Linking Recommendations

- **Stable programs**: Link against the shared library (`liblibrtmp2.so`,
  produced by `cargo build --release`).
- **Embedders**: Link against the static library (`liblibrtmp2.a`) for
  isolation.
- **Rust consumers**: Depend on the `librtmp2` crate directly instead of the
  FFI layer.

## ABI Check Checklist (before each release)

1. Run `scripts/abi-baseline.sh compare <previous-tag>` (`abidw` +
   `abi-compliance-checker` against the previous release).
2. No removal or reordering of `#[repr(C)]` struct fields.
3. No changes to `#[no_mangle] pub extern "C"` function signatures.
4. New enum values are appended (not inserted).
5. `soname` bumped on ABI break (not expected before 1.0.0).
