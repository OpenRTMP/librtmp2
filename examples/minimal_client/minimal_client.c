/**
 * minimal_client.c — Example: minimal RTMP client
 *
 * Connects to an RTMP server, publishes a stream, and sends a single
 * synthetic video frame.
 * Usage: ./minimal_client rtmp://host:port/app/stream_key
 */
#include "librtmp2/librtmp2.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv)
{
    if (argc < 2) {
        fprintf(stderr, "Usage: %s rtmp://host:port/app/stream\n", argv[0]);
        return 1;
    }

    printf("[minimal_client] librtmp2 v%s\n", lrtmp2_version_string());
    printf("[minimal_client] Connecting to %s\n", argv[1]);

    lrtmp2_server_config_t config;
    memset(&config, 0, sizeof(config));

    lrtmp2_client_t *client = lrtmp2_client_create(&config);
    if (!client) {
        fprintf(stderr, "Failed to create client\n");
        return 1;
    }

    int rc = lrtmp2_client_connect(client, argv[1]);
    if (rc != 0) {
        fprintf(stderr, "Failed to connect\n");
        lrtmp2_client_destroy(client);
        return 1;
    }
    printf("[minimal_client] Connected\n");

    rc = lrtmp2_client_publish(client);
    if (rc != 0) {
        fprintf(stderr, "Failed to publish\n");
        lrtmp2_client_destroy(client);
        return 1;
    }
    printf("[minimal_client] Publishing\n");

    uint8_t payload[64];
    memset(payload, 0xAB, sizeof(payload));

    lrtmp2_frame_t frame;
    memset(&frame, 0, sizeof(frame));
    frame.type = LRTMP2_FRAME_VIDEO;
    frame.timestamp = 0;
    frame.size = sizeof(payload);
    frame.data = payload;
    frame.video_codec = LRTMP2_VIDEO_H264;
    frame.video_frame_type = 1; /* keyframe */

    rc = lrtmp2_client_send_frame(client, &frame);
    if (rc != 0) {
        fprintf(stderr, "Failed to send frame\n");
        lrtmp2_client_destroy(client);
        return 1;
    }
    printf("[minimal_client] Sent %u-byte video frame\n", frame.size);

    lrtmp2_client_destroy(client);
    return 0;
}
