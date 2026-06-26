#ifndef LRTMP2_CORE_ALLOC_H
#define LRTMP2_CORE_ALLOC_H

#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include "librtmp2/types.h"

#define LRTMP2_MALLOC(sz)  lrtmp2_malloc(sz)
#define LRTMP2_CALLOC(n,sz) lrtmp2_calloc(n, sz)
#define LRTMP2_REALLOC(p,sz) lrtmp2_realloc(p, sz)
#define LRTMP2_FREE(p)     lrtmp2_free(p)

typedef void *(*lrtmp2_alloc_fn)(size_t size, void *userdata);
typedef void *(*lrtmp2_realloc_fn)(void *ptr, size_t size, void *userdata);
typedef void  (*lrtmp2_free_fn)(void *ptr, void *userdata);

/* Standard allocator wrappers */
static inline void *std_alloc(size_t size, void *ud) { (void)ud; return malloc(size); }
static inline void *std_realloc(void *p, size_t size, void *ud) { (void)ud; return realloc(p, size); }
static inline void  std_free(void *p, void *ud) { (void)ud; free(p); }

void lrtmp2_set_allocator(lrtmp2_alloc_fn alloc, lrtmp2_realloc_fn realloc, lrtmp2_free_fn free_fn, void *userdata);
void *lrtmp2_malloc(size_t size);
void *lrtmp2_calloc(size_t nmemb, size_t size);
void *lrtmp2_realloc(void *ptr, size_t size);
void  lrtmp2_free(void *ptr);

#endif
