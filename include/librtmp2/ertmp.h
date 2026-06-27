/**
 * ertmp.h — Enhanced RTMP extension types (public API)
 *
 * Public types and constants for Enhanced RTMP v1/v2 support.
 * Consumers should include this header; internal code uses
 * "ertmp/ertmp.h" which extends this with internal declarations.
 */
#ifndef LRTMP2_ERTMP_H
#define LRTMP2_ERTMP_H

#include "librtmp2/types.h"
#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Enhanced RTMP v1 VideoPacketType ────────────────────────────── */
#define LRTMP2_ERTMP_PACKET_TYPE_SEQUENCE_START          0
#define LRTMP2_ERTMP_PACKET_TYPE_CODED_FRAMES            1
#define LRTMP2_ERTMP_PACKET_TYPE_SEQUENCE_END            2
#define LRTMP2_ERTMP_PACKET_TYPE_CODED_FRAMES_X          3
#define LRTMP2_ERTMP_PACKET_TYPE_METADATA                4
#define LRTMP2_ERTMP_PACKET_TYPE_MPEG2TS_SEQUENCE_START  5

/* ── Enhanced RTMP v1 AudioPacketType ────────────────────────────── */
#define LRTMP2_ERTMP_AUDIO_PACKET_TYPE_SEQUENCE_START    0
#define LRTMP2_ERTMP_AUDIO_PACKET_TYPE_CODED_FRAMES      1
#define LRTMP2_ERTMP_AUDIO_PACKET_TYPE_MULTICHANNEL      2
#define LRTMP2_ERTMP_AUDIO_PACKET_TYPE_METADATA          3

/* ── Enhanced RTMP v1 AudioSampleRate (legacy) ───────────────────── */
#define LRTMP2_ERTMP_AUDIO_RATE_5500   0
#define LRTMP2_ERTMP_AUDIO_RATE_11025  1
#define LRTMP2_ERTMP_AUDIO_RATE_22050  2
#define LRTMP2_ERTMP_AUDIO_RATE_44100  3

/* ── Enhanced RTMP v1 AudioSampleSize (legacy) ───────────────────── */
#define LRTMP2_ERTMP_AUDIO_SAMPLE_8BIT   0
#define LRTMP2_ERTMP_AUDIO_SAMPLE_16BIT  1

/* ── Parsed video tag header ─────────────────────────────────────── */
typedef struct {
    uint8_t  is_ex_header;
    uint8_t  packet_type;
    char     fourcc[5];
    uint8_t  frame_type;
    uint32_t composition_time;
    size_t   header_size;
} lrtmp2_video_header_t;

/* ── Parsed audio tag header ─────────────────────────────────────── */
typedef struct {
    uint8_t  is_ex_header;
    uint8_t  packet_type;
    char     fourcc[5];
    uint8_t  audio_codec;
    uint8_t  sample_rate;
    uint8_t  sample_size;
    uint8_t  channels;
    uint8_t  aac_packet_type;
    size_t   header_size;
} lrtmp2_audio_header_t;

/* ── FourCC codec registry ───────────────────────────────────────── */
int         lrtmp2_fourcc_to_video_codec(const char *fourcc, lrtmp2_video_codec_t *out);
int         lrtmp2_fourcc_to_audio_codec(const char *fourcc, lrtmp2_audio_codec_t *out);
const char *lrtmp2_video_codec_to_fourcc(lrtmp2_video_codec_t codec);
const char *lrtmp2_audio_codec_to_fourcc(lrtmp2_audio_codec_t codec);
const char *lrtmp2_fourcc_video_name(const char *fourcc);
const char *lrtmp2_fourcc_audio_name(const char *fourcc);

/* ── Video/audio tag header parsers ──────────────────────────────── */
int lrtmp2_ertmp_exvideo_parse(const uint8_t *data, size_t len,
                                lrtmp2_video_header_t *hdr);
int lrtmp2_ertmp_fourcc_parse(const uint8_t *data, size_t len, lrtmp2_fourcc_t *fourcc);
int lrtmp2_ertmp_exaudio_parse(const uint8_t *data, size_t len,
                                lrtmp2_audio_header_t *hdr);
const char *lrtmp2_ertmp_version_string(void);

/* ── HDR color info ─────────────────────────────────────────────── */
typedef struct {
    uint16_t color_primaries;
    uint16_t transfer_chars;
    uint16_t matrix_coeffs;
} lrtmp2_hdr_info_t;

