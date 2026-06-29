/**
 * server.c — RTMP server listener
 *
 * Listens on a TCP port, accepts connections, and feeds data to connection handlers.
 * Uses a simple poll/select model for single-threaded operation.
 */
/* Expose POSIX/BSD socket APIs (getaddrinfo, MSG_DONTWAIT, ...) when built with
 * a strict -std=c11: the meson build sets c_std=c11, under which glibc hides
 * them unless a feature-test macro is requested first. Must precede all includes. */
#ifndef _DEFAULT_SOURCE
#define _DEFAULT_SOURCE 1
#endif
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
#include <netdb.h>
#include <unistd.h>
#include <fcntl.h>
#include <poll.h>
#define close_socket close
#define INVALID_SOCKET -1
#define SOCKET_ERROR -1
#endif

#include "core/net.h"

#include <stdio.h>

static int server_count_active_connections(lrtmp2_server_t *server)
{
    int count = 0;
    pthread_mutex_lock((pthread_mutex_t *)&server->connections_mutex);
    for (lrtmp2_conn_t *c = server->connections; c; c = c->next) {
        if (c->client_fd >= 0 && c->state < LRTMP2_STATE_CLOSING) {
            count++;
        }
    }
    pthread_mutex_unlock((pthread_mutex_t *)&server->connections_mutex);
    return count;
}

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
    server->tls_ctx = NULL;
    pthread_mutex_init(&server->streams_mutex, NULL);
    pthread_mutex_init(&server->connections_mutex, NULL);

    /* When TLS termination is requested, build the shared server context up
     * front so a bad cert/key (or a TLS-less build) fails fast at create time
     * rather than on the first accepted connection. */
    if (config->tls_enabled) {
        server->tls_ctx = lrtmp2_tls_ctx_new_server(config->tls_cert_file,
                                                    config->tls_key_file);
        if (!server->tls_ctx) {
            LRTMP2_LOG_ERROR("Server TLS enabled but context could not be created");
            pthread_mutex_destroy(&server->streams_mutex);
            pthread_mutex_destroy(&server->connections_mutex);
            LRTMP2_FREE(server);
            return NULL;
        }
    }

    LRTMP2_LOG_INFO("Server created (max_connections=%d, chunk_size=%d, tls=%s)",
                     config->max_connections, config->chunk_size,
                     server->tls_ctx ? "on" : "off");
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

    if (server->tls_ctx) {
        lrtmp2_tls_ctx_free(server->tls_ctx);
    }

    if (server->server_fd != INVALID_SOCKET) {
        close_socket(server->server_fd);
    }

    LRTMP2_LOG_INFO("Server destroyed");
    LRTMP2_FREE(server);
}

/* Parse a TCP port from a NUL-terminated string. Returns the port on success,
 * or -1 if the string is empty, non-numeric, has trailing junk, or falls
 * outside 1..65535. atoi() silently turned all of those into a bogus port. */
static int parse_port(const char *s)
{
    if (!s || *s == '\0') return -1;
    char *end = NULL;
    long v = strtol(s, &end, 10);
    if (end == s || *end != '\0' || v < 1 || v > 65535) return -1;
    return (int)v;
}

