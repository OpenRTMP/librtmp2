/**
 * chunk_state.c — Per-connection chunk-stream state
 */
#include "chunk_state.h"
#include "core/log.h"
#include <string.h>

#define LRTMP2_CHUNK_STREAMS_INITIAL_CAP 8

static void registry_free_streams(lrtmp2_chunk_registry_t *reg)
{
    if (!reg->streams) return;
    for (size_t i = 0; i < reg->count; i++) {
        lrtmp2_chunk_stream_t *s = reg->streams[i];
        if (!s) continue;
        if (s->reassembly_buf) lrtmp2_buffer_destroy(s->reassembly_buf);
        LRTMP2_FREE(s);
    }
    LRTMP2_FREE(reg->streams);
    reg->streams = NULL;
    reg->count = 0;
    reg->capacity = 0;
}

void lrtmp2_chunk_registry_init(lrtmp2_chunk_registry_t *reg)
{
    if (!reg) return;
    /* Free anything left over from a previous lifecycle so a re-init does not
     * leak streams or their reassembly buffers. */
    if (reg->initialized) {
        registry_free_streams(reg);
    }
    reg->streams = NULL;
    reg->count = 0;
    reg->capacity = 0;
    reg->default_chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;
    reg->initialized = 1;
}

lrtmp2_chunk_stream_t *lrtmp2_chunk_stream_get(lrtmp2_chunk_registry_t *reg, uint32_t csid)
{
    if (!reg) return NULL;
    if (!reg->initialized) lrtmp2_chunk_registry_init(reg);

    /* Find an existing stream for this csid. */
    for (size_t i = 0; i < reg->count; i++) {
        if (reg->streams[i]->in_use && reg->streams[i]->csid == csid) {
            return reg->streams[i];
        }
    }

    /* Reuse a previously freed slot if one exists (keeps its reassembly buffer
     * allocated for reuse). */
    for (size_t i = 0; i < reg->count; i++) {
        lrtmp2_chunk_stream_t *s = reg->streams[i];
        if (!s->in_use) {
            lrtmp2_buffer_t *buf = s->reassembly_buf;
            memset(s, 0, sizeof(*s));
            s->reassembly_buf = buf;
            if (buf) lrtmp2_buffer_reset(buf);
            s->csid = csid;
            s->in_use = 1;
            s->chunk_size = reg->default_chunk_size;
            return s;
        }
    }

    /* Need a new stream node — guard against unbounded growth from a hostile
     * peer announcing endless csids. */
    if (reg->count >= LRTMP2_MAX_CHUNK_STREAMS) {
        LRTMP2_LOG_ERROR("Chunk stream cap (%d) reached for csid=%u",
                          LRTMP2_MAX_CHUNK_STREAMS, csid);
        return NULL;
    }

    /* Grow the pointer array if full. */
    if (reg->count == reg->capacity) {
        size_t new_cap = reg->capacity ? reg->capacity * 2 : LRTMP2_CHUNK_STREAMS_INITIAL_CAP;
        if (new_cap > LRTMP2_MAX_CHUNK_STREAMS) new_cap = LRTMP2_MAX_CHUNK_STREAMS;
        lrtmp2_chunk_stream_t **grown =
            LRTMP2_REALLOC(reg->streams, new_cap * sizeof(*grown));
        if (!grown) {
            LRTMP2_LOG_ERROR("Failed to grow chunk stream array for csid=%u", csid);
            return NULL;
        }
        reg->streams = grown;
        reg->capacity = new_cap;
    }

    lrtmp2_chunk_stream_t *s = LRTMP2_CALLOC(1, sizeof(*s));
    if (!s) {
        LRTMP2_LOG_ERROR("Failed to allocate chunk stream for csid=%u", csid);
        return NULL;
    }
    s->csid = csid;
    s->in_use = 1;
    s->chunk_size = reg->default_chunk_size;
    reg->streams[reg->count++] = s;
    LRTMP2_LOG_DEBUG("Allocated chunk stream csid=%u (slot %zu)", csid, reg->count - 1);
    return s;
}

void lrtmp2_chunk_stream_set_all_chunk_size(lrtmp2_chunk_registry_t *reg, uint32_t chunk_size)
{
    if (!reg) return;
    if (!reg->initialized) lrtmp2_chunk_registry_init(reg);
    /* Apply to existing streams and remember it for streams created later. */
    reg->default_chunk_size = chunk_size;
    for (size_t i = 0; i < reg->count; i++) {
        if (reg->streams[i]->in_use) {
            reg->streams[i]->chunk_size = chunk_size;
        }
    }
}

void lrtmp2_chunk_registry_destroy(lrtmp2_chunk_registry_t *reg)
{
    if (!reg) return;
    registry_free_streams(reg);
    reg->initialized = 0;
}

int lrtmp2_chunk_registry_check_reassembly_budget(lrtmp2_chunk_registry_t *reg,
                                                   const lrtmp2_chunk_stream_t *cs,
                                                   size_t additional)
{
    if (!reg || !cs || additional == 0) return LRTMP2_OK;

    size_t total = 0;
    for (size_t i = 0; i < reg->count; i++) {
        lrtmp2_chunk_stream_t *s = reg->streams[i];
        if (!s || !s->in_use || !s->reassembly_buf) continue;
        total += s->reassembly_buf->size;
    }

    /* `reassembly_buf->size` already includes bytes staged for `cs`; only the
     * net growth `additional` needs to fit under the cap. */
    if (total + additional > LRTMP2_MAX_REASSEMBLY_BYTES_PER_CONN) {
        LRTMP2_LOG_WARN("Per-connection reassembly cap (%zu bytes) exceeded",
                        (size_t)LRTMP2_MAX_REASSEMBLY_BYTES_PER_CONN);
        return LRTMP2_ERR_CHUNK;
    }
    return LRTMP2_OK;
}

void lrtmp2_chunk_stream_reset(lrtmp2_chunk_registry_t *reg, lrtmp2_chunk_stream_t *stream)
{
    if (!stream) return;
    /* Keep the reassembly buffer allocated for reuse; just clear its contents
     * and reset the rest of the stream state. */
    lrtmp2_buffer_t *buf = stream->reassembly_buf;
    memset(stream, 0, sizeof(*stream));
    stream->reassembly_buf = buf;
    if (buf) lrtmp2_buffer_reset(buf);
    /* Restore the negotiated chunk size (not the wire default) so a reset stream
     * keeps framing correctly after a SetChunkSize. */
    stream->chunk_size = (reg && reg->initialized) ? reg->default_chunk_size
                                                   : LRTMP2_DEFAULT_CHUNK_SIZE;
}
