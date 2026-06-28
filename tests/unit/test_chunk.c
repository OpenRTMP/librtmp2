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
    /* Within a single fmt=0 message (continued via fmt=3), msg_length is fixed.
     * A fmt=1 header starts a new message and must not splice prior bytes. */
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
    c2[4] = 0x00; c2[5] = 0x00; c2[6] = 0x0A; /* msg_length=10 — new message */
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
    if (rc <= 0 || !rm.is_complete || rm.msg_length != 10 || rl != 10) {
        printf("FAIL: fmt=1 should start a fresh 10-byte message, got rc=%d complete=%d len=%zu\n",
               rc, rm.is_complete, rl);
        lrtmp2_chunk_registry_destroy(&reg);
        lrtmp2_buffer_destroy(buf);
        return 0;
    }
    if (rp[0] != 'B') {
        printf("FAIL: fmt=1 payload splice: first byte '%c', expected 'B'\n", rp[0]);
        lrtmp2_chunk_registry_destroy(&reg);
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    lrtmp2_chunk_registry_destroy(&reg);
    lrtmp2_buffer_destroy(buf);
    printf("PASS: fmt=1 starts fresh message instead of splicing partial data\n");
    return 1;
}

int test_chunk_fmt2_partial_no_ts_drift(void)
{
    /* Regression: a fmt=1/2 header whose payload has not fully arrived yet must
     * not apply its timestamp delta a second time when the chunk is re-parsed
     * after the buffer rollback. */
    lrtmp2_chunk_registry_t reg;
    memset(&reg, 0, sizeof(reg));
    lrtmp2_chunk_registry_init(&reg);
    lrtmp2_chunk_stream_t *cs = lrtmp2_chunk_stream_get(&reg, 3);
    cs->chunk_size = 128;

    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    lrtmp2_chunk_message_t rm;
    const uint8_t *rp = NULL;
    size_t rl = 0;

    /* Chunk 1: fmt=0, csid=3, ts=1000, len=4, type=0x14, then 4 payload bytes */
    uint8_t c1[12 + 4];
    memset(c1, 0, sizeof(c1));
    c1[0] = 0x03;
    c1[1] = 0x00; c1[2] = 0x03; c1[3] = 0xE8; /* ts=1000 */
    c1[4] = 0x00; c1[5] = 0x00; c1[6] = 0x04; /* len=4 */
    c1[7] = 0x14;
    memset(c1 + 12, 'A', 4);
    lrtmp2_buffer_write(buf, c1, sizeof(c1));

    int rc = lrtmp2_chunk_read(buf, &reg, cs, &rm, &rp, &rl);
    if (rc <= 0 || !rm.is_complete || rm.timestamp != 1000) {
        printf("FAIL: chunk1 read rc=%d complete=%d ts=%u\n", rc, rm.is_complete, rm.timestamp);
        goto fail;
    }

    /* Chunk 2: fmt=2, csid=3, delta=500 — feed the header WITHOUT its payload. */
    uint8_t c2_hdr[4];
    c2_hdr[0] = 0x83;                                      /* fmt=2, csid=3 */
    c2_hdr[1] = 0x00; c2_hdr[2] = 0x01; c2_hdr[3] = 0xF4; /* delta=500 */
    lrtmp2_buffer_write(buf, c2_hdr, sizeof(c2_hdr));

    rc = lrtmp2_chunk_read(buf, &reg, cs, &rm, &rp, &rl);
    if (rc != 0) {  /* header present, payload missing -> need_more (0) */
        printf("FAIL: chunk2 header-only expected rc=0, got %d\n", rc);
        goto fail;
    }

    /* Deliver the 4-byte payload and read again — timestamp must be 1500, not 2000. */
    uint8_t c2_payload[4];
    memset(c2_payload, 'B', 4);
    lrtmp2_buffer_write(buf, c2_payload, sizeof(c2_payload));

    rc = lrtmp2_chunk_read(buf, &reg, cs, &rm, &rp, &rl);
    if (rc <= 0 || !rm.is_complete) {
        printf("FAIL: chunk2 full read rc=%d complete=%d\n", rc, rm.is_complete);
        goto fail;
    }
    if (rm.timestamp != 1500) {
        printf("FAIL: timestamp drift — expected 1500, got %u\n", rm.timestamp);
        goto fail;
    }

    lrtmp2_chunk_registry_destroy(&reg);
    lrtmp2_buffer_destroy(buf);
    printf("PASS: fmt=2 partial read does not double-apply timestamp delta\n");
    return 1;
fail:
    lrtmp2_chunk_registry_destroy(&reg);
    lrtmp2_buffer_destroy(buf);
    return 0;
}

