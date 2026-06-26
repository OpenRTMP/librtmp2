/**
 * client.c — Outbound RTMP client
 */
#include "client.h"
#include "core/log.h"
#include "core/alloc.h"
#include <string.h>
#include "librtmp2/types.h"

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
#define close_socket closesocket
#define SHUT_RDWR SD_BOTH
#else
#include <unistd.h>
#include <stdio.h>
#define close_socket close
#endif
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

lrtmp2_client_t *lrtmp2_client_create(lrtmp2_server_config_t *config)
{
    lrtmp2_client_t *client = LRTMP2_CALLOC(1, sizeof(lrtmp2_client_t));
    if (!client) return NULL;

    client->client_fd = -1;
    lrtmp2_handshake_client_init(&client->handshake);

    LRTMP2_LOG_DEBUG("Client created");
    return client;
}

void lrtmp2_client_destroy(lrtmp2_client_t *client)
{
    if (!client) return;
    if (client->client_fd >= 0) {
        close_socket(client->client_fd);
    }
    LRTMP2_FREE(client);
}

int lrtmp2_client_connect(lrtmp2_client_t *client, const char *url)
{
    if (!client || !url) return LRTMP2_ERR_INTERNAL;

    /* Parse URL: rtmp://host:port/app/stream_key */
    char host[256];
    int port = 1935;
    char app[256];
    char stream_key[256];

    /* Simple URL parser */
    const char *p = url;
    if (strncmp(p, "rtmp://", 7) == 0) p += 7;

    const char *slash = strchr(p, '/');
    if (slash) {
        size_t host_len = (size_t)(slash - p);
        if (host_len >= sizeof(host)) return LRTMP2_ERR_INTERNAL;
        memcpy(host, p, host_len);
        host[host_len] = '\0';

        const char *app_start = slash + 1;
        const char *stream_slash = strchr(app_start, '/');
        if (stream_slash) {
            size_t app_len = (size_t)(stream_slash - app_start);
            if (app_len >= sizeof(app)) return LRTMP2_ERR_INTERNAL;
            memcpy(app, app_start, app_len);
            app[app_len] = '\0';
            snprintf(stream_key, sizeof(stream_key), "%s", stream_slash + 1);
        } else {
            snprintf(app, sizeof(app), "%s", app_start);
            stream_key[0] = '\0';
        }
    } else {
        snprintf(host, sizeof(host), "%s", p);
        app[0] = '\0';
        stream_key[0] = '\0';
    }

    /* Extract port from host if present */
    char *colon = strrchr(host, ':');
    if (colon) {
        port = atoi(colon + 1);
        *colon = '\0';
    }

    LRTMP2_LOG_INFO("Connecting to rtmp://%s:%d/%s/%s", host, port, app, stream_key);

    /* Create socket and connect */
    client->client_fd = socket(AF_INET, SOCK_STREAM, 0);
    if (client->client_fd < 0) return LRTMP2_ERR_IO;

    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons((uint16_t)port);

    if (inet_pton(AF_INET, host, &addr.sin_addr) <= 0) {
        close_socket(client->client_fd);
        client->client_fd = -1;
        return LRTMP2_ERR_IO;
    }

    if (connect(client->client_fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close_socket(client->client_fd);
        client->client_fd = -1;
        return LRTMP2_ERR_IO;
    }

    /* Generate C0+C1 */
    int rc = lrtmp2_handshake_client_generate_c0c1(&client->handshake);
    if (rc != LRTMP2_OK) return rc;

    /* Send C0+C1 */
    size_t to_send = client->handshake.out.size;
    ssize_t sent = send(client->client_fd, client->handshake.out.data, to_send, 0);
    if ((size_t)sent != to_send) {
        return LRTMP2_ERR_IO;
    }

    LRTMP2_LOG_DEBUG("Sent C0+C1 (%zu bytes)", to_send);
    return LRTMP2_OK;
}
