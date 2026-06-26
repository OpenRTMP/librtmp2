/**
 * play.c — Play flow
 */
#include "session/play.h"
#include "core/log.h"
#include "librtmp2/types.h"

int lrtmp2_play_begin(lrtmp2_conn_t *conn, const char *stream_name)
{
    if (!conn) return LRTMP2_ERR_INTERNAL;
    LRTMP2_LOG_INFO("Begin play: %s", stream_name);
    return LRTMP2_OK;
}
