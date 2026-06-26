/**
 * test_ffmpeg_ingest.c — Interop test: ingest a live stream from real ffmpeg.
 *
 * Listens on a TCP port and waits for an external RTMP publisher (ffmpeg) to
 * connect, handshake, publish and push H.264 video + AAC audio. Every byte of
 * each delivered frame is read (so an ASan build catches any over-read), and
 * the test succeeds once at least one video AND one audio frame have arrived.
 *
 * Exit codes: 0 = success, 1 = setup error, 2 = timed out without both a video
 * and an audio frame.
 *
 * Usage: ./test_ffmpeg_ingest [bind_addr:port] [timeout_seconds]
 */
#include "librtmp2/librtmp2.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static long g_video_frames = 0;
static long g_audio_frames = 0;
static unsigned long long g_total_bytes = 0;

static int on_publish(lrtmp2_conn_t *conn, const char *app, const char *key, void *u)
{
    (void)conn; (void)u;
    printf("[interop] publish: app=%s key=%s\n", app, key);
    return 0;
}

static int on_frame(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *u)
{
    (void)conn; (void)u;
    /* Touch every byte so ASan/UBSan flags any out-of-bounds delivery. */
    volatile unsigned long sum = 0;
    for (size_t i = 0; i < frame->size; i++) sum += frame->data[i];
    (void)sum;
    g_total_bytes += frame->size;
    if (frame->type == LRTMP2_FRAME_VIDEO) g_video_frames++;
    else if (frame->type == LRTMP2_FRAME_AUDIO) g_audio_frames++;
    return 0;
}

int main(int argc, char **argv)
{
    const char *bind_addr = (argc > 1) ? argv[1] : "127.0.0.1:11935";
    int timeout_s = (argc > 2) ? atoi(argv[2]) : 25;

    lrtmp2_server_config_t cfg;
    memset(&cfg, 0, sizeof(cfg));
    cfg.max_connections = 4;
    cfg.chunk_size = 4096;
    cfg.on_publish_cb = on_publish;
    cfg.on_frame_cb = on_frame;

    lrtmp2_server_t *server = lrtmp2_server_create(&cfg);
    if (!server) { fprintf(stderr, "[interop] server_create failed\n"); return 1; }
    if (lrtmp2_server_listen(server, bind_addr) != 0) {
        fprintf(stderr, "[interop] listen failed on %s\n", bind_addr);
        lrtmp2_server_destroy(server);
        return 1;
    }
    printf("[interop] listening on %s (timeout %ds)\n", bind_addr, timeout_s);

    time_t start = time(NULL);
    int success = 0;
    while (time(NULL) - start < timeout_s) {
        lrtmp2_server_poll(server, 200);
        if (g_video_frames > 0 && g_audio_frames > 0) { success = 1; break; }
    }

    printf("[interop] video=%ld audio=%ld bytes=%llu\n",
           g_video_frames, g_audio_frames, g_total_bytes);
    lrtmp2_server_destroy(server);

    if (success) {
        printf("[interop] PASS: received video and audio from real publisher\n");
        return 0;
    }
    fprintf(stderr, "[interop] FAIL: timed out (video=%ld audio=%ld)\n",
            g_video_frames, g_audio_frames);
    return 2;
}
