/**
 * reconnect.c — Enhanced RTMP v2 reconnect mechanism
 *
 * Per E-RTMP v2 §12, a reconnect request is sent (usually by the server)
 * to redirect or restart a session. The client MUST attempt to reconnect.
 *
 * Serialised structure (server → client, 8-byte payload):
 *   replay(4) + limit(4), both big-endian UI32
 *   replay = whether the client SHOULD replay buffered events (0 or 1)
 *   limit  = how many of those events to replay
 */

#include "ertmp.h"
#include <string.h>

int lrtmp2_ertmp_reconnect_parse(lrtmp2_reconnect_t *rc, const uint8_t *data, size_t len)
{
    if (!rc || !data || len != 8) return LRTMP2_ERR_IO;

    memset(rc, 0, sizeof(*rc));
    rc->replay = (uint32_t)((uint32_t)data[0]<<24) | ((uint32_t)data[1]<<16) |
                    ((uint32_t)data[2]<<8)  |  (uint32_t)data[3];
    rc->limit = (uint32_t)((uint32_t)data[4]<<24) | ((uint32_t)data[5]<<16) |
                   ((uint32_t)data[6]<<8)  |  (uint32_t)data[7];
    return LRTMP2_OK;
}

size_t lrtmp2_ertmp_reconnect_write(const lrtmp2_reconnect_t *rc, uint8_t *buf, size_t buf_size)
{
    if (!rc || !buf || buf_size < 8) return 0;

    buf[0] = (uint8_t)(rc->replay >> 24);
    buf[1] = (uint8_t)(rc->replay >> 16);
    buf[2] = (uint8_t)(rc->replay >> 8);
    buf[3] = (uint8_t)(rc->replay);
    buf[4] = (uint8_t)(rc->limit >> 24);
    buf[5] = (uint8_t)(rc->limit >> 16);
    buf[6] = (uint8_t)(rc->limit >> 8);
    buf[7] = (uint8_t)(rc->limit);
    return 8;
}
