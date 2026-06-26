/**
 * Core type definitions for librtmp2
 */
#ifndef LRTMP2_TYPES_H
#define LRTMP2_TYPES_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque forward-declared types */
typedef struct lrtmp2_server    lrtmp2_server_t;
typedef struct lrtmp2_client    lrtmp2_client_t;
typedef struct lrtmp2_conn      lrtmp2_conn_t;
typedef struct lrtmp2_stream    lrtmp2_stream_t;
typedef struct lrtmp2_frame     lrtmp2_frame_t;
typedef struct lrtmp2_error     lrtmp2_error_t;

/* Error codes */
typedef enum {
    LRTMP2_OK = 0,
    LRTMP2_ERR_IO = -1,
    LRTMP2_ERR_TIMEOUT = -2,
    LRTMP2_ERR_PROTOCOL = -3,
    LRTMP2_ERR_HANDSHAKE = -4,
    LRTMP2_ERR_CHUNK = -5,
    LRTMP2_ERR_AMF = -6,
    LRTMP2_ERR_UNSUPPORTED = -7,
    LRTMP2_ERR_AUTH = -8,
    LRTMP2_ERR_INTERNAL = -9,
} lrtmp2_error_code_t;

/* Connection state machine */
typedef enum {
    LRTMP2_STATE_TCP_ACCEPTED = 0,
    LRTMP2_STATE_HANDSHAKE,
    LRTMP2_STATE_CONNECTED,
    LRTMP2_STATE_CAPS_NEGOTIATED,
    LRTMP2_STATE_APP_CONNECTED,
    LRTMP2_STATE_STREAM_CREATED,
    LRTMP2_STATE_PUBLISHING,
    LRTMP2_STATE_PLAYING,
    LRTMP2_STATE_CLOSING,
    LRTMP2_STATE_CLOSED,
} lrtmp2_conn_state_t;

/* Frame types */
typedef enum {
    LRTMP2_FRAME_AUDIO = 0,
    LRTMP2_FRAME_VIDEO = 1,
    LRTMP2_FRAME_SCRIPT = 2,
    LRTMP2_FRAME_METADATA = 3,
} lrtmp2_frame_type_t;

/* Audio codec IDs */
typedef enum {
    LRTMP2_AUDIO_PCM       = 0,
    LRTMP2_AUDIO_ADPCM     = 1,
    LRTMP2_AUDIO_MP3       = 2,
    LRTMP2_AUDIO_PCM_LE    = 3,
    LRTMP2_AUDIO_NELLY_16K = 4,
    LRTMP2_AUDIO_NELLY_8K  = 5,
    LRTMP2_AUDIO_NELLY     = 6,
    LRTMP2_AUDIO_G711_A    = 7,
    LRTMP2_AUDIO_G711_U    = 8,
    LRTMP2_AUDIO_AAC       = 10,
    LRTMP2_AUDIO_SPEEX     = 11,
    LRTMP2_AUDIO_OPUS      = 14,
} lrtmp2_audio_codec_t;

/* Video codec IDs (legacy) */
typedef enum {
    LRTMP2_VIDEO_JPEG      = 1,
    LRTMP2_VIDEO_SORENSON   = 2,
    LRTMP2_VIDEO_SCREEN    = 3,
    LRTMP2_VIDEO_VP6       = 4,
    LRTMP2_VIDEO_VP6A      = 5,
    LRTMP2_VIDEO_SCREEN2   = 6,
    LRTMP2_VIDEO_H264      = 7,
    LRTMP2_VIDEO_H265      = 12,  /* Legacy ID, usually E-RTMP */
    LRTMP2_VIDEO_AV1       = 13,  /* Legacy ID, usually E-RTMP */
} lrtmp2_video_codec_t;

/* FourCC for E-RTMP */
typedef struct {
    char cc[5];  /* null-terminated, e.g. "hvc1" */
} lrtmp2_fourcc_t;

/* A parsed frame */
struct lrtmp2_frame {
    lrtmp2_frame_type_t type;
    uint32_t timestamp;
    uint32_t composition_time;
    uint32_t size;
    const uint8_t *data;
    /* Audio-specific */
    lrtmp2_audio_codec_t audio_codec;
    uint32_t audio_sample_rate;
    uint8_t audio_channels;
    uint8_t audio_bit_depth;
    lrtmp2_fourcc_t audio_fourcc;  /* enhanced: FourCC from ExAudioTagHeader */
    /* Video-specific */
    lrtmp2_video_codec_t video_codec;
    lrtmp2_fourcc_t video_fourcc;
    uint8_t video_frame_type;  /* 1=keyframe, 2=inter, etc. */
    /* Script/metadata flag */
    uint8_t is_metadata;
};

/* Error info */
struct lrtmp2_error {
    lrtmp2_error_code_t code;
    char message[256];
};

#ifdef __cplusplus
}
#endif

#endif /* LRTMP2_TYPES_H */
