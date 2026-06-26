/**
 * test_server_ertmp_v1.c — Integration test: E-RTMP v1 enhanced video/audio ingest
 *
 * Tests that the server correctly parses Enhanced RTMP v1 video (ExVideoTagHeader
 * with FourCC) and audio (ExAudioTagHeader with FourCC) frames, in addition to
 * legacy frames.
 *
 * Flow: handshake → connect → createStream → publish → enhanced HEVC video →
 *        enhanced Opus audio → verify frames delivered with correct FourCC/codec.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "server/server.h"
#include "session/conn.h"
#include "core/log.h"
#include "core/alloc.h"
#include "core/bytes.h"
#include "ertmp/ertmp.h"

#define MAX_FRAMES 16

typedef struct {
    lrtmp2_frame_type_t type;
    uint32_t size;
    lrtmp2_video_codec_t vcodec;
    lrtmp2_audio_codec_t acodec;
    char fourcc[5];
    uint8_t frame_type;
} frame_record_t;

static frame_record_t g_frames[MAX_FRAMES];
static int g_frame_count = 0;
static int g_publish_called = 0;

static int on_frame_cb(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *userdata) {
    (void)conn; (void)userdata;
    if (g_frame_count >= MAX_FRAMES) return 0;
    frame_record_t *r = &g_frames[g_frame_count++];
    r->type = frame->type;
    r->size = frame->size;
    r->vcodec = frame->video_codec;
    r->acodec = frame->audio_codec;
    r->frame_type = frame->video_frame_type;
    memcpy(r->fourcc, frame->type == LRTMP2_FRAME_VIDEO ? frame->video_fourcc.cc : frame->audio_fourcc.cc, 5);
    printf("  [cb] frame #%d: type=%s size=%u fourcc=%s\n",
           g_frame_count,
           frame->type == LRTMP2_FRAME_VIDEO ? "video" : "audio",
           frame->size, r->fourcc);
    return 0;
}

static int on_publish_cb(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata) {
    (void)conn; (void)app; (void)stream_key; (void)userdata;
    g_publish_called = 1;
    return 0;
}

/* Build AMF0 connect body */
static size_t build_connect_body(uint8_t *buf, size_t buf_size) {
    (void)buf_size;
    uint8_t *p = buf;
    *p++ = 0x02; *p++ = 0x00; *p++ = 0x07; memcpy(p, "connect", 7); p += 7;
    *p++ = 0x00;
    uint64_t one = 0x3FF0000000000000ULL;
    for (int i = 7; i >= 0; i--) *p++ = (uint8_t)((one >> (i*8)) & 0xFF);
    *p++ = 0x03;
    *p++ = 0x00; *p++ = 0x03; memcpy(p, "app", 3); p += 3;
    *p++ = 0x02; *p++ = 0x00; *p++ = 0x04; memcpy(p, "live", 4); p += 4;
    *p++ = 0x00; *p++ = 0x00; *p++ = 0x09;
    return (size_t)(p - buf);
}

static size_t build_create_stream_body(uint8_t *buf) {
    uint8_t *p = buf;
    *p++ = 0x02; *p++ = 0x00; *p++ = 0x0C; memcpy(p, "createStream", 12); p += 12;
    *p++ = 0x00;
    uint64_t two = 0x4000000000000000ULL;
    for (int i = 7; i >= 0; i--) *p++ = (uint8_t)((two >> (i*8)) & 0xFF);
    *p++ = 0x05;
    return (size_t)(p - buf);
}

static size_t build_publish_body(uint8_t *buf, const char *stream_name) {
    uint8_t *p = buf;
    *p++ = 0x02; *p++ = 0x00; *p++ = 0x07; memcpy(p, "publish", 7); p += 7;
    *p++ = 0x00;
    uint64_t three = 0x4008000000000000ULL;
    for (int i = 7; i >= 0; i--) *p++ = (uint8_t)((three >> (i*8)) & 0xFF);
    *p++ = 0x05;
    size_t slen = strlen(stream_name);
    *p++ = 0x02; *p++ = (uint8_t)(slen >> 8); *p++ = (uint8_t)slen; memcpy(p, stream_name, slen); p += slen;
    *p++ = 0x02; *p++ = 0x00; *p++ = 0x04; memcpy(p, "live", 4); p += 4;
    return (size_t)(p - buf);
}

