/**
 * test_client_publish.c — Integration test: real client <-> real server over loopback TCP
 *
 * Spins up an lrtmp2_server_t listening on 127.0.0.1, then drives a real
 * lrtmp2_client_t through connect() -> publish() -> send_frame() against it
 * on a separate thread, while the main thread pumps the server's accept/recv
 * loop. Exercises the full real-socket handshake, chunk fragmentation (the
 * publish command + frame payload are big enough to require it for some
 * configurations) and reassembly path end-to-end.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>
#include <time.h>

#include "server/server.h"
#include "session/conn.h"
#include "client/client.h"
#include "core/log.h"

#define TEST_PORT 19357

static uint8_t g_received_data[4096];
static size_t g_received_len = 0;
static int g_frame_received = 0;
static int g_publish_called = 0;
static char g_publish_app[256];
static char g_publish_stream[256];

static int on_frame_cb(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *userdata) {
    (void)conn; (void)userdata;
    fprintf(stderr, "  [cb] on_frame: type=%d size=%u\n", frame->type, frame->size);
    if (frame->size > 0 && frame->size <= sizeof(g_received_data)) {
        memcpy(g_received_data, frame->data, frame->size);
        g_received_len = frame->size;
        g_frame_received = 1;
    }
    return 0;
}

static int on_publish_cb(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata) {
    (void)conn; (void)userdata;
    fprintf(stderr, "  [cb] on_publish: app=%s stream=%s\n", app, stream_key);
    g_publish_called = 1;
    snprintf(g_publish_app, sizeof(g_publish_app), "%s", app);
    snprintf(g_publish_stream, sizeof(g_publish_stream), "%s", stream_key);
    return 0;
}

static void *client_thread_fn(void *arg) {
    (void)arg;

    lrtmp2_server_config_t config;
    memset(&config, 0, sizeof(config));

    lrtmp2_client_t *client = lrtmp2_client_create(&config);
    if (!client) {
        fprintf(stderr, "client thread: create failed\n");
        return NULL;
    }

    char url[64];
    snprintf(url, sizeof(url), "rtmp://127.0.0.1:%d/live/mystream", TEST_PORT);

    int rc = lrtmp2_client_connect(client, url);
    if (rc != 0) {
        fprintf(stderr, "client thread: connect failed rc=%d\n", rc);
        lrtmp2_client_destroy(client);
        return NULL;
    }

    rc = lrtmp2_client_publish(client);
    if (rc != 0) {
        fprintf(stderr, "client thread: publish failed rc=%d\n", rc);
        lrtmp2_client_destroy(client);
        return NULL;
    }

    /* A keyframe video tag: FrameType=key(1)/CodecID=AVC(7), AVCPacketType=NALU,
     * composition time = 0, then a fake NALU payload. */
    uint8_t video_data[300];
    size_t i = 0;
    video_data[i++] = 0x17;
    video_data[i++] = 0x01;
    video_data[i++] = 0x00; video_data[i++] = 0x00; video_data[i++] = 0x00;
    for (; i < sizeof(video_data); i++) video_data[i] = (uint8_t)(i & 0xFF);

    lrtmp2_frame_t frame;
    memset(&frame, 0, sizeof(frame));
    frame.type = LRTMP2_FRAME_VIDEO;
    frame.timestamp = 0;
    frame.size = sizeof(video_data);
    frame.data = video_data;

    rc = lrtmp2_client_send_frame(client, &frame);
    if (rc != 0) {
        fprintf(stderr, "client thread: send_frame failed rc=%d\n", rc);
    }

    /* Give the server time to process before tearing down the socket. */
    usleep(200 * 1000);

    lrtmp2_client_destroy(client);
    return NULL;
}

int main(void) {
    printf("=== librtmp2 integration: real client <-> server over loopback TCP ===\n\n");

    lrtmp2_server_config_t config;
    memset(&config, 0, sizeof(config));
    config.max_connections = 10;
    config.chunk_size = 4096;
    config.on_frame_cb = on_frame_cb;
    config.on_publish_cb = on_publish_cb;

    lrtmp2_server_t *server = lrtmp2_server_create(&config);
    if (!server) { printf("FAIL: server_create\n"); return 1; }

    char bind_addr[64];
    snprintf(bind_addr, sizeof(bind_addr), "127.0.0.1:%d", TEST_PORT);
    if (lrtmp2_server_listen(server, bind_addr) != 0) {
        printf("FAIL: server_listen\n");
        lrtmp2_server_destroy(server);
        return 1;
    }

    pthread_t client_thread;
    pthread_create(&client_thread, NULL, client_thread_fn, NULL);

    /* Pump the server: accept the connection, then keep processing it
     * until the frame arrives or we time out. */
    struct timespec start, now;
    clock_gettime(CLOCK_MONOTONIC, &start);
    int accepted = 0;
    for (;;) {
        if (!accepted) {
            int rc = lrtmp2_server_poll(server, 50);
            if (rc == LRTMP2_OK) accepted = 1;
        } else {
            lrtmp2_server_process_connections(server);
        }
        if (g_frame_received) break;

        clock_gettime(CLOCK_MONOTONIC, &now);
        double elapsed = (now.tv_sec - start.tv_sec) + (now.tv_nsec - start.tv_nsec) / 1e9;
        if (elapsed > 5.0) {
            printf("FAIL: timed out waiting for frame\n");
            break;
        }
        usleep(10 * 1000);
    }

    pthread_join(client_thread, NULL);

    int success = 0;
    if (g_publish_called && g_frame_received &&
        strcmp(g_publish_app, "live") == 0 && strcmp(g_publish_stream, "mystream") == 0 &&
        g_received_len == 300 && memcmp(g_received_data, "\x17\x01\x00\x00\x00", 5) == 0) {
        printf("PASS: real client published and frame round-tripped over TCP (app=%s stream=%s, len=%zu)\n",
               g_publish_app, g_publish_stream, g_received_len);
        success = 1;
    } else {
        printf("FAIL: publish_called=%d frame_received=%d len=%zu\n",
               g_publish_called, g_frame_received, g_received_len);
    }

    lrtmp2_server_destroy(server);
    return success ? 0 : 1;
}
