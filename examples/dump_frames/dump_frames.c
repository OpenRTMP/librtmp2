/**
 * dump_frames.c — Example: receive an RTMP stream and dump frames to disk
 *
 * Connects to a publishing RTMP server as a player, receives audio/video/
 * script frames, and writes each frame's raw payload to a numbered file.
 *
 * Usage: ./dump_frames rtmp://host:port/app/stream_key [output_dir]
 *
 * Output files:
 *   <output_dir>/frame_000000.video  (H.264/HEVC/AV1 NAL units)
 *   <output_dir>/frame_000001.audio  (AAC/PCM/Opus raw)
 *   <output_dir>/frame_000002.script (AMF0/AMF3 onMetaData)
 *   <output_dir>/frame_000003.metadata
 */
/* _POSIX_C_SOURCE needed for clock_gettime / CLOCK_MONOTONIC */
#define _POSIX_C_SOURCE 199309L

#include "librtmp2/librtmp2.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <sys/stat.h>
#include <time.h>

static volatile int g_running = 1;

static void signal_handler(int sig)
{
    (void)sig;
    g_running = 0;
}

static const char *frame_type_ext(lrtmp2_frame_type_t type)
{
    switch (type) {
        case LRTMP2_FRAME_AUDIO:  return "audio";
        case LRTMP2_FRAME_VIDEO:  return "video";
        case LRTMP2_FRAME_SCRIPT: return "script";
        case LRTMP2_FRAME_METADATA: return "metadata";
        default: return "unknown";
    }
}

static const char *video_codec_str(lrtmp2_video_codec_t c)
{
    switch (c) {
        case LRTMP2_VIDEO_H264: return "H264";
        case LRTMP2_VIDEO_H265: return "H265";
        case LRTMP2_VIDEO_AV1:  return "AV1";
        default: return "other";
    }
}

static const char *audio_codec_str(lrtmp2_audio_codec_t c)
{
    switch (c) {
        case LRTMP2_AUDIO_AAC:  return "AAC";
        case LRTMP2_AUDIO_OPUS: return "Opus";
        case LRTMP2_AUDIO_MP3:  return "MP3";
        case LRTMP2_AUDIO_G711_A: return "G711-A";
        case LRTMP2_AUDIO_G711_U: return "G711-U";
        default: return "PCM/other";
    }
}

static int g_frame_count = 0;
static int g_video_count = 0;
static int g_audio_count = 0;
static int g_script_count = 0;
static int g_metadata_count = 0;
static char g_output_dir[512];

static int on_frame(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *userdata)
{
    (void)conn; (void)userdata;

    const char *ext = frame_type_ext(frame->type);
    char path[1024];
    snprintf(path, sizeof(path), "%s/frame_%06d.%s", g_output_dir, g_frame_count, ext);

    FILE *fp = fopen(path, "wb");
    if (fp) {
        fwrite(frame->data, 1, frame->size, fp);
        fclose(fp);
    }

    /* Log frame info */
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    double wall = (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;

    switch (frame->type) {
        case LRTMP2_FRAME_VIDEO:
            printf("[dump_frames] #%d VIDEO ts=%u size=%u codec=%s ft=%u (%.3fs)\n",
                   g_frame_count, frame->timestamp, frame->size,
                   video_codec_str(frame->video_codec), frame->video_frame_type, wall);
            g_video_count++;
            break;
        case LRTMP2_FRAME_AUDIO:
            printf("[dump_frames] #%d AUDIO ts=%u size=%u codec=%s sr=%u ch=%u (%.3fs)\n",
                   g_frame_count, frame->timestamp, frame->size,
                   audio_codec_str(frame->audio_codec),
                   frame->audio_sample_rate, frame->audio_channels, wall);
            g_audio_count++;
            break;
        case LRTMP2_FRAME_SCRIPT:
            printf("[dump_frames] #%d SCRIPT ts=%u size=%u (%.3fs)\n",
                   g_frame_count, frame->timestamp, frame->size, wall);
            g_script_count++;
            break;
        case LRTMP2_FRAME_METADATA:
            printf("[dump_frames] #%d METADATA ts=%u size=%u (%.3fs)\n",
                   g_frame_count, frame->timestamp, frame->size, wall);
            g_metadata_count++;
            break;
    }

    g_frame_count++;
    return 0;
}

static int on_play(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata)
{
    (void)conn; (void)userdata;
    printf("[dump_frames] Playing: app=%s key=%s\n", app, stream_key);
    return 0;
}

static void on_close(lrtmp2_conn_t *conn, void *userdata)
{
    (void)conn; (void)userdata;
    printf("[dump_frames] Connection closed\n");
    g_running = 0;
}

int main(int argc, char **argv)
{
    if (argc < 2) {
        fprintf(stderr, "Usage: %s rtmp://host:port/app/stream_key [output_dir]\n", argv[0]);
        fprintf(stderr, "  output_dir defaults to ./dumped_frames\n");
        return 1;
    }

    const char *url = argv[1];
    const char *output_dir = (argc > 2) ? argv[2] : "./dumped_frames";
    strncpy(g_output_dir, output_dir, sizeof(g_output_dir) - 1);
    g_output_dir[sizeof(g_output_dir) - 1] = '\0';

    /* Create output directory */
    mkdir(output_dir, 0755);

    signal(SIGINT, signal_handler);
    signal(SIGTERM, signal_handler);

    printf("[dump_frames] librtmp2 v%s\n", lrtmp2_version_string());
    printf("[dump_frames] Connecting to %s\n", url);
    printf("[dump_frames] Output directory: %s\n", output_dir);

    lrtmp2_server_config_t config;
    memset(&config, 0, sizeof(config));
    config.on_play_cb = on_play;
    config.on_frame_cb = on_frame;
    config.on_close_cb = on_close;

    lrtmp2_client_t *client = lrtmp2_client_create(&config);
    if (!client) {
        fprintf(stderr, "Failed to create client\n");
        return 1;
    }

    int rc = lrtmp2_client_connect(client, url);
    if (rc != 0) {
        fprintf(stderr, "Failed to connect to %s\n", url);
        lrtmp2_client_destroy(client);
        return 1;
    }
    printf("[dump_frames] Connected\n");

    rc = lrtmp2_client_play(client);
    if (rc != 0) {
        fprintf(stderr, "Failed to start playback\n");
        lrtmp2_client_destroy(client);
        return 1;
    }
    printf("[dump_frames] Playing — press Ctrl+C to stop\n");

    while (g_running) {
        rc = lrtmp2_client_poll(client, 1000);
        if (rc != LRTMP2_OK && rc != LRTMP2_ERR_TIMEOUT) {
            fprintf(stderr, "[dump_frames] Poll error: %d\n", rc);
            break;
        }
    }

    printf("\n[dump_frames] Summary:\n");
    printf("  Total frames:  %d\n", g_frame_count);
    printf("  Video frames:  %d\n", g_video_count);
    printf("  Audio frames:  %d\n", g_audio_count);
    printf("  Script frames: %d\n", g_script_count);
    printf("  Metadata:      %d\n", g_metadata_count);

    lrtmp2_client_destroy(client);
    return 0;
}
