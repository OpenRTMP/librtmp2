/**
 * server.c — RTMP server listener
 *
 * Listens on a TCP port, accepts connections, and feeds data to connection handlers.
 * Uses a simple poll/select model for single-threaded operation.
 */
#include "server.h"
#include "session/conn.h"
#include "session/state_machine.h"
#include "core/log.h"
#include "core/alloc.h"
#include <string.h>
#include <stdlib.h>
#include <errno.h>
#include "librtmp2/types.h"

/* Platform-specific includes */
#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
typedef int socklen_t;
#define close_socket closesocket
#define SHUT_RDWR SD_BOTH
#else
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <fcntl.h>
#include <poll.h>
#define close_socket close
#define INVALID_SOCKET -1
#define SOCKET_ERROR -1
#endif

#include <stdio.h>

lrtmp2_server_t *lrtmp2_server_create(const lrtmp2_server_config_t *config)
{
    if (!config) return NULL;

    lrtmp2_server_t *server = LRTMP2_CALLOC(1, sizeof(lrtmp2_server_t));
    if (!server) return NULL;

    server->config = config;
    server->running = 0;
    server->streams = NULL;
    server->connections = NULL;
    server->server_fd = INVALID_SOCKET;
    pthread_mutex_init(&server->streams_mutex, NULL);
    pthread_mutex_init(&server->connections_mutex, NULL);

    LRTMP2_LOG_INFO("Server created (max_connections=%d, chunk_size=%d)",
                     config->max_connections, config->chunk_size);
    return server;
}

void lrtmp2_server_destroy(lrtmp2_server_t *server)
{
    if (!server) return;

    /* Destroy all connections */
    pthread_mutex_lock(&server->connections_mutex);
    lrtmp2_conn_t *conn = server->connections;
    while (conn) {
        lrtmp2_conn_t *next = conn->next;
        if (conn->client_fd >= 0) {
            close_socket(conn->client_fd);
        }
        lrtmp2_conn_destroy(conn);
        conn = next;
    }
    server->connections = NULL;
    pthread_mutex_unlock(&server->connections_mutex);

    /* Destroy all streams */
    pthread_mutex_lock(&server->streams_mutex);
    lrtmp2_stream_t *s = server->streams;
    while (s) {
        lrtmp2_stream_t *next = s->next;
        lrtmp2_stream_destroy(s);
        s = next;
    }
    pthread_mutex_unlock(&server->streams_mutex);

    pthread_mutex_destroy(&server->streams_mutex);
    pthread_mutex_destroy(&server->connections_mutex);

    if (server->server_fd != INVALID_SOCKET) {
        close_socket(server->server_fd);
    }

    LRTMP2_LOG_INFO("Server destroyed");
    LRTMP2_FREE(server);
}

