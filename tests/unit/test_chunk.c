/**
 * test_chunk.c — Unit tests for chunk reader/writer
 */
#include "chunk/chunk_reader.h"
#include "chunk/chunk_writer.h"
#include "chunk/chunk_state.h"
#include "core/buffer.h"
#include <stdio.h>
#include <string.h>

int test_chunk_write_read_basic(void)
{
    lrtmp2_buffer_t *out = lrtmp2_buffer_create();
    lrtmp2_chunk_streams_init();

    /* Write a simple chunk */
    lrtmp2_chunk_message_t msg;
    memset(&msg, 0, sizeof(msg));
    msg.csid = 2;
    msg.fmt = 0;
    msg.timestamp = 1000;
    msg.msg_length = 11;
    msg.msg_type_id = 0x08;  /* audio */
    msg.msg_stream_id = 1;

    uint8_t payload[] = "hello world";
    int rc = lrtmp2_chunk_write(out, &msg, payload, 11);
    if (rc != 0) {
        printf("FAIL: chunk_write returned %d\n", rc);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    /* Read it back */
    lrtmp2_chunk_stream_t *cs = lrtmp2_chunk_stream_get(2);
    lrtmp2_chunk_message_t read_msg;
    uint8_t read_payload[256];
    size_t read_len;

    out->read_pos = 0;
    rc = lrtmp2_chunk_read(out, cs, &read_msg, read_payload, sizeof(read_payload), &read_len);
    if (rc <= 0) {
        printf("FAIL: chunk_read returned %d\n", rc);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    if (read_msg.msg_length != 11) {
        printf("FAIL: expected msg_length 11, got %u\n", read_msg.msg_length);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    if (read_len != 11) {
        printf("FAIL: expected payload len 11, got %zu\n", read_len);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    if (memcmp(read_payload, payload, 11) != 0) {
        printf("FAIL: payload mismatch\n");
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    lrtmp2_buffer_destroy(out);
    printf("PASS: chunk write/read basic\n");
    return 1;
}

int test_chunk_multi_fragment(void)
{
    /* Test chunk reassembly: write a large message in multiple chunks */
    lrtmp2_buffer_t *out = lrtmp2_buffer_create();
    lrtmp2_chunk_streams_init();

    uint8_t large_payload[512];
    memset(large_payload, 0xAB, sizeof(large_payload));

    lrtmp2_chunk_message_t msg;
    memset(&msg, 0, sizeof(msg));
    msg.csid = 4;
    msg.fmt = 0;
    msg.timestamp = 5000;
    msg.msg_length = 512;
    msg.msg_type_id = 0x09;  /* video */
    msg.msg_stream_id = 1;

    /* A single write call fragments the 512-byte payload internally into
     * multiple physical chunks (csid's chunk_size defaults to 128), emitting
     * fmt=3 continuation headers for each chunk after the first. */
    lrtmp2_chunk_write(out, &msg, large_payload, 512);

    /* Read and reassemble */
    lrtmp2_chunk_stream_t *cs = lrtmp2_chunk_stream_get(4);
    cs->chunk_size = 128;
    cs->reassembly_bytes_read = 0;

    uint8_t reassembled[1024];
    size_t total_read = 0;
    lrtmp2_chunk_message_t read_msg;
    uint8_t buf[1024];
    size_t rlen;

    out->read_pos = 0;

    /* Read physical chunks until the full message is reassembled */
    int complete = 0;
    for (int i = 0; i < 10 && !complete; i++) {
        int rc = lrtmp2_chunk_read(out, cs, &read_msg, buf, sizeof(buf), &rlen);
        if (rc <= 0) {
            printf("FAIL: chunk_read fragment %d returned %d\n", i, rc);
            lrtmp2_buffer_destroy(out);
            return 0;
        }
        if (read_msg.is_complete) {
            memcpy(reassembled, buf, rlen);
            total_read = rlen;
            complete = 1;
        }
    }

    if (total_read != 512) {
        printf("FAIL: expected 512 bytes reassembled, got %zu\n", total_read);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    if (memcmp(reassembled, large_payload, 512) != 0) {
        printf("FAIL: reassembled data mismatch\n");
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    lrtmp2_buffer_destroy(out);
    printf("PASS: chunk multi-fragment reassembly\n");
    return 1;
}

int test_chunk_main(void)
{
    int passed = 0;
    printf("Running chunk tests...\n");
    if (test_chunk_write_read_basic()) passed++;
    if (test_chunk_multi_fragment()) passed++;
    lrtmp2_chunk_streams_destroy();
    printf("Chunk tests: %d/2 passed\n", passed);
    return (passed >= 2) ? 0 : 1;
}
