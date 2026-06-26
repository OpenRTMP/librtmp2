/**
 * Public error codes and helpers
 */
#ifndef LRTMP2_ERRORS_H
#define LRTMP2_ERRORS_H

#include "types.h"

#ifdef __cplusplus
extern "C" {
#endif

const char *lrtmp2_error_string(lrtmp2_error_code_t code);

#ifdef __cplusplus
}
#endif

#endif /* LRTMP2_ERRORS_H */
