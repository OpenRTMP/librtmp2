#ifndef LRTMP2_CORE_LOG_H
#define LRTMP2_CORE_LOG_H

typedef enum {
    LRTMP2_LOG_DEBUG = 0,
    LRTMP2_LOG_INFO  = 1,
    LRTMP2_LOG_WARN  = 2,
    LRTMP2_LOG_ERROR = 3,
} lrtmp2_log_level_t;

typedef void (*lrtmp2_log_fn)(lrtmp2_log_level_t level, const char *msg, void *userdata);

void lrtmp2_log_set_level(lrtmp2_log_level_t level);
void lrtmp2_log_set_callback(lrtmp2_log_fn fn, void *userdata);
void lrtmp2_log(lrtmp2_log_level_t level, const char *file, int line, const char *fmt, ...);

#define LRTMP2_LOG_DEBUG(...) lrtmp2_log(LRTMP2_LOG_DEBUG, __FILE__, __LINE__, __VA_ARGS__)
#define LRTMP2_LOG_INFO(...)  lrtmp2_log(LRTMP2_LOG_INFO,  __FILE__, __LINE__, __VA_ARGS__)
#define LRTMP2_LOG_WARN(...)  lrtmp2_log(LRTMP2_LOG_WARN,  __FILE__, __LINE__, __VA_ARGS__)
#define LRTMP2_LOG_ERROR(...) lrtmp2_log(LRTMP2_LOG_ERROR, __FILE__, __LINE__, __VA_ARGS__)

#endif
