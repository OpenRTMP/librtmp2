/**
 * test_ertmp.c — Unit tests for Enhanced RTMP v1 modules
 *
 * Covers:
 * - FourCC parsing and codec lookup (fourcc.c)
 * - ExVideoTagHeader parsing (exvideo.c) — enhanced + legacy
 * - ExAudioTagHeader parsing (exaudio.c) — enhanced + legacy
 * - HDR colorInfo parse/write (metadata.c)
 * - fourCcList parse/write (connect_caps.c)
 * - videocodecid from FourCC (metadata.c)
 */
#include <stdio.h>
#include <string.h>
#include "ertmp/ertmp.h"
#include "librtmp2/types.h"

static int g_failures = 0;

#define ASSERT(cond, msg) do { \
    if (!(cond)) { printf("  FAIL: %s\n", msg); g_failures++; } \
    else { printf("  PASS: %s\n", msg); } \
} while (0)

/* ── FourCC tests ─────────────────────────────────────────────────── */

static void test_fourcc(void) {
    printf("\n--- FourCC ---\n");

    /* Parse raw 4 bytes */
    lrtmp2_fourcc_t cc;
    uint8_t raw[] = { 'h', 'v', 'c', '1' };
    int rc = lrtmp2_ertmp_fourcc_parse(raw, 4, &cc);
    ASSERT(rc == LRTMP2_OK, "fourcc_parse hvc1");
    ASSERT(strcmp(cc.cc, "hvc1") == 0, "fourcc content hvc1");

    /* Video codec lookup */
    lrtmp2_video_codec_t vcodec = LRTMP2_VIDEO_H264;
    rc = lrtmp2_fourcc_to_video_codec("hvc1", &vcodec);
    ASSERT(rc == LRTMP2_OK, "fourcc_to_video_codec hvc1");
    ASSERT(vcodec == LRTMP2_VIDEO_H265, "hvc1 → H265");

    rc = lrtmp2_fourcc_to_video_codec("av01", &vcodec);
    ASSERT(rc == LRTMP2_OK, "fourcc_to_video_codec av01");
    ASSERT(vcodec == LRTMP2_VIDEO_AV1, "av01 → AV1");

    rc = lrtmp2_fourcc_to_video_codec("avc1", &vcodec);
    ASSERT(rc == LRTMP2_OK, "fourcc_to_video_codec avc1");
    ASSERT(vcodec == LRTMP2_VIDEO_H264, "avc1 → H264");

    /* Unknown codec — fallback */
    rc = lrtmp2_fourcc_to_video_codec("XXXX", &vcodec);
    ASSERT(rc == LRTMP2_ERR_UNSUPPORTED, "unknown fourcc → UNSUPPORTED");
    ASSERT(vcodec == LRTMP2_VIDEO_H264, "unknown fourcc → fallback H264");

    /* Audio codec lookup */
    lrtmp2_audio_codec_t acodec = LRTMP2_AUDIO_AAC;
    rc = lrtmp2_fourcc_to_audio_codec("Opus", &acodec);
    ASSERT(rc == LRTMP2_OK, "fourcc_to_audio_codec Opus");
    ASSERT(acodec == LRTMP2_AUDIO_OPUS, "Opus → OPUS");

    rc = lrtmp2_fourcc_to_audio_codec("mp4a", &acodec);
    ASSERT(rc == LRTMP2_OK, "fourcc_to_audio_codec mp4a");
    ASSERT(acodec == LRTMP2_AUDIO_AAC, "mp4a → AAC");

    /* Reverse lookup */
    ASSERT(strcmp(lrtmp2_video_codec_to_fourcc(LRTMP2_VIDEO_H265), "hvc1") == 0,
           "H265 → hvc1");
    ASSERT(strcmp(lrtmp2_video_codec_to_fourcc(LRTMP2_VIDEO_H264), "avc1") == 0,
           "H264 → avc1");
    ASSERT(strcmp(lrtmp2_audio_codec_to_fourcc(LRTMP2_AUDIO_OPUS), "Opus") == 0,
           "OPUS → Opus");

    /* Names */
    ASSERT(strcmp(lrtmp2_fourcc_video_name("hvc1"), "H.265/HEVC") == 0,
           "video name hvc1");
    ASSERT(lrtmp2_fourcc_video_name("ZZZZ") == NULL, "unknown video name → NULL");
    ASSERT(strcmp(lrtmp2_fourcc_audio_name("Opus"), "Opus") == 0,
           "audio name Opus");

    /* Edge cases */
    rc = lrtmp2_ertmp_fourcc_parse(NULL, 4, &cc);
    ASSERT(rc == LRTMP2_ERR_IO, "fourcc_parse NULL data → IO");

    rc = lrtmp2_ertmp_fourcc_parse(raw, 2, &cc);
    ASSERT(rc == LRTMP2_ERR_IO, "fourcc_parse short data → IO");
}

