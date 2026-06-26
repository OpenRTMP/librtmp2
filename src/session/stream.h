#ifndef LRTMP2_SESSION_STREAM_H
#define LRTMP2_SESSION_STREAM_H

#include <stdint.h>
#include "librtmp2/types.h"

struct lrtmp2_conn;
struct lrtmp2_server;

typedef struct lrtmp2_stream {
    uint32_t       stream_id;
    struct lrtmp2_conn *conn;
    struct lrtmp2_server *server;
    int            is_publishing;
    int            is_playing;
    /* Linked list for server's stream list */
    struct lrtmp2_stream *next;
} lrtmp2_stream_t;

lrtmp2_stream_t *lrtmp2_stream_create(struct lrtmp2_conn *conn, uint32_t stream_id);
void lrtmp2_stream_destroy(lrtmp2_stream_t *stream);

#endif
