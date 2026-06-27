/**
 * amf3.c — AMF3 encoder/decoder (minimal implementation)
 */
#include "amf3.h"
#include "core/bytes.h"
#include "core/log.h"
#include <string.h>
#include "librtmp2/types.h"

/* AM3 type markers */
#define AMF3_UNDEFINED   0x00
#define AMF3_NULL        0x01
#define AMF3_FALSE       0x02
#define AMF3_TRUE        0x03
#define AMF3_INTEGER     0x04
#define AMF3_DOUBLE      0x05
#define AMF3_STRING      0x06
#define AMF3_XML_DOC     0x07
#define AMF3_DATE        0x08
#define AMF3_ARRAY       0x09
#define AMF3_OBJECT      0x0A
#define AMF3_XML         0x0B
#define AMF3_BYTE_ARRAY  0x0C

static int amf3_read_u8(lrtmp2_buffer_t *buf, uint8_t *val)
{
    return lrtmp2_buffer_read(buf, val, 1) == 0 ? 0 : -1;
}

/* Read a U29 variable-length integer (AMF3 §1.3.1): up to four bytes, the
 * first three contributing 7 bits each with bit 7 as a continuation flag; if a
 * fourth byte is reached it contributes all 8 bits. */
static int amf3_read_u29(lrtmp2_buffer_t *buf, uint32_t *val)
{
    uint32_t result = 0;
    uint8_t b;
    for (int i = 0; i < 4; i++) {
        if (amf3_read_u8(buf, &b) != 0) return -1;
        if (i < 3) {
            result = (result << 7) | (uint32_t)(b & 0x7F);
            if ((b & 0x80) == 0) {
                *val = result;
                return 0;
            }
        } else {
            result = (result << 8) | b;
        }
    }
    *val = result;
    return 0;
}

/* Write a 29-bit value in U29 variable-length form (1–4 bytes). */
static int amf3_write_u29(lrtmp2_buffer_t *buf, uint32_t val)
{
    uint8_t b[4];
    size_t n;
    val &= 0x1FFFFFFF;
    if (val < 0x80) {
        b[0] = (uint8_t)val;
        n = 1;
    } else if (val < 0x4000) {
        b[0] = (uint8_t)((val >> 7) | 0x80);
        b[1] = (uint8_t)(val & 0x7F);
        n = 2;
    } else if (val < 0x200000) {
        b[0] = (uint8_t)((val >> 14) | 0x80);
        b[1] = (uint8_t)(((val >> 7) & 0x7F) | 0x80);
        b[2] = (uint8_t)(val & 0x7F);
        n = 3;
    } else {
        b[0] = (uint8_t)((val >> 22) | 0x80);
        b[1] = (uint8_t)(((val >> 15) & 0x7F) | 0x80);
        b[2] = (uint8_t)(((val >> 8) & 0x7F) | 0x80);
        b[3] = (uint8_t)(val & 0xFF);
        n = 4;
    }
    return lrtmp2_buffer_write(buf, b, n);
}

static int amf3_read_double(lrtmp2_buffer_t *buf, double *val)
{
    uint8_t b[8];
    if (lrtmp2_buffer_read(buf, b, 8) != 0) return -1;
    uint64_t bits = 0;
    for (int i = 0; i < 8; i++) bits |= ((uint64_t)b[i]) << ((7 - i) * 8);
    memcpy(val, &bits, 8);
    return 0;
}

int lrtmf2_amf3_read_type(lrtmp2_buffer_t *buf, uint8_t *type)
{
    return amf3_read_u8(buf, type);
}

int lrtmf2_amf3_read_null(lrtmp2_buffer_t *buf)
{
    uint8_t t;
    if (amf3_read_u8(buf, &t) != 0) return LRTMP2_ERR_IO;
    if (t != AMF3_NULL) return LRTMP2_ERR_AMF;
    return LRTMP2_OK;
}

int lrtmf2_amf3_read_integer(lrtmp2_buffer_t *buf, uint32_t *val)
{
    return amf3_read_u29(buf, val) == 0 ? LRTMP2_OK : LRTMP2_ERR_IO;
}

int lrtmf2_amf3_read_double(lrtmp2_buffer_t *buf, double *val)
{
    return amf3_read_double(buf, val) == 0 ? LRTMP2_OK : LRTMP2_ERR_IO;
}

int lrtmf2_amf3_read_boolean(lrtmp2_buffer_t *buf, int *val)
{
    uint8_t t;
    if (amf3_read_u8(buf, &t) != 0) return LRTMP2_ERR_IO;
    if (t == AMF3_TRUE) *val = 1;
    else if (t == AMF3_FALSE) *val = 0;
    else return LRTMP2_ERR_AMF;
    return LRTMP2_OK;
}

int lrtmf2_amf3_read_string(lrtmp2_buffer_t *buf, char *out, size_t max_len, size_t *out_len)
{
    uint8_t t;
    if (amf3_read_u8(buf, &t) != 0 || t != AMF3_STRING) return LRTMP2_ERR_AMF;

    uint32_t ref;
    if (amf3_read_u29(buf, &ref) != 0) return LRTMP2_ERR_IO;

    /* U29 encoding: low bit = inline flag */
    uint32_t len = ref >> 1;
    int inline_bit = ref & 1;

    if (inline_bit == 0) {
        /* String reference — not fully implemented, just return empty */
        out[0] = '\0';
        *out_len = 0;
        return LRTMP2_OK;
    }

    if (max_len == 0 || len >= max_len) return LRTMP2_ERR_AMF;

    if (lrtmp2_buffer_read(buf, (uint8_t *)out, len) != 0) return LRTMP2_ERR_IO;
    out[len] = '\0';
    *out_len = len;
    return LRTMP2_OK;
}

int lrtmf2_amf3_write_null(lrtmp2_buffer_t *buf)
{
    uint8_t t = AMF3_NULL;
    return lrtmp2_buffer_write(buf, &t, 1);
}

int lrtmf2_amf3_write_integer(lrtmp2_buffer_t *buf, uint32_t val)
{
    uint8_t t = AMF3_INTEGER;
    lrtmp2_buffer_write(buf, &t, 1);
    return amf3_write_u29(buf, val);
}

int lrtmf2_amf3_write_double(lrtmp2_buffer_t *buf, double val)
{
    uint8_t t = AMF3_DOUBLE;
    lrtmp2_buffer_write(buf, &t, 1);
    uint64_t bits;
    memcpy(&bits, &val, 8);
    uint8_t b[8];
    for (int i = 0; i < 8; i++) b[i] = (uint8_t)(bits >> ((7 - i) * 8));
    return lrtmp2_buffer_write(buf, b, 8);
}

int lrtmf2_amf3_write_string(lrtmp2_buffer_t *buf, const char *str)
{
    size_t len = strlen(str);
    /* The inline length is carried in the high 28 bits of a U29 (low bit is the
     * inline flag), so lengths must fit in 28 bits. */
    if (len > 0x0FFFFFFF) return LRTMP2_ERR_AMF;
    uint8_t t = AMF3_STRING;
    lrtmp2_buffer_write(buf, &t, 1);
    /* U29: (len << 1) | 1 = inline */
    int rc = amf3_write_u29(buf, (uint32_t)((len << 1) | 1));
    if (rc != LRTMP2_OK) return rc;
    return lrtmp2_buffer_write(buf, (const uint8_t *)str, len);
}
