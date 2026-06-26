#ifndef LRTMP2_MESSAGE_INTERNAL_H
#define LRTMP2_MESSAGE_INTERNAL_H

#include "core/buffer.h"
#include "chunk/chunk_state.h"
#include "session/conn.h"

/* Message type IDs */
#define RTMP_MSG_SET_CHUNK_SIZE       0x01
#define RTMP_MSG_ABORT_MESSAGE        0x02
#define RTMP_MSG_ACKNOWLEDGEMENT      0x03
#define RTMP_MSG_USER_CONTROL         0x04
#define RTMP_MSG_WINDOW_ACK_SIZE      0x05
#define RTMP_MSG_SET_PEER_BANDWIDTH   0x06
#define RTMP_MSG_AMF0_COMMAND         0x14
#define RTMP_MSG_AMF0_DATA            0x12
#define RTMP_MSG_AMF3_COMMAND         0x11
#define RTMP_MSG_AMF3_DATA            0x0F
#define RTMP_MSG_AGGREGATE            0x16

/* Chunk writer functions */
int lrtmp2_chunk_write(lrtmp2_buffer_t *out,
                        const lrtmp2_chunk_message_t *msg,
                        const uint8_t *payload, size_t payload_len);
int lrtmp2_chunk_write_extended_timestamp(lrtmp2_buffer_t *out, uint32_t timestamp);

/* Message write helpers */
int lrtmp2_msg_write_set_chunk_size(lrtmp2_buffer_t *buf, uint32_t chunk_size);

/* Message decoder */
int lrtmp2_msg_decode(lrtmp2_conn_t *conn, const lrtmp2_chunk_message_t *chunk,
                       const uint8_t *payload, size_t payload_len);

#endif
