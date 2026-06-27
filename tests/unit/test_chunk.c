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
    lrtmp2_chunk_registry_t reg;
    memset(&reg, 0, sizeof(reg));
    lrtmp2_chunk_registry_init(&reg);

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
    int rc = lrtmp2_chunk_write(out, &msg, payload, 11, LRTMP2_DEFAULT_CHUNK_SIZE);
    if (rc != 0) {
        printf("FAIL: chunk_write returned %d\n", rc);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    /* Read it back */
    lrtmp2_chunk_stream_t *cs = lrtmp2_chunk_stream_get(&reg, 2);
    lrtmp2_chunk_message_t read_msg;
    const uint8_t *read_payload = NULL;
    size_t read_len;

    out->read_pos = 0;
    rc = lrtmp2_chunk_read(out, &reg, cs, &read_msg, &read_payload, &read_len);
    if (rc <= 0) {
        printf("FAIL: chunk_read returned %d\n", rc);
        lrtmp2_chunk_registry_destroy(&reg);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    if (read_msg.msg_length != 11) {
        printf("FAIL: expected msg_length 11, got %u\n", read_msg.msg_length);
        lrtmp2_chunk_registry_destroy(&reg);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    if (read_len != 11) {
        printf("FAIL: expected payload len 11, got %zu\n", read_len);
        lrtmp2_chunk_registry_destroy(&reg);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    if (memcmp(read_payload, payload, 11) != 0) {
        printf("FAIL: payload mismatch\n");
        lrtmp2_chunk_registry_destroy(&reg);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    lrtmp2_chunk_registry_destroy(&reg);
    lrtmp2_buffer_destroy(out);
    printf("PASS: chunk write/read basic\n");
    return 1;
}

int test_chunk_multi_fragment(void)
{
    /* Test chunk reassembly: write a large message in multiple chunks */
    lrtmp2_buffer_t *out = lrtmp2_buffer_create();
    lrtmp2_chunk_registry_t reg;
    memset(&reg, 0, sizeof(reg));
    lrtmp2_chunk_registry_init(&reg);

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
    lrtmp2_chunk_write(out, &msg, large_payload, 512, LRTMP2_DEFAULT_CHUNK_SIZE);

    /* Read and reassemble */
    lrtmp2_chunk_stream_t *cs = lrtmp2_chunk_stream_get(&reg, 4);
    cs->chunk_size = 128;
    cs->reassembly_bytes_read = 0;

    uint8_t reassembled[1024];
    size_t total_read = 0;
    lrtmp2_chunk_message_t read_msg;
    const uint8_t *rbuf = NULL;
    size_t rlen;

    out->read_pos = 0;

    /* Read physical chunks until the full message is reassembled */
    int complete = 0;
    for (int i = 0; i < 10 && !complete; i++) {
        int rc = lrtmp2_chunk_read(out, &reg, cs, &read_msg, &rbuf, &rlen);
        if (rc <= 0) {
            printf("FAIL: chunk_read fragment %d returned %d\n", i, rc);
            lrtmp2_chunk_registry_destroy(&reg);
            lrtmp2_buffer_destroy(out);
            return 0;
        }
        if (read_msg.is_complete) {
            memcpy(reassembled, rbuf, rlen);
            total_read = rlen;
            complete = 1;
        }
    }

    if (total_read != 512) {
        printf("FAIL: expected 512 bytes reassembled, got %zu\n", total_read);
        lrtmp2_chunk_registry_destroy(&reg);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    if (memcmp(reassembled, large_payload, 512) != 0) {
        printf("FAIL: reassembled data mismatch\n");
        lrtmp2_chunk_registry_destroy(&reg);
        lrtmp2_buffer_destroy(out);
        return 0;
    }

    lrtmp2_chunk_registry_destroy(&reg);
    lrtmp2_buffer_destroy(out);
    printf("PASS: chunk multi-fragment reassembly\n");
    return 1;
}

int test_chunk_registry_grow(void)
{
    /* Open far more chunk streams than the old fixed limit (8) to verify the
     * registry grows dynamically, hands back distinct stable nodes, returns the
     * same node for a repeated csid, and reuses freed slots. */
    lrtmp2_chunk_registry_t reg;
    memset(&reg, 0, sizeof(reg));
    lrtmp2_chunk_registry_init(&reg);

    enum { N = 100 };
    lrtmp2_chunk_stream_t *nodes[N];
    for (int i = 0; i < N; i++) {
        nodes[i] = lrtmp2_chunk_stream_get(&reg, (uint32_t)(100 + i));
        if (!nodes[i]) {
            printf("FAIL: registry_get returned NULL at csid %d\n", 100 + i);
            lrtmp2_chunk_registry_destroy(&reg);
            return 0;
        }
    }

    /* Repeated lookups must return the identical node, and nodes must be
     * distinct from one another. */
    for (int i = 0; i < N; i++) {
        if (lrtmp2_chunk_stream_get(&reg, (uint32_t)(100 + i)) != nodes[i]) {
            printf("FAIL: repeated get for csid %d returned a different node\n", 100 + i);
            lrtmp2_chunk_registry_destroy(&reg);
            return 0;
        }
        for (int j = i + 1; j < N; j++) {
            if (nodes[i] == nodes[j]) {
                printf("FAIL: csid %d and %d share a node\n", 100 + i, 100 + j);
                lrtmp2_chunk_registry_destroy(&reg);
                return 0;
            }
        }
    }

    /* Free one slot and confirm a new csid reuses that node rather than growing. */
    nodes[0]->in_use = 0;
    size_t count_before = reg.count;
    lrtmp2_chunk_stream_t *reused = lrtmp2_chunk_stream_get(&reg, 9999);
    if (reused != nodes[0] || reg.count != count_before) {
        printf("FAIL: freed slot was not reused (count %zu -> %zu)\n", count_before, reg.count);
        lrtmp2_chunk_registry_destroy(&reg);
        return 0;
    }

    lrtmp2_chunk_registry_destroy(&reg);
    printf("PASS: chunk registry dynamic growth\n");
    return 1;
}

int test_chunk_reject_msg_length_shrink(void)
{
    /* A peer must not be able to shrink msg_length below bytes already
     * reassembled for the current message on the same csid. */
    lrtmp2_chunk_registry_t reg;
    memset(&reg, 0, sizeof(reg));
    lrtmp2_chunk_registry_init(&reg);
    lrtmp2_chunk_stream_t *cs = lrtmp2_chunk_stream_get(&reg, 3);
    cs->chunk_size = 128;

    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();

    uint8_t c1[12 + 128];
    c1[0] = 0x03;
    c1[4] = 0x01; c1[5] = 0xF4; /* msg_length=500 */
    c1[7] = 0x14;
    memset(c1 + 12, 'A', 128);
    lrtmp2_buffer_write(buf, c1, sizeof(c1));

    uint8_t c2[8 + 128];
    c2[0] = 0x43; /* fmt=1 */
    c2[4] = 0x00; c2[5] = 0x00; c2[6] = 0x0A; /* msg_length=10 */
    c2[7] = 0x14;
    memset(c2 + 8, 'B', 128);
    lrtmp2_buffer_write(buf, c2, sizeof(c2));

    buf->read_pos = 0;
    lrtmp2_chunk_message_t rm;
    const uint8_t *rp = NULL;
    size_t rl = 0;

    int rc = lrtmp2_chunk_read(buf, &reg, cs, &rm, &rp, &rl);
    if (rc <= 0) {
        printf("FAIL: first chunk_read returned %d\n", rc);
        lrtmp2_chunk_registry_destroy(&reg);
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    rc = lrtmp2_chunk_read(buf, &reg, cs, &rm, &rp, &rl);
    if (rc != LRTMP2_ERR_CHUNK || rm.is_complete) {
        printf("FAIL: expected LRTMP2_ERR_CHUNK on msg_length shrink, got %d complete=%d\n",
               rc, rm.is_complete);
        lrtmp2_chunk_registry_destroy(&reg);
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    lrtmp2_chunk_registry_destroy(&reg);
    lrtmp2_buffer_destroy(buf);
    printf("PASS: chunk rejects msg_length shrink mid-reassembly\n");
    return 1;
}

int test_chunk_main(void)
{
    int passed = 0;
    printf("Running chunk tests...\n");
    if (test_chunk_write_read_basic()) passed++;
    if (test_chunk_multi_fragment()) passed++;
    if (test_chunk_registry_grow()) passed++;
    if (test_chunk_reject_msg_length_shrink()) passed++;
    printf("Chunk tests: %d/4 passed\n", passed);
    return (passed >= 4) ? 0 : 1;
}