/* ── ExVideoTagHeader tests ────────────────────────────────────────── */

static void test_exvideo(void) {
    printf("\n--- ExVideoTagHeader ---\n");

    lrtmp2_video_header_t hdr;

    /* Legacy H.264 keyframe, sequence header */
    uint8_t legacy_seq[] = { 0x17 }; /* FrameType=1(key), CodecID=7(AVC), AVCPacketType=0(seq) */
    int rc = lrtmp2_ertmp_exvideo_parse(legacy_seq, sizeof(legacy_seq), &hdr);
    ASSERT(rc == LRTMP2_OK, "legacy h264 seq header parse");
    ASSERT(hdr.is_ex_header == 0, "legacy → is_ex_header=0");
    ASSERT(hdr.frame_type == 1, "legacy → frame_type=1 (key)");
    ASSERT(hdr.header_size == 1, "legacy → header_size=1");

    /* Legacy H.264 NALU with composition time */
    uint8_t legacy_nalu[] = { 0x27, 0x01, 0x00, 0x00, 0x1A }; /* AVC NALU, CT=26ms */
    rc = lrtmp2_ertmp_exvideo_parse(legacy_nalu, sizeof(legacy_nalu), &hdr);
    ASSERT(rc == LRTMP2_OK, "legacy h264 NALU parse");
    ASSERT(hdr.is_ex_header == 0, "legacy NALU → is_ex_header=0");

    /* Enhanced HEVC keyframe, coded frames */
    uint8_t enh_hevc[] = {
        0x91,               /* IsExHeader=1, FrameType=1(key), PacketType=1(CodedFrames) */
        'h', 'v', 'c', '1', /* FourCC */
        0x00, 0x00, 0x00   /* CompositionTime = 0 */
    };
    rc = lrtmp2_ertmp_exvideo_parse(enh_hevc, sizeof(enh_hevc), &hdr);
    ASSERT(rc == LRTMP2_OK, "enhanced HEVC keyframe parse");
    ASSERT(hdr.is_ex_header == 1, "enhanced → is_ex_header=1");
    ASSERT(hdr.frame_type == 1, "enhanced → frame_type=1 (key)");
    ASSERT(hdr.packet_type == 1, "enhanced → packet_type=1 (coded frames)");
    ASSERT(strcmp(hdr.fourcc, "hvc1") == 0, "enhanced → fourcc=hvc1");
    ASSERT(hdr.composition_time == 0, "enhanced → composition_time=0");
    ASSERT(hdr.header_size == 8, "enhanced HEVC coded → header_size=8");

    /* Enhanced AV1 (no composition time) */
    uint8_t enh_av1[] = {
        0x91,               /* IsExHeader=1, FrameType=1, PacketType=1 */
        'a', 'v', '0', '1'  /* FourCC */
    };
    rc = lrtmp2_ertmp_exvideo_parse(enh_av1, sizeof(enh_av1), &hdr);
    ASSERT(rc == LRTMP2_OK, "enhanced AV1 parse");
    ASSERT(strcmp(hdr.fourcc, "av01") == 0, "enhanced → fourcc=av01");
    ASSERT(hdr.header_size == 5, "enhanced AV1 → header_size=5");

    /* Enhanced sequence start */
    uint8_t enh_seq[] = {
        0x90,               /* IsExHeader=1, FrameType=1, PacketType=0(seq start) */
        'a', 'v', 'c', '1'
    };
    rc = lrtmp2_ertmp_exvideo_parse(enh_seq, sizeof(enh_seq), &hdr);
    ASSERT(rc == LRTMP2_OK, "enhanced seq start parse");
    ASSERT(hdr.packet_type == 0, "seq start → packet_type=0");
    ASSERT(hdr.header_size == 5, "seq start → header_size=5");

    /* Edge cases */
    rc = lrtmp2_ertmp_exvideo_parse(NULL, 0, &hdr);
    ASSERT(rc == LRTMP2_ERR_IO, "NULL data → IO");

    uint8_t short_data[] = { 0x80 }; /* IsExHeader but no FourCC */
    rc = lrtmp2_ertmp_exvideo_parse(short_data, sizeof(short_data), &hdr);
    ASSERT(rc == LRTMP2_ERR_IO, "short enhanced → IO");
}

