#ifndef LRTMP2_CHUNK_READER_H
#define LRTMP2_CHUNK_READER_H

#include "core/buffer.h"
#include "chunk/chunk_state.h"
#include "librtmp2/types.h"

/* Same as message definition in chunk_state.h */

/**
 * Read and reassemble one RTMP message from `buf`, which may span several
 * physical chunks. Returns >0 on progress (a physical chunk was consumed),
 * 0 = need more data in `buf`, <0 = error. `msg->is_complete` tells the
 * caller whether the full message has been reassembled; until then,
 * `*out_len` is 0. On completion, the full message (up to `out_cap` bytes)
 * is copied into `out_buf` and `*out_len` is set to its length.
 */
int lrtmp2_chunk_read(lrtmp2_buffer_t *buf,
                       lrtmp2_chunk_stream_t *stream,
                       lrtmp2_chunk_message_t *msg,
                       uint8_t *out_buf, size_t out_cap, size_t *out_len);

#endif
