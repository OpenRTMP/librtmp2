/**
 * video.h — Video tag types (public API)
 */
#ifndef LRTMP2_VIDEO_H
#define LRTMP2_VIDEO_H

#include "librtmp2/types.h"
#include <stdint.h>
#include <stddef.h>

typedef struct {
    uint8_t frame_type;      /* 1=keyframe, 2=inter, 3=disposable inter, 4=generated keyframe, 5=video info */
    lrtmp2_video_codec_t codec;
    uint8_t  avc_packet_type; /* 0=sequence header, 1=NALU, 2=end of sequence (H264/H265) */
    uint32_t composition_time;
    const uint8_t *data;
    size_t   size;
} lrtmp2_video_tag_t;

int lrtmp2_video_tag_parse(const uint8_t *data, size_t len, lrtmp2_video_tag_t *tag);

#endif
