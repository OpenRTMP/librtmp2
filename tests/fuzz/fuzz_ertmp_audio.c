/**
 * fuzz_ertmp_audio.c — Fuzz harness for E-RTMP ExAudioTagHeader parser
 */
#include <stdint.h>
#include <stddef.h>
#include "ertmp/ertmp.h"

int fuzz_ertmp_audio(const uint8_t *data, size_t size) {
    if (size < 1) return 0;

    lrtmp2_audio_header_t hdr;
    lrtmp2_ertmp_exaudio_parse(data, size, &hdr);
    return 0;
}
