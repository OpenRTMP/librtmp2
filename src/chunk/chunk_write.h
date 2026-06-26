#ifndef LRTMP2_CHUNK_WRITE_H
#define LRTMP2_CHUNK_WRITE_H

#include "core/buffer.h"
#include "chunk/chunk_state.h"

int lrtmp2_chunk_write(lrtmp2_buffer_t *out,
                        const lrtmp2_chunk_message_t *msg,
                        const uint8_t *payload, size_t payload_len);
int lrtmp2_chunk_write_extended_timestamp(lrtmp2_buffer_t *out, uint32_t timestamp);

#endif
