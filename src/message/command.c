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
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include "librtmp2/types.h"

/* Maximum key/value pairs accepted in a connect command object. A hostile peer
 * can otherwise send a multi-megabyte object with tens of thousands of simple
 * entries and burn CPU in the parse loop. */
#define LRTMP2_MAX_CONNECT_OBJECT_KEYS 256

/* lrtmf2_amf0_read_number() reads a raw 8-byte double without consuming
 * a type marker, so callers must strip the AMF0_NUMBER marker themselves. */
static int amf0_read_number_value(lrtmp2_buffer_t *buf, double *val)
{
    amf0_type_t type;
    if (lrtmf2_amf0_read_type(buf, &type) != LRTMP2_OK || type != AMF0_NUMBER) {
        return LRTMP2_ERR_AMF;
    }
    return lrtmf2_amf0_read_number(buf, val);
}

/* Read an AMF0 string value into `out`, truncating (not failing) when it is
 * longer than `out_size`. Unlike lrtmf2_amf0_read_string(), an over-long
 * string does not abort the whole parse: the excess is drained so the stream
 * stays aligned for the next field. This keeps a `connect` carrying a long
 * tcUrl/flashVer usable instead of dropping the connection. */
static int amf0_read_string_trunc(lrtmp2_buffer_t *buf, char *out, size_t out_size)
{
    uint8_t type;
    if (lrtmp2_buffer_read(buf, &type, 1) != LRTMP2_OK) return LRTMP2_ERR_AMF;
    if (type != AMF0_STRING) return LRTMP2_ERR_AMF;

    uint8_t lb[2];
    if (lrtmp2_buffer_read(buf, lb, 2) != LRTMP2_OK) return LRTMP2_ERR_AMF;
    size_t slen = ((size_t)lb[0] << 8) | lb[1];

    if (lrtmp2_buffer_available(buf) < slen) return LRTMP2_ERR_IO;

    size_t copy = (out_size && slen >= out_size) ? out_size - 1 : slen;
    if (copy > 0 && lrtmp2_buffer_read(buf, (uint8_t *)out, copy) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }
    if (out_size > 0) out[copy] = '\0';
    lrtmp2_buffer_drain(buf, slen - copy);
    return LRTMP2_OK;
}

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

int lrtmp2_cmd_build_create_stream_result(lrtmp2_buffer_t *buf, double transaction_id, double stream_id)
{
    lrtmf2_amf0_write_string(buf, "_result");
    lrtmf2_amf0_write_number(buf, transaction_id);
    lrtmf2_amf0_write_null(buf);
    lrtmf2_amf0_write_number(buf, stream_id);
    return LRTMP2_OK;
}

int lrtmp2_cmd_build_onstatus(lrtmp2_buffer_t *buf, const char *level, const char *code, const char *description)
{
    lrtmf2_amf0_write_string(buf, "onStatus");
    lrtmf2_amf0_write_number(buf, 0.0);
    lrtmf2_amf0_write_null(buf);
    lrtmf2_amf0_write_object_begin(buf);
    lrtmf2_amf0_write_object_key(buf, "level");
    lrtmf2_amf0_write_string(buf, level);
    lrtmf2_amf0_write_object_key(buf, "code");
    lrtmf2_amf0_write_string(buf, code);
    lrtmf2_amf0_write_object_key(buf, "description");
    lrtmf2_amf0_write_string(buf, description);
    lrtmf2_amf0_write_object_end(buf);
    return LRTMP2_OK;
}

/* --- Decoder --- */

