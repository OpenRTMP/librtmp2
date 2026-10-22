#!/usr/bin/env bash
# abi-baseline.sh — Generate or compare ABI dumps for librtmp2
#
# Usage:
#   ./scripts/abi-baseline.sh dump          # Generate baseline ABI dump
#   ./scripts/abi-baseline.sh compare HEAD  # Compare current vs HEAD
#   ./scripts/abi-baseline.sh compare v0.1.0 # Compare current vs tag
#
# Requires (install on Ubuntu):
#   sudo apt-get install -y abigail-tools libabigail-dev
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
ABI_DIR="$PROJECT_DIR/abi-dumps"

mkdir -p "$ABI_DIR"

build_and_dump() {
    local label="$1"

    echo "=== Building ($label) ==="
    cd "$PROJECT_DIR"
    cargo clean --release
    cargo build --release --all-features

    # `cargo build` honours CARGO_TARGET_DIR, so the cdylib does not
    # necessarily land in $PROJECT_DIR/target. Ask cargo where it actually
    # wrote its artifacts instead of assuming the default layout.
    local target_dir
    target_dir=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
        | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' || true)
    if [[ -z "$target_dir" ]]; then
        target_dir="${CARGO_TARGET_DIR:-$PROJECT_DIR/target}"
    fi

    echo "=== Generating ABI dump ($label) ==="
    abidw \
        "$target_dir/release/liblibrtmp2.so" \
        --out-file "$ABI_DIR/librtmp2-${label}.xml"

    echo "✅ Dump saved: $ABI_DIR/librtmp2-${label}.xml"
}

case "${1:-}" in
    dump)
        build_and_dump "baseline"
        ;;
    compare)
        BASELINE_REF="${2:-HEAD}"
        # Resolve the requested ref itself. `git describe --tags` always yields
        # the nearest *tag* reachable from the ref, never the ref, so
        # `compare HEAD` used to diff against the last release instead of the
        # commit the caller asked for.
        BASELINE=$(git -C "$PROJECT_DIR" rev-parse --verify --quiet "${BASELINE_REF}^{commit}" || echo "")

        if [[ -z "$BASELINE" ]]; then
            echo "Cannot resolve baseline ref: $BASELINE_REF"
            exit 1
        fi

        echo "Baseline: $BASELINE ($BASELINE_REF)"

        # Build baseline
        cd "$PROJECT_DIR"
        git stash || true
        git checkout "$BASELINE"
        build_and_dump "baseline"
        git checkout - || git checkout main
        git stash pop || true

        # Build current
        build_and_dump "current"

        # abidw emits libabigail ABIXML, so compare those dumps with abidiff.
        echo "=== Running ABI compatibility check ==="
        set +e
        abidiff \
            "$ABI_DIR/librtmp2-baseline.xml" \
            "$ABI_DIR/librtmp2-current.xml" \
            2>&1 | tee "$ABI_DIR/abi-check-result.txt"
        ABIDIFF_STATUS=${PIPESTATUS[0]}
        set -e

        # abidiff uses a bitmask exit status:
        #   1/2 = execution or usage error, 4 = compatible ABI change,
        #   8 = incompatible ABI change. Compatible changes are allowed.
        if (( ABIDIFF_STATUS & 8 )); then
            echo "❌ ABI BREAKING CHANGES DETECTED!"
            exit 1
        fi
        if (( ABIDIFF_STATUS & 3 )); then
            echo "❌ abidiff failed with status $ABIDIFF_STATUS"
            exit "$ABIDIFF_STATUS"
        fi
        if (( ABIDIFF_STATUS & 4 )); then
            echo "ℹ️ ABI changed, but no incompatible changes were detected"
        fi

        echo "✅ ABI check passed"
        ;;
    *)
        echo "Usage: $0 {dump|compare [ref]}"
        exit 1
        ;;
esac
