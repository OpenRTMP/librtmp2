/**
 * Additional ERTMP stubs
 */
#include "ertmp.h"
#include "librtmp2/types.h"

int lrtmp2_ertmp_metadata_parse(const uint8_t *data, size_t len, char *buf, size_t buf_size) {
    (void)data; (void)len; (void)buf; (void)buf_size;
    return LRTMP2_ERR_UNSUPPORTED;
}

int lrtmp2_ertmp_caps_negotiate(lrtmp2_buffer_t *buf) {
    (void)buf;
    return LRTMP2_ERR_UNSUPPORTED;
}
