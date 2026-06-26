/**
 * test_server_ingest.c — Integration test: RTMP server handshake + full session
 *
 * Tests the full server ingress pipeline without real network threads:
 *   1. Create a server and connection
 *   2. Push a crafted RTMP byte stream: handshake, connect, createStream,
 *      publish, and a video chunk built from tests/test_data/test.h264
 *   3. Process and verify handshake + command dispatch (connect/createStream/
 *      publish) drives the connection into PUBLISHING state, and that the
 *      video frame is delivered via the on_frame callback.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "server/server.h"
#include "session/conn.h"
#include "session/state_machine.h"
#include "core/log.h"
#include "core/alloc.h"
#include "core/bytes.h"

#define MAX_RECEIVED (256 * 1024)

static uint8_t g_received_data[MAX_RECEIVED];
static size_t g_received_len = 0;
static int g_frame_received = 0;
static int g_video_frames = 0;
static int g_audio_frames = 0;
static char g_last_video_fourcc[5] = {0};
static int g_publish_called = 0;
static char g_publish_app[256];
static char g_publish_stream[256];

static int on_frame_cb(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *userdata) {
    (void)conn; (void)userdata;
    fprintf(stderr, "  [cb] on_frame: type=%d size=%u\n", frame->type, frame->size);
    if (frame->type == LRTMP2_FRAME_VIDEO) {
        g_video_frames++;
        memcpy(g_last_video_fourcc, frame->video_fourcc.cc, 5);
    } else if (frame->type == LRTMP2_FRAME_AUDIO) {
        g_audio_frames++;
    }
    if (frame->size > 0 && frame->size <= MAX_RECEIVED) {
        memcpy(g_received_data, frame->data, frame->size);
        g_received_len = frame->size;
        g_frame_received++;
    }
    return 0;
}

static int on_publish_cb(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata) {
    (void)conn; (void)userdata;
    fprintf(stderr, "  [cb] on_publish: app=%s stream=%s\n", app, stream_key);
    g_publish_called = 1;
    snprintf(g_publish_app, sizeof(g_publish_app), "%s", app);
    snprintf(g_publish_stream, sizeof(g_publish_stream), "%s", stream_key);
    return 0;
}

/* --- Minimal AMF0 byte-builder helpers (mirrors what a real RTMP client sends) --- */

static uint8_t *amf_put_string(uint8_t *p, const char *s) {
    *p++ = 0x02; /* AMF0_STRING */
    uint16_t len = (uint16_t)strlen(s);
    *p++ = (uint8_t)(len >> 8);
    *p++ = (uint8_t)(len & 0xFF);
    memcpy(p, s, len);
    return p + len;
}

static uint8_t *amf_put_number(uint8_t *p, double value) {
    *p++ = 0x00; /* AMF0_NUMBER */
    uint64_t bits;
    memcpy(&bits, &value, sizeof(bits));
    for (int i = 7; i >= 0; i--) *p++ = (uint8_t)((bits >> (i * 8)) & 0xFF);
    return p;
}

static uint8_t *amf_put_null(uint8_t *p) {
    *p++ = 0x05; /* AMF0_NULL */
    return p;
}

/* Build AMF0 connect body (without chunk header) */
static size_t build_connect_body(uint8_t *buf, size_t buf_size) {
    (void)buf_size;
    uint8_t *p = buf;
    p = amf_put_string(p, "connect");
    p = amf_put_number(p, 1.0);
    /* { "app": "live" } */
    *p++ = 0x03;
    *p++ = 0x00; *p++ = 0x03; memcpy(p, "app", 3); p += 3;
    *p++ = 0x02; *p++ = 0x00; *p++ = 0x04; memcpy(p, "live", 4); p += 4;
    *p++ = 0x00; *p++ = 0x00; *p++ = 0x09;
    return (size_t)(p - buf);
}

static size_t build_create_stream_body(uint8_t *buf) {
    uint8_t *p = buf;
    p = amf_put_string(p, "createStream");
    p = amf_put_number(p, 2.0);
    p = amf_put_null(p);
    return (size_t)(p - buf);
}

static size_t build_publish_body(uint8_t *buf, const char *stream_name) {
    uint8_t *p = buf;
    p = amf_put_string(p, "publish");
    p = amf_put_number(p, 3.0);
    p = amf_put_null(p);
    p = amf_put_string(p, stream_name);
    p = amf_put_string(p, "live");
    return (size_t)(p - buf);
}

