#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="${ROOT_DIR}/include/librtmp2"
OUTPUT_FILE="${OUTPUT_DIR}/librtmp2.h"

# Keep in sync with CBINDGEN_VERSION in .github/workflows/c-header.yml so
# local and CI header generation stay reproducible.
CBINDGEN_VERSION="${CBINDGEN_VERSION:-0.27.0}"

if ! command -v cbindgen >/dev/null 2>&1; then
    echo "Error: cbindgen is not installed." >&2
    echo "Install it with: cargo install --locked --version ${CBINDGEN_VERSION} cbindgen" >&2
    exit 1
fi

INSTALLED_VERSION="$(cbindgen --version | awk '{print $2}')"
if [ "${INSTALLED_VERSION}" != "${CBINDGEN_VERSION}" ]; then
    echo "Error: cbindgen ${CBINDGEN_VERSION} is required, found ${INSTALLED_VERSION}." >&2
    echo "Install it with: cargo install --locked --version ${CBINDGEN_VERSION} --force cbindgen" >&2
    exit 1
fi

mkdir -p "${OUTPUT_DIR}"

cbindgen \
    --config "${ROOT_DIR}/cbindgen.toml" \
    --crate librtmp2 \
    --output "${OUTPUT_FILE}" \
    "${ROOT_DIR}"

echo "Generated ${OUTPUT_FILE}"
