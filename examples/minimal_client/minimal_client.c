/**
 * minimal_client.c — Example: minimal RTMP client
 *
 * Connects to an RTMP server and plays a stream.
 * Usage: ./minimal_client rtmp://host:port/app/stream_key
 */
#include "librtmp2.h"
#include <stdio.h>
#include <stdlib.h>

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

    printf("[minimal_client] Connected (stub — full client in Phase 2)\n");

    lrtmp2_client_destroy(client);
    return 0;
}