/* Wrap a body in an RTMP chunk on the given csid */
static size_t wrap_chunk(uint8_t *out, size_t out_size,
                          uint8_t csid, uint32_t msg_type,
                          const uint8_t *body, size_t body_len) {
    if (out_size < 12 + body_len) return 0;
    uint8_t *p = out;
    /* Basic header: fmt=0, csid */
    *p++ = csid;
    /* Message header: timestamp(3) + msg_length(3) + msg_type(1) + stream_id(4) */
    *p++ = 0x00; *p++ = 0x00; *p++ = 0x00; /* timestamp = 0 */
    *p++ = (uint8_t)((body_len >> 16) & 0xFF);
    *p++ = (uint8_t)((body_len >> 8) & 0xFF);
    *p++ = (uint8_t)(body_len & 0xFF);
    *p++ = (uint8_t)msg_type;
    *p++ = 0x00; *p++ = 0x00; *p++ = 0x00; *p++ = 0x00; /* stream_id = 0 */
    /* Payload */
    memcpy(p, body, body_len); p += body_len;
    return (size_t)(p - out);
}

/* Extract the first IDR slice NALU (starting with 0x65) from an Annex-B
 * encoded buffer (start codes 00 00 00 01). Returns NALU length, or 0. */
static size_t extract_idr_nalu(const uint8_t *data, size_t len, const uint8_t **nalu_out) {
    size_t i = 0;
    while (i + 4 < len) {
        if (data[i] == 0x00 && data[i+1] == 0x00 && data[i+2] == 0x00 && data[i+3] == 0x01) {
            size_t nalu_start = i + 4;
            size_t nalu_end = len;
            for (size_t j = nalu_start; j + 4 <= len; j++) {
                if (data[j] == 0x00 && data[j+1] == 0x00 && data[j+2] == 0x00 && data[j+3] == 0x01) {
                    nalu_end = j;
                    break;
                }
            }
            if ((data[nalu_start] & 0x1F) == 5) { /* IDR slice */
                *nalu_out = data + nalu_start;
                return nalu_end - nalu_start;
            }
            i = nalu_end;
        } else {
            i++;
        }
    }
    return 0;
}

