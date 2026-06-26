/**
 * fuzz_modex.c — Fuzz harness for E-RTMP v2 ModEx parser
 */
#include <stdint.h>
#include <stddef.h>
#include "ertmp/ertmp.h"

int fuzz_modex(const uint8_t *data, size_t size) {
    if (size < 1) return 0;

    lrtmp2_modex_t modex;
    lrtmp2_ertmp_modex_parse(&modex, data, size);

    if (modex.type == LRTMP2_MODEX_TYPE_TIMESTAMP) {
        uint8_t buf[16];
        lrtmp2_ertmp_modex_write(&modex, buf, sizeof(buf));
    }

    return 0;
}