int lrtmp2_server_listen(lrtmp2_server_t *server, const char *bind_addr)
{
    if (!server || !bind_addr) return LRTMP2_ERR_INTERNAL;

    /* Parse "host:port" / "[v6]:port" / bare host. An empty host (e.g. ":1935"
     * or just a port) means "wildcard" — bind every local interface. */
    char host[256];
    char port[16];
    if (lrtmp2_split_host_port(bind_addr, host, sizeof(host), port, sizeof(port), "1935") != 0) {
        LRTMP2_LOG_ERROR("Invalid bind address: %s", bind_addr);
        return LRTMP2_ERR_INTERNAL;
    }
    if (parse_port(port) < 0) {
        LRTMP2_LOG_ERROR("Invalid port in bind address: %s", port);
        return LRTMP2_ERR_INTERNAL;
    }

    /* Resolve with getaddrinfo so hostnames, IPv4 and IPv6 all work. AI_PASSIVE
     * yields a wildcard address when no host is given. A numeric "0.0.0.0"
     * selects the IPv4 wildcard; "::" (or an empty host, picking the first
     * result) selects IPv6 with V6ONLY disabled below, so a single socket
     * accepts both families. */
    struct addrinfo hints;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_flags = AI_PASSIVE;

    const char *node = (host[0] != '\0') ? host : NULL;
    struct addrinfo *res = NULL;
    int gai = getaddrinfo(node, port, &hints, &res);
    if (gai != 0) {
        LRTMP2_LOG_ERROR("Cannot resolve bind address '%s': %s", bind_addr, gai_strerror(gai));
        return LRTMP2_ERR_IO;
    }

    server->server_fd = INVALID_SOCKET;
    for (struct addrinfo *rp = res; rp; rp = rp->ai_next) {
        int fd = socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (fd == INVALID_SOCKET) continue;

        int opt = 1;
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, (const char *)&opt, sizeof(opt));
#ifdef IPV6_V6ONLY
        if (rp->ai_family == AF_INET6) {
            int off = 0;  /* let the IPv6 socket also accept IPv4-mapped clients */
            setsockopt(fd, IPPROTO_IPV6, IPV6_V6ONLY, (const char *)&off, sizeof(off));
        }
#endif
        if (bind(fd, rp->ai_addr, (socklen_t)rp->ai_addrlen) == 0) {
            server->server_fd = fd;
            break;
        }
        close_socket(fd);
    }
    freeaddrinfo(res);

    if (server->server_fd == INVALID_SOCKET) {
        LRTMP2_LOG_ERROR("Failed to bind %s: %s", bind_addr, strerror(errno));
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
    LRTMP2_LOG_INFO("Server listening on %s:%s", node ? node : "*", port);
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
            /* A pending TLS handshake can leave SSL_accept() wanting to write
             * (e.g. a multi-record certificate chain) rather than read. POLLIN
             * alone would then never wake us, stalling the handshake until its
             * timeout instead of completing as soon as the socket is ready. */
            pfds[nfds].events = POLLIN;
            if (c->transport && lrtmp2_transport_tls_handshake_wants_write(c->transport)) {
                pfds[nfds].events |= POLLOUT;
            }
            pfds[nfds].revents = 0;
            nfds++;
        }
    }
    pthread_mutex_unlock((pthread_mutex_t *)&server->connections_mutex);

    int rc = poll(pfds, nfds, timeout_ms);
    int listen_ready = (rc > 0 && (pfds[0].revents & POLLIN));
    /* listen_ready has captured everything we need from pfds; free it now so the
     * error/EINTR early-returns below don't leak the array. */
    LRTMP2_FREE(pfds);

    if (rc < 0) {
        if (errno == EINTR) return 0;
        return LRTMP2_ERR_IO;
    }

    /* Accept a new connection if the listen socket is ready. */
    if (listen_ready) {
        struct sockaddr_storage client_addr;
        socklen_t addr_len = sizeof(client_addr);
        int client_fd = accept(server->server_fd, (struct sockaddr *)&client_addr, &addr_len);
        /* Format the peer as host:port for logging, IPv4 or IPv6. getnameinfo
         * writes into caller-supplied buffers (thread-safe) and handles both
         * families; IPv6 peers are bracketed, e.g. "[2001:db8::1]:54321". */
        char client_ep[INET6_ADDRSTRLEN + 16];
        if (client_fd != INVALID_SOCKET) {
            char hbuf[INET6_ADDRSTRLEN];
            char sbuf[8];
            if (getnameinfo((struct sockaddr *)&client_addr, addr_len,
                            hbuf, sizeof(hbuf), sbuf, sizeof(sbuf),
                            NI_NUMERICHOST | NI_NUMERICSERV) == 0) {
                if (client_addr.ss_family == AF_INET6) {
                    snprintf(client_ep, sizeof(client_ep), "[%s]:%s", hbuf, sbuf);
                } else {
                    snprintf(client_ep, sizeof(client_ep), "%s:%s", hbuf, sbuf);
                }
            } else {
                snprintf(client_ep, sizeof(client_ep), "unknown");
            }
        } else {
            client_ep[0] = '\0';
        }
        if (client_fd == INVALID_SOCKET) {
            LRTMP2_LOG_WARN("Accept failed: %s", strerror(errno));
        } else if (server->config->max_connections > 0 &&
                   server_count_active_connections(server) >= server->config->max_connections) {
            LRTMP2_LOG_WARN("Rejecting connection from %s: max_connections=%d reached",
                             client_ep, server->config->max_connections);
            close_socket(client_fd);
        } else {
            LRTMP2_LOG_INFO("New connection from %s", client_ep);
            lrtmp2_conn_t *conn = lrtmp2_conn_create((lrtmp2_server_t *)server, server->config);
            if (!conn) {
                LRTMP2_LOG_ERROR("Failed to create connection context");
                close_socket(client_fd);
                return LRTMP2_OK;
            }

            /* Attach the transport: a TLS session (terminating RTMPS) when the
             * server has a TLS context, otherwise plaintext. For RTMPS the TLS
             * handshake is completed incrementally in process_connections() so
             * a slow peer cannot block the whole poll loop. */
            lrtmp2_transport_t *transport;
            if (server->tls_ctx) {
                transport = lrtmp2_transport_new_tls_server(server->tls_ctx, client_fd);
                if (!transport) {
                    LRTMP2_LOG_WARN("Dropping %s: TLS transport setup failed", client_ep);
                    lrtmp2_conn_destroy(conn);
                    close_socket(client_fd);
                    return LRTMP2_OK;
                }
            } else {
                transport = lrtmp2_transport_new_plain(client_fd);
                if (!transport) {
                    LRTMP2_LOG_ERROR("Failed to create transport");
                    lrtmp2_conn_destroy(conn);
                    close_socket(client_fd);
                    return LRTMP2_OK;
                }
            }

            {
                conn->client_fd = client_fd;
                conn->transport = transport;
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

    /* Phase 1 — snapshot the connections to service.
     *
     * We must NOT hold connections_mutex while running host callbacks: conn_recv
     * drives on_frame/on_publish/on_play, and the teardown below fires on_close.
     * A callback that re-enters a connections_mutex-taking API — e.g.
     * lrtmp2_server_broadcast(), the intended way to relay frames to players —
     * would self-deadlock on this non-recursive mutex. (accept's on_connect_cb is
     * already invoked outside the lock for the same reason.) So we copy the
     * current connection pointers under the lock, release it, then do all network
     * I/O and callbacks unlocked, and finally reap under the lock again. */
    pthread_mutex_lock((pthread_mutex_t *)&server->connections_mutex);
    size_t n_conns = 0;
    for (lrtmp2_conn_t *c = server->connections; c; c = c->next) n_conns++;
    lrtmp2_conn_t **snapshot = NULL;
    if (n_conns > 0) {
        snapshot = LRTMP2_MALLOC(n_conns * sizeof(*snapshot));
        if (!snapshot) {
            pthread_mutex_unlock((pthread_mutex_t *)&server->connections_mutex);
            return LRTMP2_ERR_INTERNAL;
        }
        size_t i = 0;
        for (lrtmp2_conn_t *c = server->connections; c && i < n_conns; c = c->next) {
            snapshot[i++] = c;
        }
    }
    pthread_mutex_unlock((pthread_mutex_t *)&server->connections_mutex);

    /* Phase 2 — service each connection with the lock released. All host
     * callbacks fire here and are free to call back into the server. */
    for (size_t i = 0; i < n_conns; i++) {
        lrtmp2_conn_t *conn = snapshot[i];
        if (conn->client_fd >= 0 && conn->state < LRTMP2_STATE_CLOSING) {
            /* Complete a pending server-side TLS handshake before reading RTMP
             * bytes. One step per poll keeps a slow peer from starving others. */
            if (conn->transport &&
                lrtmp2_transport_tls_handshake_pending(conn->transport)) {
                int hs = lrtmp2_transport_tls_handshake_advance(conn->transport);
                if (hs < 0) {
                    conn->state = LRTMP2_STATE_CLOSING;
                    continue;
                }
                if (hs == 0) {
                    continue;
                }
            }

            /* Drain currently readable data. A single recv() is not enough for
             * TLS: SSL_read can leave a further record already decrypted in the
             * SSL buffer that poll() will never wake us for, so we loop until the
             * transport reports "would block" (again). Plaintext behaves the same
             * way and simply stops once the socket buffer is empty.
             *
             * The loop is bounded per poll iteration so one constantly-readable
             * peer cannot monopolise the single-threaded server and starve other
             * connections; whatever is left is picked up on the next poll(). */
            enum { MAX_DRAIN_READS = 64 };
            uint8_t tmp_buf[4096];
            for (int reads = 0; reads < MAX_DRAIN_READS; reads++) {
                int again = 0;
                ssize_t n = lrtmp2_transport_recv(conn->transport, tmp_buf,
                                                  sizeof(tmp_buf), &again);
                if (n > 0) {
                    /* conn_recv already drives conn_process (and flushes
                     * responses) internally; just flush any trailing queued
                     * bytes afterwards. A negative return means malformed/
                     * oversized input — tear the connection down rather than
                     * spinning on un-parseable bytes. */
                    if (lrtmp2_conn_recv(conn, tmp_buf, (size_t)n) == LRTMP2_OK) {
                        lrtmp2_conn_flush(conn);
                    } else {
                        conn->state = LRTMP2_STATE_CLOSING;
                        break;
                    }
                    /* keep draining until the transport would block */
                } else if (n == 0) {
                    /* Client disconnected gracefully */
                    conn->state = LRTMP2_STATE_CLOSING;
                    break;
                } else {
                    /* n < 0: stop on would-block, tear down on a real error. */
                    if (!again && errno != EAGAIN && errno != EWOULDBLOCK) {
                        conn->state = LRTMP2_STATE_CLOSING;
                    }
                    break;
                }
            }
        }

        /* Close the socket and fire on_close exactly once (gated on a still-open
         * fd), still outside the lock. Closing here — rather than only on the
         * n==0 path — avoids leaking the socket fd on the error path; the struct
         * itself is unlinked and freed in phase 3. Free the transport first so
         * any TLS close_notify goes out before the fd is closed. */
        if (conn->state >= LRTMP2_STATE_CLOSING && conn->client_fd >= 0) {
            if (conn->transport) {
                lrtmp2_transport_free(conn->transport);
                conn->transport = NULL;
            }
            close_socket(conn->client_fd);
            conn->client_fd = -1;
            if (server->config->on_close_cb) {
                server->config->on_close_cb(conn, server->config->userdata);
            }
        }
    }

    LRTMP2_FREE(snapshot);

    /* Phase 3 — reap. Re-traverse the live list under the lock and unlink/destroy
     * every connection that has reached CLOSING. Re-deriving the list here (rather
     * than reusing the snapshot) keeps the unlink correct even if the list changed
     * while unlocked, and avoids freeing through a stale snapshot pointer.
     * conn_destroy fires no host callbacks, so it is safe under the lock. */
    pthread_mutex_lock((pthread_mutex_t *)&server->connections_mutex);
    lrtmp2_conn_t *conn = server->connections;
    lrtmp2_conn_t *prev = NULL;
    while (conn) {
        lrtmp2_conn_t *next = conn->next;
        if (conn->state >= LRTMP2_STATE_CLOSING) {
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

void lrtmp2_stream_remove_owned_by_conn(lrtmp2_server_t *server, struct lrtmp2_conn *conn)
{
    if (!server) return;
    pthread_mutex_lock(&server->streams_mutex);
    lrtmp2_stream_t *s = server->streams;
    lrtmp2_stream_t *prev = NULL;
    while (s) {
        lrtmp2_stream_t *next = s->next;
        if (s->conn == conn) {
            if (prev) prev->next = next; else server->streams = next;
            lrtmp2_stream_destroy(s);
        } else {
            prev = s;
        }
        s = next;
    }
    pthread_mutex_unlock(&server->streams_mutex);
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
