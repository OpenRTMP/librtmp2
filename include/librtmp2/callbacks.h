/**
 * Callback types for librtmp2
 */
#ifndef LRTMP2_CALLBACKS_H
#define LRTMP2_CALLBACKS_H

#include "types.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef int  (*lrtmp2_on_connect_cb)(lrtmp2_conn_t *conn, void *userdata);
typedef int  (*lrtmp2_on_publish_cb)(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata);
typedef int  (*lrtmp2_on_play_cb)(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata);
typedef int  (*lrtmp2_on_frame_cb)(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *userdata);
typedef void (*lrtmp2_on_close_cb)(lrtmp2_conn_t *conn, void *userdata);

#ifdef __cplusplus
}
#endif

#endif /* LRTMP2_CALLBACKS_H */
