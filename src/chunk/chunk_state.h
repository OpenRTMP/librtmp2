#ifndef LRTMP2_CHUNK_STATE_H
#define LRTMP2_CHUNK_STATE_H

#include "core/buffer.h"
#include "core/alloc.h"
#include "librtmp2/types.h"

#define LRTMP2_DEFAULT_CHUNK_SIZE 128

typedef struct {
    uint32_t csid;
    uint32_t chunk_size;        /* peer's chunk size (SetChunkSize) */
    uint32_t type0_timestamp;   /* running timestamp for this chunk stream */
    uint32_t type0_msg_length;
    uint8_t  type0_msg_type_id;
    uint32_t type0_msg_stream_id;
    uint32_t reassembly_bytes_read;  /* bytes read so far for current message */
    lrtmp2_buffer_t *reassembly_buf; /* buffer for reassembling partial messages */
    int in_use;
} lrtmp2_chunk_stream_t;

typedef struct {
    uint32_t timestamp;
    uint32_t msg_length;
    uint8_t  msg_type_id;
    uint32_t msg_stream_id;
    uint32_t csid;
    uint8_t  fmt;
    int      is_complete;
} lrtmp2_chunk_message_t;

void lrtmp2_chunk_streams_init(void);
lrtmp2_chunk_stream_t *lrtmp2_chunk_stream_get(uint32_t csid);
void lrtmp2_chunk_stream_reset(lrtmp2_chunk_stream_t *stream);

#endif
