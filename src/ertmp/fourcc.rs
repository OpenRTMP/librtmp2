//! FourCC codec registry and dispatch for Enhanced RTMP v1
//!
//! Mirrors `src/ertmp/fourcc.c`.

use crate::types::{VideoCodec, AudioCodec, Result, ErrorCode};

/* ── Video FourCC table ── */

struct VideoFourCcEntry {
    fourcc: [u8; 4],
    codec: VideoCodec,
    name: &'static str,
}

static VIDEO_FOURCCS: &[VideoFourCcEntry] = &[
    VideoFourCcEntry { fourcc: *b"avc1", codec: VideoCodec::H264, name: "H.264/AVC" },
    VideoFourCcEntry { fourcc: *b"hvc1", codec: VideoCodec::H265, name: "H.265/HEVC" },
    VideoFourCcEntry { fourcc: *b"av01", codec: VideoCodec::Av1, name: "AV1" },
    VideoFourCcEntry { fourcc: *b"vp09", codec: VideoCodec::Vp6, name: "VP9" },
];

/* ── Audio FourCC table ── */

struct AudioFourCcEntry {
    fourcc: [u8; 4],
    codec: AudioCodec,
    name: &'static str,
}

static AUDIO_FOURCCS: &[AudioFourCcEntry] = &[
    AudioFourCcEntry { fourcc: *b"Opus", codec: AudioCodec::Opus, name: "Opus" },
    AudioFourCcEntry { fourcc: *b"mp4a", codec: AudioCodec::Aac, name: "AAC" },
    AudioFourCcEntry { fourcc: *b"mp3 ", codec: AudioCodec::Mp3, name: "MP3" },
    AudioFourCcEntry { fourcc: *b"ec-3", codec: AudioCodec::G711A, name: "Dolby Digital Plus" },
];

/// Convert a FourCC string to a video codec.
pub fn fourcc_to_video_codec(fourcc: &[u8]) -> Result<VideoCodec> {
    for entry in VIDEO_FOURCCS {
        if fourcc[..4] == entry.fourcc {
            return Ok(entry.codec);
        }
    }
    Ok(VideoCodec::H264) // fallback
}

/// Convert a FourCC string to an audio codec.
pub fn fourcc_to_audio_codec(fourcc: &[u8]) -> Result<AudioCodec> {
    for entry in AUDIO_FOURCCS {
        if fourcc[..4] == entry.fourcc {
            return Ok(entry.codec);
        }
    }
    Ok(AudioCodec::Aac) // fallback
}

/// Convert a video codec to its FourCC string.
pub fn video_codec_to_fourcc(codec: VideoCodec) -> &'static str {
    for entry in VIDEO_FOURCCS {
        if entry.codec == codec {
            return std::str::from_utf8(&entry.fourcc).unwrap_or("avc1");
        }
    }
    "avc1"
}

/// Convert an audio codec to its FourCC string.
pub fn audio_codec_to_fourcc(codec: AudioCodec) -> &'static str {
    for entry in AUDIO_FOURCCS {
        if entry.codec == codec {
            return std::str::from_utf8(&entry.fourcc).unwrap_or("mp4a");
        }
    }
    "mp4a"
}

/// Get the human-readable name for a video FourCC.
pub fn fourcc_video_name(fourcc: &[u8]) -> Option<&'static str> {
    for entry in VIDEO_FOURCCS {
        if fourcc[..4] == entry.fourcc {
            return Some(entry.name);
        }
    }
    None
}

/// Get the human-readable name for an audio FourCC.
pub fn fourcc_audio_name(fourcc: &[u8]) -> Option<&'static str> {
    for entry in AUDIO_FOURCCS {
        if fourcc[..4] == entry.fourcc {
            return Some(entry.name);
        }
    }
    None
}
