#ifndef LRTMP2_CORE_BYTES_H
#define LRTMP2_CORE_BYTES_H

#include <stdint.h>
#include "librtmp2/types.h"

uint16_t lrtmp2_byteswap16(uint16_t val);
uint32_t lrtmp2_byteswap32(uint32_t val);
uint64_t lrtmp2_byteswap64(uint64_t val);

uint32_t lrtmp2_ntoh24(const uint8_t *buf);
void     lrtmp2_hton24(uint8_t *buf, uint32_t val);

uint32_t lrtmp2_hton32(uint32_t val);
uint32_t lrtmp2_ntoh32(const uint8_t *buf);

#endif
