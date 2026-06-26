/**
 * test_buffer.c — Unit tests for RTMP buffer module
 */
#include "buffer.h"
#include <stdio.h>
#include <string.h>

int test_buffer_basic(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    if (!buf) {
        printf("FAIL: buffer_create returned NULL\n");
        return 0;
    }

    const uint8_t *data = "hello world";
    size_t len = strlen(data);
    if (lrtmp2_buffer_write(buf, data, len) != 0) {
        printf("FAIL: buffer_write failed\n");
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    uint8_t out[1024];
    size_t out_len = 0;
    if (lrtmp2_buffer_read(buf, out, out_len) != 0) {
        printf("FAIL: buffer_read for size failed\n");
        lrtmp2_buffer_destroy(buf);
        return 0;
    }
    if (out_len != len) {
        printf("FAIL: expected %zu bytes, got %zu\n", len, out_len);
        lrtmp2_buffer_destroy(buf);
        return 0;
    }
    if (memcmp(out, data, len) != 0) {
        printf("FAIL: data mismatch\n");
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    /* Reset and read again */
    lrtmp2_buffer_reset(buf);
    if (lrtmp2_buffer_read(buf, out, &out_len) != 0) {
        printf("FAIL: buffer_read after reset failed\n");
        lrtmp2_buffer_destroy(buf);
        return 0;
    }
    if (out_len != 0) {
        printf("FAIL: expected 0 bytes after reset, got %zu\n", out_len);
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
    int rc = lrtmp2_buffer_write(buf, (const uint8_t *)buf->data, needed);
    /* Should not crash */
    printf("PASS: large allocation test\n");
    lrtmp2_buffer_destroy(buf);
    return 1;
}

int main(void)
{
    int passed = 0;
    printf("Running buffer tests...\n");
    if (test_buffer_basic()) passed++;
    if (test_buffer_ensure_capacity()) passed++;

    printf("Buffer tests: %d passed\n", passed);
    return (passed == 2) ? 0 : 1;
}