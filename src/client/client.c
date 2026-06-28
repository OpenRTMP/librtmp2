/**
 * client.c — Outbound RTMP client
 *
 * Mirrors the server-side handshake/chunk/command machinery in session/conn.c,
 * but drives it from the client's perspective: send C0/C1, block for S0/S1/S2,
 * send the app-level connect/createStream/publish (or play) commands and block
 * for the matching _result/onStatus, then stream frames.
 *
 * Incoming AMF0 command responses ("_result"/"onStatus") can't be decoded via
 * message.c's lrtmp2_msg_decode()/lrtmp2_conn_handle_command(), since those are
 * server-oriented (they expect "connect"/"publish"/... requests, not client-side
 * responses). So this file reads chunks directly via lrtmp2_chunk_read() and
 * does its own minimal message dispatch.
 */
/* Expose POSIX/BSD socket APIs (getaddrinfo, ...) when built with a strict
 * -std=c11: the meson build sets c_std=c11, under which glibc hides them unless
 * a feature-test macro is requested first. Must precede all includes. */
#ifndef _DEFAULT_SOURCE
#define _DEFAULT_SOURCE 1
#endif
#include "client.h"
#include "core/log.h"
#include "core/alloc.h"
#include "chunk/chunk_state.h"
#include "chunk/chunk_reader.h"
#include "chunk/chunk_writer.h"
#include "message/command.h"
#include "message/control.h"
#include "amf/amf.h"
#include <string.h>
#include <stdlib.h>
#include <errno.h>
#include "librtmp2/types.h"

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
#define close_socket closesocket
#define SHUT_RDWR SD_BOTH
#else
#include <unistd.h>
#include <stdio.h>
#include <poll.h>
#include <netdb.h>
#define close_socket close
#endif
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include "core/net.h"

/* RTMP message type IDs (mirrors message.c) */
#define RTMP_MSG_SET_CHUNK_SIZE       0x01
#define RTMP_MSG_AUDIO                0x08
#define RTMP_MSG_VIDEO                0x09
#define RTMP_MSG_AMF0_COMMAND         0x14

static int on_connect_result(lrtmp2_client_t *client, lrtmp2_buffer_t *buf);

lrtmp2_client_t *lrtmp2_client_create(const lrtmp2_server_config_t *config)
{
    lrtmp2_client_t *client = LRTMP2_CALLOC(1, sizeof(lrtmp2_client_t));
    if (!client) return NULL;

    client->client_fd = -1;
    client->state = LRTMP2_CLIENT_DISCONNECTED;
    client->config = config;
    lrtmp2_handshake_client_init(&client->handshake);

    client->send_buffer = lrtmp2_buffer_create();
    client->recv_buffer = lrtmp2_buffer_create();
    if (!client->send_buffer || !client->recv_buffer) {
        if (client->send_buffer) lrtmp2_buffer_destroy(client->send_buffer);
        if (client->recv_buffer) lrtmp2_buffer_destroy(client->recv_buffer);
        LRTMP2_FREE(client);
        return NULL;
    }

    LRTMP2_LOG_DEBUG("Client created");
    return client;
}

void lrtmp2_client_destroy(lrtmp2_client_t *client)
{
    if (!client) return;
    if (client->transport) {
        /* Frees TLS state (sends close_notify) but not the fd. */
        lrtmp2_transport_free(client->transport);
        client->transport = NULL;
    }
    if (client->client_fd >= 0) {
        close_socket(client->client_fd);
    }
    if (client->send_buffer) lrtmp2_buffer_destroy(client->send_buffer);
    if (client->recv_buffer) lrtmp2_buffer_destroy(client->recv_buffer);
    lrtmp2_handshake_cleanup(&client->handshake);
    lrtmp2_chunk_registry_destroy(&client->chunk_reg);
    LRTMP2_FREE(client);
}

/* --- Blocking socket helpers --- */

static int client_send_raw(lrtmp2_client_t *client, const uint8_t *data, size_t len)
{
    if (client->client_fd < 0 || !client->transport) return LRTMP2_ERR_INTERNAL;
    return (lrtmp2_transport_send(client->transport, data, len) == 0)
               ? LRTMP2_OK : LRTMP2_ERR_IO;
}

