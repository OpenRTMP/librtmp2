/**
 * amf0.c — AMF0 encoder/decoder
 *
 * AMF0 types:
 *   0x00 Number (double, 8 bytes)
 *   0x01 Boolean (1 byte)
 *   0x02 String (2-byte length + UTF-8)
 *   0x03 Object (key-value pairs, terminated by 0x00 0x00 0x09)
 *   0x05 Null
 *   0x06 Undefined
 *   0x08 ECMA Array (4-byte count + key-value pairs + end marker)
 *   0x0C Long String (4-byte length + UTF-8)
 *   0x12 Typed Object (class name + object)
 */
#include "amf.h"
#include "core/bytes.h"
#include "core/log.h"
#include "core/alloc.h"
#include <string.h>
#include <stdlib.h>
#include <math.h>
#include "librtmp2/types.h"

/* ==================== HELPERS ==================== */

static int amf_read_u8(lrtmp2_buffer_t *buf, uint8_t *val)
{
    return lrtmp2_buffer_read(buf, val, 1) == 0 ? LRTMP2_OK : LRTMP2_ERR_IO;
}

static int amf_read_u16(lrtmp2_buffer_t *buf, uint16_t *val)
{
    uint8_t b[2];
    if (lrtmp2_buffer_read(buf, b, 2) != 0) return LRTMP2_ERR_IO;
    /* AMF0 stores multi-byte values in big-endian (network byte order) */
    *val = (uint16_t)((b[0] << 8) | b[1]);
    return LRTMP2_OK;
}

static int amf_read_u32(lrtmp2_buffer_t *buf, uint32_t *val)
{
    uint8_t b[4];
    if (lrtmp2_buffer_read(buf, b, 4) != 0) return LRTMP2_ERR_IO;
    *val = ((uint32_t)b[0] << 24) | ((uint32_t)b[1] << 16) | ((uint32_t)b[2] << 8) | b[3];
    return LRTMP2_OK;
}

static int amf_read_i32(lrtmp2_buffer_t *buf, int32_t *val)
{
    return amf_read_u32(buf, (uint32_t *)val);
}

static int amf_read_double(lrtmp2_buffer_t *buf, double *val)
{
    uint8_t b[8];
    if (lrtmp2_buffer_read(buf, b, 8) != 0) return LRTMP2_ERR_IO;
    /* IEEE 754 big-endian */
    uint64_t bits = 0;
    for (int i = 0; i < 8; i++) {
        bits |= ((uint64_t)b[i]) << ((7 - i) * 8);
    }
    memcpy(val, &bits, 8);
    return LRTMP2_OK;
}

/* ==================== ENCODER ==================== */

int lrtmf2_amf0_write_number(lrtmp2_buffer_t *buf, double value)
{
    uint8_t hdr = AMF0_NUMBER;
    lrtmp2_buffer_write(buf, &hdr, 1);
    uint64_t bits;
    memcpy(&bits, &value, 8);
    uint8_t b[8];
    for (int i = 0; i < 8; i++) {
        b[i] = (uint8_t)(bits >> ((7 - i) * 8));
    }
    return lrtmp2_buffer_write(buf, b, 8);
}

int lrtmf2_amf0_write_boolean(lrtmp2_buffer_t *buf, int value)
{
    uint8_t hdr = AMF0_BOOLEAN;
    lrtmp2_buffer_write(buf, &hdr, 1);
    uint8_t val = value ? 1 : 0;
    return lrtmp2_buffer_write(buf, &val, 1);
}

int lrtmf2_amf0_write_null(lrtmp2_buffer_t *buf)
{
    uint8_t hdr = AMF0_NULL;
    return lrtmp2_buffer_write(buf, &hdr, 1);
}

int lrtmf2_amf0_write_undefined(lrtmp2_buffer_t *buf)
{
    uint8_t hdr = AMF0_UNDEFINED;
    return lrtmp2_buffer_write(buf, &hdr, 1);
}

int lrtmf2_amf0_write_string(lrtmp2_buffer_t *buf, const char *str)
{
    size_t len = strlen(str);
    if (len > UINT16_MAX) return LRTMP2_ERR_AMF;

    uint8_t hdr = AMF0_STRING;
    lrtmp2_buffer_write(buf, &hdr, 1);
    uint16_t net_len = lrtmp2_byteswap16((uint16_t)len);
    lrtmp2_buffer_write(buf, (uint8_t *)&net_len, 2);
    return lrtmp2_buffer_write(buf, (const uint8_t *)str, len);
}

