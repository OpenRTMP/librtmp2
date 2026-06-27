/**
 * chunk_reader.c — RTMP chunk reader
 *
 * Parses the chunk basic header + message header + extended timestamp + payload.
 * Handles chunk types 0, 1, 2, 3 and the 4 message header formats.
 */
#include "chunk_reader.h"
#include "chunk_state.h"
#include "core/bytes.h"
#include "core/log.h"
#include "core/alloc.h"
#include "librtmp2/types.h"
#include <string.h>

/**
 * Read a single chunk from `buf`, splitting payload into `out_payload` (up to
 * `chunk_size` bytes), and merging with previous partial data via chunk_stream.
 *
 * Returns number of payload bytes written into `out`, or negative error code.
 */
int lrtmp2_chunk_read(lrtmp2_buffer_t *buf,
                       lrtmp2_chunk_registry_t *registry,
                       lrtmp2_chunk_stream_t *stream,
                       lrtmp2_chunk_message_t *msg,
                       const uint8_t **out_payload, size_t *out_len)
{
    if (!buf || !msg || !out_payload || !out_len || (!registry && !stream)) {
        return LRTMP2_ERR_INTERNAL;
    }
    *out_payload = NULL;
    *out_len = 0;

    if (buf->size - buf->read_pos == 0) {
        return 0;  /* no data */
    }

    /* Save read position for rollback on incomplete reads */
    size_t start_pos = buf->read_pos;


    /* --- Basic header: fmt(2 bits) + csid(6/14/22 bits) --- */
    uint8_t first_byte;
    if (lrtmp2_buffer_read(buf, &first_byte, 1) != 0) goto need_more;

    uint8_t fmt = (first_byte >> 6) & 0x03;
    uint32_t csid = first_byte & 0x3F;

    if (csid == 0) {
        /* 2-byte form: csid = 64 + next byte */
        uint8_t b;
        if (lrtmp2_buffer_read(buf, &b, 1) != 0) goto need_more;
        csid = 64 + b;
    } else if (csid == 1) {
        /* 3-byte form: the indicator byte is followed by exactly 2 bytes,
         * csid = 64 + byte0 + byte1*256 (byte0 low, byte1 high). */
        uint8_t b[2];
        if (lrtmp2_buffer_read(buf, b, 2) != 0) goto need_more;
        csid = 64 + b[0] + b[1] * 256;
    }
    /* csid 2-63: 1-byte form, no extra read needed */

    /* Read chunk stream state — or use provided external state */
    lrtmp2_chunk_stream_t *cs = stream ? stream : lrtmp2_chunk_stream_get(registry, csid);
    if (!cs) return LRTMP2_ERR_INTERNAL;

    /* --- Message header: depends on fmt --- */
    uint32_t timestamp = 0;
    uint32_t msg_length = 0;
    uint8_t  msg_type_id = 0;
    uint32_t msg_stream_id = 0;

    switch (fmt) {
        case 0:
            /* 11 bytes: timestamp(3) + msg_length(3) + msg_type(1) + stream_id(4) */
            {
                uint8_t ts_bytes[3], len_bytes[3];
                if (lrtmp2_buffer_read(buf, ts_bytes, 3) != 0) goto need_more;
                timestamp = lrtmp2_ntoh24(ts_bytes);
                if (lrtmp2_buffer_read(buf, len_bytes, 3) != 0) goto need_more;
                msg_length = lrtmp2_ntoh24(len_bytes);
                if (lrtmp2_buffer_read(buf, &msg_type_id, 1) != 0) goto need_more;
                uint8_t sid[4];
                if (lrtmp2_buffer_read(buf, sid, 4) != 0) goto need_more;
                msg_stream_id = lrtmp2_ntoh32(sid);
            }
            /* Extended timestamp check */
            if (timestamp == 0xFFFFFF) {
                uint8_t ext[4];
                if (lrtmp2_buffer_read(buf, ext, 4) != 0) goto need_more;
                timestamp = lrtmp2_ntoh32(ext);
            }
            /* fmt=0 starts a new message; discard any partial reassembly. */
            cs->reassembly_bytes_read = 0;
            if (cs->reassembly_buf) {
                lrtmp2_buffer_reset(cs->reassembly_buf);
            }
            /* Update state */
            cs->type0_timestamp = timestamp;
            cs->type0_msg_length = msg_length;
            cs->type0_msg_type_id = msg_type_id;
            cs->type0_msg_stream_id = msg_stream_id;
            break;

        case 1:
            /* 7 bytes: delta_timestamp(3) + msg_length(3) + msg_type(1) */
            {
                uint8_t ts_bytes[3], len_bytes[3];
                if (lrtmp2_buffer_read(buf, ts_bytes, 3) != 0) goto need_more;
                timestamp = lrtmp2_ntoh24(ts_bytes);
                if (lrtmp2_buffer_read(buf, len_bytes, 3) != 0) goto need_more;
                msg_length = lrtmp2_ntoh24(len_bytes);
                if (lrtmp2_buffer_read(buf, &msg_type_id, 1) != 0) goto need_more;
            }
            if (timestamp == 0xFFFFFF) {
                uint8_t ext[4];
                if (lrtmp2_buffer_read(buf, ext, 4) != 0) goto need_more;
                timestamp = lrtmp2_ntoh32(ext);
            }
            /* Same stream_id as previous */
            msg_stream_id = cs->type0_msg_stream_id;
            timestamp += cs->type0_timestamp;
            /* Update running timestamp for subsequent delta-based chunks */
            cs->type0_timestamp = timestamp;
            cs->type0_msg_length = msg_length;
            cs->type0_msg_type_id = msg_type_id;
            /* cs->type0_msg_stream_id unchanged */
            break;

        case 2:
            /* 3 bytes: delta_timestamp(3) */
            {
                uint8_t ts_bytes[3];
                if (lrtmp2_buffer_read(buf, ts_bytes, 3) != 0) goto need_more;
                timestamp = lrtmp2_ntoh24(ts_bytes);
            }
            if (timestamp == 0xFFFFFF) {
                uint8_t ext[4];
                if (lrtmp2_buffer_read(buf, ext, 4) != 0) goto need_more;
                timestamp = lrtmp2_ntoh32(ext);
            }
            /* Same msg_length, msg_type_id, msg_stream_id as previous */
            msg_length = cs->type0_msg_length;
            msg_type_id = cs->type0_msg_type_id;
            msg_stream_id = cs->type0_msg_stream_id;
            timestamp += cs->type0_timestamp;
            cs->type0_timestamp = timestamp;
            break;

        case 3:
            /* 0 bytes: exact same header as previous chunk of same type */
            timestamp = cs->type0_timestamp;
            msg_length = cs->type0_msg_length;
            msg_type_id = cs->type0_msg_type_id;
            msg_stream_id = cs->type0_msg_stream_id;
            break;

        default:
            return LRTMP2_ERR_CHUNK;
    }

    /* --- Payload: read up to chunk_size bytes for this physical chunk,
     * accumulating into the chunk stream's reassembly buffer until the full
     * message (msg_length bytes) has been collected. --- */
    if (cs->reassembly_bytes_read > msg_length) {
        LRTMP2_LOG_WARN("chunk csid=%u: msg_length shrank below reassembly progress "
                        "(%u < %u)", csid, msg_length, cs->reassembly_bytes_read);
        return LRTMP2_ERR_CHUNK;
    }
    size_t remaining = (size_t)(msg_length - cs->reassembly_bytes_read);
    if (cs->chunk_size == 0) {
        LRTMP2_LOG_WARN("chunk csid=%u: invalid chunk_size=0", csid);
        return LRTMP2_ERR_CHUNK;
    }
    size_t to_read = (remaining < cs->chunk_size) ? remaining : cs->chunk_size;

    if (buf->size - buf->read_pos < to_read) {
        goto need_more;  /* not enough data for this chunk payload */
    }

    if (!cs->reassembly_buf) {
        cs->reassembly_buf = lrtmp2_buffer_create();
        if (!cs->reassembly_buf) return LRTMP2_ERR_INTERNAL;
    }
    if (cs->reassembly_bytes_read == 0) {
        lrtmp2_buffer_reset(cs->reassembly_buf);
    }

    if (to_read > 0) {
        uint8_t tmp[4096];
        size_t off = 0;
        while (off < to_read) {
            size_t n = to_read - off;
            if (n > sizeof(tmp)) n = sizeof(tmp);
            if (lrtmp2_buffer_read(buf, tmp, n) != 0) goto need_more;
            if (lrtmp2_buffer_write(cs->reassembly_buf, tmp, n) != LRTMP2_OK) {
                /* Reassembly buffer couldn't grow (OOM or size cap). Bail out
                 * rather than reporting a complete message backed by a
                 * buffer smaller than msg_length (would cause an over-read). */
                return LRTMP2_ERR_INTERNAL;
            }
            off += n;
        }
    }

    cs->reassembly_bytes_read += to_read;

    /* Fill message info */
    msg->timestamp = timestamp;
    msg->msg_length = msg_length;
    msg->msg_type_id = msg_type_id;
    msg->msg_stream_id = msg_stream_id;
    msg->csid = csid;
    msg->fmt = fmt;
    msg->is_complete = (cs->reassembly_bytes_read >= msg_length);

    LRTMP2_LOG_DEBUG("chunk read: csid=%u fmt=%u ts=%u len=%u payload=%zu/%u complete=%d",
                      csid, fmt, timestamp, msg_length, to_read,
                      cs->reassembly_bytes_read, msg->is_complete);

    if (msg->is_complete) {
        /* Hand back the reassembly buffer directly (zero-copy). Valid until the
         * next read on this stream. */
        *out_payload = (msg_length > 0 && cs->reassembly_buf) ? cs->reassembly_buf->data : NULL;
        *out_len = msg_length;
        cs->reassembly_bytes_read = 0;  /* reset for next message on this csid */
        return 1;
    }

    /* Progress made on this physical chunk, but message not complete yet:
     * return a positive sentinel (never 0) so callers don't mistake this
     * for "no data available" and stop looping. */
    return 1;

need_more:
    buf->read_pos = start_pos;  /* rollback */
    return 0;
}
