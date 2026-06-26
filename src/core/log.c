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
    int offset = 0;

    /* Timestamp */
    time_t now = time(NULL);
    struct tm *tm_info = localtime(&now);
    offset += snprintf(buf + offset, sizeof(buf) - offset,
                       "[%04d-%02d-%02d %02d:%02d:%02d] ",
                       tm_info->tm_year + 1900, tm_info->tm_mon + 1, tm_info->tm_mday,
                       tm_info->tm_hour, tm_info->tm_min, tm_info->tm_sec);

    /* Level + location */
    const char *basename = strrchr(file, '/');
    basename = basename ? basename + 1 : file;
    offset += snprintf(buf + offset, sizeof(buf) - offset,
                       "[%s] %s:%d: ",
                       level_strings[level], basename, line);

    /* Message */
    va_list args;
    va_start(args, fmt);
    offset += vsnprintf(buf + offset, sizeof(buf) - offset, fmt, args);
    va_end(args);

    offset += snprintf(buf + offset, sizeof(buf) - offset, "\n");

    if (g_log_fn) {
        g_log_fn(level, buf, g_log_userdata);
    } else {
        fprintf(stderr, "%s", buf);
    }
}
