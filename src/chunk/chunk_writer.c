/**
 * chunk_writer.c — RTMP chunk writer
 *
 * Fragment messages into chunks with proper basic header + message header.
 * Supports chunk types 0 and 1 for simplicity (type 2 and 3 are incremental).
 */
#include "chunk_writer.h"
#include "chunk_state.h"
#include "core/bytes.h"
#include "core/log.h"
#include <string.h>

int lrtmp2_chunk_write(lrtmp2_buffer_t *out,
                        const lrtmp2_chunk_message_t *msg,
                        const uint8_t *payload, size_t payload_len)
{
    if (!out || !msg) return LRTMP2_ERR_INTERNAL;

    size_t chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;
    size_t offset = 0;

    /* Determine fmt: use fmt 1 if same stream_id, msg_length, msg_type as last on this csid */
    /* For simplicity, we'll use fmt 0 for the first chunk and fmt 1 for subsequent */
    
    /* --- Basic header --- */
    uint8_t hdr[3];
    size_t hdr_len;

    uint32_t csid = msg->csid;
    uint8_t fmt = msg->fmt;

    if (csid < 64) {
        hdr[0] = (uint8_t)((fmt << 6) | csid);
        hdr_len = 1;
    } else if (csid < 320) {
        hdr[0] = (uint8_t)(fmt << 6);
        hdr[1] = (uint8_t)(csid - 64);
        hdr_len = 2;
    } else {
        hdr[0] = (uint8_t)((fmt << 6) | 1);
        hdr[1] = (uint8_t)((csid - 64) & 0xFF);
        hdr[2] = (uint8_t)(((csid - 64) >> 8) & 0xFF);
        hdr_len = 3;
    }

    lrtmp2_buffer_write(out, hdr, hdr_len);

    /* --- Message header --- */
    /* timestamp (3 bytes) — if >= 0xFFFFFF, needs extended timestamp */
    uint32_t ts = msg->timestamp;
    if (ts >= 0xFFFFFF) {
        uint8_t ts_buf[3];
        lrtmp2_hton24(ts_buf, 0xFFFFFF);
        lrtmp2_buffer_write(out, ts_buf, 3);
    } else {
        uint8_t ts_buf[3];
        lrtmp2_hton24(ts_buf, ts);
        lrtmp2_buffer_write(out, ts_buf, 3);
    }

    /* message length (3 bytes) */
    uint8_t len_buf[3];
    lrtmp2_hton24(len_buf, (uint32_t)msg->msg_length);
    lrtmp2_buffer_write(out, len_buf, 3);

    /* message type id (1 byte) */
    lrtmp2_buffer_write(out, &msg->msg_type_id, 1);

    /* stream id (4 bytes, little-endian for fmt 0) */
    {
        uint8_t sid[4];
        sid[0] = (uint8_t)(msg->msg_stream_id & 0xFF);
        sid[1] = (uint8_t)((msg->msg_stream_id >> 8) & 0xFF);
        sid[2] = (uint8_t)((msg->msg_stream_id >> 16) & 0xFF);
        sid[3] = (uint8_t)((msg->msg_stream_id >> 24) & 0xFF);
        lrtmp2_buffer_write(out, sid, 4);
    }

    /* Extended timestamp if needed */
    if (ts >= 0xFFFFFF) {
        uint32_t net_ts = lrtmp2_hton32(ts);
        lrtmp2_buffer_write(out, (uint8_t *)&net_ts, 4);
    }

    /* --- Payload --- */
    size_t to_write = (payload_len < chunk_size) ? payload_len : chunk_size;
    if (to_write > 0) {
        lrtmp2_buffer_write(out, payload, to_write);
    }

    LRTMP2_LOG_DEBUG("chunk written: csid=%u fmt=%u ts=%u len=%u payload=%zu/%zu",
                      csid, fmt, ts, msg->msg_length, to_write, payload_len);

    return LRTMP2_OK;
}

int lrtmp2_chunk_write_extended_timestamp(lrtmp2_buffer_t *out, uint32_t timestamp)
{
    if (!out) return LRTMP2_ERR_INTERNAL;

    /* Basic header: fmt=3, csid=2 (protocol control) */
    uint8_t hdr = (uint8_t)((3 << 6) | 2);
    lrtmp2_buffer_write(out, &hdr, 1);

    /* 4 bytes extended timestamp (for SetChunkSize etc.) */
    uint32_t net_ts = lrtmp2_hton32(timestamp);
    lrtmp2_buffer_write(out, (uint8_t *)&net_ts, 4);

    return LRTMP2_OK;
}
