/**
 * test_message.c — Unit tests for message dispatch (src/message/message.c)
 */
#include "message/message.h"
#include "session/conn.h"
#include <stdio.h>
#include <string.h>

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

int test_message_main(void)
{
    int passed = 0;
    printf("Running message dispatch tests...\n");
    if (test_zero_length_message_not_rejected()) passed++;
    printf("Message tests: %d/1 passed\n", passed);
    return (passed >= 1) ? 0 : 1;
}
