/**
 * test_message.c — Unit tests for message dispatch (src/message/message.c)
 */
#include "message/message.h"
#include "message/command.h"
#include "amf/amf.h"
#include "core/buffer.h"
#include "session/conn.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

/* A zero-length message is delivered by the chunk reader as payload=NULL,
 * payload_len=0, is_complete=1 — that is legitimate wire input (e.g. some
 * peers send empty audio/video/data messages), not an internal error, and
 * must not be rejected by lrtmp2_msg_decode(). */
static int test_zero_length_message_not_rejected(void)
{
    int passed = 1;
    static const uint8_t types[] = { 0x12 /* AMF0_DATA */, 0x08 /* AUDIO */,
                                      0x09 /* VIDEO */, 0x0F /* AMF3_DATA */ };

    lrtmp2_conn_t *conn = lrtmp2_conn_create(NULL, NULL);
    if (!conn) {
        printf("FAIL: could not create conn\n");
        return 0;
    }

    for (size_t i = 0; i < sizeof(types); i++) {
        lrtmp2_chunk_message_t chunk;
        memset(&chunk, 0, sizeof(chunk));
        chunk.msg_type_id = types[i];
        chunk.msg_length = 0;
        chunk.is_complete = 1;

        int rc = lrtmp2_msg_decode(conn, &chunk, NULL, 0);
        if (rc != LRTMP2_OK) {
            printf("FAIL: zero-length message type 0x%02x rejected (rc=%d)\n",
                   types[i], rc);
            passed = 0;
        }
    }

    lrtmp2_conn_destroy(conn);

    if (passed) {
        printf("PASS: zero-length messages accepted for all tested types\n");
    }
    return passed;
}

static int test_aggregate_subtag_cap(void)
{
    /* Each zero-size sub-tag is 15 bytes (11-byte header + 4-byte prev size). */
    enum { N = 4097 };
    size_t payload_len = (size_t)N * 15;
    uint8_t *payload = malloc(payload_len);
    if (!payload) {
        printf("FAIL: malloc for aggregate payload\n");
        return 0;
    }
    for (int i = 0; i < N; i++) {
        size_t off = (size_t)i * 15;
        payload[off] = 0x08; /* audio */
        memset(payload + off + 1, 0, 10);
        memset(payload + off + 11, 0, 4);
    }

    lrtmp2_conn_t *conn = lrtmp2_conn_create(NULL, NULL);
    if (!conn) {
        printf("FAIL: could not create conn\n");
        free(payload);
        return 0;
    }
    lrtmp2_chunk_message_t chunk;
    memset(&chunk, 0, sizeof(chunk));
    chunk.msg_type_id = RTMP_MSG_AGGREGATE;

    int rc = lrtmp2_msg_decode_aggregate(conn, &chunk, payload, payload_len);
    free(payload);
    lrtmp2_conn_destroy(conn);

    if (rc != LRTMP2_ERR_PROTOCOL) {
        printf("FAIL: aggregate over cap should return ERR_PROTOCOL, got %d\n", rc);
        return 0;
    }
    printf("PASS: aggregate sub-tag cap enforced\n");
    return 1;
}

static void amf_write_u16(lrtmp2_buffer_t *buf, uint16_t v)
{
    uint8_t b[2] = { (uint8_t)(v >> 8), (uint8_t)v };
    lrtmp2_buffer_write(buf, b, 2);
}

static int test_connect_object_key_cap(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    if (!buf) {
        printf("FAIL: could not create buffer\n");
        return 0;
    }
    lrtmf2_amf0_write_string(buf, "connect");
    lrtmf2_amf0_write_number(buf, 1.0);
    uint8_t obj = AMF0_OBJECT;
    lrtmp2_buffer_write(buf, &obj, 1);

    /* 257 keys with NULL values — one over the cap. */
    for (int i = 0; i < 257; i++) {
        char key[16];
        snprintf(key, sizeof(key), "k%d", i);
        amf_write_u16(buf, (uint16_t)strlen(key));
        lrtmp2_buffer_write(buf, (const uint8_t *)key, strlen(key));
        uint8_t null_type = AMF0_NULL;
        lrtmp2_buffer_write(buf, &null_type, 1);
    }
    uint8_t end[3] = { 0, 0, 0x09 };
    lrtmp2_buffer_write(buf, end, 3);
    buf->read_pos = 0;

    lrtmp2_connect_info_t info;
    int rc = lrtmp2_cmd_read_connect(buf, &info);
    lrtmp2_buffer_destroy(buf);

    if (rc != LRTMP2_ERR_AMF) {
        printf("FAIL: connect object over cap should return ERR_AMF, got %d\n", rc);
        return 0;
    }
    printf("PASS: connect object key cap enforced\n");
    return 1;
}

int test_message_main(void)
{
    int passed = 0;
    int total = 3;
    printf("Running message dispatch tests...\n");
    if (test_zero_length_message_not_rejected()) passed++;
    if (test_aggregate_subtag_cap()) passed++;
    if (test_connect_object_key_cap()) passed++;
    printf("Message tests: %d/%d passed\n", passed, total);
    return (passed >= total) ? 0 : 1;
}
