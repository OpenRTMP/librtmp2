/**
 * message.c — RTMP message reassembly
 *
 * After chunk reassembly, messages are dispatched to the appropriate handler.
 */
#include "message.h"
#include "message/control.h"
#include "message/command.h"
#include "core/log.h"
#include <string.h>
#include "librtmp2/types.h"

/* RTMP message type IDs */
#define RTMP_MSG_SET_CHUNK_SIZE       0x01
#define RTMP_MSG_ABORT_MESSAGE        0x02
#define RTMP_MSG_ACKNOWLEDGEMENT      0x03
#define RTMP_MSG_USER_CONTROL         0x04
#define RTMP_MSG_WINDOW_ACK_SIZE      0x05
#define RTMP_MSG_SET_PEER_BANDWIDTH   0x06
#define RTMP_MSG_AUDIO                0x08
#define RTMP_MSG_VIDEO                0x09
#define RTMP_MSG_AMF3_DATA            0x0F
#define RTMP_MSG_AMF3_SHARED_OBJECT   0x10
#define RTMP_MSG_AMF0_COMMAND         0x14
#define RTMP_MSG_AMF0_DATA            0x12
#define RTMP_MSG_AGGREGATE            0x16


int lrtmp2_msg_decode(lrtmp2_conn_t *conn, const lrtmp2_chunk_message_t *chunk,
                       const uint8_t *payload, size_t payload_len)
{
    if (!conn || !chunk || !payload) return LRTMP2_ERR_INTERNAL;

    lrtmp2_buffer_t buf;
    memset(&buf, 0, sizeof(buf));
    buf.data = (uint8_t *)payload;  /* safe: we only read */
    buf.size = payload_len;
    buf.read_pos = 0;
    buf.capacity = payload_len;

    switch (chunk->msg_type_id) {
        case RTMP_MSG_SET_CHUNK_SIZE:
            {
                uint32_t cs;
                if (lrtmp2_msg_read_set_chunk_size(payload, &cs) == LRTMP2_OK) {
                    conn->peer_chunk_size = cs;
                    LRTMP2_LOG_INFO("Peer SetChunkSize: %u", cs);
                }
            }
            break;

        case RTMP_MSG_ABORT_MESSAGE:
            {
                uint32_t csid;
                if (lrtmp2_msg_read_abort_message(payload, &csid) == LRTMP2_OK) {
                    LRTMP2_LOG_INFO("AbortMessage: csid=%u", csid);
                }
            }
            break;

        case RTMP_MSG_ACKNOWLEDGEMENT:
            {
                uint32_t seq;
                if (lrtmp2_msg_read_acknowledgement_size(payload, &seq) == LRTMP2_OK) {
                    LRTMP2_LOG_DEBUG("Acknowledgement: seq=%u", seq);
                }
            }
            break;

        case RTMP_MSG_WINDOW_ACK_SIZE:
            {
                uint32_t win;
                if (lrtmp2_msg_read_window_ack_size(payload, &win) == LRTMP2_OK) {
                    conn->window_ack_size = win;
                    LRTMP2_LOG_INFO("WindowAckSize: %u", win);
                }
            }
            break;

        case RTMP_MSG_SET_PEER_BANDWIDTH:
            {
                uint32_t win;
                uint8_t limit;
                if (lrtmp2_msg_read_set_peer_bandwidth(payload, &win, &limit) == LRTMP2_OK) {
                    LRTMP2_LOG_INFO("SetPeerBandwidth: win=%u limit=%u", win, limit);
                }
            }
            break;

        case RTMP_MSG_USER_CONTROL:
            {
                uint16_t evt;
                uint32_t p1, p2;
                if (lrtmp2_msg_read_user_control(payload, &evt, &p1, &p2) == LRTMP2_OK) {
                    LRTMP2_LOG_DEBUG("UserControl: event=%u p1=%u p2=%u", evt, p1, p2);
                }
            }
            break;

        case RTMP_MSG_AUDIO:
            {
                lrtmp2_frame_t frame;
                memset(&frame, 0, sizeof(frame));
                frame.type = LRTMP2_FRAME_AUDIO;
                frame.timestamp = chunk->timestamp;
                frame.size = payload_len;
                frame.data = payload;
                /* Parse audio tag: first byte = codec(4) + sample_rate(2) + channels(1) + bit_depth(1) */
                if (payload_len > 0) {
                    uint8_t tag = payload[0];
                    frame.audio_codec = (lrtmp2_audio_codec_t)((tag >> 4) & 0x0F);
                    frame.audio_sample_rate = (tag >> 2) & 0x03;
                    frame.audio_bit_depth = (tag >> 1) & 0x01;
                    frame.audio_channels = tag & 0x01;
                }
                if (conn->on_frame_cb) {
                    conn->on_frame_cb(conn, &frame, conn->userdata);
                }
            }
            break;

        case RTMP_MSG_VIDEO:
            {
                lrtmp2_frame_t frame;
                memset(&frame, 0, sizeof(frame));
                frame.type = LRTMP2_FRAME_VIDEO;
                frame.timestamp = chunk->timestamp;
                frame.size = payload_len;
                frame.data = payload;
                /* Parse video tag: first byte = frame_type(4) + codec(4) */
                if (payload_len > 0) {
                    uint8_t tag = payload[0];
                    frame.video_frame_type = (tag >> 4) & 0x0F;
                    frame.video_codec = (lrtmp2_video_codec_t)(tag & 0x0F);
                }
                if (conn->on_frame_cb) {
                    conn->on_frame_cb(conn, &frame, conn->userdata);
                }
            }
            break;

        case RTMP_MSG_AMF0_COMMAND:
            {
                /* This is a connect/publish/play command — deliver as frame */
                lrtmp2_frame_t frame;
                memset(&frame, 0, sizeof(frame));
                frame.type = LRTMP2_FRAME_SCRIPT;
                frame.timestamp = chunk->timestamp;
                frame.size = payload_len;
                frame.data = payload;
                frame.is_metadata = 1;
                if (conn->on_frame_cb) {
                    conn->on_frame_cb(conn, &frame, conn->userdata);
                }
            }
            break;

        case RTMP_MSG_AMF0_DATA:
            LRTMP2_LOG_DEBUG("AMF0 data message, %zu bytes", payload_len);
            break;

        case RTMP_MSG_AMF3_DATA:
        case RTMP_MSG_AMF3_SHARED_OBJECT:
            LRTMP2_LOG_DEBUG("AMF3 message, %zu bytes", payload_len);
            break;

        default:
            LRTMP2_LOG_WARN("Unknown message type: 0x%02x", chunk->msg_type_id);
            break;
    }

    return LRTMP2_OK;
}
