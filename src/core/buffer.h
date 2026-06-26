#ifndef LRTMP2_CORE_BUFFER_H
#define LRTMP2_CORE_BUFFER_H

#include <stdint.h>
#include <stddef.h>
#include "core/alloc.h"
#include "librtmp2/types.h"

#define LRTMP2_BUFFER_MAX_SIZE (64 * 1024 * 1024)  /* 64 MB */

typedef struct {
    uint8_t *data;
    size_t   size;       /* bytes written */
    size_t   capacity;   /* allocated */
    size_t   read_pos;   /* read cursor */
} lrtmp2_buffer_t;

lrtmp2_buffer_t *lrtmp2_buffer_create(void);
void lrtmp2_buffer_destroy(lrtmp2_buffer_t *buf);
int  lrtmp2_buffer_write(lrtmp2_buffer_t *buf, const uint8_t *data, size_t len);  /* returns bytes written, <0 on error */
int  lrtmp2_buffer_read(lrtmp2_buffer_t *buf, uint8_t *out, size_t len);  /* returns bytes read, <0 on error */
size_t lrtmp2_buffer_peek(const lrtmp2_buffer_t *buf, const uint8_t **data);
void lrtmp2_buffer_reset(lrtmp2_buffer_t *buf);
void lrtmp2_buffer_drain(lrtmp2_buffer_t *buf, size_t len);

static inline size_t lrtmp2_buffer_available(const lrtmp2_buffer_t *buf) {
    return buf->size - buf->read_pos;
}

static inline size_t lrtmp2_buffer_space(const lrtmp2_buffer_t *buf) {
    return buf->capacity - buf->size;
}

#endif
