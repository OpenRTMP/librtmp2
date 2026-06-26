#!/usr/bin/env bash
#
# enhanced_rtmp_interop.sh — Enhanced-RTMP (FourCC) ingest interop test.
#
# Publishes an AV1 stream with ffmpeg, which ffmpeg muxes using the
# Enhanced-RTMP extended video tag (FourCC "av01") rather than the legacy FLV
# codec id. This exercises librtmp2's E-RTMP parsing path against a real
# encoder — the same path OBS uses for HEVC/AV1.
#
# Skips (exit 0) if ffmpeg has no AV1 encoder. Built with ASan/UBSan.
#
# Requires: ffmpeg on PATH and a C compiler.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PORT="${PORT:-11945}"
ADDR="127.0.0.1:${PORT}"
SAN="${SAN:-1}"
BIN="tests/interop/run_ffmpeg_ingest"

command -v ffmpeg >/dev/null 2>&1 || { echo "ffmpeg not found on PATH"; exit 1; }

# Pick an available AV1 encoder; skip the test if none is present.
# (Capture the list once: piping into `grep -q` under `set -o pipefail` would
# report failure when grep closes the pipe early and ffmpeg gets SIGPIPE.)
ENCODERS="$(ffmpeg -hide_banner -encoders 2>/dev/null || true)"
AV1_ENC=""
for enc in libaom-av1 libsvtav1 librav1e; do
    if printf '%s\n' "$ENCODERS" | grep -q " $enc "; then AV1_ENC="$enc"; break; fi
done
if [ -z "$AV1_ENC" ]; then
    echo "No AV1 encoder available in ffmpeg; skipping Enhanced-RTMP interop test."
    exit 0
fi
echo "Using AV1 encoder: $AV1_ENC"

CFLAGS="-Wall -Wextra -Iinclude -Isrc -g -O1"
LDFLAGS="-lpthread -lm"
if [ "$SAN" = "1" ]; then
    CFLAGS="$CFLAGS -fsanitize=address,undefined -fno-omit-frame-pointer"
    LDFLAGS="-fsanitize=address,undefined $LDFLAGS"
fi

SRCS=$(ls src/core/*.c src/handshake/*.c src/chunk/*.c src/message/*.c \
          src/amf/*.c src/flv/*.c src/ertmp/*.c src/session/*.c \
          src/server/*.c src/client/*.c)

echo "== building ingest test (SAN=$SAN) =="
# shellcheck disable=SC2086
${CC:-cc} $CFLAGS tests/interop/test_ffmpeg_ingest.c $SRCS -o "$BIN" $LDFLAGS

echo "== starting ingest server on $ADDR =="
"./$BIN" "$ADDR" 40 1 0 >/tmp/eR_server.log 2>&1 &
SRV=$!
cleanup() { kill "$SRV" 2>/dev/null || true; wait "$SRV" 2>/dev/null || true; }
trap cleanup EXIT
sleep 1

echo "== publishing AV1 (Enhanced-RTMP) stream with ffmpeg =="
set +e
EXTRA=""
[ "$AV1_ENC" = "libaom-av1" ] && EXTRA="-cpu-used 8"
[ "$AV1_ENC" = "libsvtav1" ] && EXTRA="-preset 12"
# shellcheck disable=SC2086
timeout 50 ffmpeg -hide_banner -loglevel error \
    -f lavfi -i "testsrc=size=320x240:rate=10:duration=2" \
    -f lavfi -i "sine=frequency=1000:duration=2" \
    -c:v "$AV1_ENC" $EXTRA -pix_fmt yuv420p -g 10 \
    -c:a aac -b:a 64k \
    -f flv "rtmp://${ADDR}/live/test"
echo "ffmpeg exit=$?"
set -e

wait "$SRV"
SRV_RC=$?
trap - EXIT

echo "== ingest server log =="
cat /tmp/eR_server.log

if [ "$SRV_RC" -ne 0 ]; then
    echo "ENHANCED-RTMP INTEROP FAILED (ingest exit=$SRV_RC)"
    exit 1
fi
echo "ENHANCED-RTMP INTEROP OK"
