/**
 * multitrack.c — Enhanced RTMP v2 multitrack streaming
 *
 * Per E-RTMP v2 §14, a track descriptor is a record that associates a
 * track name with a media type (audio, video, metadata). It is used to
 * declare multiple tracks within a single stream.
 *
 * Serialised structure (8 + name_len bytes):
 *   track_type(4, AMF0 number == enum value) +
 *   track_name(AMF0 string: 2-byte len + N bytes)
 */

#include "ertmp.h"
#include <string.h>

int lrtmp2_ertmp_multitrack_parse(lrtmp2_multitrack_t *mt, const uint8_t *data, size_t len)
{
    /* Need at least the AMF0_NUMBER marker (1) + the 8-byte value before the
     * track-name field. The name-length read below is guarded separately. */
    if (!mt || !data || len < 9) return LRTMP2_ERR_IO;

    memset(mt, 0, sizeof(*mt));

    /* track_type is sent as AMF0_NUMBER (UI64 big-endian) */
    if (data[0] != 0x00) return LRTMP2_ERR_PROTOCOL;

    /* Read the type from bytes 1..8 as a 64-bit value */
    uint64_t type_val = 0;
    for (int i = 0; i < 8; i++) {
        type_val = (type_val << 8) | data[1 + i];
    }
    mt->type = (lrtmp2_multitrack_type_t)(uint32_t)type_val;

    /* track_name is AMF0_STRING: 2-byte len + N bytes */
    size_t name_offset = 9;
    if (name_offset + 2 > len) return LRTMP2_ERR_IO;
    uint16_t name_len = (uint16_t)((data[name_offset] << 8) | data[name_offset + 1]);
    name_offset += 2;
    if (name_offset + name_len > len) return LRTMP2_ERR_IO;

    if (name_len >= sizeof(mt->track_name)) name_len = sizeof(mt->track_name) - 1;
    memcpy(mt->track_name, &data[name_offset], name_len);
    mt->track_name[name_len] = '\0';
    return LRTMP2_OK;
}

size_t lrtmp2_ertmp_multitrack_write(const lrtmp2_multitrack_t *mt, uint8_t *buf, size_t buf_size)
{
    if (!mt || !buf) return 0;

    size_t name_len = strlen(mt->track_name);
    /* The name length is carried in a 2-byte AMF0 string-length field; a longer
     * name could not be represented (the copied bytes and the length would
     * disagree), so refuse it rather than emit a corrupt record. */
    if (name_len > 0xFFFF) return 0;
    size_t needed = 1 + 8 + 2 + name_len; /* marker(1) + number(8) + str_len(2) + name(N) */
    if (buf_size < needed) return 0;

    /* AMF0_NUMBER marker */
    *buf++ = 0x00;
    /* 8-byte number, big-endian (matches lrtmp2_ertmp_multitrack_parse) */
    uint64_t type_val = (uint64_t)mt->type;
    for (int i = 7; i >= 0; i--) *buf++ = (uint8_t)((type_val >> (i * 8)) & 0xFF);
    /* AMF0_STRING: 2-byte length + N bytes */
    *buf++ = (uint8_t)(name_len >> 8);
    *buf++ = (uint8_t)(name_len);
    memcpy(buf, mt->track_name, name_len);
    buf += name_len;
    return needed;
}
