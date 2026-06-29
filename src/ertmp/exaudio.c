/**
 * exaudio.c — Enhanced RTMP v1 ExAudioTagHeader parsing
 *
 * AudioTagHeader byte 0:
 *   bit 7   IsExHeader
 *   if IsExHeader:
 *     bits 6-4  AudioFrameType
 *     bits 3-0  AudioPacketType
 *   else (legacy):
 *     bits 7-4  SoundFormat (codec ID)
 *     bits 3-2  SoundRate (sample rate index)
 *     bit  1    SoundSize (sample size)
 *     bit  0    SoundType (channels)
 *
 * When IsExHeader is set, byte 0 is followed by a 4-byte FourCC that
 * replaces the legacy SoundFormat field. The payload begins immediately
 * after the FourCC.
 */
#include "ertmp.h"
#include <string.h>

int lrtmp2_ertmp_exaudio_parse(const uint8_t *data, size_t len,
                                lrtmp2_audio_header_t *hdr)
{
    if (!hdr) return LRTMP2_ERR_INTERNAL;
    memset(hdr, 0, sizeof(*hdr));
    if (!data || len < 1) return LRTMP2_ERR_IO;

    uint8_t b0 = data[0];
    /* Per E-RTMP v1, IsExHeader=1 indicates enhanced layout with FourCC.
     * However, legacy SoundFormat values 8-15 (incl. AAC=10) also have bit 7
     * set, since legacy SoundFormat occupies the full top nibble. A length
     * check alone can't disambiguate: ordinary AAC frames are always >=5
     * bytes. Instead, only treat it as enhanced if bytes 1-4 spell a FourCC
     * this library actually recognizes as an audio codec; legacy
     * AACPacketType/payload bytes essentially never collide with one.
     */
    lrtmp2_audio_codec_t fourcc_codec = LRTMP2_AUDIO_AAC;
    hdr->is_ex_header = ((b0 & 0x80) && len >= 5 &&
                          lrtmp2_fourcc_to_audio_codec((const char *)&data[1],
                                                        &fourcc_codec) == LRTMP2_OK)
                            ? 1 : 0;

    if (!hdr->is_ex_header) {
        /* Legacy layout: SoundFormat | SoundRate | SoundSize | SoundType */
        hdr->audio_codec = (lrtmp2_audio_codec_t)((b0 >> 4) & 0x0F);
        hdr->sample_rate = (uint8_t)((b0 >> 2) & 0x03);
        hdr->sample_size = (uint8_t)((b0 >> 1) & 0x01);
        hdr->channels = b0 & 0x01;
        hdr->header_size = 1;

        if (hdr->audio_codec == LRTMP2_AUDIO_AAC && len >= 2) {
            hdr->aac_packet_type = data[1];
            hdr->header_size = 2;
        }
        return LRTMP2_OK;
    }

    /* Enhanced layout: IsExHeader=1, AudioFrameType, AudioPacketType, FourCC */
    hdr->packet_type = b0 & 0x0F;

    memcpy(hdr->fourcc, &data[1], 4);
    hdr->fourcc[4] = '\0';
    hdr->header_size = 5;
    hdr->audio_codec = fourcc_codec;

    return LRTMP2_OK;
}
