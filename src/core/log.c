/**
 * log.c — Logging subsystem
 */
#include "core/log.h"
#include <stdio.h>
#include <stdarg.h>
#include <time.h>
#include <string.h>

static lrtmp2_log_level_t g_log_level = LRTMP2_LOG_INFO;
static lrtmp2_log_fn g_log_fn = NULL;
static void *g_log_userdata = NULL;

static const char *level_strings[] = {
    "DEBUG", "INFO", "WARN", "ERROR"
};

void lrtmp2_log_set_level(lrtmp2_log_level_t level)
{
    g_log_level = level;
}

void lrtmp2_log_set_callback(lrtmp2_log_fn fn, void *userdata)
{
    g_log_fn = fn;
    g_log_userdata = userdata;
}

void lrtmp2_log(lrtmp2_log_level_t level, const char *file, int line, const char *fmt, ...)
{
    if (level < g_log_level) return;

    char buf[1024];
    size_t offset = 0;

    /* snprintf/vsnprintf return the number of bytes they *would* have written,
     * which can exceed the supplied size. Advancing `offset` by that raw value
     * would make `buf + offset` point past the buffer and `sizeof(buf) - offset`
     * underflow (size_t) to a huge value on the next call — an out-of-bounds
     * write. Clamp after every step so `offset` never passes the buffer end. */
    #define LOG_ADVANCE(n) do {                                  \
        int n_ = (n);                                            \
        if (n_ > 0) {                                            \
            offset += (size_t)n_;                                \
            if (offset >= sizeof(buf)) offset = sizeof(buf) - 1; \
        }                                                        \
    } while (0)

    /* Timestamp */
    time_t now = time(NULL);
    struct tm *tm_info = localtime(&now);
    LOG_ADVANCE(snprintf(buf + offset, sizeof(buf) - offset,
                       "[%04d-%02d-%02d %02d:%02d:%02d] ",
                       tm_info->tm_year + 1900, tm_info->tm_mon + 1, tm_info->tm_mday,
                       tm_info->tm_hour, tm_info->tm_min, tm_info->tm_sec));

    /* Level + location */
    const char *basename = strrchr(file, '/');
    basename = basename ? basename + 1 : file;
    LOG_ADVANCE(snprintf(buf + offset, sizeof(buf) - offset,
                       "[%s] %s:%d: ",
                       level_strings[level], basename, line));

    /* Message */
    va_list args;
    va_start(args, fmt);
    LOG_ADVANCE(vsnprintf(buf + offset, sizeof(buf) - offset, fmt, args));
    va_end(args);

    LOG_ADVANCE(snprintf(buf + offset, sizeof(buf) - offset, "\n"));

    #undef LOG_ADVANCE

    if (g_log_fn) {
        g_log_fn(level, buf, g_log_userdata);
    } else {
        fprintf(stderr, "%s", buf);
    }
}
