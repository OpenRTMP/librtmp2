#ifndef LRTMP2_CHUNK_STATE_H
#define LRTMP2_CHUNK_STATE_H

#include "core/buffer.h"
#include "core/alloc.h"
#include "librtmp2/types.h"

#define LRTMP2_DEFAULT_CHUNK_SIZE 128

/* Upper bound on the number of distinct chunk streams a single connection may
 * open. The registry grows on demand up to this cap; the cap exists only to keep
 * a malicious or buggy peer from exhausting memory by announcing endless csids.
 * RTMP csids range up to 65599, but real clients use a small handful. */
#define LRTMP2_MAX_CHUNK_STREAMS 4096

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

/* Per-connection chunk-stream registry. Each connection (or client) owns one of
 * these so concurrent connections served in a single thread do not share — and
 * corrupt — each other's chunk streams, reassembly buffers, or negotiated chunk
 * size.
 *
 * Streams are individually heap-allocated and referenced through a dynamically
 * grown pointer array, so the registry handles an arbitrary number of parallel
 * chunk streams (up to LRTMP2_MAX_CHUNK_STREAMS) and a stream pointer handed to
 * a caller stays valid even when the registry grows to admit new csids. */
typedef struct {
    lrtmp2_chunk_stream_t **streams; /* array of stream pointers (heap) */
    size_t count;                    /* number of allocated stream nodes */
    size_t capacity;                 /* capacity of the streams array */
    /* Chunk size applied to chunk streams created after the peer's SetChunkSize.
     * Without this, streams opened later (e.g. ffmpeg's audio/video csids, which
     * it opens after announcing its chunk size) would default to 128 and the
     * incoming media would be mis-framed. This is also the connection's current
     * view of the peer's chunk size. */
    uint32_t default_chunk_size;
    int initialized;
} lrtmp2_chunk_registry_t;

typedef struct {
    uint32_t timestamp;
    uint32_t msg_length;
    uint8_t  msg_type_id;
    uint32_t msg_stream_id;
    uint32_t csid;
    uint8_t  fmt;
    int      is_complete;
} lrtmp2_chunk_message_t;

/* Initialize (or re-initialize) a registry. Frees any streams/reassembly buffers
 * left over from a previous lifecycle so a re-init does not leak them. */
void lrtmp2_chunk_registry_init(lrtmp2_chunk_registry_t *reg);

/* Find an existing chunk stream for `csid` in `reg`, or allocate a new one,
 * growing the registry as needed. Returns NULL on allocation failure or when the
 * LRTMP2_MAX_CHUNK_STREAMS cap is reached. */
lrtmp2_chunk_stream_t *lrtmp2_chunk_stream_get(lrtmp2_chunk_registry_t *reg, uint32_t csid);

void lrtmp2_chunk_stream_reset(lrtmp2_chunk_registry_t *reg, lrtmp2_chunk_stream_t *stream);

/* SetChunkSize applies to all chunk streams from a given peer, but chunk_size
 * is tracked per-csid; this propagates a new size to every active stream and
 * remembers it for streams created later. */
void lrtmp2_chunk_stream_set_all_chunk_size(lrtmp2_chunk_registry_t *reg, uint32_t chunk_size);

/* Destroy all chunk streams in `reg` and free reassembly buffers and the stream
 * array. Call at connection close. The registry struct itself is not freed. */
void lrtmp2_chunk_registry_destroy(lrtmp2_chunk_registry_t *reg);

#endif
