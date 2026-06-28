/**
 * transport.c — plaintext + optional TLS byte transport.
 *
 * The plaintext path is always available. The TLS path is compiled in when
 * LRTMP2_HAVE_TLS is defined and links against OpenSSL.
 */
/* Expose POSIX socket APIs (MSG_DONTWAIT, fcntl flags) under a strict -std=c11,
 * where glibc otherwise hides them. Must precede all includes. */
#ifndef _DEFAULT_SOURCE
#define _DEFAULT_SOURCE 1
#endif

#include "core/transport.h"
#include "core/alloc.h"
#include "core/log.h"

#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdint.h>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/select.h>

#ifdef LRTMP2_HAVE_TLS
#include <openssl/ssl.h>
#include <openssl/err.h>
#include <openssl/x509v3.h>
#endif

struct lrtmp2_transport {
    int  fd;
    int  is_tls;
#ifdef LRTMP2_HAVE_TLS
    SSL *ssl;  /* NULL for plaintext */
#endif
};

struct lrtmp2_tls_ctx {
#ifdef LRTMP2_HAVE_TLS
    SSL_CTX *ctx;
#else
    int unused;
#endif
};

int lrtmp2_tls_available(void)
{
#ifdef LRTMP2_HAVE_TLS
    return 1;
#else
    return 0;
#endif
}

#ifdef LRTMP2_HAVE_TLS
static int set_nonblocking(int fd)
{
    int flags = fcntl(fd, F_GETFL, 0);
    if (flags < 0) return -1;
    return fcntl(fd, F_SETFL, flags | O_NONBLOCK);
}

static int set_blocking(int fd)
{
    int flags = fcntl(fd, F_GETFL, 0);
    if (flags < 0) return -1;
    return fcntl(fd, F_SETFL, flags & ~O_NONBLOCK);
}
#endif

/* ---------------- plaintext ---------------- */

lrtmp2_transport_t *lrtmp2_transport_new_plain(int fd)
{
    lrtmp2_transport_t *t = LRTMP2_CALLOC(1, sizeof(*t));
    if (!t) return NULL;
    t->fd = fd;
    t->is_tls = 0;
    return t;
}

static ssize_t plain_recv(lrtmp2_transport_t *t, void *buf, size_t len, int *again)
{
    ssize_t n = recv(t->fd, buf, len, MSG_DONTWAIT);
    if (n < 0 && again && (errno == EAGAIN || errno == EWOULDBLOCK)) {
        *again = 1;
    }
    return n;
}

static int plain_send(lrtmp2_transport_t *t, const void *buf, size_t len)
{
    const char *p = buf;
    size_t sent = 0;
    while (sent < len) {
        ssize_t n = send(t->fd, p + sent, len - sent, 0);
        if (n <= 0) {
            if (n < 0 && errno == EINTR) continue;
            return -1;
        }
        sent += (size_t)n;
    }
    return 0;
}

/* ---------------- TLS ---------------- */

#ifdef LRTMP2_HAVE_TLS

static void log_ssl_errors(const char *what)
{
    unsigned long e;
    char ebuf[256];
    while ((e = ERR_get_error()) != 0) {
        ERR_error_string_n(e, ebuf, sizeof(ebuf));
        LRTMP2_LOG_ERROR("TLS %s: %s", what, ebuf);
    }
}

lrtmp2_tls_ctx_t *lrtmp2_tls_ctx_new_server(const char *cert_file, const char *key_file)
{
    if (!cert_file || !key_file) {
        LRTMP2_LOG_ERROR("TLS server: cert_file and key_file are required");
        return NULL;
    }

    lrtmp2_tls_ctx_t *c = LRTMP2_CALLOC(1, sizeof(*c));
    if (!c) return NULL;

    c->ctx = SSL_CTX_new(TLS_server_method());
    if (!c->ctx) {
        log_ssl_errors("SSL_CTX_new");
        LRTMP2_FREE(c);
        return NULL;
    }

    /* RTMPS in practice rides on TLS 1.2+. */
    SSL_CTX_set_min_proto_version(c->ctx, TLS1_2_VERSION);

    if (SSL_CTX_use_certificate_chain_file(c->ctx, cert_file) != 1) {
        log_ssl_errors("use_certificate_chain_file");
        LRTMP2_LOG_ERROR("TLS server: failed to load certificate '%s'", cert_file);
        SSL_CTX_free(c->ctx);
        LRTMP2_FREE(c);
        return NULL;
    }
    if (SSL_CTX_use_PrivateKey_file(c->ctx, key_file, SSL_FILETYPE_PEM) != 1) {
        log_ssl_errors("use_PrivateKey_file");
        LRTMP2_LOG_ERROR("TLS server: failed to load private key '%s'", key_file);
        SSL_CTX_free(c->ctx);
        LRTMP2_FREE(c);
        return NULL;
    }
    if (SSL_CTX_check_private_key(c->ctx) != 1) {
        log_ssl_errors("check_private_key");
        LRTMP2_LOG_ERROR("TLS server: private key does not match certificate");
        SSL_CTX_free(c->ctx);
        LRTMP2_FREE(c);
        return NULL;
    }

    LRTMP2_LOG_INFO("TLS server context loaded (cert=%s)", cert_file);
    return c;
}

