/**
 * fourcc.c — FourCC codec registry and dispatch for Enhanced RTMP v1
 *
 * Maps FourCC codes (e.g. "avc1", "hvc1", "av01", "vp09", "Opus", "mp4a")
 * to internal codec IDs, frame type names, and provides a lookup API.
 *
 * Per E-RTMP v1 §4, the FourCC replaces the legacy 4-bit CodecID field
 * when IsExHeader is set in the VideoTagHeader or AudioTagHeader byte 0.
 */
#include "ertmp.h"
#include "librtmp2/types.h"
#include <string.h>

/* ── Video FourCC table ──────────────────────────────────────────────── */

typedef struct {
    const char   fourcc[5];      /* null-terminated, e.g. "hvc1" */
    lrtmp2_video_codec_t codec;  /* internal enum               */
    const char  *name;           /* human-readable              */
} video_fourcc_entry_t;

static const video_fourcc_entry_t s_video_fourccs[] = {
    { "avc1", LRTMP2_VIDEO_H264, "H.264/AVC"   },
    { "hvc1", LRTMP2_VIDEO_H265, "H.265/HEVC"  },
    { "av01", LRTMP2_VIDEO_AV1,  "AV1"         },
    { "vp09", LRTMP2_VIDEO_VP6,  "VP9"         },  /* VP9 maps here; no dedicated VP9 enum yet */
    {    "",  LRTMP2_VIDEO_H264, NULL           },  /* sentinel */
};

/* ── Audio FourCC table ──────────────────────────────────────────────── */

typedef struct {
    const char   fourcc[5];
    lrtmp2_audio_codec_t codec;
    const char  *name;
} audio_fourcc_entry_t;

static const audio_fourcc_entry_t s_audio_fourccs[] = {
    { "Opus", LRTMP2_AUDIO_OPUS,  "Opus"   },
    { "mp4a", LRTMP2_AUDIO_AAC,   "AAC"    },
    { "mp3 ", LRTMP2_AUDIO_MP3,   "MP3"    },   /* note trailing space per spec */
    { "ec-3", LRTMP2_AUDIO_G711_A,"Dolby Digital Plus" },  /* closest existing */
    {    "",  LRTMP2_AUDIO_AAC,   NULL      },   /* sentinel */
};

/* ── Public API ──────────────────────────────────────────────────────── */

int lrtmp2_fourcc_to_video_codec(const char *fourcc, lrtmp2_video_codec_t *out)
{
    if (!fourcc || !out) return LRTMP2_ERR_INTERNAL;
    for (const video_fourcc_entry_t *e = s_video_fourccs; e->name; e++) {
        if (memcmp(fourcc, e->fourcc, 4) == 0) {
            *out = e->codec;
            return LRTMP2_OK;
        }
    }
    *out = LRTMP2_VIDEO_H264; /* fallback — unknown codec */
    return LRTMP2_ERR_UNSUPPORTED;
}

int lrtmp2_fourcc_to_audio_codec(const char *fourcc, lrtmp2_audio_codec_t *out)
{
    if (!fourcc || !out) return LRTMP2_ERR_INTERNAL;
    for (const audio_fourcc_entry_t *e = s_audio_fourccs; e->name; e++) {
        if (memcmp(fourcc, e->fourcc, 4) == 0) {
            *out = e->codec;
            return LRTMP2_OK;
        }
    }
    *out = LRTMP2_AUDIO_AAC; /* fallback */
    return LRTMP2_ERR_UNSUPPORTED;
}

const char *lrtmp2_video_codec_to_fourcc(lrtmp2_video_codec_t codec)
{
    for (const video_fourcc_entry_t *e = s_video_fourccs; e->name; e++) {
        if (e->codec == codec) return e->fourcc;
    }
    return "avc1"; /* fallback */
}

const char *lrtmp2_audio_codec_to_fourcc(lrtmp2_audio_codec_t codec)
{
    for (const audio_fourcc_entry_t *e = s_audio_fourccs; e->name; e++) {
        if (e->codec == codec) return e->fourcc;
    }
    return "mp4a"; /* fallback */
}

const char *lrtmp2_fourcc_video_name(const char *fourcc)
{
    if (!fourcc) return NULL;
    for (const video_fourcc_entry_t *e = s_video_fourccs; e->name; e++) {
        if (memcmp(fourcc, e->fourcc, 4) == 0) return e->name;
    }
    return NULL;
}

const char *lrtmp2_fourcc_audio_name(const char *fourcc)
{
    if (!fourcc) return NULL;
    for (const audio_fourcc_entry_t *e = s_audio_fourccs; e->name; e++) {
        if (memcmp(fourcc, e->fourcc, 4) == 0) return e->name;
    }
    return NULL;
}
