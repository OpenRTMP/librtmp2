/**
 * chunk_state.c — Per-connection chunk-stream state
 */
#include "chunk_state.h"
#include "core/log.h"
#include <string.h>

void lrtmp2_chunk_registry_init(lrtmp2_chunk_registry_t *reg)
{
    if (!reg) return;
    /* Free any reassembly buffers left over from a previous lifecycle so a
     * re-init does not leak them. */
    if (reg->initialized) {
        for (int i = 0; i < LRTMP2_MAX_CHUNK_STREAMS; i++) {
            if (reg->streams[i].reassembly_buf) {
                lrtmp2_buffer_destroy(reg->streams[i].reassembly_buf);
            }
        }
    }
    memset(reg->streams, 0, sizeof(reg->streams));
    reg->default_chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;
    reg->initialized = 1;
}

lrtmp2_chunk_stream_t *lrtmp2_chunk_stream_get(lrtmp2_chunk_registry_t *reg, uint32_t csid)
{
    if (!reg) return NULL;
    if (!reg->initialized) lrtmp2_chunk_registry_init(reg);

    /* Find existing or allocate new */
    for (int i = 0; i < LRTMP2_MAX_CHUNK_STREAMS; i++) {
        if (reg->streams[i].csid == csid && reg->streams[i].in_use) {
            return &reg->streams[i];
        }
    }

    /* Allocate a new slot */
    for (int i = 0; i < LRTMP2_MAX_CHUNK_STREAMS; i++) {
        if (!reg->streams[i].in_use) {
            memset(&reg->streams[i], 0, sizeof(reg->streams[i]));
            reg->streams[i].csid = csid;
            reg->streams[i].in_use = 1;
            reg->streams[i].chunk_size = reg->default_chunk_size;
            LRTMP2_LOG_DEBUG("Allocated chunk stream csid=%u (slot %d)", csid, i);
            return &reg->streams[i];
        }
    }

    LRTMP2_LOG_ERROR("No free chunk stream slots for csid=%u", csid);
    return NULL;
}

void lrtmp2_chunk_stream_set_all_chunk_size(lrtmp2_chunk_registry_t *reg, uint32_t chunk_size)
{
    if (!reg) return;
    if (!reg->initialized) lrtmp2_chunk_registry_init(reg);
    /* Apply to existing streams and remember it for streams created later. */
    reg->default_chunk_size = chunk_size;
    for (int i = 0; i < LRTMP2_MAX_CHUNK_STREAMS; i++) {
        if (reg->streams[i].in_use) {
            reg->streams[i].chunk_size = chunk_size;
        }
    }
}

void lrtmp2_chunk_registry_destroy(lrtmp2_chunk_registry_t *reg)
{
    if (!reg) return;
    for (int i = 0; i < LRTMP2_MAX_CHUNK_STREAMS; i++) {
        if (reg->streams[i].reassembly_buf) {
            lrtmp2_buffer_destroy(reg->streams[i].reassembly_buf);
            reg->streams[i].reassembly_buf = NULL;
        }
        reg->streams[i].in_use = 0;
    }
    reg->initialized = 0;
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
