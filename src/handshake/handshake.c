/**
 * handshake.c — Legacy RTMP handshake (C0/C1/C2 <-> S0/S1/S2)
 */
#include "handshake.h"
#include "core/bytes.h"
#include "core/log.h"
#include "core/alloc.h"
#include <string.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>

#define RTMP_VERSION     0x03
#define HANDSHAKE_SIZE   1536

/* SplitMix64: a small, fast PRNG used purely to fill the handshake's random
 * payload. */
static uint64_t splitmix64(uint64_t *state)
{
    uint64_t z = (*state += 0x9E3779B97F4A7C15ULL);
    z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9ULL;
    z = (z ^ (z >> 27)) * 0x94D049BB133111EBULL;
    return z ^ (z >> 31);
}

/* Fill `buf` with pseudo-random bytes. The previous implementation used the
 * unseeded libc rand(), so the bytes were identical on every process start.
 * We seed a per-call PRNG from the clock plus the destination address: this
 * avoids touching the application's global rand() state and stays race-free
 * for concurrent handshakes (each fills a distinct buffer). The simple RTMP
 * handshake does not require cryptographic randomness — S2 merely echoes the
 * peer's payload — so this is about determinism/quality, not security. */
static void fill_random(uint8_t *buf, size_t len)
{
    uint64_t state = (uint64_t)time(NULL);
    state ^= (uint64_t)(uintptr_t)buf;
    state ^= (uint64_t)clock() << 16;

    size_t i = 0;
    while (i < len) {
        uint64_t r = splitmix64(&state);
        for (int b = 0; b < 8 && i < len; b++, i++) {
            buf[i] = (uint8_t)(r >> (b * 8));
        }
    }
}

static uint32_t get_time(void)
{
    return (uint32_t)(time(NULL) & 0xFFFFFFFF);
}

void lrtmp2_handshake_cleanup(lrtmp2_handshake_t *hs)
{
    if (!hs) return;
    LRTMP2_FREE(hs->out.data);
    hs->out.data = NULL;
    hs->out.size = 0;
    hs->out.capacity = 0;
    hs->out.read_pos = 0;
}

/* --- Server-side handshake --- */

int lrtmp2_handshake_server_init(lrtmp2_handshake_t *hs)
{
    if (!hs) return LRTMP2_ERR_INTERNAL;
    memset(hs, 0, sizeof(*hs));
    hs->state = LRTMP2_HS_SERVER_WAIT_C0;
    return LRTMP2_OK;
}

int lrtmp2_handshake_server_read_c0(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf)
{
    if (!hs || !buf) return LRTMP2_ERR_INTERNAL;

    uint8_t ver;
    if (lrtmp2_buffer_read(buf, &ver, 1) != LRTMP2_OK) {
        return LRTMP2_ERR_IO;
    }

    if (ver != RTMP_VERSION) {
        LRTMP2_LOG_ERROR("Unsupported RTMP version: 0x%02x", ver);
        return LRTMP2_ERR_HANDSHAKE;
    }

    hs->version = ver;
    hs->state = LRTMP2_HS_SERVER_WAIT_C1;
    LRTMP2_LOG_DEBUG("Got C0, version=0x%02x", ver);
    return LRTMP2_OK;
}

int lrtmp2_handshake_server_read_c1(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf)
{
    if (!hs || !buf) return LRTMP2_ERR_INTERNAL;

    if (lrtmp2_buffer_available(buf) < HANDSHAKE_SIZE) {
        return LRTMP2_ERR_IO;  /* need more data */
    }

    uint8_t c1[HANDSHAKE_SIZE];
    if (lrtmp2_buffer_read(buf, c1, HANDSHAKE_SIZE) != LRTMP2_OK) {
        return LRTMP2_ERR_IO;
    }

    hs->peer_time = lrtmp2_ntoh32(c1);

    uint8_t s1[HANDSHAKE_SIZE];
    uint32_t server_time = get_time();
    uint32_t net_time = lrtmp2_hton32(server_time);
    memcpy(s1, &net_time, 4);
    memset(s1 + 4, 0, 4);
    fill_random(s1 + 8, 1528);

    uint8_t s2[HANDSHAKE_SIZE];
    memcpy(s2, &net_time, 4);
    uint32_t peer_net = lrtmp2_hton32(hs->peer_time);
    memcpy(s2 + 4, &peer_net, 4);
    fill_random(s2 + 8, 1528);

    if (!hs->out.data) {
        hs->out.data = LRTMP2_MALLOC(2 * HANDSHAKE_SIZE);
        if (!hs->out.data) return LRTMP2_ERR_INTERNAL;
        hs->out.capacity = 2 * HANDSHAKE_SIZE;
    }
    hs->out.size = 0;
    hs->out.read_pos = 0;
    lrtmp2_buffer_write(&hs->out, s1, HANDSHAKE_SIZE);
    lrtmp2_buffer_write(&hs->out, s2, HANDSHAKE_SIZE);

    hs->state = LRTMP2_HS_SERVER_WAIT_C2;
    LRTMP2_LOG_DEBUG("Got C1 (peer_time=%u), queued S1+S2", hs->peer_time);
    return LRTMP2_OK;
}

