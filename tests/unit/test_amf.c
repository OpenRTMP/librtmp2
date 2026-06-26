/**
 * test_amf.c — Unit tests for AMF0 encoder/decoder
 */
#include "amf/amf.h"
#include "core/buffer.h"
#include <stdio.h>
#include <string.h>
#include <math.h>

int test_amf0_number_roundtrip(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    double input = 123.456;
    lrtmf2_amf0_write_number(buf, input);

    /* Skip type marker, read number */
    uint8_t type;
    lrtmp2_buffer_read(buf, &type, 1);
    if (type != AMF0_NUMBER) {
        printf("FAIL: expected NUMBER type 0x00, got 0x%02x\n", type);
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    double output;
    lrtmf2_amf0_read_number(buf, &output);
    if (fabs(output - input) > 0.001) {
        printf("FAIL: number roundtrip: expected %f, got %f\n", input, output);
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    lrtmp2_buffer_destroy(buf);
    printf("PASS: AMF0 number roundtrip\n");
    return 1;
}

int test_amf0_string_roundtrip(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    const char *input = "hello RTMP";
    lrtmf2_amf0_write_string(buf, input);

    uint8_t type;
    lrtmp2_buffer_read(buf, &type, 1);
    if (type != AMF0_STRING) {
        printf("FAIL: expected STRING type 0x02, got 0x%02x\n", type);
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    char output[256];
    size_t out_len;
    lrtmf2_amf0_read_string(buf, output, sizeof(output), &out_len);
    if (strcmp(output, input) != 0) {
        printf("FAIL: string roundtrip: expected '%s', got '%s'\n", input, output);
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    lrtmp2_buffer_destroy(buf);
    printf("PASS: AMF0 string roundtrip\n");
    return 1;
}

int test_amf0_boolean(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    lrtmf2_amf0_write_boolean(buf, 1);

    uint8_t type;
    lrtmp2_buffer_read(buf, &type, 1);
    if (type != AMF0_BOOLEAN) {
        printf("FAIL: expected BOOLEAN type\n");
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    int val;
    lrtmf2_amf0_read_boolean(buf, &val);
    if (val != 1) {
        printf("FAIL: expected true, got %d\n", val);
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    lrtmp2_buffer_destroy(buf);
    printf("PASS: AMF0 boolean\n");
    return 1;
}

int test_amf0_null(void)
{
    lrtmp2_buffer_t *buf = lrtmp2_buffer_create();
    lrtmf2_amf0_write_null(buf);

    amf0_type_t type;
    lrtmf2_amf0_read_type(buf, &type);
    if (type != AMF0_NULL) {
        printf("FAIL: expected NULL type\n");
        lrtmp2_buffer_destroy(buf);
        return 0;
    }

    lrtmp2_buffer_destroy(buf);
    printf("PASS: AMF0 null\n");
    return 1;
}

int main(void)
{
    int passed = 0;
    printf("Running AMF0 tests...\n");
    if (test_amf0_number_roundtrip()) passed++;
    if (test_amf0_string_roundtrip()) passed++;
    if (test_amf0_boolean()) passed++;
    if (test_amf0_null()) passed++;
    printf("AMF0 tests: %d/4 passed\n", passed);
    return (passed == 4) ? 0 : 1;
}
