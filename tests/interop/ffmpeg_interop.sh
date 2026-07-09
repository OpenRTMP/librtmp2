#!/usr/bin/env bash
#
# ffmpeg_interop.sh — Interop smoke test against the real ffmpeg RTMP client.
#
# Builds the run_ffmpeg_ingest example, starts it as an RTMP server, then
# publishes a short generated H.264 + AAC stream to it with ffmpeg. The
# example exits 0 once it has ingested both a video and an audio frame.
#
# Requires: ffmpeg on PATH and a Rust toolchain.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PORT="${PORT:-11935}"
ADDR="127.0.0.1:${PORT}"
HOST="${ADDR%%:*}"
PORT_NUM="${ADDR##*:}"
BIN_NAME="ffmpeg_ingest"
MAX_ATTEMPTS="${MAX_ATTEMPTS:-3}"

command -v ffmpeg >/dev/null 2>&1 || { echo "ffmpeg not found on PATH"; exit 1; }

echo "== building $BIN_NAME =="
cargo build --example "$BIN_NAME" --all-features
BIN="$(find target -type f -name "$BIN_NAME" -path '*/examples/*' | head -n1)"

run_once() {
    echo "== starting ingest server on $ADDR =="
    LOG="$(mktemp /tmp/interop_server.XXXXXX.log)"
    # Require several frames and at least one >=8 KiB frame, so a video keyframe
    # that spans multiple chunks exercises multi-chunk reassembly.
    "$BIN" "$ADDR" 25 5 8192 >"$LOG" 2>&1 &
    SRV=$!

    cleanup() { kill "$SRV" 2>/dev/null || true; wait "$SRV" 2>/dev/null || true; }
    trap cleanup EXIT

    # Wait for the listener to accept TCP connections instead of a fixed sleep,
    # which is either too short (flaky) or too long (slow) depending on load.
    for _ in $(seq 1 50); do
        if ! kill -0 "$SRV" 2>/dev/null; then
            echo "ingest server exited during startup"
            cat "$LOG" || true
            return 1
        fi
        if (exec 3<>"/dev/tcp/${HOST}/${PORT_NUM}") 2>/dev/null; then
            exec 3>&-
            break
        fi
        sleep 0.2
    done
    if ! kill -0 "$SRV" 2>/dev/null; then
        echo "ingest server exited before RTMP port was ready"
        cat "$LOG" || true
        return 1
    fi

    echo "== publishing test stream with ffmpeg =="
    set +e
    timeout 30 ffmpeg -hide_banner -loglevel error \
        -f lavfi -i "testsrc=size=1280x720:rate=25:duration=4" \
        -f lavfi -i "sine=frequency=1000:duration=4" \
        -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 25 -b:v 6M -maxrate 6M -bufsize 6M \
        -c:a aac -b:a 128k \
        -f flv "rtmp://${ADDR}/live/test"
    FF_RC=$?
    set -e
    echo "ffmpeg exit=$FF_RC"

    # A non-zero ffmpeg exit is expected when the ingest server is satisfied and
    # disconnects before ffmpeg finishes its full duration (it sees "Connection
    # reset by peer"). Give the server a short grace period to finish processing
    # buffered RTMP data and exit before treating a still-running server as a
    # hard failure.
    if [ "$FF_RC" -ne 0 ] && kill -0 "$SRV" 2>/dev/null; then
        for _ in $(seq 1 50); do
            kill -0 "$SRV" 2>/dev/null || break
            sleep 0.2
        done
    fi

    if [ "$FF_RC" -ne 0 ] && kill -0 "$SRV" 2>/dev/null; then
        echo "INTEROP FAILED (ffmpeg publish exit=$FF_RC, ingest server still running)"
        cat "$LOG" || true
        return 1
    fi

    # Wait for the ingest server to finish (it exits 0 on success).
    wait "$SRV"
    SRV_RC=$?
    trap - EXIT

    echo "== ingest server log =="
    cat "$LOG"

    if [ "$SRV_RC" -ne 0 ]; then
        echo "INTEROP FAILED (ingest server exit=$SRV_RC)"
        return 1
    fi
    return 0
}

for attempt in $(seq 1 "$MAX_ATTEMPTS"); do
    if [ "$attempt" -gt 1 ]; then
        echo "== retrying interop (attempt ${attempt}/${MAX_ATTEMPTS}) =="
        sleep 1
    fi
    if run_once; then
        echo "INTEROP OK"
        exit 0
    fi
done

echo "INTEROP FAILED after ${MAX_ATTEMPTS} attempts"
exit 1
