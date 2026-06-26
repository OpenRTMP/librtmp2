/**
 * errors.c — Error code to string mapping
 */
#include "librtmp2/errors.h"
#include "librtmp2/types.h"

const char *lrtmp2_error_string(lrtmp2_error_code_t code)
{
    switch (code) {
        case LRTMP2_OK:             return "OK";
        case LRTMP2_ERR_IO:         return "I/O error";
        case LRTMP2_ERR_TIMEOUT:    return "Timeout";
        case LRTMP2_ERR_PROTOCOL:   return "Protocol error";
        case LRTMP2_ERR_HANDSHAKE:  return "Handshake error";
        case LRTMP2_ERR_CHUNK:      return "Chunk error";
        case LRTMP2_ERR_AMF:        return "AMF error";
        case LRTMP2_ERR_UNSUPPORTED: return "Unsupported";
        case LRTMP2_ERR_AUTH:       return "Authentication error";
        case LRTMP2_ERR_INTERNAL:   return "Internal error";
        default:                    return "Unknown error";
    }
}
