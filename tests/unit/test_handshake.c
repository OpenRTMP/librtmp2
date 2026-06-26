#include <stdio.h>
#include <string.h>
#include "handshake/handshake.h"
#include "core/buffer.h"
#include "core/alloc.h"

int test_handshake_server(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    lrtmp2_handshake_t hs;
    uint8_t c0[1] = {0x03};  /* version 3 */
    uint8_t c1[1536];
    uint8_t c2[1536];
    int rc = 1;

    /* Build a fake C1: time=0x12345678, zero, random */
    uint32_t net_time = 0x78563412;  /* 0x12345678 in network byte order */
    memcpy(c1, &net_time, 4);
    memset(c1+4, 0, 4);  /* zero */
    /* fill rest with a pattern */
    for (int i=8; i<1536; i++) c1[i] = (uint8_t)i;

    /* Build C2: echo time (same as C1 time) + peer time + random */
    uint32_t net_peer = 0x21436587;  /* 0x87654321 in network byte order */
    memcpy(c2, &net_time, 4);  /* time */
    memcpy(c2+4, &net_peer, 4); /* peer time */
    for (int i=8; i<1536; i++) c2[i] = (uint8_t)(~i);

    lrtmp2_buffer_write(buf, c0, 1);
    lrtmp2_buffer_write(buf, c1, 1536);
    lrtmp2_buffer_write(buf, c2, 1536);

    lrtmp2_handshake_server_init(&hs);

    /* Read C0 */
    int rc2 = lrtmp2_handshake_server_read_c0(&hs, buf);
    if (rc2 != 0) {
        printf("FAIL: server read C0 returned %d\n", rc2);
        rc = 0;
        goto cleanup;
    }

    /* Read C1 */
    rc2 = lrtmp2_handshake_server_read_c1(&hs, buf);
    if (rc2 != 0) {
        printf("FAIL: server read C1 returned %d\n", rc2);
        rc = 0;
        goto cleanup;
    }

    /* Read C2 */
    rc2 = lrtmp2_handshake_server_read_c2(&hs, buf);
    if (rc2 != 0) {
        printf("FAIL: server read C2 returned %d\n", rc2);
        rc = 0;
        goto cleanup;
    }

    if (lrtmp2_handshake_complete(&hs)) {
        printf("PASS: server handshake completed\n");
    } else {
        printf("FAIL: server handshake not complete\n");
        rc = 0;
    }
cleanup:
    lrtmp2_handshake_cleanup(&hs);
    lrtmp2_buffer_destroy(buf);
    return rc;
}

int test_handshake_client(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    lrtmp2_handshake_t hs;
    uint8_t s0[1] = {0x03};
    uint8_t s1[1536];
    uint8_t s2[1536];
    int rc = 1;

    /* Build fake S1 */
    uint32_t net_time = 0x78563412;
    memcpy(s1, &net_time, 4);
    memset(s1+4, 0, 4);
    for (int i = 8; i < 1536; i++) s1[i] = (uint8_t)(i ^ 0x55);

    /* Build fake S2 */
    memcpy(s2, &net_time, 4);
    memset(s2+4, 0, 4);
    for (int i = 8; i < 1536; i++) s2[i] = (uint8_t)(i ^ 0xAA);

    lrtmp2_buffer_write(buf, s0, 1);
    lrtmp2_buffer_write(buf, s1, 1536);
    lrtmp2_buffer_write(buf, s2, 1536);

    lrtmp2_handshake_client_init(&hs);

    /* Read S0 */
    int rc2 = lrtmp2_handshake_client_read_s0(&hs, buf);
    if (rc2 != 0) {
        printf("FAIL: client read S0 returned %d\n", rc2);
        rc = 0;
        goto cleanup_client;
    }

    /* Read S1 */
    rc2 = lrtmp2_handshake_client_read_s1(&hs, buf);
    if (rc2 != 0) {
        printf("FAIL: client read S1 returned %d\n", rc2);
        rc = 0;
        goto cleanup_client;
    }

    /* Generate C0+C1 and feed it to server-style read for S2 comparison */
    lrtmp2_handshake_client_generate_c0c1(&hs);

    /* Read S2 */
    rc2 = lrtmp2_handshake_client_read_s2(&hs, buf);
    if (rc2 != 0) {
        printf("FAIL: client read S2 returned %d\n", rc2);
        rc = 0;
        goto cleanup_client;
    }

    if (lrtmp2_handshake_complete(&hs)) {
        printf("PASS: client handshake completed\n");
    } else {
        printf("FAIL: client handshake not complete\n");
        rc = 0;
    }
cleanup_client:
    lrtmp2_handshake_cleanup(&hs);
    lrtmp2_buffer_destroy(buf);
    return rc;
}

int test_handshake_main(void)
{
    int passed = 0;
    printf("Running handshake tests...\n");
    if (test_handshake_server()) passed++;
    if (test_handshake_client()) passed++;
    printf("Handshake tests: %d/2 passed\n", passed);
    return (passed >= 2) ? 0 : 1;
}
