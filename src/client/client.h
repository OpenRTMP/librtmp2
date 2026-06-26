#ifndef LRTMP2_CLIENT_CLIENT_H
#define LRTMP2_CLIENT_CLIENT_H

#include "handshake/handshake.h"
#include "session/conn.h"
#include "librtmp2/types.h"

typedef struct lrtmp2_client {
    int client_fd;
    lrtmp2_handshake_t handshake;
} lrtmp2_client_t;

lrtmp2_client_t *lrtmp2_client_create(lrtmp2_server_config_t *config);
void lrtmp2_client_destroy(lrtmp2_client_t *client);
int  lrtmp2_client_connect(lrtmp2_client_t *client, const char *url);

#endif
