/**
 * video_tag.c — FLV video tag parser
 */
#include "flv/video_tag.h"
#include "core/bytes.h"
#include <string.h>
#include "librtmp2/types.h"

int lrtmp2_video_tag_parse(const uint8_t *data, size_t len, lrtmp2_video_tag_t *tag)
{
    if (!data || !tag || len < 1) return LRTMP2_ERR_INTERNAL;

    tag->frame_type = (data[0] >> 4) & 0x0F;
    tag->codec = (lrtmp2_video_codec_t)(data[0] & 0x0F);

    if (len >= 5 && (tag->codec == LRTMP2_VIDEO_H264 || tag->codec == LRTMP2_VIDEO_H265)) {
        tag->avc_packet_type = data[1];  /* 0=sequence header, 1=NALU, 2=end of sequence */
        tag->composition_time = lrtmp2_ntoh24(data + 2);
    }

    tag->data = data;
    tag->size = len;
    return LRTMP2_OK;
}