int test_chunk_fmt3_extended_timestamp(void)
{
    /* Regression: continuation (fmt=3) chunks of a message that uses extended
     * timestamps repeat the 4-byte extended timestamp before their payload. The
     * reader must consume it instead of treating it as payload bytes. */
    lrtmp2_buffer_t *out = lrtmp2_buffer_create();
    lrtmp2_chunk_registry_t reg;
    memset(&reg, 0, sizeof(reg));
    lrtmp2_chunk_registry_init(&reg);

    uint8_t payload[300];
    for (int i = 0; i < 300; i++) payload[i] = (uint8_t)i;

    lrtmp2_chunk_message_t msg;
    memset(&msg, 0, sizeof(msg));
    msg.csid = 6;
    msg.fmt = 0;
    msg.timestamp = 0x01000000;   /* >= 0xFFFFFF -> extended timestamp in use */
    msg.msg_length = 300;
    msg.msg_type_id = 0x09;       /* video */
    msg.msg_stream_id = 1;

    /* Writer fragments into 128-byte chunks and emits fmt=3 continuation chunks,
     * each repeating the 4-byte extended timestamp. */
    lrtmp2_chunk_write(out, &msg, payload, 300, LRTMP2_DEFAULT_CHUNK_SIZE);

    lrtmp2_chunk_stream_t *cs = lrtmp2_chunk_stream_get(&reg, 6);
    cs->chunk_size = 128;

    uint8_t reassembled[512];
    size_t total = 0;
    lrtmp2_chunk_message_t rm;
    const uint8_t *rp = NULL;
    size_t rl = 0;
    out->read_pos = 0;
    int complete = 0;
    for (int i = 0; i < 10 && !complete; i++) {
        int rc = lrtmp2_chunk_read(out, &reg, cs, &rm, &rp, &rl);
        if (rc <= 0) { printf("FAIL: ext-ts fragment %d rc=%d\n", i, rc); goto fail; }
        if (rm.is_complete) { memcpy(reassembled, rp, rl); total = rl; complete = 1; }
    }
    if (total != 300 || memcmp(reassembled, payload, 300) != 0) {
        printf("FAIL: ext-ts reassembly mismatch (total=%zu)\n", total);
        goto fail;
    }
    if (rm.timestamp != 0x01000000) {
        printf("FAIL: ext-ts value wrong: got %u\n", rm.timestamp);
        goto fail;
    }
    lrtmp2_chunk_registry_destroy(&reg);
    lrtmp2_buffer_destroy(out);
    printf("PASS: fmt=3 continuation consumes extended timestamp\n");
    return 1;
fail:
    lrtmp2_chunk_registry_destroy(&reg);
    lrtmp2_buffer_destroy(out);
    return 0;
}

