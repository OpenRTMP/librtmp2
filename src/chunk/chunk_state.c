/**
 * chunk_state.c — Per-chunk-stream state
 */
#include "chunk_state.h"
#include "core/log.h"
#include <string.h>

#define MAX_CHUNK_STREAMS 8

static __thread lrtmp2_chunk_stream_t g_streams[MAX_CHUNK_STREAMS];
static __thread int g_initialized = 0;
/* Chunk size applied to chunk streams created after the peer's SetChunkSize.
 * Without this, streams opened later (e.g. ffmpeg's audio/video csids, which
 * it opens after announcing its chunk size) would default to 128 and the
 * incoming media would be mis-framed. */
static __thread uint32_t g_default_chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;

void lrtmp2_chunk_streams_init(void)
{
    /* Free any reassembly buffers left over from a previous lifecycle so a
     * re-init does not leak them. */
    if (g_initialized) {
        for (int i = 0; i < MAX_CHUNK_STREAMS; i++) {
            if (g_streams[i].reassembly_buf) {
                lrtmp2_buffer_destroy(g_streams[i].reassembly_buf);
            }
        }
    }
    memset(g_streams, 0, sizeof(g_streams));
    g_default_chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;
    g_initialized = 1;
}

lrtmp2_chunk_stream_t *lrtmp2_chunk_stream_get(uint32_t csid)
{
    if (!g_initialized) lrtmp2_chunk_streams_init();

    /* Find existing or allocate new */
    for (int i = 0; i < MAX_CHUNK_STREAMS; i++) {
        if (g_streams[i].csid == csid && g_streams[i].in_use) {
            return &g_streams[i];
        }
    }

    /* Allocate a new slot */
    for (int i = 0; i < MAX_CHUNK_STREAMS; i++) {
        if (!g_streams[i].in_use) {
            memset(&g_streams[i], 0, sizeof(g_streams[i]));
            g_streams[i].csid = csid;
            g_streams[i].in_use = 1;
            g_streams[i].chunk_size = g_default_chunk_size;
            LRTMP2_LOG_DEBUG("Allocated chunk stream csid=%u (slot %d)", csid, i);
            return &g_streams[i];
        }
    }

    LRTMP2_LOG_ERROR("No free chunk stream slots for csid=%u", csid);
    return NULL;
}

void lrtmp2_chunk_stream_set_all_chunk_size(uint32_t chunk_size)
{
    if (!g_initialized) lrtmp2_chunk_streams_init();
    /* Apply to existing streams and remember it for streams created later. */
    g_default_chunk_size = chunk_size;
    for (int i = 0; i < MAX_CHUNK_STREAMS; i++) {
        if (g_streams[i].in_use) {
            g_streams[i].chunk_size = chunk_size;
        }
    }
}

void lrtmp2_chunk_streams_destroy(void)
{
    for (int i = 0; i < MAX_CHUNK_STREAMS; i++) {
        if (g_streams[i].reassembly_buf) {
            lrtmp2_buffer_destroy(g_streams[i].reassembly_buf);
            g_streams[i].reassembly_buf = NULL;
        }
        g_streams[i].in_use = 0;
    }
    g_initialized = 0;
}

void lrtmp2_chunk_stream_reset(lrtmp2_chunk_stream_t *stream)
{
    if (!stream) return;
    /* Keep the reassembly buffer allocated for reuse; just clear its contents
     * and reset the rest of the stream state. */
    lrtmp2_buffer_t *buf = stream->reassembly_buf;
    memset(stream, 0, sizeof(*stream));
    stream->reassembly_buf = buf;
    if (buf) lrtmp2_buffer_reset(buf);
    stream->chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;
}
