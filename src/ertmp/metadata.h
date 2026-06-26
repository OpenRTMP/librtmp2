#ifndef LRTMP2_ERTMP_METADATA_H
#define LRTMP2_ERTMP_METADATA_H

#include "core/buffer.h"
#include <stdint.h>
#include <stddef.h>
#include "librtmp2/types.h"

int lrtmp2_ertmp_metadata_parse(const uint8_t *data, size_t len, char *buf, size_t buf_size);
int lrtmp2_ertmp_caps_negotiate(lrtmp2_buffer_t *buf);

#endif