int lrtmp2_server_listen(lrtmp2_server_t *server, const char *bind_addr)
{
    if (!server || !bind_addr) return LRTMP2_ERR_INTERNAL;

    /* Parse bind address (format: "host:port") */
    char host[256];
    int port = 1935;  /* default RTMP port */

    const char *colon = strrchr(bind_addr, ':');
    if (colon) {
        size_t host_len = (size_t)(colon - bind_addr);
        if (host_len >= sizeof(host)) return LRTMP2_ERR_INTERNAL;
        memcpy(host, bind_addr, host_len);
        host[host_len] = '\0';
        port = atoi(colon + 1);
    } else {
        snprintf(host, sizeof(host), "%s", bind_addr);
    }

    /* Create socket */
    server->server_fd = socket(AF_INET, SOCK_STREAM, 0);
    if (server->server_fd == INVALID_SOCKET) {
        LRTMP2_LOG_ERROR("Failed to create socket: %s", strerror(errno));
        return LRTMP2_ERR_IO;
    }

    /* Allow address reuse */
    int opt = 1;
    setsockopt(server->server_fd, SOL_SOCKET, SO_REUSEADDR, (const char *)&opt, sizeof(opt));

    /* Bind */
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons((uint16_t)port);

    if (strcmp(host, "0.0.0.0") == 0) {
        addr.sin_addr.s_addr = INADDR_ANY;
    } else if (inet_pton(AF_INET, host, &addr.sin_addr) <= 0) {
        LRTMP2_LOG_ERROR("Invalid bind address: %s", host);
        close_socket(server->server_fd);
        server->server_fd = INVALID_SOCKET;
        return LRTMP2_ERR_IO;
    }

    if (bind(server->server_fd, (struct sockaddr *)&addr, sizeof(addr)) == SOCKET_ERROR) {
        LRTMP2_LOG_ERROR("Failed to bind: %s", strerror(errno));
        close_socket(server->server_fd);
        server->server_fd = INVALID_SOCKET;
        return LRTMP2_ERR_IO;
    }

    /* Listen */
    if (listen(server->server_fd, SOMAXCONN) == SOCKET_ERROR) {
        LRTMP2_LOG_ERROR("Failed to listen: %s", strerror(errno));
        close_socket(server->server_fd);
        server->server_fd = INVALID_SOCKET;
        return LRTMP2_ERR_IO;
    }

    server->running = 1;
    LRTMP2_LOG_INFO("Server listening on %s:%d", host, port);
    return LRTMP2_OK;
}

int lrtmp2_server_poll(lrtmp2_server_t *server, int timeout_ms)
{
    if (!server || !server->running || server->server_fd == INVALID_SOCKET) {
        return LRTMP2_ERR_INTERNAL;
    }

    /* Poll the listen socket plus every active client socket so a single
     * poll() call drives both accepting new connections and servicing data on
     * existing ones. (Polling only the listen fd would never wake for client
     * data, so handshakes/commands would never be processed.) */
    enum { POLL_BASE = 1 };
    nfds_t cap = POLL_BASE;
    pthread_mutex_lock((pthread_mutex_t *)&server->connections_mutex);
    for (lrtmp2_conn_t *c = server->connections; c; c = c->next) {
        if (c->client_fd >= 0 && c->state < LRTMP2_STATE_CLOSING) cap++;
    }
    struct pollfd *pfds = LRTMP2_MALLOC(cap * sizeof(*pfds));
    if (!pfds) {
        pthread_mutex_unlock((pthread_mutex_t *)&server->connections_mutex);
        return LRTMP2_ERR_INTERNAL;
    }
    pfds[0].fd = server->server_fd;
    pfds[0].events = POLLIN;
    pfds[0].revents = 0;
    nfds_t nfds = POLL_BASE;
    for (lrtmp2_conn_t *c = server->connections; c && nfds < cap; c = c->next) {
        if (c->client_fd >= 0 && c->state < LRTMP2_STATE_CLOSING) {
            pfds[nfds].fd = c->client_fd;
            pfds[nfds].events = POLLIN;
            pfds[nfds].revents = 0;
            nfds++;
        }
    }
    pthread_mutex_unlock((pthread_mutex_t *)&server->connections_mutex);

    int rc = poll(pfds, nfds, timeout_ms);
    int listen_ready = (rc > 0 && (pfds[0].revents & POLLIN));
    LRTMP2_FREE(pfds);

    if (rc < 0) {
        if (errno == EINTR) return 0;
        return LRTMP2_ERR_IO;
    }

    /* Accept a new connection if the listen socket is ready. */
    if (listen_ready) {
        struct sockaddr_in client_addr;
        socklen_t addr_len = sizeof(client_addr);
        int client_fd = accept(server->server_fd, (struct sockaddr *)&client_addr, &addr_len);
        if (client_fd == INVALID_SOCKET) {
            LRTMP2_LOG_WARN("Accept failed: %s", strerror(errno));
        } else {
            LRTMP2_LOG_INFO("New connection from %s:%d",
                             inet_ntoa(client_addr.sin_addr), ntohs(client_addr.sin_port));
            lrtmp2_conn_t *conn = lrtmp2_conn_create((lrtmp2_server_t *)server, server->config);
            if (!conn) {
                LRTMP2_LOG_ERROR("Failed to create connection context");
                close_socket(client_fd);
            } else {
                conn->client_fd = client_fd;
                pthread_mutex_lock((pthread_mutex_t *)&server->connections_mutex);
                conn->next = server->connections;
                server->connections = conn;
                pthread_mutex_unlock((pthread_mutex_t *)&server->connections_mutex);
                if (server->config->on_connect_cb) {
                    server->config->on_connect_cb(conn, server->config->userdata);
                }
            }
        }
    }

    /* Service data on all active connections (recv/process/flush). */
    lrtmp2_server_process_connections(server);

    return LRTMP2_OK;
}

