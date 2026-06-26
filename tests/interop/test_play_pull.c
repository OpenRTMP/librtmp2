/**
 * test_play_pull.c — Interop test: play (pull) a live stream from a real RTMP
 * server (mediamtx, fed by ffmpeg).
 *
 * Connects as an RTMP client, issues play, and pumps incoming messages. Every
 * byte of each delivered frame is read (so an ASan build catches any
 * over-read). Succeeds once at least one video AND one audio frame arrive.
 *
 * Exit codes: 0 = success, 1 = setup/connect error, 2 = timed out without both
 * a video and an audio frame.
 *
 * Usage: ./test_play_pull rtmp://host:port/app/stream [timeout_seconds]
 */
#include "librtmp2/librtmp2.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static long g_video_frames = 0;
static long g_audio_frames = 0;
static unsigned long long g_total_bytes = 0;

static int on_frame(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *u)
{
    (void)conn; (void)u;
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
    const char *url = (argc > 1) ? argv[1] : "rtmp://127.0.0.1:1935/live/test";
    int timeout_s = (argc > 2) ? atoi(argv[2]) : 25;

    lrtmp2_server_config_t cfg;
    memset(&cfg, 0, sizeof(cfg));
    cfg.chunk_size = 4096;
    cfg.on_frame_cb = on_frame;

    lrtmp2_client_t *client = lrtmp2_client_create(&cfg);
    if (!client) { fprintf(stderr, "[interop-play] client_create failed\n"); return 1; }

    printf("[interop-play] connecting to %s\n", url);
    if (lrtmp2_client_connect(client, url) != 0) {
        fprintf(stderr, "[interop-play] connect failed\n");
        lrtmp2_client_destroy(client);
        return 1;
    }
    if (lrtmp2_client_play(client) != 0) {
        fprintf(stderr, "[interop-play] play failed\n");
        lrtmp2_client_destroy(client);
        return 1;
    }
    printf("[interop-play] play started, pumping frames\n");

    time_t start = time(NULL);
    int success = 0;
    while (time(NULL) - start < timeout_s) {
        int rc = lrtmp2_client_poll(client, 200);
        if (rc < 0) { fprintf(stderr, "[interop-play] poll error rc=%d\n", rc); break; }
        if (g_video_frames > 0 && g_audio_frames > 0) { success = 1; break; }
    }

    printf("[interop-play] video=%ld audio=%ld bytes=%llu\n",
           g_video_frames, g_audio_frames, g_total_bytes);
    lrtmp2_client_destroy(client);

    if (success) {
        printf("[interop-play] PASS: pulled video and audio from real RTMP server\n");
        return 0;
    }
    fprintf(stderr, "[interop-play] FAIL: timed out (video=%ld audio=%ld)\n",
            g_video_frames, g_audio_frames);
    return 2;
}
