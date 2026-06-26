#ifndef LRTMP2_CLIENT_CLIENT_H
#define LRTMP2_CLIENT_CLIENT_H

#include "handshake/handshake.h"
#include "session/conn.h"
#include "core/buffer.h"
#include "librtmp2/types.h"

typedef enum {
    LRTMP2_CLIENT_DISCONNECTED = 0,
    LRTMP2_CLIENT_HANDSHAKING,
    LRTMP2_CLIENT_CONNECTED,
    LRTMP2_CLIENT_APP_CONNECTED,
    LRTMP2_CLIENT_STREAM_CREATED,
    LRTMP2_CLIENT_PUBLISHING,
    LRTMP2_CLIENT_PLAYING,
} lrtmp2_client_state_t;

typedef struct lrtmp2_client {
    int client_fd;
    lrtmp2_handshake_t handshake;
    lrtmp2_client_state_t state;
    lrtmp2_buffer_t *send_buffer;
    lrtmp2_buffer_t *recv_buffer;
    uint32_t peer_chunk_size;
    uint32_t stream_id;
    char app[256];
    char stream_key[256];
    const lrtmp2_server_config_t *config; /* borrowed: used for on_frame_cb during play */
} lrtmp2_client_t;

lrtmp2_client_t *lrtmp2_client_create(const lrtmp2_server_config_t *config);
void lrtmp2_client_destroy(lrtmp2_client_t *client);
int  lrtmp2_client_connect(lrtmp2_client_t *client, const char *url);

/* Phase 2: app-level command flow, run after lrtmp2_client_connect() */
int lrtmp2_client_publish(lrtmp2_client_t *client);
int lrtmp2_client_play(lrtmp2_client_t *client);

/* Send one audio/video frame while publishing */
int lrtmp2_client_send_frame(lrtmp2_client_t *client, const lrtmp2_frame_t *frame);

/* Pump incoming data while playing: delivers frames via config->on_frame_cb.
 * Blocks for up to timeout_ms waiting for data; returns LRTMP2_OK on success
 * (including a timeout with no data), or a negative error code on I/O/protocol
 * failure or peer disconnect. */
int lrtmp2_client_poll(lrtmp2_client_t *client, int timeout_ms);

#endif
