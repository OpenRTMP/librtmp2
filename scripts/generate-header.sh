#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="${ROOT_DIR}/include/librtmp2"
OUTPUT_FILE="${OUTPUT_DIR}/librtmp2.h"

if ! command -v cbindgen >/dev/null 2>&1; then
    echo "Error: cbindgen is not installed." >&2
    echo "Install it with: cargo install --locked cbindgen" >&2
    exit 1
fi

mkdir -p "${OUTPUT_DIR}"

cbindgen \
    --config "${ROOT_DIR}/cbindgen.toml" \
    --crate librtmp2 \
    --output "${OUTPUT_FILE}" \
    "${ROOT_DIR}"

echo "Generated ${OUTPUT_FILE}"
