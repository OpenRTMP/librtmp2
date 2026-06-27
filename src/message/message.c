/**
 * message.c — RTMP message reassembly
 *
 * After chunk reassembly, messages are dispatched to the appropriate handler.
 */
#include "message.h"
#include "message/control.h"
#include "message/command.h"
#include "session/conn.h"
#include "core/log.h"
#include "ertmp/ertmp.h"
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


/* Build a frame from an audio message payload and hand it to on_frame_cb. */
static void deliver_audio_frame(lrtmp2_conn_t *conn, uint32_t timestamp,
                                const uint8_t *payload, size_t payload_len)
{
    lrtmp2_frame_t frame;
    memset(&frame, 0, sizeof(frame));
    frame.type = LRTMP2_FRAME_AUDIO;
    frame.timestamp = timestamp;
    frame.size = payload_len;
    frame.data = payload;

    lrtmp2_audio_header_t ah;
    if (lrtmp2_ertmp_exaudio_parse(payload, payload_len, &ah) == LRTMP2_OK &&
        ah.header_size <= payload_len) {
        frame.audio_codec = ah.audio_codec;
        frame.audio_sample_rate = ah.sample_rate;
        frame.audio_bit_depth = ah.sample_size;
        frame.audio_channels = ah.channels;
        if (ah.is_ex_header) {
            memcpy(frame.audio_fourcc.cc, ah.fourcc, sizeof(ah.fourcc));
        }
    }

    if (conn->on_frame_cb) {
        conn->on_frame_cb(conn, &frame, conn->userdata);
    }
}

/* Build a frame from a video message payload and hand it to on_frame_cb. */
static void deliver_video_frame(lrtmp2_conn_t *conn, uint32_t timestamp,
                                const uint8_t *payload, size_t payload_len)
{
    lrtmp2_frame_t frame;
    memset(&frame, 0, sizeof(frame));
    frame.type = LRTMP2_FRAME_VIDEO;
    frame.timestamp = timestamp;
    frame.size = payload_len;
    frame.data = payload;

    lrtmp2_video_header_t vh;
    if (lrtmp2_ertmp_exvideo_parse(payload, payload_len, &vh) == LRTMP2_OK &&
        vh.header_size <= payload_len) {
        frame.video_frame_type = vh.frame_type;
        frame.composition_time = vh.composition_time;
        if (vh.is_ex_header) {
            memcpy(frame.video_fourcc.cc, vh.fourcc, sizeof(vh.fourcc));
            if (memcmp(vh.fourcc, "avc1", 4) == 0) {
                frame.video_codec = LRTMP2_VIDEO_H264;
            } else if (memcmp(vh.fourcc, "hvc1", 4) == 0) {
                frame.video_codec = LRTMP2_VIDEO_H265;
            } else if (memcmp(vh.fourcc, "av01", 4) == 0) {
                frame.video_codec = LRTMP2_VIDEO_AV1;
            }
        } else if (payload_len > 0) {
            frame.video_codec = (lrtmp2_video_codec_t)(payload[0] & 0x0F);
        }
    }

    if (conn->on_frame_cb) {
        conn->on_frame_cb(conn, &frame, conn->userdata);
    }
}

/* Decode an Aggregate message (type 0x16): a back-to-back sequence of FLV tags
 * (TagType, DataSize, Timestamp, StreamID, body, PreviousTagSize). Audio/video
 * sub-tags are delivered as individual frames; other tag types are skipped.
 * Timestamps in an aggregate are relative to the first tag, offset by the
 * aggregate message timestamp. */
int lrtmp2_msg_decode_aggregate(lrtmp2_conn_t *conn, const lrtmp2_chunk_message_t *chunk,
                                const uint8_t *payload, size_t payload_len)
{
    size_t pos = 0;
    int have_base = 0;
    uint32_t base_ts = 0;

    while (pos + 11 <= payload_len) {
        uint8_t tag_type = payload[pos];
        uint32_t data_size = ((uint32_t)payload[pos + 1] << 16) |
                             ((uint32_t)payload[pos + 2] << 8) |
                             (uint32_t)payload[pos + 3];
        uint32_t ts = ((uint32_t)payload[pos + 4] << 16) |
                      ((uint32_t)payload[pos + 5] << 8) |
                      (uint32_t)payload[pos + 6] |
                      ((uint32_t)payload[pos + 7] << 24); /* extended byte */

        size_t body = pos + 11;
        if (body + data_size > payload_len) {
            LRTMP2_LOG_WARN("Aggregate sub-tag overruns message (%u bytes)", data_size);
            break;
        }

        if (!have_base) { base_ts = ts; have_base = 1; }
        uint32_t out_ts = chunk->timestamp + (ts - base_ts);

        if (tag_type == 0x08) {
            deliver_audio_frame(conn, out_ts, payload + body, data_size);
        } else if (tag_type == 0x09) {
            deliver_video_frame(conn, out_ts, payload + body, data_size);
        } else {
            LRTMP2_LOG_DEBUG("Aggregate: skipping sub-tag type %u", tag_type);
        }

        /* Advance past body + 4-byte PreviousTagSize trailer. */
        pos = body + data_size + 4;
    }

    return LRTMP2_OK;
}

