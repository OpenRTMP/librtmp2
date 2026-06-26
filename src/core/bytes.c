/**
 * bytes.c — Byte-swapping and endian helpers
 */
#include "core/bytes.h"

uint16_t lrtmp2_byteswap16(uint16_t val)
{
    return (uint16_t)((val >> 8) | (val << 8));
}

uint32_t lrtmp2_byteswap32(uint32_t val)
{
    return ((val >> 24) & 0xFF)       |
           ((val >> 8)  & 0xFF00)     |
           ((val << 8)  & 0xFF0000)   |
           ((val << 24) & 0xFF000000);
}

uint64_t lrtmp2_byteswap64(uint64_t val)
{
    return ((val >> 56) & 0xFFULL)        |
           ((val >> 40) & 0xFF00ULL)       |
           ((val >> 24) & 0xFF0000ULL)     |
           ((val >> 8)  & 0xFF000000ULL)   |
           ((val << 8)  & 0xFF00000000ULL) |
           ((val << 24) & 0xFF0000000000ULL) |
           ((val << 40) & 0xFF000000000000ULL) |
           ((val << 56) & 0xFF00000000000000ULL);
}

uint32_t lrtmp2_ntoh24(const uint8_t *buf)
{
    return ((uint32_t)buf[0] << 16) |
           ((uint32_t)buf[1] << 8)  |
           ((uint32_t)buf[2]);
}

void lrtmp2_hton24(uint8_t *buf, uint32_t val)
{
    buf[0] = (uint8_t)((val >> 16) & 0xFF);
    buf[1] = (uint8_t)((val >> 8) & 0xFF);
    buf[2] = (uint8_t)(val & 0xFF);
}

uint32_t lrtmp2_hton32(uint32_t val)
{
    uint32_t result;
    uint8_t *p = (uint8_t *)&result;
    p[0] = (uint8_t)((val >> 24) & 0xFF);
    p[1] = (uint8_t)((val >> 16) & 0xFF);
    p[2] = (uint8_t)((val >> 8) & 0xFF);
    p[3] = (uint8_t)(val & 0xFF);
    return result;
}

uint32_t lrtmp2_ntoh32(const uint8_t *buf)
{
    return ((uint32_t)buf[0] << 24) |
           ((uint32_t)buf[1] << 16) |
           ((uint32_t)buf[2] << 8)  |
           ((uint32_t)buf[3]);
}
