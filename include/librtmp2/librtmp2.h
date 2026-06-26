/**
 * librtmp2.h — Main public header for librtmp2
 *
 * Include this to use the full library API.
 */
#ifndef LRTMP2_H
#define LRTMP2_H

#include "version.h"
#include "types.h"
#include "errors.h"
#include "callbacks.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ==================== Server API ==================== */

typedef struct lrtmp2_server_config {
    int max_connections;
    int chunk_size;
    /* Callbacks */
    lrtmp2_on_connect_cb   on_connect_cb;
    lrtmp2_on_publish_cb  on_publish_cb;
    lrtmp2_on_play_cb      on_play_cb;
    lrtmp2_on_frame_cb     on_frame_cb;
    lrtmp2_on_close_cb     on_close_cb;
    int (*on_send_data)(struct lrtmp2_conn *conn, const uint8_t *data, size_t len, void *userdata);
    void *userdata;
} lrtmp2_server_config_t;

lrtmp2_server_t *lrtmp2_server_create(const lrtmp2_server_config_t *config);
void             lrtmp2_server_destroy(lrtmp2_server_t *server);
int              lrtmp2_server_listen(lrtmp2_server_t *server, const char *bind_addr);
int              lrtmp2_server_poll(lrtmp2_server_t *server, int timeout_ms);
void             lrtmp2_server_stop(lrtmp2_server_t *server);

/* ==================== Client API ==================== */

lrtmp2_client_t *lrtmp2_client_create(const lrtmp2_server_config_t *config);
void             lrtmp2_client_destroy(lrtmp2_client_t *client);
int              lrtmp2_client_connect(lrtmp2_client_t *client, const char *url);

/* ==================== Frame API ==================== */

/* Frames are delivered via on_frame_cb callback. See librtmp2/types.h for lrtmp2_frame_t. */

/* ==================== Utility ==================== */

const char *lrtmp2_version_string(void);
int lrtmp2_version_major(void);
int lrtmp2_version_minor(void);
int lrtmp2_version_patch(void);

#ifdef __cplusplus
}
#endif

#endif /* LRTMP2_H */
