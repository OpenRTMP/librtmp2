/**
 * connect_caps.c — Enhanced RTMP v1 fourCcList + E-RTMP v2 capability layer
 *
 * Per E-RTMP v1 §6, the connect object may carry a "fourCcList" array
 * advertising supported FourCC codecs. This module parses and serializes
 * that list.
 *
 * Per E-RTMP v2 §11, the connect/response exchange carries a "capsEx"
 * object (videoCodecId + audioCodecId FourCC-encoded) and a
 * "videoFourCcInfoMap" array sent in the connect result.
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

/* ── E-RTMP v2 capsEx ──────────────────────────────────────────── */

int lrtmp2_ertmp_caps_exit_parse(lrtmp2_caps_exit_t *caps, const uint8_t *data, size_t len)
{
    if (!caps) return LRTMP2_ERR_INTERNAL;
    if (!data || len < 8) return LRTMP2_ERR_IO;

    memset(caps, 0, sizeof(*caps));
    caps->version = 1;
    caps->video_codec_32 = (int)((uint32_t)data[0]<<24 | (uint32_t)data[1]<<16 |
                                (uint32_t)data[2]<<8  | (uint32_t)data[3]);
    caps->audio_codec_32 = (int)((uint32_t)data[4]<<24 | (uint32_t)data[5]<<16 |
                                (uint32_t)data[6]<<8  | (uint32_t)data[7]);
    return LRTMP2_OK;
}

size_t lrtmp2_ertmp_caps_exit_write(const lrtmp2_caps_exit_t *caps, uint8_t *buf, size_t buf_size)
{
    if (!caps || !buf || buf_size < 8) return 0;

    uint32_t vc = (uint32_t)caps->video_codec_32;
    uint32_t ac = (uint32_t)caps->audio_codec_32;
    buf[0] = (uint8_t)(vc >> 24); buf[1] = (uint8_t)(vc >> 16);
    buf[2] = (uint8_t)(vc >> 8);  buf[3] = (uint8_t)(vc);
    buf[4] = (uint8_t)(ac >> 24); buf[5] = (uint8_t)(ac >> 16);
    buf[6] = (uint8_t)(ac >> 8);  buf[7] = (uint8_t)(ac);
    return 8;
}

/* ── E-RTMP v2 videoFourCcInfoMap ──────────────────────────────── */

int lrtmp2_ertmp_video_fourcc_info_map_parse(lrtmp2_video_fourcc_info_map_t *map,
                                              const uint8_t *data, size_t len)
{
    if (!map || !data || len < 4) return LRTMP2_ERR_IO;
    memset(map, 0, sizeof(*map));

    /* Same ECMAArray layout as fourCcList: U32 count × N × (2-byte len + 4-byte FourCC) */
    uint32_t count = ((uint32_t)data[0]<<24) | ((uint32_t)data[1]<<16) |
                    ((uint32_t)data[2]<<8)  |  (uint32_t)data[3];
    if (count > LRTMP2_MAX_FOURCCS) count = LRTMP2_MAX_FOURCCS;

    size_t offset = 4;
    for (uint32_t i = 0; i < count; i++) {
        if (offset + 6 > len) break;
        uint16_t slen = (uint16_t)((data[offset] << 8) | data[offset+1]);
        offset += 2;
        if (slen != 4 || offset + 4 > len) break;
        memcpy(map->entries[map->count].cc, &data[offset], 4);
        map->entries[map->count].cc[4] = '\0';
        map->count++;
        offset += 4;
    }
    return (int)map->count;
}

size_t lrtmp2_ertmp_video_fourcc_info_map_write(const lrtmp2_video_fourcc_info_map_t *map,
                                                 uint8_t *buf, size_t buf_size)
{
    if (!map || !buf) return 0;
    size_t needed = 4 + map->count * 6;
    if (buf_size < needed) return 0;

    buf[0] = (uint8_t)(map->count >> 24);
    buf[1] = (uint8_t)(map->count >> 16);
    buf[2] = (uint8_t)(map->count >> 8);
    buf[3] = (uint8_t)map->count;

    size_t offset = 4;
    for (size_t i = 0; i < map->count; i++) {
        buf[offset++] = 0; buf[offset++] = 4;
        memcpy(&buf[offset], map->entries[i].cc, 4);
        offset += 4;
    }
    return offset;
}


