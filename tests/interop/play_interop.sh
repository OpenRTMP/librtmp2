#!/usr/bin/env bash
#
# play_interop.sh — Play (pull) interop test against a real RTMP server.
#
# Starts mediamtx as a real RTMP server, has ffmpeg publish a looping
# H.264 + AAC stream into it, then uses the librtmp2 client to play (pull) that
# stream. The play test exits 0 once it has pulled both a video and an audio
# frame. Built with ASan/UBSan by default so any over-read is caught.
#
# Requires: ffmpeg on PATH, a C compiler, and a mediamtx binary (set MEDIAMTX
# to its path, or have `mediamtx` on PATH).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PORT="${PORT:-11940}"
ADDR="127.0.0.1:${PORT}"
URL="rtmp://${ADDR}/live/test"
SAN="${SAN:-1}"
MEDIAMTX="${MEDIAMTX:-mediamtx}"
BIN="tests/interop/run_play_pull"

command -v ffmpeg >/dev/null 2>&1 || { echo "ffmpeg not found on PATH"; exit 1; }
command -v "$MEDIAMTX" >/dev/null 2>&1 || [ -x "$MEDIAMTX" ] || { echo "mediamtx not found (set MEDIAMTX)"; exit 1; }

CFLAGS="-Wall -Wextra -Iinclude -Isrc -g -O1"
LDFLAGS="-lpthread -lm"
if [ "$SAN" = "1" ]; then
    CFLAGS="$CFLAGS -fsanitize=address,undefined -fno-omit-frame-pointer"
    LDFLAGS="-fsanitize=address,undefined $LDFLAGS"
fi

SRCS=$(ls src/core/*.c src/handshake/*.c src/chunk/*.c src/message/*.c \
          src/amf/*.c src/flv/*.c src/ertmp/*.c src/session/*.c \
          src/server/*.c src/client/*.c)

echo "== building play-pull test (SAN=$SAN) =="
# shellcheck disable=SC2086
${CC:-cc} $CFLAGS tests/interop/test_play_pull.c $SRCS -o "$BIN" $LDFLAGS

PUB=""; MTX=""
cleanup() {
    [ -n "$PUB" ] && kill "$PUB" 2>/dev/null || true
    [ -n "$MTX" ] && kill "$MTX" 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT

echo "== starting mediamtx RTMP server on :$PORT =="
# Minimal config: accept any publish/read path (catch-all).
MTX_CFG="$(mktemp /tmp/mediamtx.XXXXXX.yml)"
printf 'paths:\n  all_others:\n' > "$MTX_CFG"
# Only RTMP is needed; disable the other listeners so they can't fail to bind.
MTX_RTMPADDRESS=":$PORT" MTX_HLS=no MTX_WEBRTC=no MTX_RTSP=no MTX_SRT=no \
    "$MEDIAMTX" "$MTX_CFG" >/tmp/mediamtx.log 2>&1 &
MTX=$!
sleep 2

echo "== publishing looping test stream with ffmpeg =="
timeout 40 ffmpeg -hide_banner -loglevel error -re -stream_loop -1 \
    -f lavfi -i "testsrc=size=640x480:rate=20" \
    -f lavfi -i "sine=frequency=1000" \
    -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 20 \
    -c:a aac -b:a 64k \
    -f flv "$URL" >/tmp/play_publish.log 2>&1 &
PUB=$!

# Give the publisher a moment to register the stream.
sleep 3

echo "== pulling stream with librtmp2 client =="
set +e
ASAN_OPTIONS=detect_leaks=1 "./$BIN" "$URL" 20
RC=$?
set -e

echo "== mediamtx log (tail) =="
tail -n 8 /tmp/mediamtx.log || true

if [ "$RC" -ne 0 ]; then
    echo "PLAY INTEROP FAILED (play client exit=$RC)"
    exit 1
fi
echo "PLAY INTEROP OK"
