/**
 * exvideo.c — Enhanced RTMP v1 VideoTagHeader / FourCC parsing
 *
 * VideoTagHeader byte 0:
 *   bit 7   IsExHeader
 *   if IsExHeader:
 *     bits 6-4  VideoFrameType
 *     bits 3-0  VideoPacketType
 *   else (legacy):
 *     bits 7-4  FrameType
 *     bits 3-0  CodecID
 *
 * When IsExHeader is set, byte 0 is followed by a 4-byte FourCC. For
 * PacketTypeCodedFrames with FourCC "avc1" or "hvc1", a 3-byte signed
 * CompositionTime offset follows the FourCC before the payload; for every
 * other packet type/codec combination the payload starts immediately
 * after the FourCC.
 */
#include "ertmp.h"
#include <string.h>
#include "librtmp2/types.h"

int lrtmp2_ertmp_fourcc_parse(const uint8_t *data, size_t len, lrtmp2_fourcc_t *fourcc)
{
    if (!fourcc) return LRTMP2_ERR_INTERNAL;
    memset(fourcc, 0, sizeof(*fourcc));
    if (!data || len < 4) return LRTMP2_ERR_IO;
    memcpy(fourcc->cc, data, 4);
    fourcc->cc[4] = '\0';
    return LRTMP2_OK;
}

static int is_composition_time_codec(const char *fourcc)
{
    return memcmp(fourcc, "avc1", 4) == 0 || memcmp(fourcc, "hvc1", 4) == 0;
}

int lrtmp2_ertmp_exvideo_parse(const uint8_t *data, size_t len,
                                lrtmp2_video_header_t *hdr)
{
    if (!hdr) return LRTMP2_ERR_INTERNAL;
    memset(hdr, 0, sizeof(*hdr));
    if (!data || len < 1) return LRTMP2_ERR_IO;

    uint8_t b0 = data[0];
    hdr->is_ex_header = (b0 & 0x80) ? 1 : 0;

    if (!hdr->is_ex_header) {
        hdr->frame_type = (b0 >> 4) & 0x0F;
        hdr->header_size = 1;
        return LRTMP2_OK;
    }

    hdr->frame_type = (b0 >> 4) & 0x07;
    hdr->packet_type = b0 & 0x0F;

    if (len < 5) return LRTMP2_ERR_IO;
    memcpy(hdr->fourcc, &data[1], 4);
    hdr->fourcc[4] = '\0';
    hdr->header_size = 5;

    if (hdr->packet_type == LRTMP2_ERTMP_PACKET_TYPE_CODED_FRAMES &&
        is_composition_time_codec(hdr->fourcc)) {
        if (len < 8) return LRTMP2_ERR_IO;
        int32_t ct = (int32_t)((data[5] << 16) | (data[6] << 8) | data[7]);
        if (ct & 0x00800000) ct |= 0xFF000000; /* sign-extend 24-bit value */
        hdr->composition_time = (uint32_t)ct;
        hdr->header_size = 8;
    }

    return LRTMP2_OK;
}

const char *lrtmp2_ertmp_version_string(void)
{
    return "E-RTMP v1 (ExVideoTagHeader/FourCC)";
}
