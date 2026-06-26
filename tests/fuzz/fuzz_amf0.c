/**
 * fuzz_amf0.c — Fuzz harness for AMF0 decoder
 */
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include "amf/amf.h"
#include "core/buffer.h"

int fuzz_amf0(const uint8_t *data, size_t size) {
    if (size < 1) return 0;

    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    if (!buf) return 0;
    lrtmp2_buffer_write(buf, data, size);

    for (int i = 0; i < 32 && lrtmp2_buffer_available(buf) > 0; i++) {
        amf0_type_t type;
        int rc = lrtmf2_amf0_read_type(buf, &type);
        if (rc != 0) break;

        switch (type) {
            case AMF0_NUMBER: {
                double val;
                lrtmf2_amf0_read_number(buf, &val);
                break;
            }
            case AMF0_BOOLEAN: {
                int val;
                lrtmf2_amf0_read_boolean(buf, &val);
                break;
            }
            case AMF0_STRING: {
                char str[256];
                size_t slen;
                lrtmf2_amf0_read_string(buf, str, sizeof(str), &slen);
                break;
            }
            case AMF0_LONG_STRING: {
                char str[1024];
                size_t slen;
                lrtmf2_amf0_read_long_string(buf, str, sizeof(str), &slen);
                break;
            }
            case AMF0_OBJECT: {
                lrtmf2_amf0_read_object_begin(buf);
                for (int j = 0; j < 16 && lrtmp2_buffer_available(buf) > 0; j++) {
                    char key[128];
                    size_t klen;
                    rc = lrtmf2_amf0_read_object_key(buf, key, sizeof(key), &klen);
                    if (rc != 0) break;
                    amf0_type_t vtype;
                    rc = lrtmf2_amf0_read_type(buf, &vtype);
                    if (rc != 0) break;
                    switch (vtype) {
                        case AMF0_NUMBER: { double v; lrtmf2_amf0_read_number(buf, &v); break; }
                        case AMF0_BOOLEAN: { int v; lrtmf2_amf0_read_boolean(buf, &v); break; }
                        case AMF0_STRING: { char s[256]; size_t l; lrtmf2_amf0_read_string(buf, s, sizeof(s), &l); break; }
                        case AMF0_LONG_STRING: { char s[1024]; size_t l; lrtmf2_amf0_read_long_string(buf, s, sizeof(s), &l); break; }
                        default: goto done;
                    }
                }
                done:
                break;
            }
            case AMF0_NULL:
            case AMF0_UNDEFINED:
                break;
            default:
                goto out;
        }
    }

out:
    lrtmp2_buffer_destroy(buf);
    return 0;
}
