/**
 * librtmp2.h — Main public header for librtmp2
 *
 * Include this to use the full library API.
 */
#ifndef LRTMP2_H
#define LRTMP2_H

#include "version.h"
#include "types.h"
#include "errors.h"
#include "callbacks.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ==================== Server API ==================== */

struct lrtmp2_server_config {
    int max_connections;
    int chunk_size;
    /* Callbacks */
    lrtmp2_on_connect_cb   on_connect_cb;
    lrtmp2_on_publish_cb  on_publish_cb;
    lrtmp2_on_play_cb      on_play_cb;
    lrtmp2_on_frame_cb     on_frame_cb;
    lrtmp2_on_close_cb     on_close_cb;
    int (*on_send_data)(struct lrtmp2_conn *conn, const uint8_t *data, size_t len, void *userdata);
    void *userdata;

    /* ---- TLS / RTMPS (optional) ----
     *
     * Server side: set tls_enabled = 1 and provide PEM cert-chain and private
     * key files to terminate TLS on every accepted connection (rtmps://). When
     * tls_enabled = 0 (default), the server speaks plaintext RTMP exactly as
     * before. If the library was built without TLS support, enabling this makes
     * lrtmp2_server_listen() fail.
     *
     * Client side: rtmps:// URLs trigger TLS automatically; these fields tune
     * verification. tls_ca_file overrides the system trust store (NULL = system
     * default) and tls_insecure = 1 skips certificate/hostname verification
     * (test/self-signed use only). */
    int         tls_enabled;       /* server: terminate TLS on accepted conns */
    const char *tls_cert_file;     /* server: PEM certificate chain file */
    const char *tls_key_file;      /* server: PEM private key file */
    const char *tls_ca_file;       /* client: CA bundle for verification (or NULL) */
    int         tls_insecure;      /* client: skip certificate verification */
};

/* Returns 1 if librtmp2 was built with TLS (RTMPS) support, 0 otherwise. */
int lrtmp2_tls_supported(void);

lrtmp2_server_t *lrtmp2_server_create(const lrtmp2_server_config_t *config);
void             lrtmp2_server_destroy(lrtmp2_server_t *server);
int              lrtmp2_server_listen(lrtmp2_server_t *server, const char *bind_addr);
int              lrtmp2_server_poll(lrtmp2_server_t *server, int timeout_ms);
void             lrtmp2_server_stop(lrtmp2_server_t *server);

/* Accessors */
int              lrtmp2_conn_get_fd(const lrtmp2_conn_t *conn);

/* ==================== Client API ==================== */

lrtmp2_client_t *lrtmp2_client_create(const lrtmp2_server_config_t *config);
void             lrtmp2_client_destroy(lrtmp2_client_t *client);
int              lrtmp2_client_connect(lrtmp2_client_t *client, const char *url);

/* App-level command flow, run after lrtmp2_client_connect() */
int              lrtmp2_client_publish(lrtmp2_client_t *client);
int              lrtmp2_client_play(lrtmp2_client_t *client);

/* Send one audio/video frame while publishing */
int              lrtmp2_client_send_frame(lrtmp2_client_t *client, const lrtmp2_frame_t *frame);

/* Pump incoming data while playing: delivers frames via config->on_frame_cb.
 * Blocks for up to timeout_ms waiting for data; returns LRTMP2_OK on success
 * (including a timeout with no data), or a negative error code on I/O/protocol
 * failure or peer disconnect. */
int              lrtmp2_client_poll(lrtmp2_client_t *client, int timeout_ms);

/* ==================== Frame API ==================== */

/* Frames are delivered via on_frame_cb callback. See librtmp2/types.h for lrtmp2_frame_t. */

/* ==================== Utility ==================== */

const char *lrtmp2_version_string(void);
int lrtmp2_version_major(void);
int lrtmp2_version_minor(void);
int lrtmp2_version_patch(void);

#ifdef __cplusplus
}
#endif

#endif /* LRTMP2_H */
