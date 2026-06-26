/**
 * connect_caps.c — Enhanced RTMP v1 fourCcList / E-RTMP v2 capability stubs
 *
 * Per E-RTMP v1 §6, the connect object may carry a "fourCcList" array
 * advertising supported FourCC codecs. This module parses and serializes
 * that list.
 *
 * E-RTMP v2 caps negotiation (capsEx, videoFourCcInfoMap) is stubbed
 * here — full v2 support is Phase 4.
 */
#include "ertmp.h"
#include "librtmp2/types.h"
#include <stddef.h>
#include <string.h>

/* ── fourCcList ───────────────────────────────────────────────────── */

int lrtmp2_ertmp_fourcc_list_init(lrtmp2_fourcc_list_t *list) {
    if (!list) return LRTMP2_ERR_INTERNAL;
    memset(list, 0, sizeof(*list));
    return LRTMP2_OK;
}

int lrtmp2_ertmp_fourcc_list_add(lrtmp2_fourcc_list_t *list, const char *cc) {
    if (!list || !cc) return LRTMP2_ERR_INTERNAL;
    if (list->count >= LRTMP2_MAX_FOURCCS) return LRTMP2_ERR_IO;
    memcpy(list->entries[list->count].cc, cc, 4);
    list->entries[list->count].cc[4] = '\0';
    list->count++;
    return LRTMP2_OK;
}

/* Parse a raw AMF0 ECMAArray blob that contains fourCcList entries.
 * Format: count (U32) × N strings of 4-byte FourCC.
 * Returns number of entries parsed, or negative error. */
int lrtmp2_ertmp_fourcc_list_parse(lrtmp2_fourcc_list_t *list,
                                    const uint8_t *data, size_t len)
{
    if (!list || !data || len < 4) return LRTMP2_ERR_IO;
    lrtmp2_ertmp_fourcc_list_init(list);

    /* Read count as big-endian U32 (AMF0 encoded) */
    uint32_t count = ((uint32_t)data[0]<<24) | ((uint32_t)data[1]<<16) |
                    ((uint32_t)data[2]<<8)  |  (uint32_t)data[3];
    if (count > LRTMP2_MAX_FOURCCS) count = LRTMP2_MAX_FOURCCS;

    size_t offset = 4;
    for (uint32_t i = 0; i < count; i++) {
        /* Each entry: 2-byte length (should be 4) + 4-byte FourCC */
        if (offset + 6 > len) break;
        uint16_t slen = (uint16_t)((data[offset] << 8) | data[offset+1]);
        offset += 2;
        if (slen != 4 || offset + 4 > len) break;
        memcpy(list->entries[list->count].cc, &data[offset], 4);
        list->entries[list->count].cc[4] = '\0';
        list->count++;
        offset += 4;
    }
    return (int)list->count;
}

size_t lrtmp2_ertmp_fourcc_list_write(const lrtmp2_fourcc_list_t *list,
                                       uint8_t *buf, size_t buf_size)
{
    if (!list || !buf) return 0;
    size_t needed = 4 + list->count * 6;
    if (buf_size < needed) return 0;

    /* Count */
    buf[0] = (uint8_t)(list->count >> 24);
    buf[1] = (uint8_t)(list->count >> 16);
    buf[2] = (uint8_t)(list->count >> 8);
    buf[3] = (uint8_t)list->count;

    size_t offset = 4;
    for (size_t i = 0; i < list->count; i++) {
        buf[offset++] = 0; buf[offset++] = 4; /* length = 4 */
        memcpy(&buf[offset], list->entries[i].cc, 4);
        offset += 4;
    }
    return offset;
}