int lrtmf2_amf0_write_long_string(lrtmp2_buffer_t *buf, const char *str)
{
    size_t len = strlen(str);
    if (len > UINT32_MAX) return LRTMP2_ERR_AMF;

    uint8_t hdr = AMF0_LONG_STRING;
    lrtmp2_buffer_write(buf, &hdr, 1);
    uint32_t net_len = lrtmp2_byteswap32((uint32_t)len);
    lrtmp2_buffer_write(buf, (uint8_t *)&net_len, 4);
    return lrtmp2_buffer_write(buf, (const uint8_t *)str, len);
}

int lrtmf2_amf0_write_object_begin(lrtmp2_buffer_t *buf)
{
    uint8_t hdr = AMF0_OBJECT;
    return lrtmp2_buffer_write(buf, &hdr, 1);
}

int lrtmf2_amf0_write_object_end(lrtmp2_buffer_t *buf)
{
    /* 0x00 0x00 0x09 = empty key + object end marker */
    uint8_t end[3] = {0x00, 0x00, 0x09};
    return lrtmp2_buffer_write(buf, end, 3);
}

int lrtmf2_amf0_write_object_key(lrtmp2_buffer_t *buf, const char *key)
{
    size_t len = strlen(key);
    if (len > UINT16_MAX) return LRTMP2_ERR_AMF;
    uint16_t net_len = lrtmp2_byteswap16((uint16_t)len);
    return lrtmp2_buffer_write(buf, (uint8_t *)&net_len, 2);
    /* Note: value follows separately */
}

int lrtmf2_amf0_write_ecma_array_begin(lrtmp2_buffer_t *buf, uint32_t count)
{
    uint8_t hdr = AMF0_ECMA_ARRAY;
    lrtmp2_buffer_write(buf, &hdr, 1);
    uint32_t net_count = lrtmp2_byteswap32(count);
    return lrtmp2_buffer_write(buf, (uint8_t *)&net_count, 4);
}

/* ==================== DECODER ==================== */

int lrtmf2_amf0_read_type(lrtmp2_buffer_t *buf, amf0_type_t *type)
{
    uint8_t t;
    int rc = amf_read_u8(buf, &t);
    if (rc != LRTMP2_OK) return rc;
    *type = (amf0_type_t)t;
    return LRTMP2_OK;
}

int lrtmf2_amf0_read_number(lrtmp2_buffer_t *buf, double *val)
{
    return amf_read_double(buf, val);
}

int lrtmf2_amf0_read_boolean(lrtmp2_buffer_t *buf, int *val)
{
    uint8_t b;
    int rc = amf_read_u8(buf, &b);
    if (rc != LRTMP2_OK) return rc;
    *val = b ? 1 : 0;
    return LRTMP2_OK;
}

int lrtmf2_amf0_read_string(lrtmp2_buffer_t *buf, char *out, size_t max_len, size_t *out_len)
{
    uint8_t type;
    int rc = amf_read_u8(buf, &type);
    if (rc != LRTMP2_OK) return rc;
    if (type != AMF0_STRING) return LRTMP2_ERR_AMF;

    uint16_t str_len;
    rc = amf_read_u16(buf, &str_len);
    if (rc != LRTMP2_OK) return rc;

    if (str_len + 1 > max_len) return LRTMP2_ERR_AMF;

    rc = lrtmp2_buffer_read(buf, (uint8_t *)out, str_len);
    if (rc != LRTMP2_OK) return rc;
    out[str_len] = '\0';
    *out_len = str_len;
    return LRTMP2_OK;
}

int lrtmf2_amf0_read_long_string(lrtmp2_buffer_t *buf, char *out, size_t max_len, size_t *out_len)
{
    uint32_t str_len;
    int rc = amf_read_u32(buf, &str_len);
    if (rc != LRTMP2_OK) return rc;
    /* str_len already in host order from amf_read_u32 */

    if (str_len + 1 > max_len) return LRTMP2_ERR_AMF;

    rc = lrtmp2_buffer_read(buf, (uint8_t *)out, str_len);
    if (rc != LRTMP2_OK) return rc;
    out[str_len] = '\0';
    *out_len = str_len;
    return LRTMP2_OK;
}

