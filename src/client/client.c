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
#include <sys/select.h>
#define close_socket close
#endif
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

/* RTMP message type IDs (mirrors message.c) */
#define RTMP_MSG_SET_CHUNK_SIZE       0x01
#define RTMP_MSG_AUDIO                0x08
#define RTMP_MSG_VIDEO                0x09
#define RTMP_MSG_AMF0_COMMAND         0x14

#define CLIENT_PAYLOAD_MAX (256 * 1024)

static int on_connect_result(lrtmp2_client_t *client, lrtmp2_buffer_t *buf);

lrtmp2_client_t *lrtmp2_client_create(const lrtmp2_server_config_t *config)
{
    lrtmp2_client_t *client = LRTMP2_CALLOC(1, sizeof(lrtmp2_client_t));
    if (!client) return NULL;

    client->client_fd = -1;
    client->state = LRTMP2_CLIENT_DISCONNECTED;
    client->config = config;
    client->peer_chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;
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
    if (client->client_fd >= 0) {
        close_socket(client->client_fd);
    }
    if (client->send_buffer) lrtmp2_buffer_destroy(client->send_buffer);
    if (client->recv_buffer) lrtmp2_buffer_destroy(client->recv_buffer);
    lrtmp2_handshake_cleanup(&client->handshake);
    lrtmp2_chunk_streams_destroy();
    LRTMP2_FREE(client);
}

/* --- Blocking socket helpers --- */

static int client_send_raw(lrtmp2_client_t *client, const uint8_t *data, size_t len)
{
    if (client->client_fd < 0) return LRTMP2_ERR_INTERNAL;
    size_t sent = 0;
    while (sent < len) {
        ssize_t n = send(client->client_fd, data + sent, len - sent, 0);
        if (n <= 0) {
            if (n < 0 && errno == EINTR) continue;
            return LRTMP2_ERR_IO;
        }
        sent += (size_t)n;
    }
    return LRTMP2_OK;
}

static int client_flush(lrtmp2_client_t *client)
{
    if (client->send_buffer->size == 0) return LRTMP2_OK;
    int rc = client_send_raw(client, client->send_buffer->data, client->send_buffer->size);
    client->send_buffer->size = 0;
    client->send_buffer->read_pos = 0;
    return rc;
}

/* Blocking recv of whatever is available; appends to recv_buffer. */
static int client_recv_more(lrtmp2_client_t *client)
{
    uint8_t tmp[4096];
    ssize_t n = recv(client->client_fd, tmp, sizeof(tmp), 0);
    if (n > 0) {
        return lrtmp2_buffer_write(client->recv_buffer, tmp, (size_t)n);
    }
    if (n == 0) {
        LRTMP2_LOG_WARN("Peer closed connection");
        return LRTMP2_ERR_IO;
    }
    if (errno == EINTR) return LRTMP2_OK;
    return LRTMP2_ERR_IO;
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
        int rc = lrtmp2_chunk_read(client->recv_buffer, NULL, &msg, &payload, &payload_len);
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
                if (lrtmp2_msg_read_set_chunk_size(payload, &cs) == LRTMP2_OK) {
                    client->peer_chunk_size = cs;
                    lrtmp2_chunk_stream_set_all_chunk_size(cs);
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

int lrtmp2_client_connect(lrtmp2_client_t *client, const char *url)
{
    if (!client || !url) return LRTMP2_ERR_INTERNAL;

    /* Parse URL: rtmp://host:port/app/stream_key */
    char host[256];
    int port = 1935;

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
            if (app_len >= sizeof(client->app)) return LRTMP2_ERR_INTERNAL;
            memcpy(client->app, app_start, app_len);
            client->app[app_len] = '\0';
            snprintf(client->stream_key, sizeof(client->stream_key), "%s", stream_slash + 1);
        } else {
            snprintf(client->app, sizeof(client->app), "%s", app_start);
            client->stream_key[0] = '\0';
        }
    } else {
        snprintf(host, sizeof(host), "%s", p);
        client->app[0] = '\0';
        client->stream_key[0] = '\0';
    }

    char *colon = strrchr(host, ':');
    if (colon) {
        port = atoi(colon + 1);
        *colon = '\0';
    }

    LRTMP2_LOG_INFO("Connecting to rtmp://%s:%d/%s/%s", host, port, client->app, client->stream_key);

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

    client->state = LRTMP2_CLIENT_HANDSHAKING;
    lrtmp2_chunk_streams_init();

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
    snprintf(tc_url, sizeof(tc_url), "rtmp://%s:%d/%s", host, port, client->app);

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

    struct timeval tv;
    tv.tv_sec = timeout_ms / 1000;
    tv.tv_usec = (timeout_ms % 1000) * 1000;
    fd_set rfds;
    FD_ZERO(&rfds);
    FD_SET(client->client_fd, &rfds);

    if (lrtmp2_buffer_available(client->recv_buffer) == 0) {
        int rc = select(client->client_fd + 1, &rfds, NULL, NULL, &tv);
        if (rc < 0) {
            if (errno == EINTR) return LRTMP2_OK;
            return LRTMP2_ERR_IO;
        }
        if (rc == 0) return LRTMP2_OK; /* timeout, no data */
    }

    return client_pump(client, NULL, NULL, 1);
}
