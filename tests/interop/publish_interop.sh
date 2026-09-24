#!/usr/bin/env bash
#
# publish_interop.sh — Publish interop test for the librtmp2 client.
#
# Encodes a short H.264 + AAC FLV with ffmpeg, publishes it through the
# flv_publish example (built on the librtmp2 client, which announces a larger
# chunk size and uses compact chunk headers) into each available third-party
# RTMP server, and reads the stream back with ffmpeg, which must decode it
# with no errors.
#
# Servers (each skipped when unavailable):
#   - MediaMTX: set MEDIAMTX to its path, or have `mediamtx` on PATH
#   - nginx-rtmp: nginx plus the ngx_rtmp_module (set NGINX_RTMP_MODULE to
#     the .so path if it isn't the Debian/Ubuntu default)
#
# Requires: ffmpeg on PATH and a Rust toolchain. Fails if no server ran.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

MEDIAMTX="${MEDIAMTX:-mediamtx}"
NGINX_RTMP_MODULE="${NGINX_RTMP_MODULE:-/usr/lib/nginx/modules/ngx_rtmp_module.so}"
BIN_NAME="flv_publish"
WORK="$(mktemp -d /tmp/publish_interop.XXXXXX)"

command -v ffmpeg >/dev/null 2>&1 || { echo "ffmpeg not found on PATH"; exit 1; }

echo "== building $BIN_NAME =="
cargo build --example "$BIN_NAME" --all-features
BIN="$(find target -type f -name "$BIN_NAME" -path '*/examples/*' | head -n1)"

echo "== encoding test FLV =="
ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i "testsrc=size=640x480:rate=25" -f lavfi -i "sine=frequency=1000" \
    -t 12 -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 25 \
    -c:a aac -b:a 64k -f flv "$WORK/src.flv"

PIDS=()
cleanup() {
    for pid in "${PIDS[@]:-}"; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
}
trap cleanup EXIT

wait_port() {
    for _ in $(seq 1 50); do
        if (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; then
            exec 3>&-
            return 0
        fi
        sleep 0.2
    done
    return 1
}

# Publish with librtmp2, read back with ffmpeg, require a clean decode.
check_server() {
    local name="$1" url="$2"
    echo "== [$name] publishing with librtmp2 client =="
    "$BIN" "$WORK/src.flv" "$url" >"$WORK/$name-pub.log" 2>&1 &
    local pub=$!
    PIDS+=("$pub")
    sleep 3
    echo "== [$name] reading back with ffmpeg =="
    set +e
    timeout 20 ffmpeg -hide_banner -loglevel error -xerror -i "$url" \
        -t 5 -f null - >"$WORK/$name-read.log" 2>&1
    local rc=$?
    wait "$pub"
    local pub_rc=$?
    set -e
    cat "$WORK/$name-pub.log"
    if [ "$rc" -ne 0 ] || [ "$pub_rc" -ne 0 ] || [ -s "$WORK/$name-read.log" ]; then
        echo "[$name] FAIL (reader exit $rc, publisher exit $pub_rc)"
        cat "$WORK/$name-read.log"
        return 1
    fi
    echo "[$name] PUBLISH INTEROP OK"
}

RAN=0

if command -v "$MEDIAMTX" >/dev/null 2>&1 || [ -x "$MEDIAMTX" ]; then
    PORT=11941
    printf 'paths:\n  all_others:\n' > "$WORK/mediamtx.yml"
    MTX_RTMPADDRESS=":$PORT" MTX_HLS=no MTX_WEBRTC=no MTX_RTSP=no MTX_SRT=no \
        "$MEDIAMTX" "$WORK/mediamtx.yml" >"$WORK/mediamtx.log" 2>&1 &
    PIDS+=("$!")
    wait_port "$PORT" || { echo "mediamtx did not start"; cat "$WORK/mediamtx.log"; exit 1; }
    check_server mediamtx "rtmp://127.0.0.1:$PORT/live/test"
    RAN=$((RAN + 1))
else
    echo "skipping MediaMTX: not found (set MEDIAMTX)"
fi

if command -v nginx >/dev/null 2>&1 && [ -e "$NGINX_RTMP_MODULE" ]; then
    PORT=11942
    cat > "$WORK/nginx.conf" <<EOF
load_module $NGINX_RTMP_MODULE;
worker_processes 1;
daemon off;
error_log $WORK/nginx-error.log info;
pid $WORK/nginx.pid;
events { worker_connections 256; }
rtmp {
    server {
        listen 127.0.0.1:$PORT;
        application live { live on; record off; }
    }
}
EOF
    nginx -c "$WORK/nginx.conf" &
    PIDS+=("$!")
    wait_port "$PORT" || { echo "nginx did not start"; cat "$WORK/nginx-error.log"; exit 1; }
    check_server nginx-rtmp "rtmp://127.0.0.1:$PORT/live/test"
    RAN=$((RAN + 1))
else
    echo "skipping nginx-rtmp: nginx or $NGINX_RTMP_MODULE not found"
fi

if [ "$RAN" -eq 0 ]; then
    echo "no RTMP server available to test against"
    exit 1
fi
echo "PUBLISH INTEROP: $RAN server(s) passed"