/* ── ExAudioTagHeader tests ────────────────────────────────────────── */

static void test_exaudio(void) {
    printf("\n--- ExAudioTagHeader ---\n");

    lrtmp2_audio_header_t hdr;

    /* Legacy AAC, 44.1kHz, 16-bit, stereo, sequence header */
    uint8_t legacy_aac[] = { 0xAF, 0x00 }; /* SoundFormat=10(AAC), Rate=3(44k), Size=1(16bit), Type=1(stereo), AACType=0(seq) */
    int rc = lrtmp2_ertmp_exaudio_parse(legacy_aac, sizeof(legacy_aac), &hdr);
    ASSERT(rc == LRTMP2_OK, "legacy AAC parse");
    ASSERT(hdr.is_ex_header == 0, "legacy → is_ex_header=0");
    ASSERT(hdr.audio_codec == LRTMP2_AUDIO_AAC, "legacy → codec=AAC");
    ASSERT(hdr.sample_rate == 3, "legacy → sample_rate=3 (44k)");
    ASSERT(hdr.sample_size == 1, "legacy → sample_size=1 (16bit)");
    ASSERT(hdr.channels == 1, "legacy → channels=1 (stereo)");
    ASSERT(hdr.aac_packet_type == 0, "legacy → aac_packet_type=0 (seq header)");
    ASSERT(hdr.header_size == 2, "legacy AAC → header_size=2");

    /* Legacy MP3, 22kHz, 16-bit, mono */
    uint8_t legacy_mp3[] = { 0x26 }; /* SoundFormat=2(MP3), Rate=1(22k), Size=1(16bit), Type=0(mono) */
    rc = lrtmp2_ertmp_exaudio_parse(legacy_mp3, sizeof(legacy_mp3), &hdr);
    ASSERT(rc == LRTMP2_OK, "legacy MP3 parse");
    ASSERT(hdr.audio_codec == LRTMP2_AUDIO_MP3, "legacy → codec=MP3");
    ASSERT(hdr.sample_rate == 1, "legacy → sample_rate=1 (22k)");
    ASSERT(hdr.channels == 0, "legacy → channels=0 (mono)");
    ASSERT(hdr.header_size == 1, "legacy MP3 → header_size=1");

    /* Enhanced Opus */
    uint8_t enh_opus[] = {
        0x81,               /* IsExHeader=1, FrameType=0, PacketType=1(coded) */
        'O', 'p', 'u', 's'  /* FourCC */
    };
    rc = lrtmp2_ertmp_exaudio_parse(enh_opus, sizeof(enh_opus), &hdr);
    ASSERT(rc == LRTMP2_OK, "enhanced Opus parse");
    ASSERT(hdr.is_ex_header == 1, "enhanced → is_ex_header=1");
    ASSERT(hdr.audio_codec == LRTMP2_AUDIO_OPUS, "enhanced → codec=OPUS");
    ASSERT(strcmp(hdr.fourcc, "Opus") == 0, "enhanced → fourcc=Opus");
    ASSERT(hdr.header_size == 5, "enhanced → header_size=5");

    /* Enhanced AAC (mp4a) */
    uint8_t enh_aac[] = {
        0x81,
        'm', 'p', '4', 'a'
    };
    rc = lrtmp2_ertmp_exaudio_parse(enh_aac, sizeof(enh_aac), &hdr);
    ASSERT(rc == LRTMP2_OK, "enhanced mp4a parse");
    ASSERT(hdr.audio_codec == LRTMP2_AUDIO_AAC, "enhanced → codec=AAC");
    ASSERT(strcmp(hdr.fourcc, "mp4a") == 0, "enhanced → fourcc=mp4a");

    /* Edge cases */
    rc = lrtmp2_ertmp_exaudio_parse(NULL, 0, &hdr);
    ASSERT(rc == LRTMP2_ERR_IO, "NULL data → IO");

    /* 0x80 = PCM stereo 5.5kHz 8bit (legacy, len < 5 → not enhanced) */
    uint8_t short_data[] = { 0x80, 0x00, 0x00, 0x00 }; /* valid legacy PCM */
    rc = lrtmp2_ertmp_exaudio_parse(short_data, sizeof(short_data), &hdr);
    ASSERT(rc == LRTMP2_OK, "short data → legacy PCM");
    ASSERT(hdr.is_ex_header == 0, "short → legacy");

    /* Truly invalid: IsExHeader=1, len=2 < 5 → treated as legacy PCM */
    uint8_t ambig[] = { 0x8F }; /* bit7=1, but only 1 byte */
    rc = lrtmp2_ertmp_exaudio_parse(ambig, sizeof(ambig), &hdr);
    ASSERT(rc == LRTMP2_OK, "ambiguous short → fallback legacy");
}

