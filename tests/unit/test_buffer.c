/**
 * test_buffer.c — Unit tests for RTMP buffer module
 */
#include "core/buffer.h"
#include <stdio.h>
#include <string.h>

int test_buffer_basic(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    if (!buf) {
        printf("FAIL: buffer_create returned NULL\n");
        return 0;
    }

    const char *data = "hello world";
    size_t len = strlen(data);
    if (lrtmp2_buffer_write(buf, (const uint8_t *)data, len) != 0) {
        printf("FAIL: buffer_write failed\n");
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    uint8_t out[1024];
    if (lrtmp2_buffer_read(buf, out, len) != 0) {
        printf("FAIL: buffer_read for size failed\n");
        lrtmp2_buffer_destroy(buf);
        return 0;
    }
    if (memcmp(out, data, len) != 0) {
        printf("FAIL: data mismatch\n");
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    lrtmp2_buffer_destroy(buf);
    printf("PASS: basic buffer test\n");
    return 1;
}

int test_buffer_ensure_capacity(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    size_t needed = 10 * 1024 * 1024;  /* 10 MB */
    uint8_t *data = malloc(needed);
    if (data) {
        memset(data, 0xAB, needed);
        int rc = lrtmp2_buffer_write(buf, data, needed);
        free(data);
        if (rc != 0) {
            printf("FAIL: large allocation write failed\n");
            lrtmp2_buffer_destroy(buf);
            return 0;
        }
    }
    printf("PASS: large allocation test\n");
    lrtmp2_buffer_destroy(buf);
    return 1;
}

int test_buffer_compact_after_read(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    if (!buf) {
        printf("FAIL: buffer_create returned NULL\n");
        return 0;
    }

    const size_t chunk = 64 * 1024;
    uint8_t *block = malloc(chunk);
    if (!block) {
        printf("FAIL: malloc failed\n");
        lrtmp2_buffer_destroy(buf);
        return 0;
    }
    memset(block, 0xCD, chunk);

    /* Fill, consume, and refill many times without compaction this would
     * eventually hit LRTMP2_BUFFER_MAX_SIZE even though no data is retained. */
    for (int i = 0; i < 512; i++) {
        if (lrtmp2_buffer_write(buf, block, chunk) != 0) {
            printf("FAIL: write failed on iteration %d\n", i);
            free(block);
            lrtmp2_buffer_destroy(buf);
            return 0;
        }
        if (lrtmp2_buffer_read(buf, block, chunk) != 0) {
            printf("FAIL: read failed on iteration %d\n", i);
            free(block);
            lrtmp2_buffer_destroy(buf);
            return 0;
        }
    }

    free(block);
    lrtmp2_buffer_destroy(buf);
    printf("PASS: compact-after-read test\n");
    return 1;
}

int test_buffer_main(void)
{
    int passed = 0;
    printf("Running buffer tests...\n");
    if (test_buffer_basic()) passed++;
    if (test_buffer_ensure_capacity()) passed++;
    if (test_buffer_compact_after_read()) passed++;

    printf("Buffer tests: %d passed\n", passed);
    return (passed == 3) ? 0 : 1;
}
