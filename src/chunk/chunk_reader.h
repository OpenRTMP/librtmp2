#ifndef LRTMP2_CHUNK_READER_H
#define LRTMP2_CHUNK_READER_H

#include "core/buffer.h"
#include "chunk/chunk_state.h"
#include "librtmp2/types.h"

/* Same as message definition in chunk_state.h */

/**
 * Read one chunk from `buf`.
 * Returns >0 = payload bytes read, 0 = need more data, <0 = error.
 * On return, `msg` is filled with the chunk metadata.
 * Payload is written to `out_buf` (caller provides buffer).
 */
int lrtmp2_chunk_read(lrtmp2_buffer_t *buf,
                       lrtmp2_chunk_stream_t *stream,
                       lrtmp2_chunk_message_t *msg,
                       uint8_t *out_buf, size_t *out_len);

#endif
