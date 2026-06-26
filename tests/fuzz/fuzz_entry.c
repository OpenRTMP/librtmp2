/**
 * fuzz_entry.c — Composite fuzzer entry point
 *
 * Routes fuzz input to individual parser harnesses based on the first byte.
 * Each sub-harness is a standalone LLVMFuzzerTestOneInput function.
 * Build with: clang -fsanitize=fuzzer,address -o fuzz_all fuzz_entry.c [objects]
 */
#include <stdint.h>
#include <stddef.h>
#include <string.h>

/* Individual harness entry points */
int fuzz_handshake(const uint8_t *data, size_t size);
int fuzz_chunk(const uint8_t *data, size_t size);
int fuzz_amf0(const uint8_t *data, size_t size);
int fuzz_ertmp_video(const uint8_t *data, size_t size);
int fuzz_ertmp_audio(const uint8_t *data, size_t size);
int fuzz_modex(const uint8_t *data, size_t size);

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size) {
    if (size < 2) return 0;

    uint8_t selector = data[0] % 6;
    const uint8_t *payload = &data[1];
    size_t payload_size = size - 1;

    switch (selector) {
        case 0: fuzz_handshake(payload, payload_size); break;
        case 1: fuzz_chunk(payload, payload_size); break;
        case 2: fuzz_amf0(payload, payload_size); break;
        case 3: fuzz_ertmp_video(payload, payload_size); break;
        case 4: fuzz_ertmp_audio(payload, payload_size); break;
        case 5: fuzz_modex(payload, payload_size); break;
    }

    return 0;
}
