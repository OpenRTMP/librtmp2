/**
 * test_control.c — Unit tests for RTMP control message validation
 */
#include "message/control.h"
#include "core/bytes.h"
#include <stdio.h>
#include <string.h>

int test_set_chunk_size_validation(void)
{
    uint32_t cs;
    uint8_t zero[4];
    uint32_t net_zero = lrtmp2_hton32(0);
    memcpy(zero, &net_zero, 4);
    if (lrtmp2_msg_read_set_chunk_size(zero, &cs) == LRTMP2_OK) {
        printf("FAIL: SetChunkSize=0 should be rejected\n");
        return 0;
    }

    uint8_t huge[4];
    uint32_t net_huge = lrtmp2_hton32(0x1000000u);
    memcpy(huge, &net_huge, 4);
    if (lrtmp2_msg_read_set_chunk_size(huge, &cs) == LRTMP2_OK) {
        printf("FAIL: SetChunkSize above 24-bit max should be rejected\n");
        return 0;
    }

    uint8_t ok[4];
    uint32_t net_ok = lrtmp2_hton32(4096);
    memcpy(ok, &net_ok, 4);
    if (lrtmp2_msg_read_set_chunk_size(ok, &cs) != LRTMP2_OK || cs != 4096) {
        printf("FAIL: valid SetChunkSize rejected\n");
        return 0;
    }

    printf("PASS: SetChunkSize bounds validated\n");
    return 1;
}

int test_control_main(void)
{
    int passed = 0;
    printf("Running control message tests...\n");
    if (test_set_chunk_size_validation()) passed++;
    printf("Control tests: %d/1 passed\n", passed);
    return (passed >= 1) ? 0 : 1;
}
