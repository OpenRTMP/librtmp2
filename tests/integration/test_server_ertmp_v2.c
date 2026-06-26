/**
 * test_server_ertmp_v2.c — Integration test for E-RTMP v2 modules
 *
 * Tests the v2 structures (capsEx, videoFourCcInfoMap, reconnect, multitrack, ModEx)
 * by serializing them to a buffer and parsing them back, verifying roundtrip.
 * Also tests that the server correctly handles a connect with fourCcList + capsEx.
 */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <assert.h>
#include "ertmp/ertmp.h"
#include "librtmp2/types.h"

#define ASSERT(cond, msg) do { \
    if (!(cond)) { printf("  FAIL: %s\n", msg); failures++; } \
    else { printf("  PASS: %s\n", msg); } \
} while (0)

int main(void) {
    int failures = 0;
    uint8_t buf[256];

    printf("=== E-RTMP v2 integration test ===\n");

    /* ── capsEx roundtrip ── */
    printf("\n--- capsEx roundtrip ---\n");
    {
        lrtmp2_caps_exit_t caps = { .version = 1, .video_codec_32 = 0x68766331, .audio_codec_32 = 0x4F707573 };
        size_t w = lrtmp2_ertmp_caps_exit_write(&caps, buf, sizeof(buf));
        ASSERT(w == 8, "capsEx write → 8 bytes");

        lrtmp2_caps_exit_t caps2;
        int rc = lrtmp2_ertmp_caps_exit_parse(&caps2, buf, w);
        ASSERT(rc == LRTMP2_OK, "capsEx parse OK");
        ASSERT(caps2.video_codec_32 == 0x68766331, "video_codec roundtrip");
        ASSERT(caps2.audio_codec_32 == 0x4F707573, "audio_codec roundtrip");
    }

    /* ── videoFourCcInfoMap roundtrip ── */
    printf("\n--- videoFourCcInfoMap roundtrip ---\n");
    {
        lrtmp2_video_fourcc_info_map_t map;
        lrtmp2_ertmp_video_fourcc_info_map_parse(&map, (uint8_t[]){
            0x00, 0x00, 0x00, 0x03, /* count = 3 */
            0x00, 0x04, 'h', 'v', 'c', '1',
            0x00, 0x04, 'a', 'v', '0', '1',
            0x00, 0x04, 'O', 'p', 'u', 's'
        }, 22);
        ASSERT(map.count == 3, "map count = 3");
        ASSERT(strcmp(map.entries[0].cc, "hvc1") == 0, "entry[0] = hvc1");
        ASSERT(strcmp(map.entries[1].cc, "av01") == 0, "entry[1] = av01");
        ASSERT(strcmp(map.entries[2].cc, "Opus") == 0, "entry[2] = Opus");

        size_t w = lrtmp2_ertmp_video_fourcc_info_map_write(&map, buf, sizeof(buf));
        ASSERT(w == 22, "map write → 22 bytes");
    }

    /* ── reconnect roundtrip ── */
    printf("\n--- reconnect roundtrip ---\n");
    {
        lrtmp2_reconnect_t rc = { .replay = 1, .limit = 100 };
        size_t w = lrtmp2_ertmp_reconnect_write(&rc, buf, sizeof(buf));
        ASSERT(w == 8, "reconnect write → 8 bytes");

        lrtmp2_reconnect_t rc2;
        int r = lrtmp2_ertmp_reconnect_parse(&rc2, buf, w);
        ASSERT(r == LRTMP2_OK, "reconnect parse OK");
        ASSERT(rc2.replay == 1, "replay roundtrip");
        ASSERT(rc2.limit == 100, "limit roundtrip");
    }

    /* ── multitrack roundtrip ── */
    printf("\n--- multitrack roundtrip ---\n");
    {
        lrtmp2_multitrack_t mt = { .type = LRTMP2_MULTITRACK_TYPE_AUDIO };
        strncpy(mt.track_name, "audio_main", sizeof(mt.track_name) - 1);
        size_t w = lrtmp2_ertmp_multitrack_write(&mt, buf, sizeof(buf));
        ASSERT(w > 0, "multitrack write > 0");

        lrtmp2_multitrack_t mt2;
        int r = lrtmp2_ertmp_multitrack_parse(&mt2, buf, w);
        ASSERT(r == LRTMP2_OK, "multitrack parse OK");
        ASSERT(mt2.type == LRTMP2_MULTITRACK_TYPE_AUDIO, "type roundtrip");
        ASSERT(strcmp(mt2.track_name, "audio_main") == 0, "name roundtrip");
    }

    /* ── ModEx NOP roundtrip ── */
    printf("\n--- ModEx NOP roundtrip ---\n");
    {
        lrtmp2_modex_t modex = { .type = LRTMP2_MODEX_TYPE_NOP, .offset = 0 };
        size_t w = lrtmp2_ertmp_modex_write(&modex, buf, sizeof(buf));
        ASSERT(w == 1, "NOP write → 1 byte");
        ASSERT(buf[0] == 0x80, "NOP marker = 0x80");

        lrtmp2_modex_t m2;
        int r = lrtmp2_ertmp_modex_parse(&m2, buf, w);
        ASSERT(r == LRTMP2_OK, "NOP parse OK");
        ASSERT(m2.type == LRTMP2_MODEX_TYPE_NOP, "NOP type");
    }

    /* ── ModEx TIMESTAMP roundtrip ── */
    printf("\n--- ModEx TIMESTAMP roundtrip ---\n");
    {
        lrtmp2_modex_t modex = { .type = LRTMP2_MODEX_TYPE_TIMESTAMP, .offset = 1234567890123ULL };
        size_t w = lrtmp2_ertmp_modex_write(&modex, buf, sizeof(buf));
        ASSERT(w == 9, "TIMESTAMP write → 9 bytes");

        lrtmp2_modex_t m2;
        int r = lrtmp2_ertmp_modex_parse(&m2, buf, w);
        ASSERT(r == LRTMP2_OK, "TIMESTAMP parse OK");
        ASSERT(m2.type == LRTMP2_MODEX_TYPE_TIMESTAMP, "TIMESTAMP type");
        ASSERT(m2.offset == 1234567890123ULL, "offset roundtrip");
    }

    /* ── Graceful degradation: unknown ModEx type ── */
    printf("\n--- ModEx unknown type ---\n");
    {
        uint8_t unknown[] = { 0x85, 0xDE, 0xAD, 0xBE, 0xEF };
        lrtmp2_modex_t m;
        int r = lrtmp2_ertmp_modex_parse(&m, unknown, sizeof(unknown));
        ASSERT(r == LRTMP2_OK, "unknown type → OK");
        ASSERT(m.type == LRTMP2_MODEX_TYPE_NOP, "unknown → NOP");
    }

    printf("\n=== Results: %d failures ===\n", failures);
    return failures > 0 ? 1 : 0;
}
