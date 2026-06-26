/**
 * fuzz_chunk.c — Fuzz harness for RTMP chunk stream reader
 */
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include "chunk/chunk_reader.h"

int fuzz_chunk(const uint8_t *data, size_t size) {
    if (size < 1) return 0;

    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    if (!buf) return 0;
    lrtmp2_buffer_write(buf, data, size);

    lrtmp2_chunk_stream_t stream;
    memset(&stream, 0, sizeof(stream));

    lrtmp2_chunk_message_t msg;
    uint8_t out_buf[4096];
    size_t out_len = 0;

    for (int i = 0; i < 64 && lrtmp2_buffer_available(buf) > 0; i++) {
        memset(&msg, 0, sizeof(msg));
        int rc = lrtmp2_chunk_read(buf, &stream, &msg,
                                     out_buf, sizeof(out_buf), &out_len);
        if (rc <= 0) break;
    }

    lrtmp2_buffer_destroy(buf);
    return 0;
}