int test_chunk_fmt1_resets_partial_reassembly(void)
{
    /* fmt=1 must start a fresh message: a partial fmt=0 body must not be
     * spliced into a later fmt=1 message on the same csid. */
    lrtmp2_chunk_registry_t reg;
    memset(&reg, 0, sizeof(reg));
    lrtmp2_chunk_registry_init(&reg);
    lrtmp2_chunk_stream_t *cs = lrtmp2_chunk_stream_get(&reg, 5);
    cs->chunk_size = 64;

    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();

    uint8_t c1[12 + 64];
    memset(c1, 0, sizeof(c1));
    c1[0] = 0x05;
    c1[4] = 0x01; c1[5] = 0x00; /* len=256 */
    c1[7] = 0x14;
    memset(c1 + 12, 0xFF, 64);
    lrtmp2_buffer_write(buf, c1, sizeof(c1));

    uint8_t c2[8 + 128];
    memset(c2, 0, sizeof(c2));
    c2[0] = 0x45;
    c2[4] = 0x00; c2[5] = 0x00; c2[6] = 0x40; /* len=64 */
    c2[7] = 0x09;
    memset(c2 + 8, 0xAA, 64);
    lrtmp2_buffer_write(buf, c2, sizeof(c2));

    buf->read_pos = 0;
    lrtmp2_chunk_message_t rm;
    const uint8_t *rp = NULL;
    size_t rl = 0;
    lrtmp2_chunk_read(buf, &reg, cs, &rm, &rp, &rl);
    int rc = lrtmp2_chunk_read(buf, &reg, cs, &rm, &rp, &rl);
    if (rc <= 0 || !rm.is_complete || rm.msg_type_id != 0x09 || rl != 64) {
        printf("FAIL: fmt=1 splice read rc=%d complete=%d type=0x%02x len=%zu\n",
               rc, rm.is_complete, rm.msg_type_id, rl);
        goto fail;
    }
    if (rp[0] != 0xAA) {
        printf("FAIL: fmt=1 did not reset reassembly (first byte 0x%02x, expected 0xAA)\n", rp[0]);
        goto fail;
    }

    lrtmp2_chunk_registry_destroy(&reg);
    lrtmp2_buffer_destroy(buf);
    printf("PASS: fmt=1 resets partial reassembly on new message\n");
    return 1;
fail:
    lrtmp2_chunk_registry_destroy(&reg);
    lrtmp2_buffer_destroy(buf);
    return 0;
}

int test_chunk_reassembly_budget_enforced(void)
{
    lrtmp2_chunk_registry_t reg;
    memset(&reg, 0, sizeof(reg));
    lrtmp2_chunk_registry_init(&reg);

    enum { CHUNK_PAYLOAD = 1024 * 1024 };
    int rejected = 0;

    for (uint32_t csid = 2; csid < 2 + 64; csid++) {
        lrtmp2_chunk_stream_t *cs = lrtmp2_chunk_stream_get(&reg, csid);
        if (!cs) break;
        cs->chunk_size = CHUNK_PAYLOAD;

        lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
        uint8_t pkt[12 + CHUNK_PAYLOAD];
        memset(pkt, 0, sizeof(pkt));
        pkt[0] = (uint8_t)csid;
        pkt[4] = (CHUNK_PAYLOAD >> 16) & 0xFF;
        pkt[5] = (CHUNK_PAYLOAD >> 8) & 0xFF;
        pkt[6] = CHUNK_PAYLOAD & 0xFF;
        pkt[7] = 0x09;
        memset(pkt + 12, 'Z', CHUNK_PAYLOAD);
        lrtmp2_buffer_write(buf, pkt, sizeof(pkt));

        buf->read_pos = 0;
        lrtmp2_chunk_message_t rm;
        const uint8_t *rp = NULL;
        size_t rl = 0;
        int rc = lrtmp2_chunk_read(buf, &reg, cs, &rm, &rp, &rl);
        lrtmp2_buffer_destroy(buf);
        if (rc == LRTMP2_ERR_CHUNK) {
            rejected = 1;
            break;
        }
        if (rc <= 0) {
            printf("FAIL: unexpected rc=%d at csid %u\n", rc, csid);
            lrtmp2_chunk_registry_destroy(&reg);
            return 0;
        }
    }

    if (!rejected) {
        printf("FAIL: reassembly budget never tripped\n");
        lrtmp2_chunk_registry_destroy(&reg);
        return 0;
    }

    lrtmp2_chunk_registry_destroy(&reg);
    printf("PASS: per-connection reassembly budget enforced\n");
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
    if (test_chunk_fmt2_partial_no_ts_drift()) passed++;
    if (test_chunk_fmt3_extended_timestamp()) passed++;
    if (test_chunk_fmt1_resets_partial_reassembly()) passed++;
    if (test_chunk_reassembly_budget_enforced()) passed++;
    printf("Chunk tests: %d/8 passed\n", passed);
    return (passed >= 8) ? 0 : 1;
}