int lrtmp2_msg_decode(lrtmp2_conn_t *conn, const lrtmp2_chunk_message_t *chunk,
                       const uint8_t *payload, size_t payload_len)
{
    if (!conn || !chunk || !payload) return LRTMP2_ERR_INTERNAL;

    switch (chunk->msg_type_id) {
        case RTMP_MSG_SET_CHUNK_SIZE:
            {
                uint32_t cs;
                if (payload_len >= 4 &&
                    lrtmp2_msg_read_set_chunk_size(payload, &cs) == LRTMP2_OK) {
                    lrtmp2_chunk_stream_set_all_chunk_size(&conn->chunk_reg, cs);
                    LRTMP2_LOG_INFO("Peer SetChunkSize: %u", cs);
                }
            }
            break;

        case RTMP_MSG_ABORT_MESSAGE:
            {
                uint32_t csid;
                if (payload_len >= 4 &&
                    lrtmp2_msg_read_abort_message(payload, &csid) == LRTMP2_OK) {
                    lrtmp2_chunk_stream_t *cs =
                        lrtmp2_chunk_stream_get(&conn->chunk_reg, csid);
                    if (cs) {
                        lrtmp2_chunk_stream_reset(&conn->chunk_reg, cs);
                    }
                    LRTMP2_LOG_INFO("AbortMessage: csid=%u", csid);
                }
            }
            break;

        case RTMP_MSG_ACKNOWLEDGEMENT:
            {
                uint32_t seq;
                if (payload_len >= 4 &&
                    lrtmp2_msg_read_acknowledgement_size(payload, &seq) == LRTMP2_OK) {
                    LRTMP2_LOG_DEBUG("Acknowledgement: seq=%u", seq);
                }
            }
            break;

        case RTMP_MSG_WINDOW_ACK_SIZE:
            {
                uint32_t win;
                if (payload_len >= 4 &&
                    lrtmp2_msg_read_window_ack_size(payload, &win) == LRTMP2_OK) {
                    conn->window_ack_size = win;
                    LRTMP2_LOG_INFO("WindowAckSize: %u", win);
                }
            }
            break;

        case RTMP_MSG_SET_PEER_BANDWIDTH:
            {
                uint32_t win;
                uint8_t limit;
                if (payload_len >= 5 &&
                    lrtmp2_msg_read_set_peer_bandwidth(payload, &win, &limit) == LRTMP2_OK) {
                    LRTMP2_LOG_INFO("SetPeerBandwidth: win=%u limit=%u", win, limit);
                }
            }
            break;

        case RTMP_MSG_USER_CONTROL:
            {
                uint16_t evt;
                uint32_t p1, p2 = 0;
                /* event(2) + param1(4) = 6 bytes minimum; the optional second
                 * param (e.g. SetBufferLength) needs 4 more. Only ask for it
                 * when those bytes are present, else read_user_control would read
                 * past the payload. */
                if (payload_len >= 6 &&
                    lrtmp2_msg_read_user_control(payload, &evt, &p1,
                                                 payload_len >= 10 ? &p2 : NULL) == LRTMP2_OK) {
                    LRTMP2_LOG_DEBUG("UserControl: event=%u p1=%u p2=%u", evt, p1, p2);
                }
            }
            break;

        case RTMP_MSG_AUDIO:
            /* frame.data/size keep the full message payload (including the
             * codec/FLV header); parsed fields expose the metadata. */
            deliver_audio_frame(conn, chunk->timestamp, payload, payload_len);
            break;

        case RTMP_MSG_VIDEO:
            deliver_video_frame(conn, chunk->timestamp, payload, payload_len);
            break;

        case RTMP_MSG_AMF0_COMMAND:
            return lrtmp2_conn_handle_command(conn, payload, payload_len);

        case RTMP_MSG_AMF3_COMMAND:
            /* AMF3 command messages are an AMF0-encoded command prefixed by a
             * single 0x00 marker byte (the command itself stays AMF0 in practice).
             * Skip the marker and dispatch the remainder like an AMF0 command. */
            if (payload_len >= 1 && payload[0] == 0x00) {
                return lrtmp2_conn_handle_command(conn, payload + 1, payload_len - 1);
            }
            return lrtmp2_conn_handle_command(conn, payload, payload_len);

        case RTMP_MSG_AMF0_DATA:
            LRTMP2_LOG_DEBUG("AMF0 data message, %zu bytes", payload_len);
            break;

        case RTMP_MSG_AMF3_DATA:
        case RTMP_MSG_AMF3_SHARED_OBJECT:
            LRTMP2_LOG_DEBUG("AMF3 message, %zu bytes", payload_len);
            break;

        case RTMP_MSG_AGGREGATE:
            return lrtmp2_msg_decode_aggregate(conn, chunk, payload, payload_len);

        default:
            LRTMP2_LOG_WARN("Unknown message type: 0x%02x", chunk->msg_type_id);
            break;
    }

    return LRTMP2_OK;
}
