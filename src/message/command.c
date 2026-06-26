/**
 * command.c — RTMP command message encoder/decoder
 *
 * Command messages use AMF-encoded data over chunk stream.
 * Common commands: connect, releaseStream, createStream, publish, play, FCUnpublish, deleteStream
 */
#include "message/command.h"
#include "amf/amf.h"
#include "core/bytes.h"
#include "core/log.h"
#include <string.h>
#include <stdlib.h>
#include "librtmp2/types.h"

/* --- Encoder --- */

int lrtmp2_cmd_build_connect(lrtmp2_buffer_t *buf, const char *app, const char *tcUrl,
                              const char *pageUrl, const char *swfUrl,
                              const char *flashVer, int audioCodecs, int videoCodecs)
{
    /* Command name: "connect" */
    lrtmf2_amf0_write_string(buf, "connect");

    /* Transaction ID: 1.0 for connect */
    lrtmf2_amf0_write_number(buf, 1.0);

    /* Command object */
    lrtmf2_amf0_write_object_begin(buf);

    lrtmf2_amf0_write_object_key(buf, "app");
    lrtmf2_amf0_write_string(buf, app);

    lrtmf2_amf0_write_object_key(buf, "type");
    lrtmf2_amf0_write_string(buf, "nonprivate");

    if (tcUrl) {
        lrtmf2_amf0_write_object_key(buf, "tcUrl");
        lrtmf2_amf0_write_string(buf, tcUrl);
    }
    if (pageUrl) {
        lrtmf2_amf0_write_object_key(buf, "pageUrl");
        lrtmf2_amf0_write_string(buf, pageUrl);
    }
    if (swfUrl) {
        lrtmf2_amf0_write_object_key(buf, "swfUrl");
        lrtmf2_amf0_write_string(buf, swfUrl);
    }
    if (flashVer) {
        lrtmf2_amf0_write_object_key(buf, "flashVer");
        lrtmf2_amf0_write_string(buf, flashVer);
    }

    lrtmf2_amf0_write_object_key(buf, "audioCodecs");
    lrtmf2_amf0_write_number(buf, (double)audioCodecs);

    lrtmf2_amf0_write_object_key(buf, "videoCodecs");
    lrtmf2_amf0_write_number(buf, (double)videoCodecs);

    lrtmf2_amf0_write_object_end(buf);

    return LRTMP2_OK;
}

int lrtmp2_cmd_build_release_stream(lrtmp2_buffer_t *buf, const char *stream_name)
{
    lrtmf2_amf0_write_string(buf, "releaseStream");
    lrtmf2_amf0_write_number(buf, 2.0);
    lrtmf2_amf0_write_null(buf);
    lrtmf2_amf0_write_string(buf, stream_name);
    return LRTMP2_OK;
}

int lrtmp2_cmd_build_create_stream(lrtmp2_buffer_t *buf, double transaction_id)
{
    lrtmf2_amf0_write_string(buf, "createStream");
    lrtmf2_amf0_write_number(buf, transaction_id);
    lrtmf2_amf0_write_null(buf);
    return LRTMP2_OK;
}

int lrtmp2_cmd_build_publish(lrtmp2_buffer_t *buf, const char *stream_name, const char *app)
{
    lrtmf2_amf0_write_string(buf, "publish");
    lrtmf2_amf0_write_number(buf, 0.0);
    lrtmf2_amf0_write_null(buf);
    lrtmf2_amf0_write_string(buf, stream_name);
    lrtmf2_amf0_write_string(buf, app);
    return LRTMP2_OK;
}

int lrtmp2_cmd_build_play(lrtmp2_buffer_t *buf, const char *stream_name)
{
    lrtmf2_amf0_write_string(buf, "play");
    lrtmf2_amf0_write_number(buf, 0.0);
    lrtmf2_amf0_write_null(buf);
    lrtmf2_amf0_write_string(buf, stream_name);
    return LRTMP2_OK;
}

int lrtmp2_cmd_build_fcpublish(lrtmp2_buffer_t *buf, const char *stream_name)
{
    lrtmf2_amf0_write_string(buf, "FCPublish");
    lrtmf2_amf0_write_number(buf, 0.0);
    lrtmf2_amf0_write_null(buf);
    lrtmf2_amf0_write_string(buf, stream_name);
    return LRTMP2_OK;
}

