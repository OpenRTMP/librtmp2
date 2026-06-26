/**
 * metadata.c — Enhanced RTMP v1 metadata / HDR / colorInfo support
 *
 * Per E-RMP v1 §8, the onMetaData script object may carry colorInfo
 * (color primaries, transfer characteristics, matrix coefficients) to
 * indicate HDR content. The videocodecid field is also enhanced to carry
 * a FourCC (UI32) for newer codecs.
 *
 * This module provides:
 * - Parse: read colorInfo sub-object from raw AMF0 data
 * - Serialize: write colorInfo into an AMF0 object
 * - Utility: map FourCC → videocodecid UI32
 */
#include "ertmp.h"
#include "librtmp2/types.h"
#include <string.h>

/* ── HDR color information ────────────────────────────────────────── */

int lrtmp2_ertmp_hdr_init(lrtmp2_hdr_info_t *hdr) {
    if (!hdr) return LRTMP2_ERR_INTERNAL;
    hdr->color_primaries = 1;  /* BT.709 default */
    hdr->transfer_chars = 1;   /* SDR default */
    hdr->matrix_coeffs = 1;    /* BT.709 default */
    return LRTMP2_OK;
}

int lrtmp2_ertmp_hdr_parse(const uint8_t *data, size_t len,
                            lrtmp2_hdr_info_t *hdr)
{
    if (!hdr) return LRTMP2_ERR_INTERNAL;
    lrtmp2_ertmp_hdr_init(hdr);
    if (!data || len < 2) return LRTMP2_ERR_IO;

    /* The colorInfo block is sent as a 10-byte blob by some encoders:
     *   color_primaries  (2 bytes, big-endian)
     *   transfer_chars   (2 bytes, big-endian)
     *   matrix_coeffs    (2 bytes, big-endian)
     *   + 4 bytes reserved / optional LF EOT marker
     * We accept either 6 or 10 bytes. */
    if (len < 6) return LRTMP2_ERR_IO;

    hdr->color_primaries = (uint16_t)((data[0] << 8) | data[1]);
    hdr->transfer_chars  = (uint16_t)((data[2] << 8) | data[3]);
    hdr->matrix_coeffs   = (uint16_t)((data[4] << 8) | data[5]);
    return LRTMP2_OK;
}

size_t lrtmp2_ertmp_hdr_write(const lrtmp2_hdr_info_t *hdr,
                               uint8_t *buf, size_t buf_size)
{
    if (!hdr || !buf || buf_size < 6) return 0;
    buf[0] = (uint8_t)(hdr->color_primaries >> 8);
    buf[1] = (uint8_t)(hdr->color_primaries);
    buf[2] = (uint8_t)(hdr->transfer_chars >> 8);
    buf[3] = (uint8_t)(hdr->transfer_chars);
    buf[4] = (uint8_t)(hdr->matrix_coeffs >> 8);
    buf[5] = (uint8_t)(hdr->matrix_coeffs);
    return 6;
}

/* ── Metadata colorInfo (AMF0 sub-object) ────────────────────────── */

int lrtmp2_ertmp_metadata_colorinfo_parse(const uint8_t *data, size_t len,
                                           lrtmp2_hdr_info_t *hdr)
{
    /* E-RTMP v1 sends colorInfo as an ECMAArray inside onMetaData.
     * For simplicity, we scan for a 6-byte or 10-byte blob starting
     * at the colorInfo key. In practice, clients write it as:
     *   colorInfo: { "colorPrimaries": N, "transferCharacteristics": N,
     *                "matrixCoefficients": N }
     * or as a raw blob. We handle the raw blob case here. */
    if (!hdr) return LRTMP2_ERR_INTERNAL;
    lrtmp2_ertmp_hdr_init(hdr);
    if (!data || len < 6) return LRTMP2_ERR_IO;
    return lrtmp2_ertmp_hdr_parse(data, len, hdr);
}

/* ── videocodecid (FourCC as UI32) ──────────────────────────────────── */

uint32_t lrtmp2_ertmp_videocodecid_from_fourcc(const char *fourcc)
{
    if (!fourcc) return 7; /* default AVC */
    uint32_t cid = 0;
    cid |= (uint32_t)(uint8_t)fourcc[0];
    cid = (cid << 8) | (uint32_t)(uint8_t)fourcc[1];
    cid = (cid << 8) | (uint32_t)(uint8_t)fourcc[2];
    cid = (cid << 8) | (uint32_t)(uint8_t)fourcc[3];
    return cid;
}

const char *lrtmp2_ertmp_metadata_parse(const uint8_t *data, size_t len,
                                        char *buf, size_t buf_size) {
    (void)data; (void)len; (void)buf; (void)buf_size;
    return "E-RTMP v1 metadata; use lrtmp2_ertmp_*_parse for specifics";
}

/* ── Stub for E-RTMP v2 capability negotiation ───────────────────── */
int lrtmp2_ertmp_caps_negotiate(lrtmp2_buffer_t *buf) {
    (void)buf;
    return LRTMP2_ERR_UNSUPPORTED;
}
