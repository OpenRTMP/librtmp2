/**
 * audio_tag.c — FLV audio tag parser
 */
#include "flv/audio_tag.h"
#include "core/bytes.h"
#include <string.h>
#include "librtmp2/types.h"

int lrtmp2_audio_tag_parse(const uint8_t *data, size_t len, lrtmp2_audio_tag_t *tag)
{
    if (!data || !tag || len < 1) return LRTMP2_ERR_INTERNAL;

    tag->codec = (lrtmp2_audio_codec_t)((data[0] >> 4) & 0x0F);
    tag->sample_rate = (data[0] >> 2) & 0x03;
    tag->bit_depth = (data[0] >> 1) & 0x01;
    tag->channels = data[0] & 0x01;

    if (tag->codec == LRTMP2_AUDIO_AAC && len >= 2) {
        tag->aac_packet_type = data[1];  /* 0=sequence header, 1=raw */
    }

    tag->data = data;
    tag->size = len;
    return LRTMP2_OK;
}