int lrtmp2_handshake_server_read_c2(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf)
{
    if (!hs || !buf) return LRTMP2_ERR_INTERNAL;

    if (lrtmp2_buffer_available(buf) < HANDSHAKE_SIZE) {
        return LRTMP2_ERR_IO;
    }

    uint8_t c2[HANDSHAKE_SIZE];
    if (lrtmp2_buffer_read(buf, c2, HANDSHAKE_SIZE) != LRTMP2_OK) {
        return LRTMP2_ERR_IO;
    }

    hs->state = LRTMP2_HS_DONE;
    LRTMP2_LOG_DEBUG("Got C2, handshake complete (server)");
    return LRTMP2_OK;
}

/* --- Client-side handshake --- */

int lrtmp2_handshake_client_init(lrtmp2_handshake_t *hs)
{
    if (!hs) return LRTMP2_ERR_INTERNAL;
    memset(hs, 0, sizeof(*hs));
    hs->state = LRTMP2_HS_CLIENT_WAIT_S0;
    return LRTMP2_OK;
}

int lrtmp2_handshake_client_generate_c0c1(lrtmp2_handshake_t *hs)
{
    if (!hs) return LRTMP2_ERR_INTERNAL;

    if (!hs->out.data) {
        hs->out.data = LRTMP2_MALLOC(1 + 2 * HANDSHAKE_SIZE);
        if (!hs->out.data) return LRTMP2_ERR_INTERNAL;
        hs->out.capacity = 1 + 2 * HANDSHAKE_SIZE;
    }

    hs->out.data[0] = RTMP_VERSION;

    uint32_t client_time = get_time();
    hs->peer_time = client_time;
    uint32_t net_time = lrtmp2_hton32(client_time);
    memcpy(hs->out.data + 1, &net_time, 4);
    memset(hs->out.data + 5, 0, 4);
    fill_random(hs->out.data + 9, 1528);

    hs->out.size = 1 + HANDSHAKE_SIZE;
    hs->out.read_pos = 0;
    hs->state = LRTMP2_HS_CLIENT_WAIT_S1;
    LRTMP2_LOG_DEBUG("Generated C0+C1 (time=%u)", client_time);
    return LRTMP2_OK;
}

int lrtmp2_handshake_client_read_s0(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf)
{
    if (!hs || !buf) return LRTMP2_ERR_INTERNAL;

    uint8_t ver;
    if (lrtmp2_buffer_read(buf, &ver, 1) != LRTMP2_OK) {
        return LRTMP2_ERR_IO;
    }

    if (ver != RTMP_VERSION) {
        LRTMP2_LOG_ERROR("Server returned unsupported version: 0x%02x", ver);
        return LRTMP2_ERR_HANDSHAKE;
    }

    hs->version = ver;
    hs->state = LRTMP2_HS_CLIENT_WAIT_S1;
    LRTMP2_LOG_DEBUG("Got S0, version=0x%02x", ver);
    return LRTMP2_OK;
}

int lrtmp2_handshake_client_read_s1(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf)
{
    if (!hs || !buf) return LRTMP2_ERR_INTERNAL;

    if (lrtmp2_buffer_available(buf) < HANDSHAKE_SIZE) {
        return LRTMP2_ERR_IO;
    }

    uint8_t s1[HANDSHAKE_SIZE];
    if (lrtmp2_buffer_read(buf, s1, HANDSHAKE_SIZE) != LRTMP2_OK) {
        return LRTMP2_ERR_IO;
    }

    hs->peer_time = lrtmp2_ntoh32(s1);

    uint8_t c2[HANDSHAKE_SIZE];
    uint32_t net_time = lrtmp2_hton32(hs->peer_time);
    memcpy(c2, &net_time, 4);
    memcpy(c2 + 4, &net_time, 4);
    fill_random(c2 + 8, 1528);

    hs->out.size = 0;
    hs->out.read_pos = 0;
    lrtmp2_buffer_write(&hs->out, c2, HANDSHAKE_SIZE);

    hs->state = LRTMP2_HS_CLIENT_WAIT_S2;
    LRTMP2_LOG_DEBUG("Got S1 (server_time=%u), queued C2", hs->peer_time);
    return LRTMP2_OK;
}

int lrtmp2_handshake_client_read_s2(lrtmp2_handshake_t *hs, lrtmp2_buffer_t *buf)
{
    if (!hs || !buf) return LRTMP2_ERR_INTERNAL;

    if (lrtmp2_buffer_available(buf) < HANDSHAKE_SIZE) {
        return LRTMP2_ERR_IO;
    }

    uint8_t s2[HANDSHAKE_SIZE];
    if (lrtmp2_buffer_read(buf, s2, HANDSHAKE_SIZE) != LRTMP2_OK) {
        return LRTMP2_ERR_IO;
    }

    hs->state = LRTMP2_HS_DONE;
    LRTMP2_LOG_DEBUG("Got S2, handshake complete (client)");
    return LRTMP2_OK;
}
