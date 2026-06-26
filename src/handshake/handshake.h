#ifndef LRTMP2_HANDSHAKE_H
#define LRTMP2_HANDSHAKE_H

#include "core/buffer.h"
#include "core/alloc.h"
#include "librtmp2/types.h"

typedef enum {
    LRTMP2_HS_SERVER_WAIT_C0 = 0,
    LRTMP2_HS_SERVER_WAIT_C1,
    LRTMP2_HS_SERVER_WAIT_C2,
    LRTMP2_HS_CLIENT_WAIT_S0,
    LRTMP2_HS_CLIENT_WAIT_S1,
    LRTMP2_HS_CLIENT_WAIT_S2,
    LRTMP2_HS_DONE,
} lrtmp2_handshake_state_t;

typedef struct {
    lrtmp2_handshake_state_t state;
    uint8_t  version;
    uint32_t peer_time;
    lrtmp2_buffer_t out;  /* queued output bytes */
} lrtmp2_handshake_t;

/* Server-side */
int lrtmp2_handshake_server_init(lrtmp2_handshake_t *hs);
int lrtmp2_handshake_server_read_c0(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf);
int lrtmp2_handshake_server_read_c1(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf);
int lrtmp2_handshake_server_read_c2(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf);

/* Client-side */
int lrtmp2_handshake_client_init(lrtmp2_handshake_t *hs);
int lrtmp2_handshake_client_generate_c0c1(lrtmp2_handshake_t *hs);
int lrtmp2_handshake_client_read_s0(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf);
int lrtmp2_handshake_client_read_s1(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf);
int lrtmp2_handshake_client_read_s2(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf);

static inline int lrtmp2_handshake_complete(const lrtmp2_handshake_t *hs) {
    return hs->state == LRTMP2_HS_DONE;
}

#endif
