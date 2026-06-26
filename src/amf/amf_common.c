/**
 * amf_common.c — Shared AMF utilities
 */
#include "amf.h"
#include "core/log.h"
#include "librtmp2/types.h"

const char *amf0_type_string(amf0_type_t type)
{
    switch (type) {
        case AMF0_NUMBER:     return "number";
        case AMF0_BOOLEAN:    return "boolean";
        case AMF0_STRING:     return "string";
        case AMF0_OBJECT:     return "object";
        case AMF0_NULL:       return "null";
        case AMF0_UNDEFINED:  return "undefined";
        case AMF0_ECMA_ARRAY: return "ecma_array";
        case AMF0_LONG_STRING: return "long_string";
        default:              return "unknown";
    }
}
