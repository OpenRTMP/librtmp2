#ifndef LRTMP2_FLV_SCRIPT_TAG_H
#define LRTMP2_FLV_SCRIPT_TAG_H

#include <stdint.h>
#include <stddef.h>
#include "librtmp2/types.h"

typedef struct {
    char name[64];           /* typically "onMetaData" */
    const uint8_t *data;
    size_t   size;
} lrtmp2_script_tag_t;

int lrtmp2_script_tag_parse(const uint8_t *data, size_t len, lrtmp2_script_tag_t *tag);

#endif