int lrtmf2_amf0_read_object_begin(lrtmp2_buffer_t *buf)
{
    uint8_t type;
    int rc = amf_read_u8(buf, &type);
    if (rc != LRTMP2_OK) return rc;
    if (type != AMF0_OBJECT) return LRTMP2_ERR_AMF;
    return LRTMP2_OK;
}

int lrtmf2_amf0_read_object_key(lrtmp2_buffer_t *buf, char *out, size_t max_len, size_t *out_len)
{
    /* Object key is a 2-byte length + UTF-8 string (no type marker) */
    uint16_t klen;
    int rc = amf_read_u16(buf, &klen);
    if (rc != LRTMP2_OK) return rc;

    if (klen + 1 > max_len) return LRTMP2_ERR_AMF;

    rc = lrtmp2_buffer_read(buf, (uint8_t *)out, klen);
    if (rc != LRTMP2_OK) return rc;
    out[klen] = '\0';
    *out_len = klen;
    return LRTMP2_OK;
}

int lrtmf2_amf0_is_object_end(lrtmp2_buffer_t *buf)
{
    /* Peeks next 3 bytes for 0x00 0x00 0x09 */
    const uint8_t *peek;
    size_t avail = lrtmp2_buffer_peek(buf, &peek);
    if (avail < 3) return 0;  /* not enough data to tell */
    return (peek[0] == 0x00 && peek[1] == 0x00 && peek[2] == 0x09) ? 1 : 0;
}

int lrtmf2_amf0_skip_value(lrtmp2_buffer_t *buf)
{
    amf0_type_t type;
    int rc = lrtmf2_amf0_read_type(buf, &type);
    if (rc != LRTMP2_OK) return rc;

    switch (type) {
        case AMF0_NUMBER:
            return amf_read_double(buf, NULL);
            {
                double tmp;
                return amf_read_double(buf, &tmp);
            }
        case AMF0_BOOLEAN:
            {
                uint8_t b;
                return amf_read_u8(buf, &b);
            }
        case AMF0_STRING:
            {
                uint16_t len;
                rc = amf_read_u16(buf, &len);
                if (rc != LRTMP2_OK) return rc;
                /* Advance read position */
                for (uint16_t i = 0; i < len; i++) {
                    uint8_t b;
                    rc = amf_read_u8(buf, &b);
                    if (rc != LRTMP2_OK) return rc;
                }
                return LRTMP2_OK;
            }
        case AMF0_LONG_STRING:
            {
                uint32_t len;
                rc = amf_read_u32(buf, &len);
                if (rc != LRTMP2_OK) return rc;
                len = len;
                for (uint32_t i = 0; i < len; i++) {
                    uint8_t b;
                    rc = amf_read_u8(buf, &b);
                    if (rc != LRTMP2_OK) return rc;
                }
                return LRTMP2_OK;
            }
        case AMF0_OBJECT:
        case AMF0_ECMA_ARRAY:
            /* Skip key-value pairs until end marker */
            while (1) {
                /* Check for object end */
                if (lrtmf2_amf0_is_object_end(buf)) {
                    /* Consume 3 bytes (0x00 0x00 0x09) */
                    uint8_t end[3];
                    lrtmp2_buffer_read(buf, end, 3);
                    return LRTMP2_OK;
                }
                /* Skip key (2 bytes) */
                uint16_t klen;
                rc = amf_read_u16(buf, &klen);
                if (rc != LRTMP2_OK) return rc;
                /* klen already in host order */
                for (uint16_t i = 0; i < klen; i++) {
                    uint8_t b;
                    rc = amf_read_u8(buf, &b);
                    if (rc != LRTMP2_OK) return rc;
                }
                /* Skip value */
                rc = lrtmf2_amf0_skip_value(buf);
                if (rc != LRTMP2_OK) return rc;
            }
            break;
        case AMF0_NULL:
        case AMF0_UNDEFINED:
            return LRTMP2_OK;
        default:
            LRTMP2_LOG_WARN("Skipping unsupported AMF0 type 0x%02x", type);
            return LRTMP2_ERR_UNSUPPORTED;
    }

    return LRTMP2_OK;
}
