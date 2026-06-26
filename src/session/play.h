#ifndef LRTMP2_SESSION_PLAY_H
#define LRTMP2_SESSION_PLAY_H

#include "session/conn.h"
#include "librtmp2/types.h"

int lrtmp2_play_begin(lrtmp2_conn_t *conn, const char *stream_name);

#endif
