/**
 * conn.c — Connection management
 */
#include "session/conn.h"
#include "session/state_machine.h"
#include "core/alloc.h"
#include "core/log.h"
#include "amf/amf.h"
#include "chunk/chunk_writer.h"
#include "chunk/chunk_write.h"
#include "chunk/chunk_reader.h"
#include "message/message.h"
#include <string.h>
#include <stdlib.h>
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

    conn->chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;
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

    while (lrtmp2_buffer_available(conn->recv_buffer) > 0) {
        rc = lrtmp2_conn_process(conn);
        if (rc == 0) break;
        if (rc < 0) return rc;
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
            if (rc == 0) return 0;
            if (rc < 0) return rc;
            conn->state = LRTMP2_STATE_HANDSHAKE;
            /* fall through */

        case LRTMP2_HS_SERVER_WAIT_C1:
            rc = lrtmp2_handshake_server_read_c1(&conn->handshake, conn->recv_buffer);
            if (rc == 0) return 0;
            if (rc < 0) return rc;
            break;

        case LRTMP2_HS_SERVER_WAIT_C2:
            rc = lrtmp2_handshake_server_read_c2(&conn->handshake, conn->recv_buffer);
            if (rc == 0) return 0;
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
        rc = lrtmp2_chunk_read(conn->recv_buffer, NULL, &msg, payload, &payload_len);
        if (rc == 0) break;
        if (rc < 0) return rc;

        if (msg.is_complete) {
            rc = lrtmp2_msg_decode(conn, &msg, payload, msg.msg_length);
            if (rc != LRTMP2_OK) return rc;
        }
    }

    return LRTMP2_OK;
}

int lrtmp2_conn_send_connect_response(lrtmp2_conn_t *conn)
{
    if (!conn) return LRTMP2_ERR_INTERNAL;

    /* Send SetChunkSize (msg type 0x01 on csid 2) */
    lrtmp2_buffer_reset(conn->send_buffer);
    {
        uint8_t scs_msg[8];
        /* Basic header: fmt=0, csid=2 */
        scs_msg[0] = 0x02;
        /* Message header: timestamp(3) + msg_length(3) + msg_type(1) */
        scs_msg[1] = scs_msg[2] = scs_msg[3] = 0; /* timestamp = 0 */
        scs_msg[4] = 0; scs_msg[5] = 0; scs_msg[6] = 4; /* length = 4 */
        scs_msg[7] = RTMP_MSG_SET_CHUNK_SIZE;
        lrtmp2_buffer_write(conn->send_buffer, scs_msg, 8);
        /* Body: 4 bytes big-endian chunk size */
        uint32_t net_cs = lrtmp2_hton32(conn->chunk_size);
        lrtmp2_buffer_write(conn->send_buffer, (uint8_t *)&net_cs, 4);
    }

    /* Build AMF0 _result command */
    uint8_t amf_data[512];
    lrtmp2_buffer_t amf_buf;
    memset(&amf_buf, 0, sizeof(amf_buf));
    amf_buf.data = amf_data;
    amf_buf.capacity = sizeof(amf_data);

    lrtmf2_amf0_write_string(&amf_buf, "_result");
    lrtmf2_amf0_write_number(&amf_buf, 1.0);
    lrtmf2_amf0_write_null(&amf_buf);

    /* Encode as chunk on csid 3 */
    lrtmp2_chunk_message_t cmd_msg;
    memset(&cmd_msg, 0, sizeof(cmd_msg));
    cmd_msg.csid = 3;
    cmd_msg.fmt = 0;
    cmd_msg.timestamp = 0;
    cmd_msg.msg_length = amf_buf.size;
    cmd_msg.msg_type_id = RTMP_MSG_AMF0_COMMAND;

    lrtmp2_buffer_reset(conn->send_buffer);
    lrtmp2_chunk_write(conn->send_buffer, &cmd_msg, amf_buf.data, amf_buf.size);

    return LRTMP2_OK;
}
