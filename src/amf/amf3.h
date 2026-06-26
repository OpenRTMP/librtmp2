#ifndef LRTMP2_AMF3_H
#define LRTMP2_AMF3_H

#include "core/buffer.h"
#include <stdint.h>
#include "librtmp2/types.h"

/* AMF3 encoder */
int lrtmf2_amf3_write_null(lrtmp2_buffer_t *buf);
int lrtmf2_amf3_write_integer(lrtmp2_buffer_t *buf, uint32_t val);
int lrtmf2_amf3_write_double(lrtmp2_buffer_t *buf, double val);
int lrtmf2_amf3_write_string(lrtmp2_buffer_t *buf, const char *str);

/* AMF3 decoder */
int lrtmf2_amf3_read_type(lrtmp2_buffer_t *buf, uint8_t *type);
int lrtmf2_amf3_read_null(lrtmp2_buffer_t *buf);
int lrtmf2_amf3_read_integer(lrtmp2_buffer_t *buf, uint32_t *val);
int lrtmf2_amf3_read_double(lrtmp2_buffer_t *buf, double *val);
int lrtmf2_amf3_read_boolean(lrtmp2_buffer_t *buf, int *val);
int lrtmf2_amf3_read_string(lrtmp2_buffer_t *buf, char *out, size_t max_len, size_t *out_len);

#endif
