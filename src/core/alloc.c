/**
 * alloc.c — Custom allocator hooks for librtmp2
 */
#include "core/alloc.h"
#include "core/log.h"
#include "librtmp2/types.h"
#include <stdlib.h>
#include <string.h>

static lrtmp2_alloc_fn g_alloc = std_alloc;
static lrtmp2_free_fn   g_free = std_free;
static lrtmp2_realloc_fn g_realloc = std_realloc;
static void *g_alloc_userdata = NULL;

void lrtmp2_set_allocator(lrtmp2_alloc_fn alloc, lrtmp2_realloc_fn realloc, lrtmp2_free_fn free_fn, void *userdata)
{
    g_alloc = alloc ? alloc : std_alloc;
    g_free = free_fn ? free_fn : std_free;
    g_realloc = realloc ? realloc : std_realloc;
    g_alloc_userdata = userdata;
}

void *lrtmp2_malloc(size_t size)
{
    void *p = g_alloc(size, g_alloc_userdata);
    if (p) {
        LRTMP2_LOG_DEBUG("alloc %zu bytes at %p", size, p);
    }
    return p;
}

void *lrtmp2_calloc(size_t nmemb, size_t size)
{
    void *p = lrtmp2_malloc(nmemb * size);
    if (p) {
        memset(p, 0, nmemb * size);
    }
    return p;
}

void *lrtmp2_realloc(void *ptr, size_t size)
{
    void *p = g_realloc(ptr, size, g_alloc_userdata);
    if (p) {
        LRTMP2_LOG_DEBUG("realloc %p -> %zu bytes at %p", ptr, size, p);
    }
    return p;
}

void lrtmp2_free(void *ptr)
{
    if (ptr) {
        LRTMP2_LOG_DEBUG("free %p", ptr);
        g_free(ptr, g_alloc_userdata);
    }
}
