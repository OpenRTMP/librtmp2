/**
 * ERTMP stubs — place holders for E-RTMP v1/v2 extensions
 */

#include "ertmp.h"
#include <string.h>
#include "librtmp2/types.h"

/* ExVideoTagHeader for E-RTMP v1 */
int lrtmp2_ertmp_exvideo_parse(const uint8_t *data, size_t len,
                                lrtmp2_video_header_t *hdr) {
    (void)data; (void)len; (void)hdr;
    return LRTMP2_ERR_UNSUPPORTED;
}

int lrtmp2_ertmp_fourcc_parse(const uint8_t *data, size_t len, lrtmp2_fourcc_t *fourcc) {
    (void)data; (void)len;
    if (fourcc) memset(fourcc, 0, sizeof(*fourcc));
    return LRTMP2_OK;
}

/* Placeholder implementation — full code in Phase 3/4 */
const char *lrtmp2_ertmp_version_string(void) {
    return "E-RTMP v1/v2 (stub)";
}