static int client_flush(lrtmp2_client_t *client)
{
    if (client->send_buffer->size == 0) return LRTMP2_OK;
    int rc = client_send_raw(client, client->send_buffer->data, client->send_buffer->size);
    client->send_buffer->size = 0;
    client->send_buffer->read_pos = 0;
    return rc;
}

/* Blocking recv of whatever is available; appends to recv_buffer. The transport
 * exposes non-blocking recv semantics (so plaintext and TLS share one path), so
 * when it reports "would block" we wait on the socket to preserve the caller's
 * blocking expectations. We use poll() (not select(), which is undefined past
 * FD_SETSIZE in fd-heavy hosts) and wait on both readability and writability:
 * a TLS read can need the socket to become writable (WANT_WRITE), which a
 * read-only wait would never satisfy. */
static int client_recv_more(lrtmp2_client_t *client)
{
    uint8_t tmp[4096];
    for (;;) {
        int again = 0;
        ssize_t n = lrtmp2_transport_recv(client->transport, tmp, sizeof(tmp), &again);
        if (n > 0) {
            return lrtmp2_buffer_write(client->recv_buffer, tmp, (size_t)n);
        }
        if (n == 0) {
            LRTMP2_LOG_WARN("Peer closed connection");
            return LRTMP2_ERR_IO;
        }
        if (again || errno == EAGAIN || errno == EWOULDBLOCK) {
            struct pollfd pfd;
            pfd.fd = client->client_fd;
            /* again==2 means the transport (TLS) needs the socket writable;
             * otherwise wait for it to become readable. */
            pfd.events = (again == 2) ? POLLOUT : POLLIN;
            pfd.revents = 0;
            int rc = poll(&pfd, 1, -1);
            if (rc < 0) {
                if (errno == EINTR) continue;
                return LRTMP2_ERR_IO;
            }
            continue;
        }
        if (errno == EINTR) continue;
        return LRTMP2_ERR_IO;
    }
}

/* Repeatedly call a buffer-based handshake step until it succeeds (blocking
 * on more socket data as needed) or fails for a real protocol reason. */
typedef int (*handshake_step_fn)(lrtmp2_handshake_t *, lrtmp2_buffer_t *);

static int client_handshake_step(lrtmp2_client_t *client, handshake_step_fn step)
{
    for (;;) {
        int rc = step(&client->handshake, client->recv_buffer);
        if (rc == LRTMP2_OK) return LRTMP2_OK;
        if (rc != LRTMP2_ERR_IO) return rc;
        int rrc = client_recv_more(client);
        if (rrc != LRTMP2_OK) return rrc;
    }
}

static int client_send_command(lrtmp2_client_t *client, uint32_t msg_stream_id,
                                const uint8_t *amf_data, size_t amf_len)
{
    lrtmp2_chunk_message_t cmd_msg;
    memset(&cmd_msg, 0, sizeof(cmd_msg));
    cmd_msg.csid = 3;
    cmd_msg.fmt = 0;
    cmd_msg.timestamp = 0;
    cmd_msg.msg_length = (uint32_t)amf_len;
    cmd_msg.msg_type_id = RTMP_MSG_AMF0_COMMAND;
    cmd_msg.msg_stream_id = msg_stream_id;

    int rc = lrtmp2_chunk_write(client->send_buffer, &cmd_msg, amf_data, amf_len,
                                LRTMP2_DEFAULT_CHUNK_SIZE);
    if (rc != LRTMP2_OK) return rc;
    return client_flush(client);
}

/* Reads and dispatches messages until a command named `wanted_name` arrives
 * (its AMF0 payload is then handed to `on_match` for parsing), or play_mode
 * causes audio/video frames to be delivered via client->config->on_frame_cb.
 * Returns LRTMP2_OK once `wanted_name` has been seen and handled (when
 * wanted_name is non-NULL), or propagates errors. */
/* client_pump() reads and dispatches messages from recv_buffer.
 * - wanted_name + on_match: block until command with named arrives, then call on_match
 * - play_mode: deliver A/V frames via on_frame_cb (one frame per call)
 * - both NULL/NULL: plays frames, returns LRTMP2_OK after one frame */
