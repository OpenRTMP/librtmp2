#ifndef LRTMP2_MESSAGE_CONTROL_H
#define LRTMP2_MESSAGE_CONTROL_H

#include "core/buffer.h"
#include <stdint.h>
#include "librtmp2/types.h"

/* Control message types */
#define LRTMP2_CTRL_SET_CHUNK_SIZE       0x01
#define LRTMP2_CTRL_ABORT_MESSAGE        0x02
#define LRTMP2_CTRL_ACKNOWLEDGEMENT      0x03
#define LRTMP2_CTRL_USER_CONTROL         0x04
#define LRTMP2_CTRL_WINDOW_ACK_SIZE      0x05
#define LRTMP2_CTRL_SET_PEER_BANDWIDTH   0x06

/* User Control event types */
#define LRTMP2_UCTRL_STREAM_BEGIN        0x00
#define LRTMP2_UCTRL_STREAM_EOF          0x01
#define LRTMP2_UCTRL_STREAM_DRY          0x02
#define LRTMP2_UCTRL_SET_BUFFER_LENGTH    0x03
#define LRTMP2_UCTRL_STREAM_IS_RECORDED  0x04
#define LRTMP2_UCTRL_PING_REQUEST        0x06
#define LRTMP2_UCTRL_PING_RESPONSE       0x07

/* Encoder */
int lrtmp2_msg_write_set_chunk_size(lrtmp2_buffer_t *buf, uint32_t chunk_size);
int lrtmp2_msg_write_abort_message(lrtmp2_buffer_t *buf, uint32_t csid);
int lrtmp2_msg_write_acknowledgement(lrtmp2_buffer_t *buf, uint32_t sequence_number);
int lrtmp2_msg_write_window_ack_size(lrtmp2_buffer_t *buf, uint32_t window_size);
int lrtmp2_msg_write_set_peer_bandwidth(lrtmp2_buffer_t *buf, uint32_t window_size, uint8_t limit_type);
int lrtmp2_msg_write_user_control_stream_begin(lrtmp2_buffer_t *buf, uint32_t stream_id);
int lrtmp2_msg_write_user_control_stream_eof(lrtmp2_buffer_t *buf, uint32_t stream_id);
int lrtmp2_msg_write_user_control_set_buffer_length(lrtmp2_buffer_t *buf, uint32_t stream_id, uint32_t ms);

/* Decoder */
int lrtmp2_msg_read_set_chunk_size(const uint8_t *data, uint32_t *chunk_size);
int lrtmp2_msg_read_abort_message(const uint8_t *data, uint32_t *csid);
int lrtmp2_msg_read_acknowledgement_size(const uint8_t *data, uint32_t *seq);
int lrtmp2_msg_read_window_ack_size(const uint8_t *data, uint32_t *window);
int lrtmp2_msg_read_set_peer_bandwidth(const uint8_t *data, uint32_t *window, uint8_t *limit);
int lrtmp2_msg_read_user_control(const uint8_t *data, uint16_t *event_type, uint32_t *param1, uint32_t *param2);

#endif
