#ifndef LRTMP2_SESSION_PUBLISH_H
#define LRTMP2_SESSION_PUBLISH_H

#include "session/stream.h"
#include "librtmp2/types.h"

int lrtmp2_publish_begin(lrtmp2_stream_t *stream, const char *stream_key);

#endif
