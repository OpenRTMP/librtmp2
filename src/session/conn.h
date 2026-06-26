#ifndef LRTMP2_SESSION_CONN_H
#define LRTMP2_SESSION_CONN_H

#include "librtmp2/types.h"
#include "librtmp2/librtmp2.h"
#include "handshake/handshake.h"
#include "chunk/chunk_state.h"
#include <stdint.h>
#include <pthread.h>

/** @file Connection lifecycle and message processing */
struct lrtmp2_server;

struct lrtmp2_server_config;  /* defined in include/librtmp2/librtmp2.h */

struct lrtmp2_conn {
    lrtmp2_server_t           *server;
    lrtmp2_server_config_t    *config;
    lrtmp2_conn_state_t        state;
    lrtmp2_handshake_t         handshake;
    lrtmp2_buffer_t           *recv_buffer;
    lrtmp2_buffer_t           *send_buffer;
    uint32_t                   chunk_size;
    uint32_t                   peer_chunk_size;
    uint32_t                   window_ack_size;
    int                        client_fd;
    char                       app[256];
    uint32_t                   next_stream_id;
    lrtmp2_stream_t           *current_stream;
    /* Callbacks */
    lrtmp2_on_connect_cb      on_connect_cb;
    lrtmp2_on_publish_cb     on_publish_cb;
    lrtmp2_on_play_cb         on_play_cb;
    lrtmp2_on_frame_cb        on_frame_cb;
    lrtmp2_on_close_cb        on_close_cb;
    int (*on_send_data)(lrtmp2_conn_t *conn, const uint8_t *data, size_t len, void *userdata);
    void                      *userdata;
    pthread_mutex_t            send_mutex;
    struct lrtmp2_conn        *next;  /* linked list for server connections */
};

/* Lifecycle */
lrtmp2_conn_t *lrtmp2_conn_create(lrtmp2_server_t *server, const lrtmp2_server_config_t *config);
void lrtmp2_conn_destroy(lrtmp2_conn_t *conn);

/* Data flow */
int lrtmp2_conn_recv(lrtmp2_conn_t *conn, const uint8_t *data, size_t len);
int lrtmp2_conn_process(lrtmp2_conn_t *conn);
int lrtmp2_conn_do_handshake(lrtmp2_conn_t *conn);
int lrtmp2_conn_read_messages(lrtmp2_conn_t *conn);

/* Command dispatch: decode AMF0 command messages (connect/createStream/publish/play/...) */
int lrtmp2_conn_handle_command(lrtmp2_conn_t *conn, const uint8_t *payload, size_t payload_len);

/* Response helpers */
int lrtmp2_conn_send_connect_response(lrtmp2_conn_t *conn, double transaction_id);
int lrtmp2_conn_send_create_stream_response(lrtmp2_conn_t *conn, double transaction_id, uint32_t stream_id);
int lrtmp2_conn_send_onstatus(lrtmp2_conn_t *conn, uint32_t stream_id, const char *level,
                               const char *code, const char *description);

/* Send raw bytes over the connection socket */
int lrtmp2_conn_send_raw(lrtmp2_conn_t *conn, const uint8_t *data, size_t len);

/* Send the current contents of send_buffer over the socket */
int lrtmp2_conn_flush(lrtmp2_conn_t *conn);

#endif