/* ── HDR colorInfo tests ──────────────────────────────────────────── */

static void test_hdr(void) {
    printf("\n--- HDR colorInfo ---\n");

    lrtmp2_hdr_info_t hdr;
    int rc = lrtmp2_ertmp_hdr_init(&hdr);
    ASSERT(rc == LRTMP2_OK, "hdr_init");
    ASSERT(hdr.color_primaries == 1, "default primaries=1 (BT.709)");
    ASSERT(hdr.transfer_chars == 1, "default transfer=1 (SDR)");
    ASSERT(hdr.matrix_coeffs == 1, "default matrix=1 (BT.709)");

    /* Parse HDR10: BT.2020 primaries, PQ transfer, BT.2020 matrix */
    uint8_t hdr10_data[] = { 0x00, 0x09, 0x00, 0x10, 0x00, 0x09 }; /* 9, 16, 9 */
    rc = lrtmp2_ertmp_hdr_parse(hdr10_data, sizeof(hdr10_data), &hdr);
    ASSERT(rc == LRTMP2_OK, "hdr10 parse");
    ASSERT(hdr.color_primaries == 9, "hdr10 → primaries=9 (BT.2020)");
    ASSERT(hdr.transfer_chars == 16, "hdr10 → transfer=16 (PQ)");
    ASSERT(hdr.matrix_coeffs == 9, "hdr10 → matrix=9 (BT.2020)");

    /* Write and re-parse roundtrip */
    uint8_t buf[16];
    size_t written = lrtmp2_ertmp_hdr_write(&hdr, buf, sizeof(buf));
    ASSERT(written == 6, "hdr_write → 6 bytes");

    lrtmp2_hdr_info_t hdr2;
    rc = lrtmp2_ertmp_hdr_parse(buf, written, &hdr2);
    ASSERT(rc == LRTMP2_OK, "hdr roundtrip parse");
    ASSERT(hdr2.color_primaries == 9, "roundtrip primaries");
    ASSERT(hdr2.transfer_chars == 16, "roundtrip transfer");
    ASSERT(hdr2.matrix_coeffs == 9, "roundtrip matrix");

    /* Short data */
    rc = lrtmp2_ertmp_hdr_parse(hdr10_data, 3, &hdr2);
    ASSERT(rc == LRTMP2_ERR_IO, "short hdr → IO");

    /* videocodecid */
    uint32_t cid = lrtmp2_ertmp_videocodecid_from_fourcc("av01");
    ASSERT(cid == 0x61763031, "videocodecid av01");
    cid = lrtmp2_ertmp_videocodecid_from_fourcc("hvc1");
    ASSERT(cid == 0x68766331, "videocodecid hvc1");
    cid = lrtmp2_ertmp_videocodecid_from_fourcc(NULL);
    ASSERT(cid == 7, "videocodecid NULL → 7 (AVC)");
}