static size_t wrap_chunk(uint8_t *out, size_t out_size,
                          uint8_t csid, uint32_t msg_type,
                          const uint8_t *body, size_t body_len) {
    if (out_size < 12 + body_len) return 0;
    uint8_t *p = out;
    *p++ = csid;
    *p++ = 0x00; *p++ = 0x00; *p++ = 0x00;
    *p++ = (uint8_t)((body_len >> 16) & 0xFF);
    *p++ = (uint8_t)((body_len >> 8) & 0xFF);
    *p++ = (uint8_t)(body_len & 0xFF);
    *p++ = (uint8_t)msg_type;
    *p++ = 0x00; *p++ = 0x00; *p++ = 0x00; *p++ = 0x00;
    memcpy(p, body, body_len); p += body_len;
    return (size_t)(p - out);
}

int main(void) {
    printf("=== librtmp2 integration: E-RTMP v1 enhanced video+audio ===\n\n");

    lrtmp2_server_config_t config;
    memset(&config, 0, sizeof(config));
    config.max_connections = 10;
    config.chunk_size = 4096;
    config.on_frame_cb = on_frame_cb;
    config.on_publish_cb = on_publish_cb;

    lrtmp2_server_t *server = lrtmp2_server_create(&config);
    if (!server) { printf("FAIL: server_create\n"); return 1; }

    lrtmp2_conn_t *conn = lrtmp2_conn_create(server, &config);
    if (!conn) { printf("FAIL: conn_create\n"); lrtmp2_server_destroy(server); return 1; }
    conn->client_fd = -1;

    uint8_t stream[65536];
    size_t total = 0;

    /* Handshake */
    stream[total++] = 0x03;
    uint32_t t = lrtmp2_hton32(0x12345678);
    memcpy(stream + total, &t, 4); total += 4;
    memset(stream + total, 0, 4); total += 4;
    for (int i = 0; i < 1528; i++) stream[total++] = (uint8_t)(i & 0xFF);
    uint32_t peer_t = lrtmp2_hton32(0x87654321);
    memcpy(stream + total, &peer_t, 4); total += 4;
    memcpy(stream + total, &t, 4); total += 4;
    for (int i = 0; i < 1528; i++) stream[total++] = (uint8_t)((i * 7 + 13) & 0xFF);

    /* Connect */
    uint8_t connect_body[256];
    size_t connect_body_len = build_connect_body(connect_body, sizeof(connect_body));
    total += wrap_chunk(stream + total, sizeof(stream) - total, 2, 0x14, connect_body, connect_body_len);

    /* CreateStream */
    uint8_t cs_body[64];
    size_t cs_body_len = build_create_stream_body(cs_body);
    total += wrap_chunk(stream + total, sizeof(stream) - total, 2, 0x14, cs_body, cs_body_len);

    /* Publish */
    uint8_t pub_body[128];
    size_t pub_body_len = build_publish_body(pub_body, "ertmp_test");
    total += wrap_chunk(stream + total, sizeof(stream) - total, 2, 0x14, pub_body, pub_body_len);

    /* ── Enhanced HEVC video frame (ExVideoTagHeader with FourCC "hvc1") ── */
    uint8_t hevc_video[1024];
    size_t hevc_len = 0;
    hevc_video[hevc_len++] = 0x91; /* IsExHeader=1, FrameType=1(key), PacketType=1(coded) */
    hevc_video[hevc_len++] = 'h'; hevc_video[hevc_len++] = 'v';
    hevc_video[hevc_len++] = 'c'; hevc_video[hevc_len++] = '1';
    hevc_video[hevc_len++] = 0x00; hevc_video[hevc_len++] = 0x00; hevc_video[hevc_len++] = 0x00; /* CT = 0 */
    /* Fake HEVC IDR NALU: NAL type 19 (IDR_W_RADL) in bits 1-6 of byte 0 */
    uint8_t fake_hevc[] = { 0x01, 0x01, 0x60, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0xFC, 0xDF, 0x09, 0x00, 0x00 };
    memcpy(hevc_video + hevc_len, fake_hevc, sizeof(fake_hevc)); hevc_len += sizeof(fake_hevc);
    total += wrap_chunk(stream + total, sizeof(stream) - total, 5, 0x09, hevc_video, hevc_len);

    /* ── Enhanced AV1 video frame (ExVideoTagHeader with FourCC "av01") ── */
    uint8_t av1_video[1024];
    size_t av1_len = 0;
    av1_video[av1_len++] = 0x91; /* IsExHeader=1, FrameType=1(key), PacketType=1(coded) */
    av1_video[av1_len++] = 'a'; av1_video[av1_len++] = 'v';
    av1_video[av1_len++] = '0'; av1_video[av1_len++] = '1';
    /* No composition time for non-avc/hvc codecs */
    uint8_t fake_av1[] = { 0x00, 0x00, 0x00, 0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00 };
    memcpy(av1_video + av1_len, fake_av1, sizeof(fake_av1)); av1_len += sizeof(fake_av1);
    total += wrap_chunk(stream + total, sizeof(stream) - total, 5, 0x09, av1_video, av1_len);

    /* ── Enhanced Opus audio frame (ExAudioTagHeader with FourCC "Opus") ── */
    uint8_t opus_audio[256];
    size_t opus_len = 0;
    opus_audio[opus_len++] = 0x81; /* IsExHeader=1, FrameType=0, PacketType=1(coded) */
    opus_audio[opus_len++] = 'O'; opus_audio[opus_len++] = 'p';
    opus_audio[opus_len++] = 'u'; opus_audio[opus_len++] = 's';
    /* Fake Opus frame */
    uint8_t fake_opus[] = { 0x7F, 0xE8, 0x01, 0x02, 0x03, 0x04, 0x05 };
    memcpy(opus_audio + opus_len, fake_opus, sizeof(fake_opus)); opus_len += sizeof(fake_opus);
    total += wrap_chunk(stream + total, sizeof(stream) - total, 4, 0x08, opus_audio, opus_len);

    /* ── Legacy H.264 video frame (for comparison) ── */
    uint8_t legacy_video[256];
    size_t legacy_vid_len = 0;
    legacy_video[legacy_vid_len++] = 0x17; /* keyframe + AVC */
    legacy_video[legacy_vid_len++] = 0x01; /* NALU */
    legacy_video[legacy_vid_len++] = 0x00; legacy_video[legacy_vid_len++] = 0x00; legacy_video[legacy_vid_len++] = 0x00;
    uint8_t fake_h264[] = { 0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00 };
    memcpy(legacy_video + legacy_vid_len, fake_h264, sizeof(fake_h264)); legacy_vid_len += sizeof(fake_h264);
    total += wrap_chunk(stream + total, sizeof(stream) - total, 5, 0x09, legacy_video, legacy_vid_len);

    printf("Built RTMP stream: %zu bytes (%d frames to send)\n", total, 4);
    /* Dump first 64 bytes */
    printf("  stream hex:");
    for (size_t i = 0; i < (total < 64 ? total : 64); i++) printf(" %02x", stream[i]);
    printf("\n");

    /* Feed to connection */
    int rc = lrtmp2_conn_recv(conn, stream, total);
    printf("conn_recv: rc=%d state=%d frames=%d publish=%d\n",
           rc, conn->state, g_frame_count, g_publish_called);

    /* Verify */
    int success = 1;

    if (conn->state != LRTMP2_STATE_PUBLISHING) {
        printf("FAIL: expected PUBLISHING, got state=%d\n", conn->state);
        success = 0;
    }
    if (!g_publish_called) {
        printf("FAIL: publish callback not called\n");
        success = 0;
    }
    if (g_frame_count < 4) {
        printf("FAIL: expected 4 frames, got %d\n", g_frame_count);
        success = 0;
    }

    /* Check frame details */
    int found_hevc = 0, found_av1 = 0, found_opus = 0, found_legacy = 0;
    for (int i = 0; i < g_frame_count; i++) {
        frame_record_t *r = &g_frames[i];
        if (r->type == LRTMP2_FRAME_VIDEO) {
            if (strcmp(r->fourcc, "hvc1") == 0 && r->vcodec == LRTMP2_VIDEO_H265) {
                found_hevc = 1;
                printf("  PASS: HEVC video frame (fourcc=hvc1, codec=H265)\n");
            } else if (strcmp(r->fourcc, "av01") == 0 && r->vcodec == LRTMP2_VIDEO_AV1) {
                found_av1 = 1;
                printf("  PASS: AV1 video frame (fourcc=av01, codec=AV1)\n");
            } else if (r->vcodec == LRTMP2_VIDEO_H264 && r->frame_type == 1) {
                found_legacy = 1;
                printf("  PASS: Legacy H.264 video frame\n");
            }
        } else if (r->type == LRTMP2_FRAME_AUDIO) {
            if (strcmp(r->fourcc, "Opus") == 0 && r->acodec == LRTMP2_AUDIO_OPUS) {
                found_opus = 1;
                printf("  PASS: Opus audio frame (fourcc=Opus, codec=OPUS)\n");
            }
        }
    }

    if (!found_hevc) { printf("FAIL: no HEVC frame received\n"); success = 0; }
    if (!found_av1) { printf("FAIL: no AV1 frame received\n"); success = 0; }
    if (!found_opus) { printf("FAIL: no Opus audio frame received\n"); success = 0; }
    if (!found_legacy) { printf("FAIL: no legacy H.264 frame received\n"); success = 0; }

    if (success) {
        printf("\nPASS: all E-RTMP v1 frame types received and parsed correctly\n");
    }

    lrtmp2_conn_destroy(conn);
    lrtmp2_server_destroy(server);
    return success ? 0 : 1;
}
