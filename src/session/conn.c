/**
 * conn.c — Connection management
 */
#include "session/conn.h"
#include "session/state_machine.h"
#include "session/stream.h"
#include "session/publish.h"
#include "session/play.h"
#include "core/alloc.h"
#include "core/log.h"
#include "amf/amf.h"
#include "chunk/chunk_writer.h"
#include "chunk/chunk_write.h"
#include "chunk/chunk_reader.h"
#include "message/message.h"
#include "message/command.h"
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>
#include <sys/socket.h>
#include "librtmp2/types.h"

/* RTMP message type IDs */
#define RTMP_MSG_AMF0_COMMAND         0x14
#define RTMP_MSG_AMF0_DATA            0x12
#define RTMP_MSG_SET_CHUNK_SIZE       0x01


#define RTMP_MSG_WINDOW_ACK_SIZE      0x05

lrtmp2_conn_t *lrtmp2_conn_create(lrtmp2_server_t *server, lrtmp2_server_config_t *config)
{
    lrtmp2_conn_t *conn = LRTMP2_CALLOC(1, sizeof(lrtmp2_conn_t));
    if (!conn) return NULL;

    conn->server = server;
    conn->config = config;
    conn->state = LRTMP2_STATE_TCP_ACCEPTED;
    conn->client_fd = -1;

    if (config) {
        conn->on_connect_cb = config->on_connect_cb;
        conn->on_publish_cb = config->on_publish_cb;
        conn->on_play_cb = config->on_play_cb;
        conn->on_frame_cb = config->on_frame_cb;
        conn->on_close_cb = config->on_close_cb;
        conn->userdata = config->userdata;
    }

    lrtmp2_handshake_server_init(&conn->handshake);
    lrtmp2_chunk_streams_init();

    conn->recv_buffer = lrtmp2_buffer_create();
    if (!conn->recv_buffer) {
        LRTMP2_FREE(conn);
        return NULL;
    }

    conn->send_buffer = lrtmp2_buffer_create();
    if (!conn->send_buffer) {
        lrtmp2_buffer_destroy(conn->recv_buffer);
        LRTMP2_FREE(conn);
        return NULL;
    }

    conn->chunk_size = (config && config->chunk_size > 0) ? (uint32_t)config->chunk_size : LRTMP2_DEFAULT_CHUNK_SIZE;
    conn->peer_chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;
    conn->window_ack_size = 0;

    pthread_mutex_init(&conn->send_mutex, NULL);

    LRTMP2_LOG_DEBUG("Connection created, state=TCP_ACCEPTED");
    return conn;
}

void lrtmp2_conn_destroy(lrtmp2_conn_t *conn)
{
    if (!conn) return;
    if (conn->recv_buffer) lrtmp2_buffer_destroy(conn->recv_buffer);
    if (conn->send_buffer) lrtmp2_buffer_destroy(conn->send_buffer);
    pthread_mutex_destroy(&conn->send_mutex);
    LRTMP2_LOG_DEBUG("Connection destroyed");
    LRTMP2_FREE(conn);
}

int lrtmp2_conn_recv(lrtmp2_conn_t *conn, const uint8_t *data, size_t len)
{
    if (!conn || !data || len == 0) return LRTMP2_ERR_INTERNAL;

    int rc = lrtmp2_buffer_write(conn->recv_buffer, data, len);
    if (rc < 0) return rc;

    /* Process all buffered data. Loop until either:
     * - No more data AND not in handshake state
     * - Error occurs
     * - State machine requests stop (rc==0 and no pending data) */
    for (int max_iter = 100; max_iter > 0; max_iter--) {
        size_t avail = lrtmp2_buffer_available(conn->recv_buffer);
        if (avail == 0 && conn->state != LRTMP2_STATE_HANDSHAKE) {
            break;
        }
        rc = lrtmp2_conn_process(conn);
        if (rc < 0) return rc;
        if (rc == 0 &&
            lrtmp2_buffer_available(conn->recv_buffer) == 0 &&
            conn->state < LRTMP2_STATE_CLOSING) {
            break;
        }
    }

    return LRTMP2_OK;
}

