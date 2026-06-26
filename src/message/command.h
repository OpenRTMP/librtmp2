#ifndef LRTMP2_MESSAGE_COMMAND_H
#define LRTMP2_MESSAGE_COMMAND_H

#include "core/buffer.h"
#include <stdint.h>
#include "librtmp2/types.h"

typedef struct {
    char name[64];
    double transaction_id;
    char app[256];
    char tcUrl[512];
    char pageUrl[512];
    char swfUrl[512];
    char flashVer[64];
    int audioCodecs;
    int videoCodecs;
} lrtmp2_connect_info_t;

/* Encoder */
int lrtmp2_cmd_build_connect(lrtmp2_buffer_t *buf, const char *app, const char *tcUrl,
                              const char *pageUrl, const char *swfUrl,
                              const char *flashVer, int audioCodecs, int videoCodecs);
int lrtmp2_cmd_build_release_stream(lrtmp2_buffer_t *buf, const char *stream_name);
int lrtmp2_cmd_build_create_stream(lrtmp2_buffer_t *buf, double transaction_id);
int lrtmp2_cmd_build_publish(lrtmp2_buffer_t *buf, const char *stream_name, const char *app);
int lrtmp2_cmd_build_play(lrtmp2_buffer_t *buf, const char *stream_name);
int lrtmp2_cmd_build_fcpublish(lrtmp2_buffer_t *buf, const char *stream_name);
int lrtmp2_cmd_build_fcunpublish(lrtmp2_buffer_t *buf, const char *stream_name);
int lrtmp2_cmd_build_deletestream(lrtmp2_buffer_t *buf, double transaction_id, uint32_t stream_id);

/* Decoder */
int lrtmp2_cmd_read_connect(lrtmp2_buffer_t *buf, lrtmp2_connect_info_t *info);
int lrtmp2_cmd_read_publish(lrtmp2_buffer_t *buf, char *stream_name, size_t max_name, char *app, size_t max_app);
int lrtmp2_cmd_read_play(lrtmp2_buffer_t *buf, char *stream_name, size_t max_name);

#endif
