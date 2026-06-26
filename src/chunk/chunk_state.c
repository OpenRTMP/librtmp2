/**
 * chunk_state.c — Per-chunk-stream state
 */
#include "chunk_state.h"
#include "core/log.h"
#include <string.h>

#define MAX_CHUNK_STREAMS 8

static lrtmp2_chunk_stream_t g_streams[MAX_CHUNK_STREAMS];
static int g_initialized = 0;

void lrtmp2_chunk_streams_init(void)
{
    memset(g_streams, 0, sizeof(g_streams));
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
            g_streams[i].chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;
            LRTMP2_LOG_DEBUG("Allocated chunk stream csid=%u (slot %d)", csid, i);
            return &g_streams[i];
        }
    }

    LRTMP2_LOG_ERROR("No free chunk stream slots for csid=%u", csid);
    return NULL;
}

void lrtmp2_chunk_stream_reset(lrtmp2_chunk_stream_t *stream)
{
    if (!stream) return;
    if (stream->reassembly_buf) {
        lrtmp2_buffer_destroy(stream->reassembly_buf);
    }
    lrtmp2_buffer_t *buf = stream->reassembly_buf;  /* save pointer */
    memset(stream, 0, sizeof(*stream));
    stream->reassembly_buf = buf;  /* keep the buffer allocated for reuse */
    stream->chunk_size = LRTMP2_DEFAULT_CHUNK_SIZE;
}