int lrtmp2_server_broadcast(lrtmp2_server_t *server, const uint8_t *data, size_t len)
{
    /* Send data to all connections — used for relaying frames to players */
    pthread_mutex_lock((pthread_mutex_t *)&server->connections_mutex);
    lrtmp2_conn_t *conn = server->connections;
    while (conn) {
        if (conn->on_send_data) {
            conn->on_send_data(conn, data, len, conn->userdata);
        }
        conn = conn->next;
    }
    pthread_mutex_unlock((pthread_mutex_t *)&server->connections_mutex);
    return LRTMP2_OK;
}

int lrtmp2_server_process_connections(lrtmp2_server_t *server)
{
    if (!server) return LRTMP2_ERR_INTERNAL;

    pthread_mutex_lock((pthread_mutex_t *)&server->connections_mutex);
    lrtmp2_conn_t *conn = server->connections;
    lrtmp2_conn_t *prev = NULL;
    while (conn) {
        lrtmp2_conn_t *next = conn->next;
        if (conn->client_fd >= 0 && conn->state < LRTMP2_STATE_CLOSING) {
            /* Try to receive data (non-blocking) */
            uint8_t tmp_buf[4096];
            ssize_t n = recv(conn->client_fd, tmp_buf, sizeof(tmp_buf), MSG_DONTWAIT);
            if (n > 0) {
                lrtmp2_conn_recv(conn, tmp_buf, (size_t)n);
                lrtmp2_conn_process(conn);
                lrtmp2_conn_flush(conn);
            } else if (n == 0) {
                /* Client disconnected */
                conn->state = LRTMP2_STATE_CLOSING;
                if (server->config->on_close_cb) {
                    server->config->on_close_cb(conn, server->config->userdata);
                }
                conn->client_fd = -1;  /* block further processing */
            } else if (n < 0 && errno != EAGAIN && errno != EWOULDBLOCK) {
                /* Real error */
                conn->state = LRTMP2_STATE_CLOSING;
            }
        }

        /* Remove dead connections from list (naturally freeded inside conn_destroy). */
        if (conn->state >= LRTMP2_STATE_CLOSING && conn->client_fd < 0) {
            if (prev) prev->next = next; else server->connections = next;
            lrtmp2_conn_destroy(conn);
        } else {
            prev = conn;
        }
        conn = next;
    }
    pthread_mutex_unlock((pthread_mutex_t *)&server->connections_mutex);
    return LRTMP2_OK;
}

void lrtmp2_stream_append_to_server(lrtmp2_server_t *server, lrtmp2_stream_t *stream)
{
    pthread_mutex_lock((pthread_mutex_t *)&server->streams_mutex);
    stream->next = server->streams;
    server->streams = stream;
    pthread_mutex_unlock((pthread_mutex_t *)&server->streams_mutex);
}

void lrtmp2_server_stop(lrtmp2_server_t *server)
{
    if (!server) return;
    server->running = 0;
    if (server->server_fd != INVALID_SOCKET) {
        shutdown(server->server_fd, SHUT_RDWR);
    }
    LRTMP2_LOG_INFO("Server stop requested");
}
