#ifndef LRTMP2_ERTMP_H
#define LRTMP2_ERTMP_H

#include "librtmp2/types.h"
#include "core/buffer.h"
#include <stdint.h>
#include <stddef.h>

typedef struct {
    uint8_t  frame_type;
    uint8_t  codec_id;
    uint8_t  avc_packet_type;
    uint8_t  composition_time[3];
} lrtmp2_video_header_t;

typedef struct {
    uint8_t  packet_type;
    uint8_t  codec_id;
} lrtmp2_uvideo_header_t;

int lrtmp2_ertmp_exvideo_parse(const uint8_t *data, size_t len,
                                lrtmp2_video_header_t *hdr);
int lrtmp2_ertmp_fourcc_parse(const uint8_t *data, size_t len, lrtmp2_fourcc_t *fourcc);
const char *lrtmp2_ertmp_version_string(void);

#endif
