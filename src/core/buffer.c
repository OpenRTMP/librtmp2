/**
 * buffer.c — Growable byte buffer
 */
#include "core/buffer.h"
#include "core/alloc.h"
#include "librtmp2/types.h"
#include <string.h>

#define BUFFER_INITIAL_SIZE 4096
#define BUFFER_GROW_FACTOR  2

lrtmp2_buffer_t *lrtmp2_buffer_create(void)
{
    lrtmp2_buffer_t *buf = LRTMP2_MALLOC(sizeof(lrtmp2_buffer_t));
    if (!buf) return NULL;

    buf->data = LRTMP2_MALLOC(BUFFER_INITIAL_SIZE);
    if (!buf->data) {
        LRTMP2_FREE(buf);
        return NULL;
    }

    buf->size = 0;
    buf->capacity = BUFFER_INITIAL_SIZE;
    buf->read_pos = 0;
    return buf;
}

void lrtmp2_buffer_destroy(lrtmp2_buffer_t *buf)
{
    if (!buf) return;
    LRTMP2_FREE(buf->data);
    LRTMP2_FREE(buf);
}

static int buffer_ensure_capacity(lrtmp2_buffer_t *buf, size_t needed)
{
    if (needed <= buf->capacity) return LRTMP2_OK;

    size_t new_cap = buf->capacity;
    while (new_cap < needed) {
        new_cap *= BUFFER_GROW_FACTOR;
        if (new_cap > LRTMP2_BUFFER_MAX_SIZE) {
            return LRTMP2_ERR_INTERNAL;
        }
    }

    uint8_t *new_data = LRTMP2_REALLOC(buf->data, new_cap);
    if (!new_data) return LRTMP2_ERR_INTERNAL;

    buf->data = new_data;
    buf->capacity = new_cap;
    return LRTMP2_OK;
}

int lrtmp2_buffer_write(lrtmp2_buffer_t *buf, const uint8_t *data, size_t len)
{
    if (!buf || !data) return LRTMP2_ERR_INTERNAL;

    size_t needed = buf->size + len;
    int rc = buffer_ensure_capacity(buf, needed);
    if (rc != LRTMP2_OK) return rc;

    memcpy(buf->data + buf->size, data, len);
    buf->size += len;
    return LRTMP2_OK;
}

int lrtmp2_buffer_read(lrtmp2_buffer_t *buf, uint8_t *out, size_t len)
{
    if (!buf || !out) return LRTMP2_ERR_INTERNAL;

    if (buf->read_pos + len > buf->size) {
        return LRTMP2_ERR_IO;  /* not enough data */
    }

    memcpy(out, buf->data + buf->read_pos, len);
    buf->read_pos += len;
    return LRTMP2_OK;
}

size_t lrtmp2_buffer_peek(const lrtmp2_buffer_t *buf, const uint8_t **data)
{
    if (!buf || !data) return 0;
    *data = buf->data + buf->read_pos;
    return buf->size - buf->read_pos;
}

void lrtmp2_buffer_reset(lrtmp2_buffer_t *buf)
{
    if (!buf) return;
    buf->size = 0;
    buf->read_pos = 0;
}

void lrtmp2_buffer_drain(lrtmp2_buffer_t *buf, size_t len)
{
    if (!buf) return;
    if (len > buf->size - buf->read_pos) {
        len = buf->size - buf->read_pos;
    }
    buf->read_pos += len;
}
