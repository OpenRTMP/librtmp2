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

static int amf3_read_u32(lrtmp2_buffer_t *buf, uint32_t *val)
{
    uint8_t b[4];
    if (lrtmp2_buffer_read(buf, b, 4) != 0) return -1;
    *val = ((uint32_t)b[0] << 24) | ((uint32_t)b[1] << 16) | ((uint32_t)b[2] << 8) | b[3];
    return 0;
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
    return amf3_read_u32(buf, val) == 0 ? LRTMP2_OK : LRTMP2_ERR_IO;
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
    if (amf3_read_u32(buf, &ref) != 0) return LRTMP2_ERR_IO;

    /* U29 encoding: low bit = inline flag */
    uint32_t len = ref >> 1;
    int inline_bit = ref & 1;

    if (inline_bit == 0) {
        /* String reference — not fully implemented, just return empty */
        out[0] = '\0';
        *out_len = 0;
        return LRTMP2_OK;
    }

    if (len + 1 > max_len) return LRTMP2_ERR_AMF;

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
    uint32_t net = lrtmp2_byteswap32(val);
    return lrtmp2_buffer_write(buf, (uint8_t *)&net, 4);
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
    uint8_t t = AMF3_STRING;
    lrtmp2_buffer_write(buf, &t, 1);
    /* U29: (len << 1) | 1 = inline */
    uint32_t u29 = lrtmp2_byteswap32((uint32_t)((len << 1) | 1));
    lrtmp2_buffer_write(buf, (uint8_t *)&u29, 4);
    return lrtmp2_buffer_write(buf, (const uint8_t *)str, len);
}