static int client_pump(lrtmp2_client_t *client, const char *wanted_name,
                        int (*on_match)(lrtmp2_client_t *, lrtmp2_buffer_t *),
                        int play_mode)
{
    lrtmp2_chunk_message_t msg;
    const uint8_t *payload = NULL;
    size_t payload_len = 0;

    for (;;) {
        int rc = lrtmp2_chunk_read(client->recv_buffer, &client->chunk_reg, NULL, &msg, &payload, &payload_len);
        if (rc == 0) {
            int rrc = client_recv_more(client);
            if (rrc != LRTMP2_OK) return rrc;
            continue;
        }
        if (rc < 0) return rc;
        if (!msg.is_complete) continue;

        switch (msg.msg_type_id) {
            case RTMP_MSG_SET_CHUNK_SIZE: {
                uint32_t cs;
                if (payload_len >= 4 &&
                    lrtmp2_msg_read_set_chunk_size(payload, &cs) == LRTMP2_OK) {
                    lrtmp2_chunk_stream_set_all_chunk_size(&client->chunk_reg, cs);
                    LRTMP2_LOG_INFO("Peer SetChunkSize: %u", cs);
                }
                break;
            }
            case RTMP_MSG_AMF0_COMMAND: {
                lrtmp2_buffer_t buf;
                memset(&buf, 0, sizeof(buf));
                buf.data = (uint8_t *)payload;
                buf.size = payload_len;
                buf.capacity = payload_len;
                buf.read_pos = 0;

                char name[64];
                memset(name, 0, sizeof(name));
                if (lrtmp2_cmd_peek_name(&buf, name, sizeof(name)) != LRTMP2_OK) {
                    LRTMP2_LOG_WARN("Failed to read command name in client pump");
                    break;
                }
                if (wanted_name && strcmp(name, wanted_name) == 0) {
                    return on_match(client, &buf);
                }
                LRTMP2_LOG_DEBUG("Ignoring command in client pump: %s", name);
                break;
            }
            case RTMP_MSG_AUDIO:
            case RTMP_MSG_VIDEO: {
                if (play_mode && client->config && client->config->on_frame_cb) {
                    lrtmp2_frame_t frame;
                    memset(&frame, 0, sizeof(frame));
                    frame.type = (msg.msg_type_id == RTMP_MSG_AUDIO) ? LRTMP2_FRAME_AUDIO : LRTMP2_FRAME_VIDEO;
                    frame.timestamp = msg.timestamp;
                    frame.size = (uint32_t)payload_len;
                    frame.data = payload;
                    if (payload_len > 0) {
                        uint8_t tag = payload[0];
                        if (msg.msg_type_id == RTMP_MSG_AUDIO) {
                            frame.audio_codec = (lrtmp2_audio_codec_t)((tag >> 4) & 0x0F);
                        } else {
                            frame.video_frame_type = (tag >> 4) & 0x0F;
                            frame.video_codec = (lrtmp2_video_codec_t)(tag & 0x0F);
                        }
                    }
                    client->config->on_frame_cb(NULL, &frame, client->config->userdata);
                }
                if (!wanted_name) return LRTMP2_OK; /* one frame per poll call */
                break;
            }
            default:
                LRTMP2_LOG_DEBUG("Ignoring message type 0x%02x in client pump", msg.msg_type_id);
                break;
        }
    }
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

int lrtmp2_client_connect(lrtmp2_client_t *client, const char *url)
{
    if (!client || !url) return LRTMP2_ERR_INTERNAL;

    /* Tear down any prior connection so reconnecting with the same client object
     * does not leak the previous transport/socket. */
    if (client->transport) {
        lrtmp2_transport_free(client->transport);
        client->transport = NULL;
    }
    if (client->client_fd >= 0) {
        close_socket(client->client_fd);
        client->client_fd = -1;
    }

    /* Parse URL: rtmp://host:port/app/stream_key (or rtmps:// for TLS). host may
     * be a hostname, an IPv4 literal, or a bracketed IPv6 literal
     * (rtmp://[::1]:1935/app/key). rtmps:// defaults to port 443. */
    char authority[256];
    const char *p = url;
    int use_tls = 0;
    const char *def_port = "1935";
    const char *scheme = "rtmp";
    if (strncmp(p, "rtmps://", 8) == 0) {
        p += 8;
        use_tls = 1;
        def_port = "443";
        scheme = "rtmps";
        if (!lrtmp2_tls_available()) {
            LRTMP2_LOG_ERROR("rtmps:// requested but librtmp2 was built without TLS support");
            return LRTMP2_ERR_INTERNAL;
        }
    } else if (strncmp(p, "rtmp://", 7) == 0) {
        p += 7;
    }

    const char *slash = strchr(p, '/');
    if (slash) {
        size_t auth_len = (size_t)(slash - p);
        if (auth_len >= sizeof(authority)) return LRTMP2_ERR_INTERNAL;
        memcpy(authority, p, auth_len);
        authority[auth_len] = '\0';

        const char *app_start = slash + 1;
        const char *stream_slash = strchr(app_start, '/');
        if (stream_slash) {
            size_t app_len = (size_t)(stream_slash - app_start);
            if (app_len >= sizeof(client->app)) return LRTMP2_ERR_INTERNAL;
            memcpy(client->app, app_start, app_len);
            client->app[app_len] = '\0';
            snprintf(client->stream_key, sizeof(client->stream_key), "%s", stream_slash + 1);
        } else {
            snprintf(client->app, sizeof(client->app), "%s", app_start);
            client->stream_key[0] = '\0';
        }
    } else {
        if (strlen(p) >= sizeof(authority)) return LRTMP2_ERR_INTERNAL;
        snprintf(authority, sizeof(authority), "%s", p);
        client->app[0] = '\0';
        client->stream_key[0] = '\0';
    }

    char host[256];
    char port[16];
    if (lrtmp2_split_host_port(authority, host, sizeof(host), port, sizeof(port), def_port) != 0) {
        LRTMP2_LOG_ERROR("Invalid host/port in URL: %s", authority);
        return LRTMP2_ERR_INTERNAL;
    }
    if (parse_port(port) < 0) {
        LRTMP2_LOG_ERROR("Invalid port in URL: %s", port);
        return LRTMP2_ERR_INTERNAL;
    }

    LRTMP2_LOG_INFO("Connecting to %s://%s:%s/%s/%s", scheme, host, port, client->app, client->stream_key);

    /* Resolve host (DNS name or numeric literal, IPv4 or IPv6) and connect to
     * the first address that accepts. */
    struct addrinfo hints;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;

    struct addrinfo *res = NULL;
    int gai = getaddrinfo(host, port, &hints, &res);
    if (gai != 0) {
        LRTMP2_LOG_ERROR("Cannot resolve '%s': %s", host, gai_strerror(gai));
        return LRTMP2_ERR_IO;
    }

    client->client_fd = -1;
    for (struct addrinfo *rp = res; rp; rp = rp->ai_next) {
        int fd = socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (fd < 0) continue;
        if (connect(fd, rp->ai_addr, (socklen_t)rp->ai_addrlen) == 0) {
            client->client_fd = fd;
            break;
        }
        close_socket(fd);
    }
    freeaddrinfo(res);

    if (client->client_fd < 0) {
        LRTMP2_LOG_ERROR("Failed to connect to %s:%s: %s", host, port, strerror(errno));
        return LRTMP2_ERR_IO;
    }

    /* Wrap the socket in a transport before any RTMP bytes flow. For rtmps://
     * this runs the TLS handshake (SNI + cert verification) up front; the rest
     * of the RTMP handshake and command flow is transport-agnostic from here. */
    const char *ca_file = client->config ? client->config->tls_ca_file : NULL;
    int insecure = client->config ? client->config->tls_insecure : 0;
    if (use_tls) {
        client->transport = lrtmp2_transport_new_tls_client(client->client_fd, host,
                                                            ca_file, insecure);
        if (!client->transport) {
            LRTMP2_LOG_ERROR("TLS handshake to %s:%s failed", host, port);
            close_socket(client->client_fd);
            client->client_fd = -1;
            return LRTMP2_ERR_IO;
        }
    } else {
        client->transport = lrtmp2_transport_new_plain(client->client_fd);
        if (!client->transport) {
            close_socket(client->client_fd);
            client->client_fd = -1;
            return LRTMP2_ERR_INTERNAL;
        }
    }

    client->state = LRTMP2_CLIENT_HANDSHAKING;
    lrtmp2_chunk_registry_init(&client->chunk_reg);

    /* C0+C1 */
    int rc = lrtmp2_handshake_client_generate_c0c1(&client->handshake);
    if (rc != LRTMP2_OK) return rc;
    rc = client_send_raw(client, client->handshake.out.data, client->handshake.out.size);
    if (rc != LRTMP2_OK) return rc;
    LRTMP2_LOG_DEBUG("Sent C0+C1 (%zu bytes)", client->handshake.out.size);

    /* S0, S1 (+queue C2), then send C2, then S2 */
    rc = client_handshake_step(client, lrtmp2_handshake_client_read_s0);
    if (rc != LRTMP2_OK) return rc;

    rc = client_handshake_step(client, lrtmp2_handshake_client_read_s1);
    if (rc != LRTMP2_OK) return rc;
    rc = client_send_raw(client, client->handshake.out.data, client->handshake.out.size);
    if (rc != LRTMP2_OK) return rc;
    LRTMP2_LOG_DEBUG("Sent C2");

    rc = client_handshake_step(client, lrtmp2_handshake_client_read_s2);
    if (rc != LRTMP2_OK) return rc;

    client->state = LRTMP2_CLIENT_CONNECTED;
    LRTMP2_LOG_INFO("Handshake complete (client)");

    /* App-level "connect" command */
    uint8_t amf_data[1024];
    lrtmp2_buffer_t amf_buf;
    memset(&amf_buf, 0, sizeof(amf_buf));
    amf_buf.data = amf_data;
    amf_buf.capacity = sizeof(amf_data);

    char tc_url[768];
    if (strchr(host, ':')) {  /* IPv6 literal -> bracket it in the URL */
        snprintf(tc_url, sizeof(tc_url), "%s://[%s]:%s/%s", scheme, host, port, client->app);
    } else {
        snprintf(tc_url, sizeof(tc_url), "%s://%s:%s/%s", scheme, host, port, client->app);
    }

    rc = lrtmp2_cmd_build_connect(&amf_buf, client->app, tc_url, NULL, NULL, "FMLE/3.0", 0, 0);
    if (rc != LRTMP2_OK) return rc;
    rc = client_send_command(client, 0, amf_buf.data, amf_buf.size);
    if (rc != LRTMP2_OK) return rc;
    LRTMP2_LOG_DEBUG("Sent connect command");

    rc = client_pump(client, "_result", on_connect_result, 0);
    if (rc != LRTMP2_OK) return rc;

    client->state = LRTMP2_CLIENT_APP_CONNECTED;
    LRTMP2_LOG_INFO("connect: app=%s", client->app);
    return LRTMP2_OK;
}

static int on_connect_result(lrtmp2_client_t *client, lrtmp2_buffer_t *buf)
{
    (void)client;
    double txn;
    return lrtmp2_cmd_read_connect_result(buf, &txn);
}

static int on_create_stream_result(lrtmp2_client_t *client, lrtmp2_buffer_t *buf)
{
    double txn;
    double stream_id;
    int rc = lrtmp2_cmd_read_create_stream_result(buf, &txn, &stream_id);
    if (rc != LRTMP2_OK) return rc;
    client->stream_id = (uint32_t)stream_id;
    return LRTMP2_OK;
}

static int on_status_ignore(lrtmp2_client_t *client, lrtmp2_buffer_t *buf)
{
    (void)client;
    lrtmf2_amf0_skip_value(buf); /* "onStatus" name marker not present here: buf starts at name */
    return LRTMP2_OK;
}

static int client_create_stream(lrtmp2_client_t *client)
{
    uint8_t amf_data[64];
    lrtmp2_buffer_t amf_buf;
    memset(&amf_buf, 0, sizeof(amf_buf));
    amf_buf.data = amf_data;
    amf_buf.capacity = sizeof(amf_data);

    int rc = lrtmp2_cmd_build_create_stream(&amf_buf, 2.0);
    if (rc != LRTMP2_OK) return rc;
    rc = client_send_command(client, 0, amf_buf.data, amf_buf.size);
    if (rc != LRTMP2_OK) return rc;

    rc = client_pump(client, "_result", on_create_stream_result, 0);
    if (rc != LRTMP2_OK) return rc;

    client->state = LRTMP2_CLIENT_STREAM_CREATED;
    LRTMP2_LOG_INFO("createStream: stream_id=%u", client->stream_id);
    return LRTMP2_OK;
}

int lrtmp2_client_publish(lrtmp2_client_t *client)
{
    if (!client) return LRTMP2_ERR_INTERNAL;
    if (client->state != LRTMP2_CLIENT_APP_CONNECTED) return LRTMP2_ERR_PROTOCOL;

    int rc = client_create_stream(client);
    if (rc != LRTMP2_OK) return rc;

    uint8_t amf_data[512];
    lrtmp2_buffer_t amf_buf;
    memset(&amf_buf, 0, sizeof(amf_buf));
    amf_buf.data = amf_data;
    amf_buf.capacity = sizeof(amf_data);

    rc = lrtmp2_cmd_build_publish(&amf_buf, client->stream_key, "live");
    if (rc != LRTMP2_OK) return rc;
    rc = client_send_command(client, client->stream_id, amf_buf.data, amf_buf.size);
    if (rc != LRTMP2_OK) return rc;

    rc = client_pump(client, "onStatus", on_status_ignore, 0);
    if (rc != LRTMP2_OK) return rc;

    client->state = LRTMP2_CLIENT_PUBLISHING;
    LRTMP2_LOG_INFO("publish: stream=%s", client->stream_key);
    return LRTMP2_OK;
}

int lrtmp2_client_play(lrtmp2_client_t *client)
{
    if (!client) return LRTMP2_ERR_INTERNAL;
    if (client->state != LRTMP2_CLIENT_APP_CONNECTED) return LRTMP2_ERR_PROTOCOL;

    int rc = client_create_stream(client);
    if (rc != LRTMP2_OK) return rc;

    uint8_t amf_data[512];
    lrtmp2_buffer_t amf_buf;
    memset(&amf_buf, 0, sizeof(amf_buf));
    amf_buf.data = amf_data;
    amf_buf.capacity = sizeof(amf_data);

    rc = lrtmp2_cmd_build_play(&amf_buf, client->stream_key);
    if (rc != LRTMP2_OK) return rc;
    rc = client_send_command(client, client->stream_id, amf_buf.data, amf_buf.size);
    if (rc != LRTMP2_OK) return rc;

    rc = client_pump(client, "onStatus", on_status_ignore, 0);
    if (rc != LRTMP2_OK) return rc;

    client->state = LRTMP2_CLIENT_PLAYING;
    LRTMP2_LOG_INFO("play: stream=%s", client->stream_key);
    return LRTMP2_OK;
}

int lrtmp2_client_send_frame(lrtmp2_client_t *client, const lrtmp2_frame_t *frame)
{
    if (!client || !frame) return LRTMP2_ERR_INTERNAL;
    if (client->state != LRTMP2_CLIENT_PUBLISHING) return LRTMP2_ERR_PROTOCOL;

    lrtmp2_chunk_message_t cmsg;
    memset(&cmsg, 0, sizeof(cmsg));
    cmsg.timestamp = frame->timestamp;
    cmsg.msg_length = frame->size;
    cmsg.msg_stream_id = client->stream_id;

    if (frame->type == LRTMP2_FRAME_AUDIO) {
        cmsg.csid = 4;
        cmsg.msg_type_id = RTMP_MSG_AUDIO;
    } else {
        cmsg.csid = 6;
        cmsg.msg_type_id = RTMP_MSG_VIDEO;
    }
    cmsg.fmt = 0;

    int rc = lrtmp2_chunk_write(client->send_buffer, &cmsg, frame->data, frame->size,
                                LRTMP2_DEFAULT_CHUNK_SIZE);
    if (rc != LRTMP2_OK) return rc;
    return client_flush(client);
}

int lrtmp2_client_poll(lrtmp2_client_t *client, int timeout_ms)
{
    if (!client) return LRTMP2_ERR_INTERNAL;
    if (client->state != LRTMP2_CLIENT_PLAYING) return LRTMP2_ERR_PROTOCOL;

    /* poll() rather than select() so a large client_fd (past FD_SETSIZE) in an
     * fd-heavy host cannot corrupt the stack. */
    if (lrtmp2_buffer_available(client->recv_buffer) == 0) {
        struct pollfd pfd;
        pfd.fd = client->client_fd;
        pfd.events = POLLIN;
        pfd.revents = 0;
        int rc = poll(&pfd, 1, timeout_ms);
        if (rc < 0) {
            if (errno == EINTR) return LRTMP2_OK;
            return LRTMP2_ERR_IO;
        }
        if (rc == 0) return LRTMP2_OK; /* timeout, no data */
    }

    return client_pump(client, NULL, NULL, 1);
}