int      lrtmp2_ertmp_hdr_init(lrtmp2_hdr_info_t *hdr);
int      lrtmp2_ertmp_hdr_parse(const uint8_t *data, size_t len, lrtmp2_hdr_info_t *hdr);
size_t   lrtmp2_ertmp_hdr_write(const lrtmp2_hdr_info_t *hdr, uint8_t *buf, size_t buf_size);
int      lrtmp2_ertmp_metadata_colorinfo_parse(const uint8_t *data, size_t len, lrtmp2_hdr_info_t *hdr);
uint32_t lrtmp2_ertmp_videocodecid_from_fourcc(const char *fourcc);

/* ── fourCcList ─────────────────────────────────────────────────── */
#define LRTMP2_MAX_FOURCCS 16

typedef struct {
    lrtmp2_fourcc_t entries[LRTMP2_MAX_FOURCCS];
    size_t count;
} lrtmp2_fourcc_list_t;

int    lrtmp2_ertmp_fourcc_list_init(lrtmp2_fourcc_list_t *list);
int    lrtmp2_ertmp_fourcc_list_add(lrtmp2_fourcc_list_t *list, const char *cc);
int    lrtmp2_ertmp_fourcc_list_parse(lrtmp2_fourcc_list_t *list, const uint8_t *data, size_t len);
size_t lrtmp2_ertmp_fourcc_list_write(const lrtmp2_fourcc_list_t *list, uint8_t *buf, size_t buf_size);

/* ── E-RTMP v2 capability negotiation ───────────────────────────── */
typedef struct {
    lrtmp2_fourcc_t entries[LRTMP2_MAX_FOURCCS];
    size_t count;
} lrtmp2_video_fourcc_info_map_t;

typedef struct {
    uint32_t version;
    int      video_codec_32;
    int      audio_codec_32;
} lrtmp2_caps_exit_t;

int   lrtmp2_ertmp_caps_exit_parse(lrtmp2_caps_exit_t *caps, const uint8_t *data, size_t len);
size_t lrtmp2_ertmp_caps_exit_write(const lrtmp2_caps_exit_t *caps, uint8_t *buf, size_t buf_size);

int   lrtmp2_ertmp_video_fourcc_info_map_parse(lrtmp2_video_fourcc_info_map_t *map, const uint8_t *data, size_t len);
size_t lrtmp2_ertmp_video_fourcc_info_map_write(const lrtmp2_video_fourcc_info_map_t *map, uint8_t *buf, size_t buf_size);

/* ── E-RTMP v2 reconnect ────────────────────────────────────────── */
typedef struct {
    uint32_t replay;
    uint32_t limit;
} lrtmp2_reconnect_t;

int   lrtmp2_ertmp_reconnect_parse(lrtmp2_reconnect_t *rc, const uint8_t *data, size_t len);
size_t lrtmp2_ertmp_reconnect_write(const lrtmp2_reconnect_t *rc, uint8_t *buf, size_t buf_size);

/* ── E-RTMP v2 multitrack ───────────────────────────────────────── */
typedef enum {
    LRTMP2_MULTITRACK_TYPE_AUDIO = 0,
    LRTMP2_MULTITRACK_TYPE_VIDEO = 1,
    LRTMP2_MULTITRACK_TYPE_METDATA = 2,
} lrtmp2_multitrack_type_t;

typedef struct {
    lrtmp2_multitrack_type_t type;
    char track_name[64];
} lrtmp2_multitrack_t;

int   lrtmp2_ertmp_multitrack_parse(lrtmp2_multitrack_t *mt, const uint8_t *data, size_t len);
size_t lrtmp2_ertmp_multitrack_write(const lrtmp2_multitrack_t *mt, uint8_t *buf, size_t buf_size);

/* ── E-RTMP v2 ModEx ────────────────────────────────────────────── */
typedef enum {
    LRTMP2_MODEX_TYPE_NOP = 0,
    LRTMP2_MODEX_TYPE_TIMESTAMP = 1,
} lrtmp2_modex_type_t;

typedef struct {
    lrtmp2_modex_type_t type;
    uint64_t offset;
} lrtmp2_modex_t;

int   lrtmp2_ertmp_modex_parse(lrtmp2_modex_t *modex, const uint8_t *data, size_t len);
size_t lrtmp2_ertmp_modex_write(const lrtmp2_modex_t *modex, uint8_t *buf, size_t buf_size);

#ifdef __cplusplus
}
#endif

#endif