void lrtmp2_tls_ctx_free(lrtmp2_tls_ctx_t *ctx)
{
    if (!ctx) return;
    if (ctx->ctx) SSL_CTX_free(ctx->ctx);
    LRTMP2_FREE(ctx);
}

/* Drive a blocking TLS handshake on an otherwise non-blocking-capable fd. The
 * fd is switched to blocking for the duration so SSL_accept/SSL_connect run to
 * completion in one call, then flipped to non-blocking for steady state. */
static int tls_handshake_blocking(SSL *ssl, int fd, int is_server)
{
    if (set_blocking(fd) != 0) return -1;
    int rc = is_server ? SSL_accept(ssl) : SSL_connect(ssl);
    if (rc != 1) {
        log_ssl_errors(is_server ? "SSL_accept" : "SSL_connect");
        return -1;
    }
    if (set_nonblocking(fd) != 0) return -1;
    return 0;
}

lrtmp2_transport_t *lrtmp2_transport_new_tls_server(lrtmp2_tls_ctx_t *ctx, int fd)
{
    if (!ctx || !ctx->ctx) return NULL;

    lrtmp2_transport_t *t = LRTMP2_CALLOC(1, sizeof(*t));
    if (!t) return NULL;
    t->fd = fd;
    t->is_tls = 1;

    t->ssl = SSL_new(ctx->ctx);
    if (!t->ssl) {
        log_ssl_errors("SSL_new");
        LRTMP2_FREE(t);
        return NULL;
    }
    SSL_set_fd(t->ssl, fd);

    if (tls_handshake_blocking(t->ssl, fd, 1) != 0) {
        LRTMP2_LOG_WARN("TLS server handshake failed");
        SSL_free(t->ssl);
        LRTMP2_FREE(t);
        return NULL;
    }

    LRTMP2_LOG_INFO("TLS server handshake complete (%s)", SSL_get_version(t->ssl));
    return t;
}

lrtmp2_transport_t *lrtmp2_transport_new_tls_client(int fd, const char *server_name,
                                                    const char *ca_file, int insecure)
{
    SSL_CTX *cctx = SSL_CTX_new(TLS_client_method());
    if (!cctx) {
        log_ssl_errors("SSL_CTX_new(client)");
        return NULL;
    }
    SSL_CTX_set_min_proto_version(cctx, TLS1_2_VERSION);

    if (!insecure) {
        if (ca_file) {
            if (SSL_CTX_load_verify_locations(cctx, ca_file, NULL) != 1) {
                log_ssl_errors("load_verify_locations");
                LRTMP2_LOG_ERROR("TLS client: failed to load CA file '%s'", ca_file);
                SSL_CTX_free(cctx);
                return NULL;
            }
        } else {
            SSL_CTX_set_default_verify_paths(cctx);
        }
        SSL_CTX_set_verify(cctx, SSL_VERIFY_PEER, NULL);
    }

    lrtmp2_transport_t *t = LRTMP2_CALLOC(1, sizeof(*t));
    if (!t) { SSL_CTX_free(cctx); return NULL; }
    t->fd = fd;
    t->is_tls = 1;

    t->ssl = SSL_new(cctx);
    /* The SSL object holds its own ref to the context, so we can drop ours; the
     * context is freed once the SSL is freed. */
    SSL_CTX_free(cctx);
    if (!t->ssl) {
        log_ssl_errors("SSL_new(client)");
        LRTMP2_FREE(t);
        return NULL;
    }
    SSL_set_fd(t->ssl, fd);

    if (server_name && *server_name) {
        /* SNI */
        SSL_set_tlsext_host_name(t->ssl, server_name);
        if (!insecure) {
            /* Hostname verification against the certificate. */
            SSL_set_hostflags(t->ssl, X509_CHECK_FLAG_NO_PARTIAL_WILDCARDS);
            if (SSL_set1_host(t->ssl, server_name) != 1) {
                log_ssl_errors("SSL_set1_host");
                SSL_free(t->ssl);
                LRTMP2_FREE(t);
                return NULL;
            }
        }
    }

    if (tls_handshake_blocking(t->ssl, fd, 0) != 0) {
        LRTMP2_LOG_ERROR("TLS client handshake failed");
        SSL_free(t->ssl);
        LRTMP2_FREE(t);
        return NULL;
    }

    LRTMP2_LOG_INFO("TLS client handshake complete (%s, cipher=%s)",
                    SSL_get_version(t->ssl), SSL_get_cipher(t->ssl));
    return t;
}

