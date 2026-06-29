#ifndef LRTMP2_CORE_TRANSPORT_HANDSHAKE_H
#define LRTMP2_CORE_TRANSPORT_HANDSHAKE_H

/**
 * transport_handshake.h — deferred server-side TLS handshake stepping.
 *
 * Internal to the server's poll loop (src/server/server.c) and the transport
 * implementation (src/core/transport.c) only. Kept out of transport.h so these
 * server-loop internals don't read like part of the transport's general
 * send/recv API to any other internal consumer.
 */

#include "core/transport.h"

/* 1 if a server-side TLS transport is still waiting on SSL_accept. */
int lrtmp2_transport_tls_handshake_pending(const lrtmp2_transport_t *t);

/* Drive one non-blocking SSL_accept step. Returns 1 when complete, 0 when more
 * I/O is needed (retry after poll), -1 on failure or handshake timeout. */
int lrtmp2_transport_tls_handshake_advance(lrtmp2_transport_t *t);

/* 1 if the last SSL_accept step returned WANT_WRITE, meaning the poll loop
 * must watch the fd for writability (not just readability) to make progress
 * on the next handshake_advance() call. Only meaningful while
 * tls_handshake_pending() is true. */
int lrtmp2_transport_tls_handshake_wants_write(const lrtmp2_transport_t *t);

#endif /* LRTMP2_CORE_TRANSPORT_HANDSHAKE_H */