int lrtmp2_conn_process(lrtmp2_conn_t *conn)
{
    switch (conn->state) {
        case LRTMP2_STATE_TCP_ACCEPTED:
        case LRTMP2_STATE_HANDSHAKE:
            return lrtmp2_conn_do_handshake(conn);

        case LRTMP2_STATE_CONNECTED:
        case LRTMP2_STATE_APP_CONNECTED:
        case LRTMP2_STATE_STREAM_CREATED:
        case LRTMP2_STATE_PUBLISHING:
        case LRTMP2_STATE_PLAYING:
        case LRTMP2_STATE_CAPS_NEGOTIATED:
            return lrtmp2_conn_read_messages(conn);

        case LRTMP2_STATE_CLOSING:
        case LRTMP2_STATE_CLOSED:
            return 0;

        default:
            return LRTMP2_ERR_INTERNAL;
    }
}

int lrtmp2_conn_do_handshake(lrtmp2_conn_t *conn)
{
    int rc;

    switch (conn->handshake.state) {
        case LRTMP2_HS_SERVER_WAIT_C0:
            rc = lrtmp2_handshake_server_read_c0(&conn->handshake, conn->recv_buffer);
            if (rc < 0) return rc;
            conn->state = LRTMP2_STATE_HANDSHAKE;
            /* fall through */

        case LRTMP2_HS_SERVER_WAIT_C1:
            rc = lrtmp2_handshake_server_read_c1(&conn->handshake, conn->recv_buffer);
            if (rc < 0) return rc;
            /* Send S0+S1+S2 to client (skip if no socket, e.g. in tests).
             * handshake.out only contains S1+S2; S0 is the 1-byte version
             * marker and must be sent ahead of it. */
            if (conn->client_fd >= 0) {
                uint8_t s0 = 0x03; /* RTMP_VERSION */
                rc = lrtmp2_conn_send_raw(conn, &s0, 1);
                if (rc != LRTMP2_OK) return rc;
                rc = lrtmp2_conn_send_raw(conn, conn->handshake.out.data, conn->handshake.out.size);
                if (rc != LRTMP2_OK) return rc;
            }
            conn->handshake.out.size = 0;
            conn->handshake.out.read_pos = 0;
            /* Still waiting on C2 — do not mark CONNECTED yet. */
            return LRTMP2_OK;

        case LRTMP2_HS_SERVER_WAIT_C2:
            rc = lrtmp2_handshake_server_read_c2(&conn->handshake, conn->recv_buffer);
            if (rc < 0) return rc;
            break;

        case LRTMP2_HS_DONE:
            break;

        default:
            return LRTMP2_ERR_PROTOCOL;
    }

    conn->state = LRTMP2_STATE_CONNECTED;
    LRTMP2_LOG_INFO("Handshake complete, connection in state CONNECTED");
    return LRTMP2_OK;
}

int lrtmp2_conn_read_messages(lrtmp2_conn_t *conn)
{
    lrtmp2_chunk_message_t msg;
    uint8_t payload[4096];
    size_t payload_len = 0;
    int rc;

    while (lrtmp2_buffer_available(conn->recv_buffer) > 0) {
        payload_len = 0;
        rc = lrtmp2_chunk_read(conn->recv_buffer, NULL, &msg, payload, sizeof(payload), &payload_len);

        if (rc == 0) break;
        if (rc < 0) return rc;

        if (msg.is_complete) {
            rc = lrtmp2_msg_decode(conn, &msg, payload, msg.msg_length);
            if (rc != LRTMP2_OK) return rc;
            /* Send any queued responses (e.g. connect result) */
            lrtmp2_conn_flush(conn);
        }
    }

    return LRTMP2_OK;
}

int lrtmp2_conn_send_raw(lrtmp2_conn_t *conn, const uint8_t *data, size_t len)
{
    if (!conn || !data || len == 0) return LRTMP2_ERR_INTERNAL;
    if (conn->client_fd < 0) return LRTMP2_OK;  /* no socket, silently skip */

    size_t sent = 0;
    while (sent < len) {
        ssize_t n = send(conn->client_fd, data + sent, len - sent, 0);
        if (n <= 0) {
            if (errno == EINTR) continue;
            return LRTMP2_ERR_IO;
        }
        sent += (size_t)n;
    }
    return LRTMP2_OK;
}

