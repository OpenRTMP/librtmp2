/**
 * script_tag.c — FLV script data (metadata) parser
 */
#include "flv/script_tag.h"
#include "amf/amf.h"
#include "core/log.h"
#include <string.h>
#include "librtmp2/types.h"

int lrtmp2_script_tag_parse(const uint8_t *data, size_t len, lrtmp2_script_tag_t *tag)
{
    if (!data || !tag || len < 2) return LRTMP2_ERR_INTERNAL;

    /* Script tag is typically an AMF0 ECMA array with metadata */
    lrtmp2_buffer_t buf;
    buf.data = (uint8_t *)data;
    buf.size = len;
    buf.read_pos = 0;
    buf.capacity = len;

    /* First value is usually a string "onMetaData" */
    amf0_type_t type;
    if (lrtmf2_amf0_read_type(&buf, &type) != LRTMP2_OK) return LRTMP2_ERR_AMF;

    if (type == AMF0_STRING) {
        size_t slen;
        if (lrtmf2_amf0_read_string(&buf, tag->name, sizeof(tag->name), &slen) != LRTMP2_OK)
            return LRTMP2_ERR_AMF;
    } else if (type == AMF0_LONG_STRING) {
        size_t slen;
        if (lrtmf2_amf0_read_long_string(&buf, tag->name, sizeof(tag->name), &slen) != LRTMP2_OK)
            return LRTMP2_ERR_AMF;
    } else {
        if (lrtmf2_amf0_skip_value(&buf) != LRTMP2_OK)
            return LRTMP2_ERR_AMF;
    }

    /* Second value is the metadata, usually an ECMA array (sometimes a plain
     * object). The type marker is consumed by read_type() here, so we must NOT
     * call read_object_begin() afterwards (it would consume a second byte and
     * desync the stream — for an ECMA array that byte is the first of the 4-byte
     * associative count). */
    if (lrtmf2_amf0_read_type(&buf, &type) == LRTMP2_OK) {
        if (type == AMF0_ECMA_ARRAY || type == AMF0_OBJECT) {
            if (type == AMF0_ECMA_ARRAY) {
                /* Consume the 4-byte associative-element count. */
                uint8_t count_bytes[4];
                if (lrtmp2_buffer_read(&buf, count_bytes, 4) != LRTMP2_OK)
                    return LRTMP2_ERR_AMF;
            }
            /* Skip to end — full metadata parsing is optional. Bail on any
             * read failure so malformed/truncated input (e.g. an array with
             * no end marker) cannot spin this loop forever. */
            while (!lrtmf2_amf0_is_object_end(&buf)) {
                char key[256];
                size_t klen;
                if (lrtmf2_amf0_read_object_key(&buf, key, sizeof(key), &klen) != LRTMP2_OK)
                    return LRTMP2_ERR_AMF;
                if (lrtmf2_amf0_skip_value(&buf) != LRTMP2_OK)
                    return LRTMP2_ERR_AMF;
            }
            uint8_t end[3];
            lrtmp2_buffer_read(&buf, end, 3);
        }
    }

    tag->data = data;
    tag->size = len;
    return LRTMP2_OK;
}
