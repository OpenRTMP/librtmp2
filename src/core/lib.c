/**
 * lib.c — Library initialization and version helpers
 */
#include "librtmp2/librtmp2.h"
#include "core/transport.h"

int lrtmp2_tls_supported(void)
{
    return lrtmp2_tls_available();
}

const char *lrtmp2_version_string(void)
{
    return LRTMP2_VERSION_STRING;
}

int lrtmp2_version_major(void)
{
    return LRTMP2_VERSION_MAJOR;
}

int lrtmp2_version_minor(void)
{
    return LRTMP2_VERSION_MINOR;
}

int lrtmp2_version_patch(void)
{
    return LRTMP2_VERSION_PATCH;
}