int main(void) {
    printf("=== librtmp2 integration: handshake + full session ===\n\n");

    /* Load the real H.264 sample (Annex-B) used to build the video chunk */
    FILE *f = fopen("tests/test_data/test.h264", "rb");
    if (!f) { printf("FAIL: could not open tests/test_data/test.h264\n"); return 1; }
    uint8_t h264_data[4096];
    size_t h264_len = fread(h264_data, 1, sizeof(h264_data), f);
    fclose(f);

    const uint8_t *idr_nalu = NULL;
    size_t idr_len = extract_idr_nalu(h264_data, h264_len, &idr_nalu);
    if (!idr_nalu || idr_len == 0) { printf("FAIL: no IDR NALU found in test.h264\n"); return 1; }

    /* Create server with callbacks */
    lrtmp2_server_config_t config;
    memset(&config, 0, sizeof(config));
    config.max_connections = 10;
    config.chunk_size = 4096;
    config.on_frame_cb = on_frame_cb;
    config.on_publish_cb = on_publish_cb;

    lrtmp2_server_t *server = lrtmp2_server_create(&config);
    if (!server) { printf("FAIL: server_create\n"); return 1; }

    /* Create a bare connection (no socket) */
    lrtmp2_conn_t *conn = lrtmp2_conn_create(server, &config);
    if (!conn) { printf("FAIL: conn_create\n"); lrtmp2_server_destroy(server); return 1; }
    conn->client_fd = -1;

    /* Build complete RTMP stream */
    uint8_t stream[65536];
    size_t total = 0;

    /* Handshake C0+C1+C2 */
    stream[total++] = 0x03;  /* C0 */
    uint32_t t = lrtmp2_hton32(0x12345678);
    memcpy(stream + total, &t, 4); total += 4;
    memset(stream + total, 0, 4); total += 4;
    for (int i = 0; i < 1528; i++) stream[total++] = (uint8_t)(i & 0xFF);
    uint32_t peer_t = lrtmp2_hton32(0x87654321);
    memcpy(stream + total, &peer_t, 4); total += 4;
    memcpy(stream + total, &t, 4); total += 4;
    for (int i = 0; i < 1528; i++) stream[total++] = (uint8_t)((i * 7 + 13) & 0xFF);

    /* Connect command as chunk on csid 2 */
    uint8_t connect_body[256];
    size_t connect_body_len = build_connect_body(connect_body, sizeof(connect_body));
    total += wrap_chunk(stream + total, sizeof(stream) - total, 2, 0x14, connect_body, connect_body_len);

    /* createStream command as chunk on csid 2 */
    uint8_t create_stream_body[64];
    size_t create_stream_body_len = build_create_stream_body(create_stream_body);
    total += wrap_chunk(stream + total, sizeof(stream) - total, 2, 0x14, create_stream_body, create_stream_body_len);

    /* publish command as chunk on csid 2 */
    uint8_t publish_body[128];
    size_t publish_body_len = build_publish_body(publish_body, "mystream");
    total += wrap_chunk(stream + total, sizeof(stream) - total, 2, 0x14, publish_body, publish_body_len);

    /* Video frame as chunk on csid 5 (video stream): AVC keyframe NALU
     * built from the real IDR slice extracted from test.h264 */
    uint8_t video_body[1024];
    size_t video_body_len = 0;
    video_body[video_body_len++] = 0x17; /* FrameType=keyframe(1), CodecID=AVC(7) */
    video_body[video_body_len++] = 0x01; /* AVCPacketType = NALU */
    video_body[video_body_len++] = 0x00; video_body[video_body_len++] = 0x00; video_body[video_body_len++] = 0x00; /* composition time */
    uint32_t nalu_len_be = lrtmp2_hton32((uint32_t)idr_len);
    memcpy(video_body + video_body_len, &nalu_len_be, 4); video_body_len += 4;
    memcpy(video_body + video_body_len, idr_nalu, idr_len); video_body_len += idr_len;

    size_t video_chunk_len = wrap_chunk(stream + total, sizeof(stream) - total,
                                         5, 0x09, video_body, video_body_len);
    total += video_chunk_len;

    /* ── Enhanced HEVC video frame (ExVideoTagHeader with FourCC "hvc1") ── */
    uint8_t hevc_body[256];
    size_t hevc_len = 0;
    hevc_body[hevc_len++] = 0x91; /* IsExHeader=1, FT=1, PT=1 */
    hevc_body[hevc_len++] = 'h'; hevc_body[hevc_len++] = 'v';
    hevc_body[hevc_len++] = 'c'; hevc_body[hevc_len++] = '1';
    hevc_body[hevc_len++] = 0x00; hevc_body[hevc_len++] = 0x00; hevc_body[hevc_len++] = 0x00;
    uint8_t fake_hevc[] = { 0x01, 0x01, 0x60, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0xFC, 0xDF, 0x09 };
    memcpy(hevc_body + hevc_len, fake_hevc, sizeof(fake_hevc)); hevc_len += sizeof(fake_hevc);
    total += wrap_chunk(stream + total, sizeof(stream) - total, 5, 0x09, hevc_body, hevc_len);

    /* ── Enhanced Opus audio frame (ExAudioTagHeader with FourCC "Opus") ── */
    uint8_t opus_body[128];
    size_t opus_len = 0;
    opus_body[opus_len++] = 0x81; /* IsExHeader=1, FT=0, PT=1 */
    opus_body[opus_len++] = 'O'; opus_body[opus_len++] = 'p';
    opus_body[opus_len++] = 'u'; opus_body[opus_len++] = 's';
    uint8_t fake_opus[] = { 0x7F, 0xE8, 0x01, 0x02, 0x03, 0x04, 0x05 };
    memcpy(opus_body + opus_len, fake_opus, sizeof(fake_opus)); opus_len += sizeof(fake_opus);
    total += wrap_chunk(stream + total, sizeof(stream) - total, 4, 0x08, opus_body, opus_len);

    printf("Built RTMP stream: %zu bytes (video_chunk=%zu, idr_len=%zu)\n", total, video_chunk_len, idr_len);

    /* Feed to connection */
    int rc = lrtmp2_conn_recv(conn, stream, total);
    printf("conn_recv: rc=%d state=%s hs=%d frames=%d publish_called=%d\n",
           rc, lrtmp2_conn_state_str(conn->state), conn->handshake.state, g_frame_received, g_publish_called);

    /* Check: full session should reach PUBLISHING, with the publish callback
     * fired and all frames delivered via on_frame */
    int success = 0;
    printf("  frames: video=%d audio=%d last_fourcc=%s\n",
           g_video_frames, g_audio_frames, g_last_video_fourcc);
    if (conn->state == LRTMP2_STATE_PUBLISHING && g_publish_called && g_frame_received >= 3 &&
        g_video_frames >= 2 && g_audio_frames >= 1 &&
        strcmp(g_publish_app, "live") == 0 && strcmp(g_publish_stream, "mystream") == 0) {
        printf("PASS: session reached PUBLISHING, publish callback fired (app=%s stream=%s), %d frames received\n",
               g_publish_app, g_publish_stream, g_frame_received);
        success = 1;
    } else {
        printf("FAIL: state=%d publish_called=%d frames=%d video=%d audio=%d\n",
               conn->state, g_publish_called, g_frame_received, g_video_frames, g_audio_frames);
    }

    lrtmp2_conn_destroy(conn);
    lrtmp2_server_destroy(server);
    return success ? 0 : 1;
}