static ssize_t tls_recv(lrtmp2_transport_t *t, void *buf, size_t len, int *again)
{
    ERR_clear_error();
    int n = SSL_read(t->ssl, buf, (int)(len > INT_MAX ? INT_MAX : len));
    if (n > 0) return n;

    int err = SSL_get_error(t->ssl, n);
    switch (err) {
        case SSL_ERROR_ZERO_RETURN:
            return 0;  /* clean TLS shutdown */
        case SSL_ERROR_WANT_READ:
        case SSL_ERROR_WANT_WRITE:
            if (again) *again = 1;
            errno = EAGAIN;
            return -1;
        case SSL_ERROR_SYSCALL:
            if (n == 0) return 0;  /* unexpected EOF, treat as closed */
            return -1;
        default:
            log_ssl_errors("SSL_read");
            return -1;
    }
}

/* Block until the fd is writable (or readable, for a renegotiation), used to
 * wait out WANT_WRITE/WANT_READ during a blocking send. */
static int wait_fd(int fd, int for_write)
{
    fd_set fds;
    FD_ZERO(&fds);
    FD_SET(fd, &fds);
    int rc = for_write ? select(fd + 1, NULL, &fds, NULL, NULL)
                       : select(fd + 1, &fds, NULL, NULL, NULL);
    return (rc > 0) ? 0 : -1;
}

static int tls_send(lrtmp2_transport_t *t, const void *buf, size_t len)
{
    const char *p = buf;
    size_t sent = 0;
    while (sent < len) {
        size_t chunk = len - sent;
        if (chunk > INT_MAX) chunk = INT_MAX;
        ERR_clear_error();
        int n = SSL_write(t->ssl, p + sent, (int)chunk);
        if (n > 0) { sent += (size_t)n; continue; }

        int err = SSL_get_error(t->ssl, n);
        if (err == SSL_ERROR_WANT_WRITE) {
            if (wait_fd(t->fd, 1) != 0) return -1;
            continue;
        }
        if (err == SSL_ERROR_WANT_READ) {
            if (wait_fd(t->fd, 0) != 0) return -1;
            continue;
        }
        log_ssl_errors("SSL_write");
        return -1;
    }
    return 0;
}

#endif /* LRTMP2_HAVE_TLS */

/* ---------------- dispatch ---------------- */

ssize_t lrtmp2_transport_recv(lrtmp2_transport_t *t, void *buf, size_t len, int *again)
{
    if (again) *again = 0;
    if (!t) return -1;
#ifdef LRTMP2_HAVE_TLS
    if (t->is_tls) return tls_recv(t, buf, len, again);
#endif
    return plain_recv(t, buf, len, again);
}

int lrtmp2_transport_send(lrtmp2_transport_t *t, const void *buf, size_t len)
{
    if (!t) return -1;
    if (len == 0) return 0;
#ifdef LRTMP2_HAVE_TLS
    if (t->is_tls) return tls_send(t, buf, len);
#endif
    return plain_send(t, buf, len);
}

int lrtmp2_transport_fd(const lrtmp2_transport_t *t)
{
    return t ? t->fd : -1;
}

int lrtmp2_transport_is_tls(const lrtmp2_transport_t *t)
{
    return t ? t->is_tls : 0;
}

void lrtmp2_transport_free(lrtmp2_transport_t *t)
{
    if (!t) return;
#ifdef LRTMP2_HAVE_TLS
    if (t->ssl) {
        /* Best-effort close_notify; ignore WANT_* since we are tearing down. */
        SSL_shutdown(t->ssl);
        SSL_free(t->ssl);
    }
#endif
    LRTMP2_FREE(t);
}

/* ---- stubs when TLS is not compiled in ---- */
#ifndef LRTMP2_HAVE_TLS
lrtmp2_tls_ctx_t *lrtmp2_tls_ctx_new_server(const char *cert_file, const char *key_file)
{
    (void)cert_file; (void)key_file;
    LRTMP2_LOG_ERROR("TLS requested but librtmp2 was built without TLS support "
                     "(rebuild with TLS=1 / meson -Dtls=enabled)");
    return NULL;
}
void lrtmp2_tls_ctx_free(lrtmp2_tls_ctx_t *ctx) { (void)ctx; }
lrtmp2_transport_t *lrtmp2_transport_new_tls_server(lrtmp2_tls_ctx_t *ctx, int fd)
{
    (void)ctx; (void)fd;
    return NULL;
}
lrtmp2_transport_t *lrtmp2_transport_new_tls_client(int fd, const char *server_name,
                                                    const char *ca_file, int insecure)
{
    (void)fd; (void)server_name; (void)ca_file; (void)insecure;
    LRTMP2_LOG_ERROR("rtmps:// requested but librtmp2 was built without TLS support");
    return NULL;
}
#endif
