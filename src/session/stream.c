/**
 * stream.c — Stream management within a connection
 */
#include "session/stream.h"
#include "session/conn.h"
#include "server/server.h"
#include "core/log.h"
#include "core/alloc.h"
#include <string.h>
#include <stdlib.h>
#include "librtmp2/types.h"

lrtmp2_stream_t *lrtmp2_stream_create(lrtmp2_conn_t *conn, uint32_t stream_id)
{
    lrtmp2_stream_t *stream = LRTMP2_CALLOC(1, sizeof(lrtmp2_stream_t));
    if (!stream) return NULL;

    stream->stream_id = stream_id;
    stream->conn = conn;
    lrtmp2_stream_append_to_server(conn->server, stream);

    LRTMP2_LOG_INFO("Stream created id=%u", stream_id);
    return stream;
}

void lrtmp2_stream_destroy(lrtmp2_stream_t *stream)
{
    if (!stream) return;
    LRTMP2_LOG_DEBUG("Stream destroyed id=%u", stream->stream_id);
    LRTMP2_FREE(stream);
}