int lrtmp2_cmd_build_fcunpublish(lrtmp2_buffer_t *buf, const char *stream_name)
{
    lrtmf2_amf0_write_string(buf, "FCUnpublish");
    lrtmf2_amf0_write_number(buf, 0.0);
    lrtmf2_amf0_write_null(buf);
    lrtmf2_amf0_write_string(buf, stream_name);
    return LRTMP2_OK;
}

int lrtmp2_cmd_build_deletestream(lrtmp2_buffer_t *buf, double transaction_id, uint32_t stream_id)
{
    lrtmf2_amf0_write_string(buf, "deleteStream");
    lrtmf2_amf0_write_number(buf, transaction_id);
    lrtmf2_amf0_write_null(buf);
    lrtmf2_amf0_write_number(buf, (double)stream_id);
    return LRTMP2_OK;
}

/* --- Decoder --- */

int lrtmp2_cmd_read_connect(lrtmp2_buffer_t *buf, lrtmp2_connect_info_t *info)
{
    if (!buf || !info) return LRTMP2_ERR_INTERNAL;
    memset(info, 0, sizeof(*info));

    /* Read command name */
    size_t name_len;
    if (lrtmf2_amf0_read_string(buf, info->name, sizeof(info->name), &name_len) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* Read transaction ID */
    if (lrtmf2_amf0_read_number(buf, &info->transaction_id) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* Read command object */
    if (lrtmf2_amf0_read_object_begin(buf) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* Parse key-value pairs */
    char key[256];
    while (!lrtmf2_amf0_is_object_end(buf)) {
        size_t key_len;
        if (lrtmf2_amf0_read_object_key(buf, key, sizeof(key), &key_len) != LRTMP2_OK) {
            return LRTMP2_ERR_AMF;
        }

        /* Read value based on type */
        amf0_type_t type;
        if (lrtmf2_amf0_read_type(buf, &type) != LRTMP2_OK) {
            return LRTMP2_ERR_AMF;
        }

        switch (type) {
            case AMF0_STRING: {
                size_t vlen;
                lrtmf2_amf0_read_string(buf, info->app, sizeof(info->app), &vlen);
                /* Store in app field — we'll parse more carefully below */
                break;
            }
            case AMF0_NUMBER:
                lrtmf2_amf0_read_number(buf, &info->transaction_id);
                break;
            case AMF0_BOOLEAN:
                {
                    int b;
                    lrtmf2_amf0_read_boolean(buf, &b);
                }
                break;
            case AMF0_NULL:
            case AMF0_UNDEFINED:
                lrtmf2_amf0_skip_value(buf);
                break;
            default:
                lrtmf2_amf0_skip_value(buf);
                break;
        }
    }

    /* Consume object end marker */
    uint8_t end[3];
    lrtmp2_buffer_read(buf, end, 3);

    return LRTMP2_OK;
}

int lrtmp2_cmd_read_publish(lrtmp2_buffer_t *buf, char *stream_name, size_t max_name, char *app, size_t max_app)
{
    if (!buf || !stream_name || !app) return LRTMP2_ERR_INTERNAL;

    /* Read command name */
    size_t len;
    char name[64];
    if (lrtmf2_amf0_read_string(buf, name, sizeof(name), &len) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* Read transaction ID (skip) */
    double txn;
    lrtmf2_amf0_read_number(buf, &txn);

    /* Read null */
    amf0_type_t type;
    lrtmf2_amf0_read_type(buf, &type);
    if (type == AMF0_NULL) {
        /* skip */
    }

    /* Read stream name */
    if (lrtmf2_amf0_read_string(buf, stream_name, max_name, &len) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* Read app/publish type (optional) */
    if (lrtmf2_amf0_read_string(buf, app, max_app, &len) != LRTMP2_OK) {
        /* Not all clients send this */
    }

    return LRTMP2_OK;
}

int lrtmp2_cmd_read_play(lrtmp2_buffer_t *buf, char *stream_name, size_t max_name)
{
    if (!buf || !stream_name) return LRTMP2_ERR_INTERNAL;

    size_t len;
    char name[64];
    if (lrtmf2_amf0_read_string(buf, name, sizeof(name), &len) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* Skip transaction ID */
    double txn;
    lrtmf2_amf0_read_number(buf, &txn);

    /* Skip null */
    amf0_type_t type;
    lrtmf2_amf0_read_type(buf, &type);

    /* Read stream name */
    return lrtmf2_amf0_read_string(buf, stream_name, max_name, &len);
}
