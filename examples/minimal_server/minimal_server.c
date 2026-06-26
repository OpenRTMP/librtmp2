/**
 * minimal_server.c — Example: minimal RTMP server
 *
 * Listens on port 1935, accepts one connection, logs incoming frames.
 * Usage: ./minimal_server [bind_addr:port]
 */
#include "librtmp2.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>

static volatile int g_running = 1;

static void signal_handler(int sig)
{
    (void)sig;
    g_running = 0;
}

static int on_connect(lrtmp2_conn_t *conn, void *userdata)
{
    (void)userdata;
    printf("[minimal_server] New connection established\n");
    return 0;  /* accept */
}

static int on_publish(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata)
{
    (void)conn; (void)userdata;
    printf("[minimal_server] Publish: app=%s key=%s\n", app, stream_key);
    return 0;  /* accept */
}

static int on_frame(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *userdata)
{
    (void)conn; (void)userdata;
    const char *type_str = "?";
    switch (frame->type) {
        case LRTMP2_FRAME_AUDIO:  type_str = "AUDIO"; break;
        case LRTMP2_FRAME_VIDEO:  type_str = "VIDEO"; break;
        case LRTMP2_FRAME_SCRIPT: type_str = "SCRIPT"; break;
        case LRTMP2_FRAME_METADATA: type_str = "METADATA"; break;
    }
    printf("[minimal_server] Frame: %s ts=%u size=%u\n", type_str, frame->timestamp, frame->size);
    return 0;
}

static void on_close(lrtmp2_conn_t *conn, void *userdata)
{
    (void)conn; (void)userdata;
    printf("[minimal_server] Connection closed\n");
}

int main(int argc, char **argv)
{
    const char *bind_addr = "0.0.0.0:1935";
    if (argc > 1) {
        bind_addr = argv[1];
    }

    signal(SIGINT, signal_handler);
    signal(SIGTERM, signal_handler);

    printf("[minimal_server] librtmp2 v%s\n", lrtmp2_version_string());
    printf("[minimal_server] Binding to %s\n", bind_addr);

    lrtmp2_server_config_t config;
    memset(&config, 0, sizeof(config));
    config.max_connections = 10;
    config.chunk_size = 4096;
    config.on_connect_cb = on_connect;
    config.on_publish_cb = on_publish;
    config.on_frame_cb = on_frame;
    config.on_close_cb = on_close;

    lrtmp2_server_t *server = lrtmp2_server_create(&config);
    if (!server) {
        fprintf(stderr, "Failed to create server\n");
        return 1;
    }

    int rc = lrtmp2_server_listen(server, bind_addr);
    if (rc != 0) {
        fprintf(stderr, "Failed to listen on %s\n", bind_addr);
        lrtmp2_server_destroy(server);
        return 1;
    }

    printf("[minimal_server] Listening on %s (Ctrl+C to stop)\n", bind_addr);

    while (g_running) {
        lrtmp2_server_poll(server, 1000);
    }

    printf("[minimal_server] Shutting down\n");
    lrtmp2_server_destroy(server);
    return 0;
}
