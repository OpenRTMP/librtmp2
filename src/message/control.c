/**
 * control.c — RTMP control message encoder/decoder
 *
 * Control messages:
 *   Type 1: SetChunkSize
 *   Type 2: AbortMessage
 *   Type 3: Acknowledgement
 *   Type 4: User Control Messages
 *   Type 5: WindowAcknowledgementSize
 *   Type 6: SetPeerBandwidth
 */
#include "message/control.h"
#include "core/bytes.h"
#include "core/log.h"
#include <string.h>
#include "librtmp2/types.h"

/* --- Encoder --- */

int lrtmp2_msg_write_set_chunk_size(lrtmp2_buffer_t *buf, uint32_t chunk_size)
{
    /* Type=1, 4-byte chunk size (must be <= 2147483647, at least 1) */
    uint8_t type = LRTMP2_CTRL_SET_CHUNK_SIZE;
    lrtmp2_buffer_write(buf, &type, 1);
    uint32_t net = lrtmp2_hton32(chunk_size);
    return lrtmp2_buffer_write(buf, (uint8_t *)&net, 4);
}

int lrtmp2_msg_write_abort_message(lrtmp2_buffer_t *buf, uint32_t csid)
{
    uint8_t type = LRTMP2_CTRL_ABORT_MESSAGE;
    lrtmp2_buffer_write(buf, &type, 1);
    uint32_t net = lrtmp2_hton32(csid);
    return lrtmp2_buffer_write(buf, (uint8_t *)&net, 4);
}

int lrtmp2_msg_write_acknowledgement(lrtmp2_buffer_t *buf, uint32_t sequence_number)
{
    uint8_t type = LRTMP2_CTRL_ACKNOWLEDGEMENT;
    lrtmp2_buffer_write(buf, &type, 1);
    uint32_t net = lrtmp2_hton32(sequence_number);
    return lrtmp2_buffer_write(buf, (uint8_t *)&net, 4);
}

int lrtmp2_msg_write_window_ack_size(lrtmp2_buffer_t *buf, uint32_t window_size)
{
    uint8_t type = LRTMP2_CTRL_WINDOW_ACK_SIZE;
    lrtmp2_buffer_write(buf, &type, 1);
    uint32_t net = lrtmp2_hton32(window_size);
    return lrtmp2_buffer_write(buf, (uint8_t *)&net, 4);
}

int lrtmp2_msg_write_set_peer_bandwidth(lrtmp2_buffer_t *buf, uint32_t window_size, uint8_t limit_type)
{
    uint8_t type = LRTMP2_CTRL_SET_PEER_BANDWIDTH;
    lrtmp2_buffer_write(buf, &type, 1);
    uint32_t net = lrtmp2_hton32(window_size);
    lrtmp2_buffer_write(buf, (uint8_t *)&net, 4);
    return lrtmp2_buffer_write(buf, &limit_type, 1);
}

/* --- User Control Events --- */

int lrtmp2_msg_write_user_control_stream_begin(lrtmp2_buffer_t *buf, uint32_t stream_id)
{
    uint16_t evt = lrtmp2_byteswap16(LRTMP2_UCTRL_STREAM_BEGIN);
    lrtmp2_buffer_write(buf, (uint8_t *)&evt, 2);
    uint32_t net = lrtmp2_hton32(stream_id);
    return lrtmp2_buffer_write(buf, (uint8_t *)&net, 4);
}

int lrtmp2_msg_write_user_control_stream_eof(lrtmp2_buffer_t *buf, uint32_t stream_id)
{
    uint16_t evt = lrtmp2_byteswap16(LRTMP2_UCTRL_STREAM_EOF);
    lrtmp2_buffer_write(buf, (uint8_t *)&evt, 2);
    uint32_t net = lrtmp2_hton32(stream_id);
    return lrtmp2_buffer_write(buf, (uint8_t *)&net, 4);
}

int lrtmp2_msg_write_user_control_set_buffer_length(lrtmp2_buffer_t *buf, uint32_t stream_id, uint32_t ms)
{
    uint16_t evt = lrtmp2_byteswap16(LRTMP2_UCTRL_SET_BUFFER_LENGTH);
    lrtmp2_buffer_write(buf, (uint8_t *)&evt, 2);
    uint32_t net_sid = lrtmp2_hton32(stream_id);
    lrtmp2_buffer_write(buf, (uint8_t *)&net_sid, 4);
    uint32_t net_ms = lrtmp2_hton32(ms);
    return lrtmp2_buffer_write(buf, (uint8_t *)&net_ms, 4);
}

/* --- Decoder --- */

int lrtmp2_msg_read_set_chunk_size(const uint8_t *data, uint32_t *chunk_size)
{
    if (!data || !chunk_size) return LRTMP2_ERR_INTERNAL;
    *chunk_size = lrtmp2_ntoh32(data);
    LRTMP2_LOG_DEBUG("SetChunkSize: %u", *chunk_size);
    return LRTMP2_OK;
}

int lrtmp2_msg_read_abort_message(const uint8_t *data, uint32_t *csid)
{
    if (!data || !csid) return LRTMP2_ERR_INTERNAL;
    *csid = lrtmp2_ntoh32(data);
    LRTMP2_LOG_DEBUG("AbortMessage: csid=%u", *csid);
    return LRTMP2_OK;
}

int lrtmp2_msg_read_acknowledgement_size(const uint8_t *data, uint32_t *seq)
{
    if (!data || !seq) return LRTMP2_ERR_INTERNAL;
    *seq = lrtmp2_ntoh32(data);
    return LRTMP2_OK;
}

int lrtmp2_msg_read_window_ack_size(const uint8_t *data, uint32_t *window)
{
    if (!data || !window) return LRTMP2_ERR_INTERNAL;
    *window = lrtmp2_ntoh32(data);
    return LRTMP2_OK;
}

int lrtmp2_msg_read_set_peer_bandwidth(const uint8_t *data, uint32_t *window, uint8_t *limit)
{
    if (!data || !window || !limit) return LRTMP2_ERR_INTERNAL;
    *window = lrtmp2_ntoh32(data);
    *limit = data[4];
    return LRTMP2_OK;
}

int lrtmp2_msg_read_user_control(const uint8_t *data, uint16_t *event_type, uint32_t *param1, uint32_t *param2)
{
    if (!data || !event_type || !param1) return LRTMP2_ERR_INTERNAL;
    *event_type = lrtmp2_byteswap16(*(const uint16_t *)data);
    *param1 = lrtmp2_ntoh32(data + 2);
    if (param2) {
        *param2 = lrtmp2_ntoh32(data + 6);
    }
    return LRTMP2_OK;
}
