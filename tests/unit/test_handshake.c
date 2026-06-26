/**
 * test_handshake.c — Unit tests for RTMP handshake
 */
#include "handshake.h"
#include "core/buffer.h"
#include <stdio.h>
#include <string.h>

int test_handshake_server(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    lrtmp2_handshake_t hs;
    uint8_t c0[1] = {0x03};  /* version 3 */
    uint8_t c1[1536];
    uint8_t c2[1536];

    /* Build a fake C1: time=0x12345678, zero, random */
    uint32_t net_time = 0x78563412;  /* 0x12345678 in network byte order */
    memcpy(c1, &net_time, 4);
    memset(c1+4, 0, 4);  /* zero */
    /* fill rest with a pattern */
    for (int i=8; i<1536; i++) c1[i] = (uint8_t)i;

    /* Build C2: echo time (same as C1 time) + peer time (we'll set to 0x87654321) + random */
    uint32_t peer_time = 0x87654321;
    uint32_t net_peer = 0x21436587;
    memcpy(c2, &net_time, 4);  /* time */
    memcpy(c2+4, &net_peer, 4); /* peer time */
    for (int i=8; i<1536; i++) c2[i] = (uint8_t)(~i);

    lrtmp2_buffer_reset(buf);
    lrtmp2_buffer_write(buf, c0, 1);
    lrtmp2_buffer_write(buf, c1, 1536);
    lrtmp2_buffer_write(buf, c2, 1536);
    lrtmp2_buffer_reset(buf);

    lrtmp2_handshake_server_init(&hs);

    /* Read C0 */
    int rc = lrtmp2_handshake_server_read_c0(&hs, buf);
    if (rc != 0) {
        printf("FAIL: server read C0 returned %d\n", rc);
        return 0;
    }
    if (!lrtmp2_handshake_complete(&hs)) {
        /* need more */
    }

    /* Read C1 */
    rc = lrtmp2_handshake_server_read_c1(&hs, buf);
    if (rc != 0) {
        printf("FAIL: server read C1 returned %d\n", rc);
        return 0;
    }

    /* Read C2 */
    rc = lrtmp2_handshake_server_read_c2(&hs, buf);
    if (rc != 0) {
        printf("FAIL: server read C2 returned %d\n", rc);
        return 0;
    }

    if (lrtmp2_handshake_complete(&hs)) {
        printf("PASS: server handshake completed\n");
        return 1;
    } else {
        printf("FAIL: server handshake not complete\n");
        return 0;
    }
}

int test_handshake_client(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    lrtmp2_handshake_t hs;
    uint8_t s0[1] = {0x03};
    uint8_t s1[1536];
    uint8_t s2[1536];

    /* Build S1: time=0x11223344 */
    uint32_t net_time = 0x44332211;
    memcpy(s1, &net_time, 4);
    memset(s1+4, 0, 4);
    for (int i=8; i<1536; i++) s1[i] = (uint8_t)(i*2);

    /* Build S2: time (echo of client's C1 time) + peer time (from client's C1) + random */
    uint32_t client_time = 0xaaaabbbb;  /* what client sent in C1 */
    uint32_t net_client = 0xbbbbaaaa;
    uint32_t peer_time = 0xccccdddd;   /* what we pretend is the peer's time from C1 */
    uint32_t net_peer = 0xddddcccc;
    memcpy(s2, &net_client, 4);   /* time */
    memcpy(s2+4, &net_peer, 4);   /* peer time */
    for (int i=8; i<1536; i++) s2[i] = (uint8_t)(~i*2);

    lrtmp2_buffer_reset(buf);
    lrtmp2_buffer_write(buf, s0, 1);
    lrtmp2_buffer_write(buf, s1, 1536);
    lrtmp2_buffer_write(buf, s2, 1536);
    lrtmp2_buffer_reset(buf);

    lrtmp2_handshake_client_init(&hs);

    /* Read S0 */
    int rc = lrtmp2_handshake_client_read_s0(&hs, buf);
    if (rc != 0) {
        printf("FAIL: client read S0 returned %d\n", rc);
        return 0;
    }

    /* Generate C0+C1 */
    rc = lrtmp2_handshake_client_generate_c0c1(&hs);
    if (rc != 0) {
        printf("FAIL: client generate C0+C1 returned %d\n", rc);
        return 0;
    }

    /* Read S1 */
    rc = lrtmp2_handshake_client_read_s1(&hs, buf);
    if (rc != 0) {
        printf("FAIL: client read S1 returned %d\n", rc);
        return 0;
    }

    /* Read S2 */
    rc = lrtmp2_handshake_client_read_s2(&hs, buf);
    if (rc != 0) {
        printf("FAIL: client read S2 returned %d\n", rc);
        return 0;
    }

    if (lrtmp2_handshake_complete(&hs)) {
        printf("PASS: client handshake completed\n");
        return 1;
    } else {
        printf("FAIL: client handshake not complete\n");
        return 0;
    }
}

int main(void)
{
    int passed = 0;
    int total = 2;

    printf("Running handshake tests...\n");

    if (test_handshake_server()) passed++;
    if (test_handshake_client()) passed++;

    printf("Handshake tests: %d/%d passed\n", passed, total);
    return (passed == total) ? 0 : 1;
}