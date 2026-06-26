/**
 * modex.c — Enhanced RTMP v2 ModEx extension mechanism
 *
 * Per E-RTMP v2 §16, ModEx is a signalling mechanism that lets packets
 * carry modifiers or extensions. The two defined types are:
 *   NOP(0)       — placeholder, no-op
 *   TIMESTAMP(1) — nanosecond-precision timestamp offset
 *
 * Serialised structure:
 *   marker(1, 0x80 | type) +
 *   For TIMESTAMP: 8-byte offset (nanoseconds, big-endian UI64)
 *   For NOP: no additional bytes
 */

#include "ertmp.h"
#include <string.h>

#define LRTMP2_MODEX_MARKER 0x80

int lrtmp2_ertmp_modex_parse(lrtmp2_modex_t *modex, const uint8_t *data, size_t len)
{
    if (!modex || !data || len < 1) return LRTMP2_ERR_IO;

    memset(modex, 0, sizeof(*modex));

    uint8_t marker = data[0];
    if ((marker & 0x80) == 0) return LRTMP2_ERR_PROTOCOL; /* missing marker high bit */

    uint8_t type = marker & 0x7F;
    switch (type) {
        case LRTMP2_MODEX_TYPE_NOP:
            modex->type = LRTMP2_MODEX_TYPE_NOP;
            modex->offset = 0;
            return LRTMP2_OK;

        case LRTMP2_MODEX_TYPE_TIMESTAMP:
            if (len < 9) return LRTMP2_ERR_IO;
            modex->type = LRTMP2_MODEX_TYPE_TIMESTAMP;
            for (int i = 0; i < 8; i++) {
                modex->offset = (modex->offset << 8) | data[1 + i];
            }
            return LRTMP2_OK;

        default:
            /* Unknown ModEx type — ignore per §16 graceful-degradation rule */
            modex->type = LRTMP2_MODEX_TYPE_NOP;
            return LRTMP2_OK;
    }
}

size_t lrtmp2_ertmp_modex_write(const lrtmp2_modex_t *modex, uint8_t *buf, size_t buf_size)
{
    if (!modex || !buf || buf_size < 1) return 0;

    buf[0] = (uint8_t)(LRTMP2_MODEX_MARKER | (uint8_t)modex->type);

    switch (modex->type) {
        case LRTMP2_MODEX_TYPE_NOP:
            return 1;

        case LRTMP2_MODEX_TYPE_TIMESTAMP:
            if (buf_size < 9) return 0;
            {
                uint64_t tmp = modex->offset;
                for (int i = 7; i >= 0; i--) {
                    buf[1 + i] = (uint8_t)(tmp & 0xFF);
                    tmp >>= 8;
                }
            }
            return 9;

        default:
            return 1;
    }
}