int lrtmp2_conn_flush(lrtmp2_conn_t *conn)
{
    if (!conn) return LRTMP2_ERR_INTERNAL;
    if (conn->client_fd < 0) return LRTMP2_OK;  /* no socket, silently skip */
    if (conn->send_buffer->size == 0) return LRTMP2_OK;

    int rc = lrtmp2_conn_send_raw(conn, conn->send_buffer->data, conn->send_buffer->size);
    conn->send_buffer->size = 0;
    conn->send_buffer->read_pos = 0;
    return rc;
}

static int lrtmp2_conn_send_command(lrtmp2_conn_t *conn, uint32_t msg_stream_id,
                                     const uint8_t *amf_data, size_t amf_len)
{
    lrtmp2_chunk_message_t cmd_msg;
    memset(&cmd_msg, 0, sizeof(cmd_msg));
    cmd_msg.csid = 3;
    cmd_msg.fmt = 0;
    cmd_msg.timestamp = 0;
    cmd_msg.msg_length = amf_len;
    cmd_msg.msg_type_id = RTMP_MSG_AMF0_COMMAND;
    cmd_msg.msg_stream_id = msg_stream_id;

    return lrtmp2_chunk_write(conn->send_buffer, &cmd_msg, amf_data, amf_len);
}

int lrtmp2_conn_send_connect_response(lrtmp2_conn_t *conn, double transaction_id)
{
    if (!conn) return LRTMP2_ERR_INTERNAL;

    /* Send SetChunkSize (msg type 0x01 on csid 2) */
    {
        lrtmp2_chunk_message_t scs_msg;
        memset(&scs_msg, 0, sizeof(scs_msg));
        scs_msg.csid = 2;
        scs_msg.fmt = 0;
        scs_msg.msg_length = 4;
        scs_msg.msg_type_id = RTMP_MSG_SET_CHUNK_SIZE;
        uint32_t net_cs = lrtmp2_hton32(conn->chunk_size);
        lrtmp2_chunk_write(conn->send_buffer, &scs_msg, (uint8_t *)&net_cs, 4);
    }

    /* Build AMF0 _result command */
    uint8_t amf_data[512];
    lrtmp2_buffer_t amf_buf;
    memset(&amf_buf, 0, sizeof(amf_buf));
    amf_buf.data = amf_data;
    amf_buf.capacity = sizeof(amf_data);

    lrtmf2_amf0_write_string(&amf_buf, "_result");
    lrtmf2_amf0_write_number(&amf_buf, transaction_id);
    lrtmf2_amf0_write_null(&amf_buf);

    return lrtmp2_conn_send_command(conn, 0, amf_buf.data, amf_buf.size);
}

int lrtmp2_conn_send_create_stream_response(lrtmp2_conn_t *conn, double transaction_id, uint32_t stream_id)
{
    if (!conn) return LRTMP2_ERR_INTERNAL;

    uint8_t amf_data[256];
    lrtmp2_buffer_t amf_buf;
    memset(&amf_buf, 0, sizeof(amf_buf));
    amf_buf.data = amf_data;
    amf_buf.capacity = sizeof(amf_data);

    lrtmp2_cmd_build_create_stream_result(&amf_buf, transaction_id, (double)stream_id);

    return lrtmp2_conn_send_command(conn, 0, amf_buf.data, amf_buf.size);
}

int lrtmp2_conn_send_onstatus(lrtmp2_conn_t *conn, uint32_t stream_id, const char *level,
                               const char *code, const char *description)
{
    if (!conn) return LRTMP2_ERR_INTERNAL;

    uint8_t amf_data[512];
    lrtmp2_buffer_t amf_buf;
    memset(&amf_buf, 0, sizeof(amf_buf));
    amf_buf.data = amf_data;
    amf_buf.capacity = sizeof(amf_data);

    lrtmp2_cmd_build_onstatus(&amf_buf, level, code, description);

    return lrtmp2_conn_send_command(conn, stream_id, amf_buf.data, amf_buf.size);
}

