#!/usr/bin/env bash
# doc-coverage.sh — report rustdoc documentation coverage the way docs.rs does.
#
# docs.rs runs rustdoc with `--show-coverage --output-format json` and sums the
# per-file counters. This script does exactly that, so the numbers it prints are
# directly comparable to the "Coverage" panel on a crate's docs.rs page.
#
# Usage:
#   scripts/doc-coverage.sh                 # all features (default)
#   scripts/doc-coverage.sh --no-default-features
#   scripts/doc-coverage.sh --all-features
#
# Requires a Rust toolchain whose rustdoc supports `--show-coverage` (stable
# since 1.86) and `--output-format json` (unstable, enabled via
# RUSTC_BOOTSTRAP). Exits non-zero if any public item is undocumented, so it can
# gate CI.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FEATURES=("--all-features")
if [[ $# -gt 0 ]]; then
    FEATURES=("$@")
fi

cd "$ROOT_DIR"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
rm -rf "$TARGET_DIR/doc"
mkdir -p "$TARGET_DIR"

RUSTC_BOOTSTRAP=1 \
RUSTDOCFLAGS="-Zunstable-options --show-coverage --output-format json" \
    cargo doc --no-deps "${FEATURES[@]}"

JSON="$TARGET_DIR/doc/librtmp2.json"
if [[ ! -f "$JSON" ]]; then
    echo "error: rustdoc did not produce $JSON" >&2
    exit 1
fi

# Sum the per-file coverage counters, mirroring docs.rs' DocCoverage::extend.
python3 - "$JSON" <<'PY'
import json
import sys

totals = {"total": 0, "with_docs": 0, "total_examples": 0, "with_examples": 0}
undocumented = []

with open(sys.argv[1], encoding="utf-8") as f:
    for line in f:
        line = line.strip()
        if not line:
            continue
        for path, cov in json.loads(line).items():
            for key in totals:
                totals[key] += cov[key]
            if cov["total"] != cov["with_docs"]:
                undocumented.append((path, cov["total"] - cov["with_docs"]))

total = totals["total"]
documented = totals["with_docs"]
pct = documented / total * 100 if total else 100.0

print(f"items documented:  {documented} out of {total} ({pct:.2f}%)")
print(
    "items with examples: "
    f"{totals['with_examples']} out of {totals['total_examples']} "
    f"({totals['with_examples'] / totals['total_examples'] * 100:.2f}%)"
    if totals["total_examples"]
    else "items with examples:  n/a"
)

if undocumented:
    print("\nundocumented items per file:", file=sys.stderr)
    for path, missing in sorted(undocumented, key=lambda entry: -entry[1]):
        print(f"  {missing:4d}  {path}", file=sys.stderr)
    sys.exit(1)
PY