int lrtmp2_cmd_peek_name(lrtmp2_buffer_t *buf, char *out, size_t max_len)
{
    if (!buf) return LRTMP2_ERR_INTERNAL;
    size_t saved_pos = buf->read_pos;
    size_t len;
    int rc = lrtmf2_amf0_read_string(buf, out, max_len, &len);
    buf->read_pos = saved_pos;
    return rc;
}

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
    if (amf0_read_number_value(buf, &info->transaction_id) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* Read command object */
    if (lrtmf2_amf0_read_object_begin(buf) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* Parse key-value pairs */
    char key[256];
    unsigned keys = 0;
    while (!lrtmf2_amf0_is_object_end(buf)) {
        if (++keys > LRTMP2_MAX_CONNECT_OBJECT_KEYS) {
            LRTMP2_LOG_WARN("connect object key count exceeds cap (%u)",
                            LRTMP2_MAX_CONNECT_OBJECT_KEYS);
            return LRTMP2_ERR_AMF;
        }
        size_t key_len;
        if (lrtmf2_amf0_read_object_key(buf, key, sizeof(key), &key_len) != LRTMP2_OK) {
            return LRTMP2_ERR_AMF;
        }

        /* Peek value type without consuming it: read_string/read_number below
         * re-read and validate their own type marker. */
        size_t type_pos = buf->read_pos;
        amf0_type_t type;
        if (lrtmf2_amf0_read_type(buf, &type) != LRTMP2_OK) {
            return LRTMP2_ERR_AMF;
        }
        buf->read_pos = type_pos;

        switch (type) {
            case AMF0_STRING: {
                char *dst = NULL;
                size_t dst_size = 0;
                if (strcmp(key, "app") == 0) {
                    dst = info->app; dst_size = sizeof(info->app);
                } else if (strcmp(key, "tcUrl") == 0) {
                    dst = info->tcUrl; dst_size = sizeof(info->tcUrl);
                } else if (strcmp(key, "pageUrl") == 0) {
                    dst = info->pageUrl; dst_size = sizeof(info->pageUrl);
                } else if (strcmp(key, "swfUrl") == 0) {
                    dst = info->swfUrl; dst_size = sizeof(info->swfUrl);
                } else if (strcmp(key, "flashVer") == 0) {
                    dst = info->flashVer; dst_size = sizeof(info->flashVer);
                }
                if (dst) {
                    if (amf0_read_string_trunc(buf, dst, dst_size) != LRTMP2_OK) {
                        return LRTMP2_ERR_AMF;
                    }
                } else if (lrtmf2_amf0_skip_value(buf) != LRTMP2_OK) {
                    /* Unknown string property: consume it so we stay aligned. */
                    return LRTMP2_ERR_AMF;
                }
                break;
            }
            case AMF0_NUMBER: {
                double value;
                if (amf0_read_number_value(buf, &value) != LRTMP2_OK) {
                    return LRTMP2_ERR_AMF;
                }
                if (strcmp(key, "audioCodecs") == 0) {
                    info->audioCodecs = (int)value;
                } else if (strcmp(key, "videoCodecs") == 0) {
                    info->videoCodecs = (int)value;
                }
                break;
            }
            case AMF0_BOOLEAN:
            case AMF0_NULL:
            case AMF0_UNDEFINED:
            default:
                /* Skip values we don't extract; bail on malformed/overly
                 * nested input so we don't loop over misaligned bytes. */
                if (lrtmf2_amf0_skip_value(buf) != LRTMP2_OK) {
                    return LRTMP2_ERR_AMF;
                }
                break;
        }
    }

    /* Consume object end marker */
    uint8_t end[3];
    lrtmp2_buffer_read(buf, end, 3);

    return LRTMP2_OK;
}

int lrtmp2_cmd_read_create_stream(lrtmp2_buffer_t *buf, double *transaction_id)
{
    if (!buf || !transaction_id) return LRTMP2_ERR_INTERNAL;

    size_t len;
    char name[64];
    if (lrtmf2_amf0_read_string(buf, name, sizeof(name), &len) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    if (amf0_read_number_value(buf, transaction_id) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* command object: typically null, but skip whatever is there */
    lrtmf2_amf0_skip_value(buf);

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
    amf0_read_number_value(buf, &txn);

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
    amf0_read_number_value(buf, &txn);

    /* Skip null */
    amf0_type_t type;
    lrtmf2_amf0_read_type(buf, &type);

    /* Read stream name */
    return lrtmf2_amf0_read_string(buf, stream_name, max_name, &len);
}

int lrtmp2_cmd_read_connect_result(lrtmp2_buffer_t *buf, double *transaction_id)
{
    if (!buf || !transaction_id) return LRTMP2_ERR_INTERNAL;

    size_t len;
    char name[64];
    if (lrtmf2_amf0_read_string(buf, name, sizeof(name), &len) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    if (amf0_read_number_value(buf, transaction_id) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* Properties object, then information object: both skippable */
    lrtmf2_amf0_skip_value(buf);
    lrtmf2_amf0_skip_value(buf);

    return LRTMP2_OK;
}

int lrtmp2_cmd_read_create_stream_result(lrtmp2_buffer_t *buf, double *transaction_id, double *stream_id)
{
    if (!buf || !transaction_id || !stream_id) return LRTMP2_ERR_INTERNAL;

    size_t len;
    char name[64];
    if (lrtmf2_amf0_read_string(buf, name, sizeof(name), &len) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    if (amf0_read_number_value(buf, transaction_id) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    /* Command object: typically null */
    lrtmf2_amf0_skip_value(buf);

    if (amf0_read_number_value(buf, stream_id) != LRTMP2_OK) {
        return LRTMP2_ERR_AMF;
    }

    return LRTMP2_OK;
}
