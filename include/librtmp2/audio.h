/**
 * audio.h — Audio tag types (public API)
 */
#ifndef LRTMP2_AUDIO_H
#define LRTMP2_AUDIO_H

#include "librtmp2/types.h"
#include <stdint.h>
#include <stddef.h>

typedef struct {
    lrtmp2_audio_codec_t codec;
    uint8_t  sample_rate;   /* 0=5.5kHz, 1=11kHz, 2=22kHz, 3=44kHz */
    uint8_t  bit_depth;     /* 0=8bit, 1=16bit */
    uint8_t  channels;      /* 0=mono, 1=stereo */
    uint8_t  aac_packet_type; /* 0=sequence header, 1=raw (AAC only) */
    const uint8_t *data;
    size_t   size;
} lrtmp2_audio_tag_t;

int lrtmp2_audio_tag_parse(const uint8_t *data, size_t len, lrtmp2_audio_tag_t *tag);

#endif
