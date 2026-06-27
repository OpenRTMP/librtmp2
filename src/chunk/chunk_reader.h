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
 * `*out_payload` is NULL and `*out_len` is 0.
 *
 * On completion `*out_payload` points at the fully reassembled message
 * (`*out_len` bytes). The pointer is owned by the chunk stream and stays valid
 * only until the next lrtmp2_chunk_read() call for the same stream, so the
 * caller must consume it before reading again. Zero-copy: there is no fixed
 * output buffer, so arbitrarily large messages are returned without truncation.
 *
 * Chunk-stream state is looked up by csid in `registry` (per-connection). Pass an
 * explicit `stream` to bypass the registry and use caller-owned state instead
 * (used by tests/fuzzers); at least one of `registry`/`stream` must be non-NULL.
 */
int lrtmp2_chunk_read(lrtmp2_buffer_t *buf,
                       lrtmp2_chunk_registry_t *registry,
                       lrtmp2_chunk_stream_t *stream,
                       lrtmp2_chunk_message_t *msg,
                       const uint8_t **out_payload, size_t *out_len);

#endif
