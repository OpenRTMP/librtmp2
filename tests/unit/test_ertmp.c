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
 * - capsEx / videoFourCcInfoMap (connect_caps.c) — E-RTMP v2
 * - reconnect (reconnect.c) — E-RTMP v2
 * - multitrack (multitrack.c) — E-RTMP v2
 * - modex (modex.c) — E-RTMP v2
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

    /* Regression: ordinary legacy AAC raw frame, len >= 5. SoundFormat=10
     * sets bit 7 (0xA0-0xAF) and real AAC payloads are virtually always
     * >=5 bytes, so a naive "bit7 + len>=5" check would misclassify this
     * as an enhanced FourCC header. Bytes[1..4] don't spell a known
     * audio FourCC, so it must still resolve to legacy AAC. */
    uint8_t legacy_aac_long[] = {
        0xAF,                   /* SoundFormat=10(AAC), 44k, 16bit, stereo */
        0x01,                   /* AACPacketType=1 (raw) */
        0x12, 0x34, 0x56, 0x78  /* raw AAC payload bytes */
    };
    rc = lrtmp2_ertmp_exaudio_parse(legacy_aac_long, sizeof(legacy_aac_long), &hdr);
    ASSERT(rc == LRTMP2_OK, "legacy AAC (long) parse");
    ASSERT(hdr.is_ex_header == 0, "legacy AAC (long) → is_ex_header=0");
    ASSERT(hdr.audio_codec == LRTMP2_AUDIO_AAC, "legacy AAC (long) → codec=AAC");
    ASSERT(hdr.sample_rate == 3, "legacy AAC (long) → sample_rate=3 (44k)");
    ASSERT(hdr.channels == 1, "legacy AAC (long) → channels=1 (stereo)");
    ASSERT(hdr.aac_packet_type == 1, "legacy AAC (long) → aac_packet_type=1 (raw)");
    ASSERT(hdr.header_size == 2, "legacy AAC (long) → header_size=2");
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

/* ── E-RTMP v2: capsEx / videoFourCcInfoMap ─────────────────────── */

static void test_caps(void) {
    printf("\n--- capsEx ---\n");

    lrtmp2_caps_exit_t caps;

    /* video=hvc1 (0x68766331), audio=Opus (0x4F707573) */
    uint8_t raw[] = { 0x68, 0x76, 0x63, 0x31, 0x4F, 0x70, 0x75, 0x73 };
    int rc = lrtmp2_ertmp_caps_exit_parse(&caps, raw, sizeof(raw));
    ASSERT(rc == LRTMP2_OK, "caps_exit_parse OK");
    ASSERT(caps.video_codec_32 == 0x68766331, "video_codec_32 == hvc1");
    ASSERT(caps.audio_codec_32 == 0x4F707573, "audio_codec_32 == Opus");

    /* Write roundtrip */
    uint8_t buf[16];
    size_t written = lrtmp2_ertmp_caps_exit_write(&caps, buf, sizeof(buf));
    ASSERT(written == 8, "caps_exit_write → 8 bytes");
    ASSERT(memcmp(buf, raw, 8) == 0, "caps_exit_write matches input");

    /* videoFourCcInfoMap */
    lrtmp2_video_fourcc_info_map_t map;
    uint8_t map_raw[] = {
        0x00, 0x00, 0x00, 0x02, /* count = 2 */
        0x00, 0x04, 'h', 'v', 'c', '1',
        0x00, 0x04, 'a', 'v', '0', '1'
    };
    rc = lrtmp2_ertmp_video_fourcc_info_map_parse(&map, map_raw, sizeof(map_raw));
    ASSERT(rc == 2, "video_fourcc_info_map_parse → 2 entries");
    ASSERT(strcmp(map.entries[0].cc, "hvc1") == 0, "map entry[0] == hvc1");
    ASSERT(strcmp(map.entries[1].cc, "av01") == 0, "map entry[1] == av01");

    size_t map_w = lrtmp2_ertmp_video_fourcc_info_map_write(&map, buf, sizeof(buf));
    ASSERT(map_w == 4 + 2 * 6, "video_fourcc_info_map_write size");
    ASSERT(memcmp(buf, map_raw, map_w) == 0, "map write matches");

    /* Edge cases */
    rc = lrtmp2_ertmp_caps_exit_parse(NULL, raw, 8);
    ASSERT(rc == LRTMP2_ERR_INTERNAL, "NULL caps → INTERNAL");
    rc = lrtmp2_ertmp_caps_exit_parse(&caps, raw, 4);
    ASSERT(rc == LRTMP2_ERR_IO, "short caps → IO");
}

/* ── E-RTMP v2: reconnect ──────────────────────────────────────────── */

static void test_reconnect(void) {
    printf("\n--- reconnect ---\n");

    lrtmp2_reconnect_t rc;

    /* replay=0x00000001, limit=0x0000000A */
    uint8_t raw[] = { 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0A };
    int rc2 = lrtmp2_ertmp_reconnect_parse(&rc, raw, sizeof(raw));
    ASSERT(rc2 == LRTMP2_OK, "reconnect_parse OK");
    ASSERT(rc.replay == 1, "replay == 1");
    ASSERT(rc.limit == 10, "limit == 10");

    /* Write roundtrip */
    uint8_t buf[16];
    size_t written = lrtmp2_ertmp_reconnect_write(&rc, buf, sizeof(buf));
    ASSERT(written == 8, "reconnect_write → 8 bytes");
    ASSERT(memcmp(buf, raw, 8) == 0, "reconnect_write matches input");

    /* Edge cases */
    rc2 = lrtmp2_ertmp_reconnect_parse(&rc, raw, 4);
    ASSERT(rc2 == LRTMP2_ERR_IO, "short reconnect → IO");

    /* limit=0 means "no limit" per spec */
    uint8_t raw2[] = { 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00 };
    rc2 = lrtmp2_ertmp_reconnect_parse(&rc, raw2, sizeof(raw2));
    ASSERT(rc2 == LRTMP2_OK, "reconnect_parse zeros OK");
    ASSERT(rc.replay == 0 && rc.limit == 0, "replay=0 limit=0");
}

