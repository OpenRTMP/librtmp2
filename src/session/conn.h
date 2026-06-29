#ifndef LRTMP2_SESSION_CONN_H
#define LRTMP2_SESSION_CONN_H

#include "librtmp2/types.h"
#include "librtmp2/librtmp2.h"
#include "handshake/handshake.h"
#include "chunk/chunk_state.h"
#include "core/transport.h"
#include <stdint.h>
#include <pthread.h>

/** @file Connection lifecycle and message processing */

/* Upper bound on streams a single connection may create via createStream. Each
 * one is heap-allocated and tracked until the connection closes; the cap stops a
 * hostile peer from exhausting memory with endless createStream commands. */
#define LRTMP2_MAX_STREAMS_PER_CONN 16

struct lrtmp2_server;

struct lrtmp2_server_config;  /* defined in include/librtmp2/librtmp2.h */

struct lrtmp2_conn {
    lrtmp2_server_t           *server;
    const lrtmp2_server_config_t *config;
    lrtmp2_conn_state_t        state;
    lrtmp2_handshake_t         handshake;
    lrtmp2_buffer_t           *recv_buffer;
    lrtmp2_buffer_t           *send_buffer;
    lrtmp2_chunk_registry_t    chunk_reg;  /* per-connection chunk-stream state;
                                            * chunk_reg.default_chunk_size tracks
                                            * the peer's negotiated chunk size */
    uint32_t                   chunk_size;
    uint32_t                   window_ack_size;   /* peer's advertised window; we
                                                   * must Acknowledge once we have
                                                   * received this many bytes */
    uint32_t                   bytes_received;    /* running total of bytes fed in
                                                   * (RTMP ack sequence number) */
    uint32_t                   bytes_at_last_ack; /* bytes_received at last ack sent */
    int                        client_fd;
    lrtmp2_transport_t        *transport;  /* wraps client_fd; plaintext or TLS.
                                            * NULL until the socket is attached
                                            * (e.g. unit tests with no socket). */
    char                       app[256];
    uint32_t                   next_stream_id;
    lrtmp2_stream_t           *current_stream;
    int                        connect_cb_fired;  /* on_connect_cb is deferred until
                                                   * any pending TLS handshake completes,
                                                   * so it fires once per connection from
                                                   * lrtmp2_server_process_connections()
                                                   * rather than at accept() time. */
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
