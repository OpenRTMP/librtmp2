/**
 * fuzz_handshake.c — Fuzz harness for RTMP handshake parser
 */
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include "handshake/handshake.h"
#include "core/buffer.h"

int fuzz_handshake(const uint8_t *data, size_t size) {
    if (size < 1) return 0;

    lrtmp2_handshake_t hs;
    lrtmp2_handshake_server_init(&hs);

    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    if (!buf) return 0;

    lrtmp2_buffer_write(buf, data, size);

    lrtmp2_handshake_server_read_c0(&hs, buf);
    lrtmp2_handshake_server_read_c1(&hs, buf);
    lrtmp2_handshake_server_read_c2(&hs, buf);

    lrtmp2_buffer_destroy(buf);
    return 0;
}
