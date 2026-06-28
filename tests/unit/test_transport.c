/**
 * test_transport.c — unit tests for the byte-transport abstraction.
 *
 * Covers the plaintext path over a socketpair (round-trip + non-blocking
 * "again" semantics) and the build-flag accessors. The TLS handshake itself is
 * exercised end-to-end by tests/integration/test_tls.c (it needs two real
 * sockets and a certificate); here we only check the graceful-failure and
 * capability surface so the suite runs in both TLS and non-TLS builds.
 */
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <sys/socket.h>
#include <unistd.h>

#include "core/transport.h"
#include "librtmp2/librtmp2.h"

static int g_pass = 0;
static int g_fail = 0;

static void check(int cond, const char *msg) {
    if (cond) { printf("PASS: %s\n", msg); g_pass++; }
    else      { printf("FAIL: %s\n", msg); g_fail++; }
}

static void test_plain_roundtrip(void) {
    int sv[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) != 0) {
        check(0, "socketpair created");
        return;
    }

    lrtmp2_transport_t *a = lrtmp2_transport_new_plain(sv[0]);
    lrtmp2_transport_t *b = lrtmp2_transport_new_plain(sv[1]);
    check(a && b, "plain transports created");
    check(!lrtmp2_transport_is_tls(a), "plain transport reports is_tls=0");
    check(lrtmp2_transport_fd(a) == sv[0], "transport_fd returns wrapped fd");

    /* Empty socket: non-blocking recv reports "again". */
    uint8_t rx[64];
    int again = 0;
    ssize_t n = lrtmp2_transport_recv(b, rx, sizeof(rx), &again);
    check(n < 0 && again == 1, "recv on empty socket signals again");

    /* Send a->b and read it back. */
    const char *payload = "hello-rtmp-transport";
    size_t len = strlen(payload);
    check(lrtmp2_transport_send(a, payload, len) == 0, "transport_send succeeds");

    again = 0;
    n = lrtmp2_transport_recv(b, rx, sizeof(rx), &again);
    check(n == (ssize_t)len && memcmp(rx, payload, len) == 0,
          "transport_recv returns the sent bytes");

    /* Peer close is observed as a clean 0-length read. */
    lrtmp2_transport_free(a);
    close(sv[0]);
    again = 0;
    n = lrtmp2_transport_recv(b, rx, sizeof(rx), &again);
    check(n == 0, "recv after peer close returns 0 (EOF)");

    lrtmp2_transport_free(b);
    close(sv[1]);
}

static void test_capability(void) {
    /* The two capability accessors must agree, and match whether TLS was
     * compiled in. */
    check(lrtmp2_tls_available() == lrtmp2_tls_supported(),
          "tls_available() and tls_supported() agree");
#ifdef LRTMP2_HAVE_TLS
    check(lrtmp2_tls_supported() == 1, "tls reported available in TLS build");
    /* Missing cert/key files fail gracefully (NULL, no crash). */
    check(lrtmp2_tls_ctx_new_server("/nonexistent/cert.pem",
                                    "/nonexistent/key.pem") == NULL,
          "tls_ctx_new_server returns NULL for missing files");
#else
    check(lrtmp2_tls_supported() == 0, "tls reported unavailable in non-TLS build");
    check(lrtmp2_transport_new_tls_client(-1, "host", NULL, 0) == NULL,
          "tls client constructor returns NULL without TLS support");
#endif
}

int test_transport_main(void) {
    printf("Running transport tests...\n");
    g_pass = 0; g_fail = 0;
    test_plain_roundtrip();
    test_capability();
    printf("Transport tests: %d/%d passed\n", g_pass, g_pass + g_fail);
    return g_fail == 0 ? 0 : 1;
}
