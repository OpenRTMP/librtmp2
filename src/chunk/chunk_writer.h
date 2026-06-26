#ifndef LRTMP2_CHUNK_WRITER_H
#define LRTMP2_CHUNK_WRITER_H

#include "core/buffer.h"
#include "chunk/chunk_state.h"
#include "librtmp2/types.h"

/**
 * Write a single chunk to `out`.
 * Caller is responsible for splitting payload into chunk_size pieces.
 */
int lrtmp2_chunk_write(lrtmp2_buffer_t *out,
                        const lrtmp2_chunk_message_t *msg,
                        const uint8_t *payload, size_t payload_len);

/**
 * Write an extended timestamp chunk (for protocol control messages).
 */
int lrtmp2_chunk_write_extended_timestamp(lrtmp2_buffer_t *out, uint32_t timestamp);

#endif
