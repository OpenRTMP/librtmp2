/**
 * fuzz_ertmp_video.c — Fuzz harness for E-RTMP ExVideoTagHeader parser
 */
#include <stdint.h>
#include <stddef.h>
#include "ertmp/ertmp.h"

int fuzz_ertmp_video(const uint8_t *data, size_t size) {
    if (size < 1) return 0;

    lrtmp2_video_header_t hdr;
    lrtmp2_ertmp_exvideo_parse(data, size, &hdr);

    if (size >= 5) {
        lrtmp2_fourcc_t cc;
        lrtmp2_ertmp_fourcc_parse(&data[1], 4, &cc);
    }

    return 0;
}
