#!/usr/bin/env bash
#
# run_ffmpeg_interop.sh — Interop smoke test against the real ffmpeg RTMP client.
#
# Builds the interop ingest test (ASan/UBSan by default), starts it as an RTMP
# server, then publishes a short generated H.264 + AAC stream to it with ffmpeg.
# The test program exits 0 once it has ingested both a video and an audio frame.
#
# Requires: ffmpeg on PATH, a C compiler, and the librtmp2 sources.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PORT="${PORT:-11935}"
ADDR="127.0.0.1:${PORT}"
SAN="${SAN:-1}"          # 1 = build the ingest test with ASan+UBSan
BIN="tests/interop/run_ffmpeg_ingest"

command -v ffmpeg >/dev/null 2>&1 || { echo "ffmpeg not found on PATH"; exit 1; }

CFLAGS="-Wall -Wextra -Iinclude -Isrc -g -O1"
LDFLAGS="-lpthread -lm"
if [ "$SAN" = "1" ]; then
    CFLAGS="$CFLAGS -fsanitize=address,undefined -fno-omit-frame-pointer"
    LDFLAGS="-fsanitize=address,undefined $LDFLAGS"
fi

SRCS=$(ls src/core/*.c src/handshake/*.c src/chunk/*.c src/message/*.c \
          src/amf/*.c src/flv/*.c src/ertmp/*.c src/session/*.c \
          src/server/*.c src/client/*.c)

echo "== building interop ingest test (SAN=$SAN) =="
# shellcheck disable=SC2086
${CC:-cc} $CFLAGS tests/interop/test_ffmpeg_ingest.c $SRCS -o "$BIN" $LDFLAGS

echo "== starting ingest server on $ADDR =="
"./$BIN" "$ADDR" 25 >/tmp/interop_server.log 2>&1 &
SRV=$!

cleanup() { kill "$SRV" 2>/dev/null || true; wait "$SRV" 2>/dev/null || true; }
trap cleanup EXIT

# Give the listener a moment to bind.
sleep 1

echo "== publishing test stream with ffmpeg =="
set +e
timeout 30 ffmpeg -hide_banner -loglevel error \
    -f lavfi -i "testsrc=size=640x480:rate=20:duration=3" \
    -f lavfi -i "sine=frequency=1000:duration=3" \
    -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 20 \
    -c:a aac -b:a 64k \
    -f flv "rtmp://${ADDR}/live/test"
FF_RC=$?
set -e
echo "ffmpeg exit=$FF_RC"

# Wait for the ingest server to finish (it exits 0 on success).
wait "$SRV"
SRV_RC=$?
trap - EXIT

echo "== ingest server log =="
cat /tmp/interop_server.log

if [ "$SRV_RC" -ne 0 ]; then
    echo "INTEROP FAILED (ingest server exit=$SRV_RC)"
    exit 1
fi
echo "INTEROP OK"
