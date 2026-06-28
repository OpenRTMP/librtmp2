/**
 * test_tls.c — Integration test: RTMPS (TLS) client <-> server over loopback TCP
 *
 * Mirrors test_client_publish.c but with TLS termination on the server and an
 * rtmps:// client. A throwaway self-signed certificate is generated at runtime
 * (no checked-in key material, nothing to expire), the server is configured for
 * TLS, and a real client connects over rtmps://, publishes one video frame, and
 * the frame is checked to have round-tripped through the encrypted transport.
 *
 * Built only when the library has TLS support (LRTMP2_HAVE_TLS); otherwise main
 * reports the test as skipped so the suite still passes a plaintext-only build.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>
#include <time.h>

#include "server/server.h"
#include "session/conn.h"
#include "client/client.h"
#include "core/log.h"

#ifndef LRTMP2_HAVE_TLS
int main(void) {
    printf("=== librtmp2 integration: RTMPS ===\n\n");
    printf("SKIP: built without TLS support (LRTMP2_HAVE_TLS undefined)\n");
    return 0;
}
#else

#include <openssl/evp.h>
#include <openssl/rsa.h>
#include <openssl/x509.h>
#include <openssl/pem.h>

#define TEST_PORT 19371

static const char *CERT_PATH = "/tmp/lrtmp2_test_cert.pem";
static const char *KEY_PATH  = "/tmp/lrtmp2_test_key.pem";

static int g_frame_received = 0;
static int g_publish_called = 0;
static size_t g_received_len = 0;
static uint8_t g_received_data[4096];

static int on_frame_cb(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *userdata) {
    (void)conn; (void)userdata;
    if (frame->size > 0 && frame->size <= sizeof(g_received_data)) {
        memcpy(g_received_data, frame->data, frame->size);
        g_received_len = frame->size;
        g_frame_received = 1;
    }
    return 0;
}

static int on_publish_cb(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata) {
    (void)conn; (void)app; (void)stream_key; (void)userdata;
    g_publish_called = 1;
    return 0;
}

/* Generate a self-signed RSA certificate and write the cert/key PEM files. */
static int generate_self_signed(const char *cert_path, const char *key_path) {
    int ok = 0;
    EVP_PKEY *pkey = EVP_RSA_gen(2048);
    if (!pkey) return -1;

    X509 *x509 = X509_new();
    if (!x509) { EVP_PKEY_free(pkey); return -1; }

    ASN1_INTEGER_set(X509_get_serialNumber(x509), 1);
    X509_gmtime_adj(X509_getm_notBefore(x509), 0);
    X509_gmtime_adj(X509_getm_notAfter(x509), 60L * 60L * 24L);  /* 1 day */
    X509_set_pubkey(x509, pkey);

    X509_NAME *name = X509_get_subject_name(x509);
    X509_NAME_add_entry_by_txt(name, "CN", MBSTRING_ASC,
                               (const unsigned char *)"localhost", -1, -1, 0);
    X509_set_issuer_name(x509, name);  /* self-signed */

    if (!X509_sign(x509, pkey, EVP_sha256())) goto done;

    FILE *kf = fopen(key_path, "wb");
    if (!kf) goto done;
    PEM_write_PrivateKey(kf, pkey, NULL, NULL, 0, NULL, NULL);
    fclose(kf);

    FILE *cf = fopen(cert_path, "wb");
    if (!cf) goto done;
    PEM_write_X509(cf, x509);
    fclose(cf);

    ok = 1;
done:
    X509_free(x509);
    EVP_PKEY_free(pkey);
    return ok ? 0 : -1;
}