int lrtmp2_conn_handle_command(lrtmp2_conn_t *conn, const uint8_t *payload, size_t payload_len)
{
    if (!conn || !payload) return LRTMP2_ERR_INTERNAL;

    lrtmp2_buffer_t buf;
    memset(&buf, 0, sizeof(buf));
    buf.data = (uint8_t *)payload;
    buf.size = payload_len;
    buf.capacity = payload_len;
    buf.read_pos = 0;

    char name[64];
    memset(name, 0, sizeof(name));
    if (lrtmp2_cmd_peek_name(&buf, name, sizeof(name)) != LRTMP2_OK) {
        LRTMP2_LOG_WARN("Failed to read command name");
        return LRTMP2_OK;
    }

    if (strcmp(name, "connect") == 0) {
        lrtmp2_connect_info_t info;
        if (lrtmp2_cmd_read_connect(&buf, &info) != LRTMP2_OK) return LRTMP2_ERR_AMF;
        snprintf(conn->app, sizeof(conn->app), "%s", info.app);
        lrtmp2_conn_transition(conn, LRTMP2_STATE_APP_CONNECTED);
        lrtmp2_conn_send_connect_response(conn, info.transaction_id);
        LRTMP2_LOG_INFO("connect: app=%s", conn->app);
        if (conn->on_connect_cb) {
            conn->on_connect_cb(conn, conn->userdata);
        }
    } else if (strcmp(name, "createStream") == 0) {
        double txn = 0.0;
        lrtmp2_cmd_read_create_stream(&buf, &txn);
        conn->next_stream_id++;
        uint32_t stream_id = conn->next_stream_id;
        conn->current_stream = lrtmp2_stream_create(conn, stream_id);
        lrtmp2_conn_transition(conn, LRTMP2_STATE_STREAM_CREATED);
        lrtmp2_conn_send_create_stream_response(conn, txn, stream_id);
        LRTMP2_LOG_INFO("createStream: stream_id=%u", stream_id);
    } else if (strcmp(name, "publish") == 0) {
        char stream_name[256];
        char publish_type[64];
        memset(stream_name, 0, sizeof(stream_name));
        memset(publish_type, 0, sizeof(publish_type));
        lrtmp2_cmd_read_publish(&buf, stream_name, sizeof(stream_name), publish_type, sizeof(publish_type));
        if (conn->current_stream) {
            lrtmp2_publish_begin(conn->current_stream, stream_name);
        }
        lrtmp2_conn_transition(conn, LRTMP2_STATE_PUBLISHING);
        uint32_t stream_id = conn->current_stream ? conn->current_stream->stream_id : 0;
        lrtmp2_conn_send_onstatus(conn, stream_id, "status", "NetStream.Publish.Start", "Publishing");
        LRTMP2_LOG_INFO("publish: stream=%s", stream_name);
        if (conn->on_publish_cb) {
            conn->on_publish_cb(conn, conn->app, stream_name, conn->userdata);
        }
    } else if (strcmp(name, "play") == 0) {
        char stream_name[256];
        memset(stream_name, 0, sizeof(stream_name));
        lrtmp2_cmd_read_play(&buf, stream_name, sizeof(stream_name));
        if (conn->current_stream) {
            lrtmp2_play_begin(conn, stream_name);
            conn->current_stream->is_playing = 1;
        }
        lrtmp2_conn_transition(conn, LRTMP2_STATE_PLAYING);
        uint32_t stream_id = conn->current_stream ? conn->current_stream->stream_id : 0;
        lrtmp2_conn_send_onstatus(conn, stream_id, "status", "NetStream.Play.Start", "Playing");
        LRTMP2_LOG_INFO("play: stream=%s", stream_name);
        if (conn->on_play_cb) {
            conn->on_play_cb(conn, conn->app, stream_name, conn->userdata);
        }
    } else if (strcmp(name, "FCPublish") == 0 || strcmp(name, "FCUnpublish") == 0 ||
               strcmp(name, "releaseStream") == 0 || strcmp(name, "deleteStream") == 0) {
        LRTMP2_LOG_DEBUG("Ignoring command: %s", name);
    } else {
        LRTMP2_LOG_WARN("Unhandled command: %s", name);
    }

    return LRTMP2_OK;
}
