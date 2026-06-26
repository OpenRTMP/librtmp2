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

/* Enhanced RTMP v1 AudioPacketType values (low nibble of byte 0 when IsExHeader is set) */
#define LRTMP2_ERTMP_AUDIO_PACKET_TYPE_SEQUENCE_START    0
#define LRTMP2_ERTMP_AUDIO_PACKET_TYPE_CODED_FRAMES      1
#define LRTMP2_ERTMP_AUDIO_PACKET_TYPE_MULTICHANNEL      2
#define LRTMP2_ERTMP_AUDIO_PACKET_TYPE_METADATA          3

/* Enhanced RTMP v1 AudioSampleRate (legacy 2-bit field when !IsExHeader) */
#define LRTMP2_ERTMP_AUDIO_RATE_5500   0
#define LRTMP2_ERTMP_AUDIO_RATE_11025  1
#define LRTMP2_ERTMP_AUDIO_RATE_22050  2
#define LRTMP2_ERTMP_AUDIO_RATE_44100  3

/* Enhanced RTMP v1 AudioSampleSize (legacy 1-bit field) */
#define LRTMP2_ERTMP_AUDIO_SAMPLE_8BIT   0
#define LRTMP2_ERTMP_AUDIO_SAMPLE_16BIT  1

typedef struct {
    uint8_t  is_ex_header;     /* bit 7 of byte 0 */
    uint8_t  packet_type;      /* low nibble of byte 0, valid only if is_ex_header */
    char     fourcc[5];        /* e.g. "avc1", "hvc1", "av01", "vp09"; empty if !is_ex_header */
    uint8_t  frame_type;       /* keyframe(1)/interframe(2)/... ; legacy high nibble if !is_ex_header */
    uint32_t composition_time; /* 24-bit signed composition time offset, sign-extended; 0 if absent */
    size_t   header_size;      /* bytes consumed by this header, so callers can skip to payload */
} lrtmp2_video_header_t;

/* ── Enhanced RTMP v1 AudioTagHeader ─────────────────────────────── */

typedef struct {
    uint8_t  is_ex_header;     /* bit 7 of byte 0 */
    uint8_t  packet_type;      /* low nibble of byte 0, valid only if is_ex_header */
    char     fourcc[5];        /* e.g. "Opus", "mp4a"; empty if !is_ex_header */
    /* Legacy fields (valid only when !is_ex_header) */
    uint8_t  audio_codec;      /* legacy 4-bit SoundFormat */
    uint8_t  sample_rate;      /* legacy 2-bit SoundRate (0=5.5k,1=11k,2=22k,3=44k) */
    uint8_t  sample_size;      /* legacy 1-bit SoundSize (0=8bit,1=16bit) */
    uint8_t  channels;         /* legacy 1-bit SoundType (0=mono,1=stereo) */
    /* AAC-specific (legacy, when audio_codec==10) */
    uint8_t  aac_packet_type;  /* 0=seq header, 1=raw */
    size_t   header_size;      /* bytes consumed by this header */
} lrtmp2_audio_header_t;

/* Parses a VideoTagHeader, dispatching to the Enhanced RTMP v1 layout when
 * the IsExHeader bit (byte 0, bit 7) is set, or the legacy FrameType/CodecID
 * layout otherwise. */
int lrtmp2_ertmp_exvideo_parse(const uint8_t *data, size_t len,
                                lrtmp2_video_header_t *hdr);
/* Reads a raw 4-byte FourCC (e.g. from an ExVideoTagHeader) into a
 * null-terminated lrtmp2_fourcc_t. */
int lrtmp2_ertmp_fourcc_parse(const uint8_t *data, size_t len, lrtmp2_fourcc_t *fourcc);
const char *lrtmp2_ertmp_version_string(void);

/* ── FourCC codec registry (fourcc.c) ─────────────────────────────── */
int  lrtmp2_fourcc_to_video_codec(const char *fourcc, lrtmp2_video_codec_t *out);
int  lrtmp2_fourcc_to_audio_codec(const char *fourcc, lrtmp2_audio_codec_t *out);
const char *lrtmp2_video_codec_to_fourcc(lrtmp2_video_codec_t codec);
const char *lrtmp2_audio_codec_to_fourcc(lrtmp2_audio_codec_t codec);
const char *lrtmp2_fourcc_video_name(const char *fourcc);
const char *lrtmp2_fourcc_audio_name(const char *fourcc);

/* ── Enhanced RTMP v1 ExAudioTagHeader (exaudio.c) ───────────────── */
int lrtmp2_ertmp_exaudio_parse(const uint8_t *data, size_t len,
                                lrtmp2_audio_header_t *hdr);

/* ── Enhanced RTMP v2 capability negotiation stubs ──────────────── */
int lrtmp2_ertmp_caps_negotiate(lrtmp2_buffer_t *buf);

/* ── HDR color info (metadata.c) ─────────────────────────────────── */
typedef struct {
    uint16_t color_primaries;
    uint16_t transfer_chars;
    uint16_t matrix_coeffs;
} lrtmp2_hdr_info_t;

int   lrtmp2_ertmp_hdr_init(lrtmp2_hdr_info_t *hdr);
int   lrtmp2_ertmp_hdr_parse(const uint8_t *data, size_t len, lrtmp2_hdr_info_t *hdr);
size_t lrtmp2_ertmp_hdr_write(const lrtmp2_hdr_info_t *hdr, uint8_t *buf, size_t buf_size);
int   lrtmp2_ertmp_metadata_colorinfo_parse(const uint8_t *data, size_t len, lrtmp2_hdr_info_t *hdr);
uint32_t lrtmp2_ertmp_videocodecid_from_fourcc(const char *fourcc);

/* ── fourCcList (connect_caps.c) ─────────────────────────────────── */
#define LRTMP2_MAX_FOURCCS 16

typedef struct {
    lrtmp2_fourcc_t entries[LRTMP2_MAX_FOURCCS];
    size_t count;
} lrtmp2_fourcc_list_t;

int    lrtmp2_ertmp_fourcc_list_init(lrtmp2_fourcc_list_t *list);
int    lrtmp2_ertmp_fourcc_list_add(lrtmp2_fourcc_list_t *list, const char *cc);
int    lrtmp2_ertmp_fourcc_list_parse(lrtmp2_fourcc_list_t *list, const uint8_t *data, size_t len);
size_t lrtmp2_ertmp_fourcc_list_write(const lrtmp2_fourcc_list_t *list, uint8_t *buf, size_t buf_size);

#endif
