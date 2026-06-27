#ifndef LRTMP2_ERTMP_INTERNAL_H
#define LRTMP2_ERTMP_INTERNAL_H

/* Include public API (types, constants, function declarations) */
#include "librtmp2/ertmp.h"

/* Internal types needed by ertmp modules */
#include "core/buffer.h"

/* ── Internal: capability negotiation (metadata.c) ──────────────── */
int lrtmp2_ertmp_caps_negotiate(lrtmp2_buffer_t *buf);

#endif