static void *client_thread_fn(void *arg) {
    (void)arg;

    lrtmp2_server_config_t config;
    memset(&config, 0, sizeof(config));
    config.tls_insecure = 1;  /* accept the throwaway self-signed cert */

    lrtmp2_client_t *client = lrtmp2_client_create(&config);
    if (!client) return NULL;

    char url[64];
    snprintf(url, sizeof(url), "rtmps://127.0.0.1:%d/live/mystream", TEST_PORT);

    if (lrtmp2_client_connect(client, url) != 0) {
        fprintf(stderr, "client: rtmps connect failed\n");
        lrtmp2_client_destroy(client);
        return NULL;
    }
    if (lrtmp2_client_publish(client) != 0) {
        fprintf(stderr, "client: publish failed\n");
        lrtmp2_client_destroy(client);
        return NULL;
    }

    uint8_t video_data[300];
    size_t i = 0;
    video_data[i++] = 0x17;
    video_data[i++] = 0x01;
    video_data[i++] = 0x00; video_data[i++] = 0x00; video_data[i++] = 0x00;
    for (; i < sizeof(video_data); i++) video_data[i] = (uint8_t)(i & 0xFF);

    lrtmp2_frame_t frame;
    memset(&frame, 0, sizeof(frame));
    frame.type = LRTMP2_FRAME_VIDEO;
    frame.timestamp = 0;
    frame.size = sizeof(video_data);
    frame.data = video_data;

    if (lrtmp2_client_send_frame(client, &frame) != 0) {
        fprintf(stderr, "client: send_frame failed\n");
    }

    usleep(200 * 1000);
    lrtmp2_client_destroy(client);
    return NULL;
}

int main(void) {
    printf("=== librtmp2 integration: RTMPS (TLS) client <-> server ===\n\n");

    if (generate_self_signed(CERT_PATH, KEY_PATH) != 0) {
        printf("FAIL: could not generate self-signed certificate\n");
        return 1;
    }

    lrtmp2_server_config_t config;
    memset(&config, 0, sizeof(config));
    config.max_connections = 10;
    config.chunk_size = 4096;
    config.on_frame_cb = on_frame_cb;
    config.on_publish_cb = on_publish_cb;
    config.tls_enabled = 1;
    config.tls_cert_file = CERT_PATH;
    config.tls_key_file = KEY_PATH;

    lrtmp2_server_t *server = lrtmp2_server_create(&config);
    if (!server) { printf("FAIL: server_create (TLS)\n"); return 1; }

    char bind_addr[64];
    snprintf(bind_addr, sizeof(bind_addr), "127.0.0.1:%d", TEST_PORT);
    if (lrtmp2_server_listen(server, bind_addr) != 0) {
        printf("FAIL: server_listen\n");
        lrtmp2_server_destroy(server);
        return 1;
    }

    pthread_t client_thread;
    pthread_create(&client_thread, NULL, client_thread_fn, NULL);

    struct timespec start, now;
    clock_gettime(CLOCK_MONOTONIC, &start);
    int accepted = 0;
    for (;;) {
        if (!accepted) {
            if (lrtmp2_server_poll(server, 50) == LRTMP2_OK) accepted = 1;
        } else {
            lrtmp2_server_process_connections(server);
        }
        if (g_frame_received) break;

        clock_gettime(CLOCK_MONOTONIC, &now);
        double elapsed = (now.tv_sec - start.tv_sec) + (now.tv_nsec - start.tv_nsec) / 1e9;
        if (elapsed > 8.0) { printf("FAIL: timed out waiting for frame over TLS\n"); break; }
        usleep(10 * 1000);
    }

    pthread_join(client_thread, NULL);

    int success = (g_publish_called && g_frame_received &&
                   g_received_len == 300 &&
                   memcmp(g_received_data, "\x17\x01\x00\x00\x00", 5) == 0);
    if (success) {
        printf("PASS: RTMPS client published and frame round-tripped over TLS (len=%zu)\n",
               g_received_len);
    } else {
        printf("FAIL: publish_called=%d frame_received=%d len=%zu\n",
               g_publish_called, g_frame_received, g_received_len);
    }

    lrtmp2_server_destroy(server);
    unlink(CERT_PATH);
    unlink(KEY_PATH);
    return success ? 0 : 1;
}

#endif /* LRTMP2_HAVE_TLS */
