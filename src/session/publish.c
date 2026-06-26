/**
 * publish.c — Publish flow
 */
#include "session/publish.h"
#include "session/stream.h"
#include "core/log.h"
#include "librtmp2/types.h"

int lrtmp2_publish_begin(lrtmp2_stream_t *stream, const char *stream_key)
{
    if (!stream) return LRTMP2_ERR_INTERNAL;
    LRTMP2_LOG_INFO("Begin publish: stream=%u key=%s", stream->stream_id, stream_key);
    stream->is_publishing = 1;
    return LRTMP2_OK;
}
