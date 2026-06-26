/**
 * main.c — Test runner
 */
#include <stdio.h>

/* Test entry points */
int test_handshake_main(void);
int test_buffer_main(void);
int test_amf_main(void);
int test_chunk_main(void);
int test_ertmp_main(void);

int main(void)
{
    int total_passed = 0;
    int total_tests = 5;

    printf("=== librtmp2 unit tests ===\n\n");

    printf("--- Handshake ---\n");
    total_passed += (test_handshake_main() == 0) ? 1 : 0;

    printf("\n--- Buffer ---\n");
    total_passed += (test_buffer_main() == 0) ? 1 : 0;

    printf("\n--- AMF0 ---\n");
    total_passed += (test_amf_main() == 0) ? 1 : 0;

    printf("\n--- Chunk ---\n");
    total_passed += (test_chunk_main() == 0) ? 1 : 0;

    printf("\n--- E-RTMP v1 ---\n");
    total_passed += (test_ertmp_main() == 0) ? 1 : 0;

    printf("\n=== Results: %d/%d suites passed ===\n", total_passed, total_tests);
    return (total_passed == total_tests) ? 0 : 1;
}