/* ── E-RTMP v2: multitrack ───────────────────────────────────────── */

static void test_multitrack(void) {
    printf("\n--- multitrack ---\n");

    lrtmp2_multitrack_t mt;

    /* type=0 (audio), name="audio_track" (11 chars) */
    uint8_t raw_audio[] = {
        0x00, /* AMF0_NUMBER marker */
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, /* number = 0 (audio) */
        0x00, 0x0B, 'a', 'u', 'd', 'i', 'o', '_', 't', 'r', 'a', 'c', 'k' /* string */
    };
    int rc = lrtmp2_ertmp_multitrack_parse(&mt, raw_audio, sizeof(raw_audio));
    ASSERT(rc == LRTMP2_OK, "multitrack_parse OK");
    ASSERT(mt.type == LRTMP2_MULTITRACK_TYPE_AUDIO, "type == audio");
    ASSERT(strcmp(mt.track_name, "audio_track") == 0, "name == audio_track");

    /* Write roundtrip */
    uint8_t buf[64];
    size_t written = lrtmp2_ertmp_multitrack_write(&mt, buf, sizeof(buf));
    ASSERT(written > 0, "multitrack_write > 0");

    /* Video track: type=1 (video), name="video" (5 chars) */
    uint8_t raw_vid[] = {
        0x00, /* marker */
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, /* number = 1 (video) */
        0x00, 0x05, 'v', 'i', 'd', 'e', 'o'
    };
    rc = lrtmp2_ertmp_multitrack_parse(&mt, raw_vid, sizeof(raw_vid));
    ASSERT(rc == LRTMP2_OK, "multitrack_parse video OK");
    ASSERT(mt.type == LRTMP2_MULTITRACK_TYPE_VIDEO, "type == video");
    ASSERT(strcmp(mt.track_name, "video") == 0, "name == video");
}

/* ── E-RTMP v2: modex ───────────────────────────────────────────── */

static void test_modex(void) {
    printf("\n--- modex ---\n");

    lrtmp2_modex_t modex;

    /* NOP */
    uint8_t nop_raw[] = { 0x80 };
    int rc = lrtmp2_ertmp_modex_parse(&modex, nop_raw, sizeof(nop_raw));
    ASSERT(rc == LRTMP2_OK, "modex_parse NOP OK");
    ASSERT(modex.type == LRTMP2_MODEX_TYPE_NOP, "NOP type");

    /* TIMESTAMP offset = 0x123456789ABCDEF0 */
    uint8_t ts_raw[] = {
        0x81, /* marker | TIMESTAMP */
        0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0
    };
    rc = lrtmp2_ertmp_modex_parse(&modex, ts_raw, sizeof(ts_raw));
    ASSERT(rc == LRTMP2_OK, "modex_parse TIMESTAMP OK");
    ASSERT(modex.type == LRTMP2_MODEX_TYPE_TIMESTAMP, "TIMESTAMP type");
    ASSERT(modex.offset == 0x123456789ABCDEF0ULL, "offset value");

    /* Write roundtrip */
    uint8_t buf[16];
    size_t written = lrtmp2_ertmp_modex_write(&modex, buf, sizeof(buf));
    ASSERT(written == 9, "modex_write TIMESTAMP → 9 bytes");
    ASSERT(memcmp(buf, ts_raw, 9) == 0, "modex_write matches");

    /* NOP write */
    modex.type = LRTMP2_MODEX_TYPE_NOP;
    modex.offset = 0;
    written = lrtmp2_ertmp_modex_write(&modex, buf, sizeof(buf));
    ASSERT(written == 1, "modex_write NOP → 1 byte");
    ASSERT(buf[0] == 0x80, "NOP byte == 0x80");

    /* Edge cases */
    rc = lrtmp2_ertmp_modex_parse(&modex, nop_raw, 0);
    ASSERT(rc == LRTMP2_ERR_IO, "empty → IO");

    /* Invalid marker (no high bit) */
    uint8_t bad_raw[] = { 0x00 };
    rc = lrtmp2_ertmp_modex_parse(&modex, bad_raw, sizeof(bad_raw));
    ASSERT(rc == LRTMP2_ERR_PROTOCOL, "no marker → PROTOCOL");

    /* Unknown type — should be ignored gracefully */
    uint8_t unknown_raw[] = { 0x85, 0xDE, 0xAD }; /* type=5, undefined */
    rc = lrtmp2_ertmp_modex_parse(&modex, unknown_raw, sizeof(unknown_raw));
    ASSERT(rc == LRTMP2_OK, "unknown type → OK (graceful)");
    ASSERT(modex.type == LRTMP2_MODEX_TYPE_NOP, "unknown → NOP");
}

/* ── Main ─────────────────────────────────────────────────────────── */

int test_ertmp_main(void) {
    printf("=== librtmp2 E-RTMP v1+v2 unit tests ===");

    test_fourcc();
    test_exvideo();
    test_exaudio();
    test_hdr();
    test_fourcc_list();
    test_caps();
    test_reconnect();
    test_multitrack();
    test_modex();

    printf("\n=== Results: %d failures ===\n", g_failures);
    return g_failures > 0 ? 1 : 0;
}
