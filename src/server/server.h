#ifndef LRTMP2_SERVER_SERVER_H
#define LRTMP2_SERVER_SERVER_H

#include "session/conn.h"
#include "session/stream.h"
#include "librtmp2/types.h"

/* Forward declarations for server struct members */
struct lrtmp2_server;

typedef struct lrtmp2_server {
    lrtmp2_server_config_t *config;
    int                     running;
    int                     server_fd;
    struct lrtmp2_conn     *connections;
    lrtmp2_stream_t         *streams;
    pthread_mutex_t        connections_mutex;
    pthread_mutex_t        streams_mutex;
} lrtmp2_server_t;

lrtmp2_server_t *lrtmp2_server_create(const lrtmp2_server_config_t *config); /* config is stored by pointer */
void lrtmp2_server_destroy(lrtmp2_server_t *server);
int  lrtmp2_server_listen(lrtmp2_server_t *server, const char *bind_addr);
int  lrtmp2_server_poll(lrtmp2_server_t *server, int timeout_ms);
void lrtmp2_server_stop(lrtmp2_server_t *server);
void lrtmp2_stream_append_to_server(lrtmp2_server_t *server, lrtmp2_stream_t *stream);

#endif
