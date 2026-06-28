#ifndef LRTMP2_CORE_TRANSPORT_H
#define LRTMP2_CORE_TRANSPORT_H

/**
 * transport.h — byte transport abstraction shared by the plaintext (RTMP) and
 * TLS (RTMPS) paths.
 *
 * A transport wraps a connected socket fd and presents a single send/recv API
 * to the chunk/handshake machinery above it, so the session, server and client
 * code never has to know whether the wire is plaintext or TLS.
 *
 * TLS support is compiled in only when LRTMP2_HAVE_TLS is defined (the default
 * build enables it; pass `make TLS=0` / `meson -Dtls=disabled` to drop the
 * OpenSSL dependency). When it is not compiled in, the TLS constructors return
 * NULL and only the plaintext path is available.
 *
 * The transport does NOT own the fd: freeing it tears down any TLS state but
 * leaves the caller to close() the socket, matching the existing ownership in
 * server.c / client.c / conn.c.
 */

#include <stddef.h>
#include <sys/types.h>  /* ssize_t */

typedef struct lrtmp2_transport lrtmp2_transport_t;

/* Server-side TLS context: holds the certificate/key and is shared across every
 * accepted connection. Opaque; created once per server. */
typedef struct lrtmp2_tls_ctx lrtmp2_tls_ctx_t;

/* ---- plaintext ---- */

/* Wrap a connected fd as a plaintext transport. Returns NULL on allocation
 * failure. The fd keeps whatever blocking mode it already had; recv() below
 * uses MSG_DONTWAIT so it is non-blocking regardless. */
lrtmp2_transport_t *lrtmp2_transport_new_plain(int fd);

/* ---- TLS (RTMPS) ---- */

/* Build a server TLS context from PEM cert-chain and private-key files.
 * Returns NULL if TLS is not compiled in, the files cannot be loaded, or the
 * key does not match the certificate. */
lrtmp2_tls_ctx_t *lrtmp2_tls_ctx_new_server(const char *cert_file, const char *key_file);
void              lrtmp2_tls_ctx_free(lrtmp2_tls_ctx_t *ctx);

/* Wrap an accepted fd in a server-side TLS session and run the TLS handshake to
 * completion (blocking). Returns NULL if TLS is unavailable or the handshake
 * fails. On success the fd is left in non-blocking mode for steady-state I/O. */
lrtmp2_transport_t *lrtmp2_transport_new_tls_server(lrtmp2_tls_ctx_t *ctx, int fd);

/* Wrap a connected client fd in a TLS session and run the client handshake
 * (blocking). `server_name` is used for SNI and, unless `insecure` is set,
 * certificate hostname verification. `ca_file` overrides the trust store
 * (NULL = system default). Returns NULL on failure. On success the fd is left
 * in non-blocking mode. */
lrtmp2_transport_t *lrtmp2_transport_new_tls_client(int fd, const char *server_name,
                                                    const char *ca_file, int insecure);

/* ---- I/O ---- */

/* Non-blocking receive. Returns the number of bytes read (>0), 0 on a clean
 * peer shutdown, or -1 on error. On -1, *again indicates a transient
 * would-block the caller should retry after waiting for the right readiness:
 *   1 = wait for the socket to become readable (EAGAIN / TLS WANT_READ)
 *   2 = wait for the socket to become writable (TLS WANT_WRITE during a read)
 *   0 = fatal error.
 * `again` may be NULL. */
ssize_t lrtmp2_transport_recv(lrtmp2_transport_t *t, void *buf, size_t len, int *again);

/* Blocking send of the whole buffer. Returns 0 on success, -1 on fatal error.
 * For TLS it waits out WANT_WRITE internally so callers keep the simple
 * "queue then flush" model they had with plaintext send(). */
int lrtmp2_transport_send(lrtmp2_transport_t *t, const void *buf, size_t len);

int  lrtmp2_transport_fd(const lrtmp2_transport_t *t);
int  lrtmp2_transport_is_tls(const lrtmp2_transport_t *t);

/* Number of bytes already decrypted and buffered inside the transport, ready to
 * be returned by recv() without the socket becoming readable. For TLS this is
 * SSL_pending(); for plaintext it is always 0. Callers that wait on the socket
 * (poll/select) should drain this first, otherwise buffered TLS data can sit
 * unserviced until more network bytes happen to arrive. */
int  lrtmp2_transport_pending(const lrtmp2_transport_t *t);

/* Tear down TLS state (sends close_notify when applicable) and free the
 * transport. Does NOT close the fd. */
void lrtmp2_transport_free(lrtmp2_transport_t *t);

/* 1 if the library was built with TLS support, 0 otherwise. */
int  lrtmp2_tls_available(void);

#endif /* LRTMP2_CORE_TRANSPORT_H */