/* ── fourCcList tests ─────────────────────────────────────────────── */

static void test_fourcc_list(void) {
    printf("\n--- fourCcList ---\n");

    lrtmp2_fourcc_list_t list;
    int rc = lrtmp2_ertmp_fourcc_list_init(&list);
    ASSERT(rc == LRTMP2_OK, "list_init");
    ASSERT(list.count == 0, "list_init → count=0");

    /* Add entries */
    rc = lrtmp2_ertmp_fourcc_list_add(&list, "avc1");
    ASSERT(rc == LRTMP2_OK, "add avc1");
    rc = lrtmp2_ertmp_fourcc_list_add(&list, "hvc1");
    ASSERT(rc == LRTMP2_OK, "add hvc1");
    rc = lrtmp2_ertmp_fourcc_list_add(&list, "av01");
    ASSERT(rc == LRTMP2_OK, "add av01");
    ASSERT(list.count == 3, "count=3 after adds");
    ASSERT(strcmp(list.entries[0].cc, "avc1") == 0, "entry[0]=avc1");
    ASSERT(strcmp(list.entries[1].cc, "hvc1") == 0, "entry[1]=hvc1");
    ASSERT(strcmp(list.entries[2].cc, "av01") == 0, "entry[2]=av01");

    /* Write */
    uint8_t buf[256];
    size_t written = lrtmp2_ertmp_fourcc_list_write(&list, buf, sizeof(buf));
    ASSERT(written == 4 + 3 * 6, "list_write size"); /* 4 (count) + 3*6 (entries) */
    ASSERT(written == 22, "list_write = 22 bytes");

    /* Parse roundtrip */
    lrtmp2_fourcc_list_t list2;
    rc = lrtmp2_ertmp_fourcc_list_parse(&list2, buf, written);
    ASSERT(rc == 3, "list_parse → 3 entries");
    ASSERT(strcmp(list2.entries[0].cc, "avc1") == 0, "roundtrip entry[0]");
    ASSERT(strcmp(list2.entries[1].cc, "hvc1") == 0, "roundtrip entry[1]");
    ASSERT(strcmp(list2.entries[2].cc, "av01") == 0, "roundtrip entry[2]");

    /* Parse raw blob */
    uint8_t raw[] = {
        0x00, 0x00, 0x00, 0x02, /* count = 2 */
        0x00, 0x04, 'h', 'v', 'c', '1',
        0x00, 0x04, 'a', 'v', '0', '1'
    };
    lrtmp2_fourcc_list_t list3;
    rc = lrtmp2_ertmp_fourcc_list_parse(&list3, raw, sizeof(raw));
    ASSERT(rc == 2, "raw parse → 2 entries");
    ASSERT(strcmp(list3.entries[0].cc, "hvc1") == 0, "raw entry[0]=hvc1");
    ASSERT(strcmp(list3.entries[1].cc, "av01") == 0, "raw entry[1]=av01");

    /* Edge cases */
    rc = lrtmp2_ertmp_fourcc_list_parse(&list3, NULL, 0);
    ASSERT(rc == LRTMP2_ERR_IO, "NULL data → IO");

    rc = lrtmp2_ertmp_fourcc_list_parse(&list3, raw, 2);
    ASSERT(rc == LRTMP2_ERR_IO, "short data → IO");
}

/* ── Main ─────────────────────────────────────────────────────────── */

int test_ertmp_main(void) {
    printf("=== librtmp2 E-RTMP v1 unit tests ===");

    test_fourcc();
    test_exvideo();
    test_exaudio();
    test_hdr();
    test_fourcc_list();

    printf("\n=== Results: %d failures ===\n", g_failures);
    return g_failures > 0 ? 1 : 0;
}
