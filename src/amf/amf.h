#ifndef LRTMP2_AMF_H
#define LRTMP2_AMF_H

#include "core/buffer.h"
#include "core/bytes.h"
#include "librtmp2/types.h"

/* AMF0 type markers */
typedef enum {
    AMF0_NUMBER     = 0x00,
    AMF0_BOOLEAN    = 0x01,
    AMF0_STRING     = 0x02,
    AMF0_OBJECT     = 0x03,
    AMF0_MOVIECLIP  = 0x04,  /* reserved */
    AMF0_NULL       = 0x05,
    AMF0_UNDEFINED  = 0x06,
    AMF0_REFERENCE  = 0x07,
    AMF0_ECMA_ARRAY = 0x08,
    AMF0_OBJECT_END = 0x09,
    AMF0_STRICT_ARRAY = 0x0A,
    AMF0_DATE       = 0x0B,
    AMF0_LONG_STRING = 0x0C,
    AMF0_UNSUPPORTED = 0x0D,
    AMF0_RECORDSET  = 0x0E,  /* reserved */
    AMF0_XML_DOC    = 0x0F,
    AMF0_TYPED_OBJECT = 0x10,
    AMF0_AVMPLUS    = 0x11,  /* switches to AMF3 */
} amf0_type_t;

/* AMF0 encoder */
int lrtmf2_amf0_write_number(lrtmp2_buffer_t *buf, double value);
int lrtmf2_amf0_write_boolean(lrtmp2_buffer_t *buf, int value);
int lrtmf2_amf0_write_null(lrtmp2_buffer_t *buf);
int lrtmf2_amf0_write_undefined(lrtmp2_buffer_t *buf);
int lrtmf2_amf0_write_string(lrtmp2_buffer_t *buf, const char *str);
int lrtmf2_amf0_write_long_string(lrtmp2_buffer_t *buf, const char *str);
int lrtmf2_amf0_write_object_begin(lrtmp2_buffer_t *buf);
int lrtmf2_amf0_write_object_end(lrtmp2_buffer_t *buf);
int lrtmf2_amf0_write_object_key(lrtmp2_buffer_t *buf, const char *key);
int lrtmf2_amf0_write_ecma_array_begin(lrtmp2_buffer_t *buf, uint32_t count);

/* AMF0 decoder */
int lrtmf2_amf0_read_type(lrtmp2_buffer_t *buf, amf0_type_t *type);
int lrtmf2_amf0_read_number(lrtmp2_buffer_t *buf, double *val);
int lrtmf2_amf0_read_boolean(lrtmp2_buffer_t *buf, int *val);
int lrtmf2_amf0_read_string(lrtmp2_buffer_t *buf, char *out, size_t max_len, size_t *out_len);
int lrtmf2_amf0_read_long_string(lrtmp2_buffer_t *buf, char *out, size_t max_len, size_t *out_len);
int lrtmf2_amf0_read_object_begin(lrtmp2_buffer_t *buf);
int lrtmf2_amf0_read_object_key(lrtmp2_buffer_t *buf, char *out, size_t max_len, size_t *out_len);
int lrtmf2_amf0_is_object_end(lrtmp2_buffer_t *buf);
int lrtmf2_amf0_skip_value(lrtmp2_buffer_t *buf);

const char *amf0_type_string(amf0_type_t type);

#endif
