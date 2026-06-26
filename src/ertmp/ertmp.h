#ifndef LRTMP2_ERTMP_H
#define LRTMP2_ERTMP_H

#include "librtmp2/types.h"
#include "core/buffer.h"
#include <stdint.h>
#include <stddef.h>

/* Enhanced RTMP v1 VideoPacketType values (low nibble of byte 0 when IsExHeader is set) */
#define LRTMP2_ERTMP_PACKET_TYPE_SEQUENCE_START          0
#define LRTMP2_ERTMP_PACKET_TYPE_CODED_FRAMES            1
#define LRTMP2_ERTMP_PACKET_TYPE_SEQUENCE_END            2
#define LRTMP2_ERTMP_PACKET_TYPE_CODED_FRAMES_X          3
#define LRTMP2_ERTMP_PACKET_TYPE_METADATA                4
#define LRTMP2_ERTMP_PACKET_TYPE_MPEG2TS_SEQUENCE_START  5

typedef struct {
    uint8_t  is_ex_header;     /* bit 7 of byte 0 */
    uint8_t  packet_type;      /* low nibble of byte 0, valid only if is_ex_header */
    char     fourcc[5];        /* e.g. "avc1", "hvc1", "av01", "vp09"; empty if !is_ex_header */
    uint8_t  frame_type;       /* keyframe(1)/interframe(2)/... ; legacy high nibble if !is_ex_header */
    uint32_t composition_time; /* 24-bit signed composition time offset, sign-extended; 0 if absent */
    size_t   header_size;      /* bytes consumed by this header, so callers can skip to payload */
} lrtmp2_video_header_t;

/* Parses a VideoTagHeader, dispatching to the Enhanced RTMP v1 layout when
 * the IsExHeader bit (byte 0, bit 7) is set, or the legacy FrameType/CodecID
 * layout otherwise. */
int lrtmp2_ertmp_exvideo_parse(const uint8_t *data, size_t len,
                                lrtmp2_video_header_t *hdr);
/* Reads a raw 4-byte FourCC (e.g. from an ExVideoTagHeader) into a
 * null-terminated lrtmp2_fourcc_t. */
int lrtmp2_ertmp_fourcc_parse(const uint8_t *data, size_t len, lrtmp2_fourcc_t *fourcc);
const char *lrtmp2_ertmp_version_string(void);

#endif
