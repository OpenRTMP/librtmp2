from pathlib import Path
from textwrap import dedent


def block(value: str) -> str:
    return dedent(value).strip("\n")


def replace_once(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text()
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"expected exactly one match in {path}, got {count}")
    file_path.write_text(text.replace(old, new, 1))


def insert_before_once(path: str, marker: str, addition: str) -> None:
    file_path = Path(path)
    text = file_path.read_text()
    if addition in text:
        return
    count = text.count(marker)
    if count != 1:
        raise RuntimeError(f"expected exactly one marker in {path}, got {count}")
    file_path.write_text(text.replace(marker, addition + marker, 1))


replace_once(
    "src/ertmp/connect_amf.rs",
    block(
        """
        let len = read_u16(buf)? as usize;
        if len < 4 || buf.available() < len {
            return Err(ErrorCode::Amf);
        }
        let mut cc = vec![0u8; len];
        buf.read(&mut cc).map_err(|_| ErrorCode::Amf)?;
        Ok(i32::from_be_bytes([cc[0], cc[1], cc[2], cc[3]]))
        """
    ),
    block(
        """
        let len = read_u16(buf)? as usize;
        if !(4..=MAX_CAPS_BLOB_BYTES).contains(&len) || buf.available() < len {
            return Err(ErrorCode::Amf);
        }
        let mut cc = [0u8; 4];
        buf.read(&mut cc).map_err(|_| ErrorCode::Amf)?;
        if len > cc.len() {
            buf.drain(len - cc.len());
        }
        Ok(i32::from_be_bytes(cc))
        """
    ),
)

replace_once(
    "src/session/conn.rs",
    block(
        """
    /// ModEx-normalized bytes used only for codec parsing and cache classification.
    pub cache_payload: Vec<u8>,
        """
    ),
    block(
        """
    /// ModEx-normalized bytes used only for codec parsing and cache classification.
    /// `None` means the normalized bytes are identical to `payload`.
    pub cache_payload: Option<Vec<u8>>,
        """
    ),
)

insert_before_once(
    "src/session/conn.rs",
    "pub struct Conn {\n",
    block(
        """
impl RelayFrame {
    /// Bytes used for codec parsing and cache classification.
    pub fn cache_payload(&self) -> &[u8] {
        self.cache_payload.as_deref().unwrap_or(&self.payload)
    }

    fn retained_bytes(&self) -> usize {
        self.payload.len().saturating_add(
            self.cache_payload
                .as_ref()
                .map(|payload| payload.len())
                .unwrap_or(0),
        )
    }
}

        """
    ),
)

replace_once(
    "src/session/conn.rs",
    block(
        """
    fn pending_relay_bytes(&self) -> usize {
        self.pending_relay.iter().map(|f| f.payload.len()).sum()
    }
        """
    ),
    block(
        """
    fn pending_relay_bytes(&self) -> usize {
        self.pending_relay
            .iter()
            .map(RelayFrame::retained_bytes)
            .sum()
    }
        """
    ),
)

replace_once(
    "src/session/conn.rs",
    block(
        """
        if self.pending_relay.len() >= MAX_PENDING_RELAY_FRAMES
            || self.pending_relay_bytes() + payload.len() > self.max_pending_relay_bytes
        {
            return Err(ErrorCode::Internal);
        }
        self.pending_relay.push(RelayFrame {
            frame_type,
            timestamp,
            payload: payload.to_vec(),
            cache_payload: cache_payload.to_vec(),
            app: self.app.clone(),
            stream_name: self.relay_route_key(),
            publisher_conn_id: self.conn_id,
        });
        """
    ),
    block(
        """
        let cache_payload = if cache_payload == payload {
            None
        } else {
            Some(cache_payload.to_vec())
        };
        let retained_bytes = payload.len().saturating_add(
            cache_payload
                .as_ref()
                .map(|payload| payload.len())
                .unwrap_or(0),
        );
        if self.pending_relay.len() >= MAX_PENDING_RELAY_FRAMES
            || self
                .pending_relay_bytes()
                .saturating_add(retained_bytes)
                > self.max_pending_relay_bytes
        {
            return Err(ErrorCode::Internal);
        }
        self.pending_relay.push(RelayFrame {
            frame_type,
            timestamp,
            payload: payload.to_vec(),
            cache_payload,
            app: self.app.clone(),
            stream_name: self.relay_route_key(),
            publisher_conn_id: self.conn_id,
        });
        """
    ),
)

server_path = Path("src/server/mod.rs")
server_text = server_path.read_text()
server_refs = server_text.count("&frame.cache_payload")
if server_refs == 0:
    raise RuntimeError("expected server cache_payload references")
server_path.write_text(server_text.replace("&frame.cache_payload", "frame.cache_payload()"))

insert_before_once(
    "src/media/init_cache.rs",
    "pub fn populate_av_frame(frame: &mut Frame, payload: &[u8]) {\n",
    block(
        """
/// Fill codec/header fields for a per-track callback extracted from an E-RTMP
/// multitrack container. Inner track payloads do not include the enhanced tag
/// header or FourCC, so metadata comes from the track descriptor.
pub fn populate_multitrack_frame(
    frame: &mut Frame,
    fourcc_value: [u8; 4],
    packet_type: u8,
    video_frame_type: u8,
) {
    let mut cc = [0u8; 5];
    cc[..4].copy_from_slice(&fourcc_value);
    match frame.frame_type {
        FrameType::Video => {
            frame.video_fourcc = FourCc { cc };
            frame.video_frame_type = video_frame_type;
            if let Ok(codec) = fourcc::fourcc_to_video_codec(&fourcc_value) {
                frame.video_codec = codec;
            }
            frame.is_metadata =
                u8::from(packet_type == crate::types::ERTMP_PACKET_TYPE_METADATA);
        }
        FrameType::Audio => {
            frame.audio_fourcc = FourCc { cc };
            if let Ok(codec) = fourcc::fourcc_to_audio_codec(&fourcc_value) {
                frame.audio_codec = codec;
            }
            frame.is_metadata =
                u8::from(packet_type == crate::types::ERTMP_AUDIO_PACKET_TYPE_METADATA);
        }
        FrameType::Script | FrameType::Metadata => {
            frame.is_metadata = 1;
        }
    }
}

        """
    ),
)

replace_once(
    "src/session/conn.rs",
    "use crate::media::{is_on_metadata_payload, normalize_modex_payload, populate_av_frame};",
    block(
        """
use crate::media::{
    is_on_metadata_payload, normalize_modex_payload, populate_av_frame,
    populate_multitrack_frame,
};
        """
    ),
)

replace_once(
    "src/session/conn.rs",
    block(
        """
            let mut track_ranges: Vec<(u8, usize, usize)> = Vec::new();
            foreach_track(frame_type, parse_payload, |track| {
                let start = track.payload.as_ptr() as usize - parse_payload.as_ptr() as usize;
                track_ranges.push((track.track_id, start, track.payload.len()));
            });
        """
    ),
    block(
        """
            let mut track_ranges: Vec<(u8, usize, usize, [u8; 4], u8, u8)> = Vec::new();
            foreach_track(frame_type, parse_payload, |track| {
                let start = track.payload.as_ptr() as usize - parse_payload.as_ptr() as usize;
                track_ranges.push((
                    track.track_id,
                    start,
                    track.payload.len(),
                    track.fourcc,
                    track.packet_type,
                    track.video_frame_type,
                ));
            });
        """
    ),
)

replace_once(
    "src/session/conn.rs",
    block(
        """
                for (track_id, start, len) in track_ranges {
                    self.invoke_on_frame_cb(
                        cb,
                        frame_type,
                        timestamp,
                        track_id,
                        &parse_payload[start..start + len],
                    );
                }
        """
    ),
    block(
        """
                for (track_id, start, len, fourcc, packet_type, video_frame_type) in track_ranges {
                    self.invoke_multitrack_on_frame_cb(
                        cb,
                        frame_type,
                        timestamp,
                        track_id,
                        fourcc,
                        packet_type,
                        video_frame_type,
                        &parse_payload[start..start + len],
                    );
                }
        """
    ),
)

insert_before_once(
    "src/session/conn.rs",
    "    fn invoke_on_frame_cb(\n",
    block(
        """
    fn invoke_multitrack_on_frame_cb(
        &mut self,
        cb: fn(&Frame),
        frame_type: FrameType,
        timestamp: u32,
        track_id: u8,
        fourcc: [u8; 4],
        packet_type: u8,
        video_frame_type: u8,
        payload: &[u8],
    ) {
        self.frame_cb_scratch.clear();
        self.frame_cb_scratch.extend_from_slice(payload);
        let mut frame = Frame {
            frame_type,
            timestamp,
            size: self.frame_cb_scratch.len() as u32,
            data: self.frame_cb_scratch.as_ptr(),
            track_id,
            ..Default::default()
        };
        populate_multitrack_frame(&mut frame, fourcc, packet_type, video_frame_type);
        cb(&frame);
    }

        """
    ),
)

replace_once(
    "src/client/mod.rs",
    "use crate::media::{is_on_metadata_payload, populate_av_frame};",
    "use crate::media::{is_on_metadata_payload, populate_av_frame, populate_multitrack_frame};",
)

replace_once(
    "src/client/mod.rs",
    block(
        """
        let mut track_ranges: Vec<(u8, usize, usize)> = Vec::new();
        foreach_track(frame_type, &payload, |track| {
            let start = track.payload.as_ptr() as usize - payload.as_ptr() as usize;
            track_ranges.push((track.track_id, start, track.payload.len()));
        });
        """
    ),
    block(
        """
        let mut track_ranges: Vec<(u8, usize, usize, [u8; 4], u8, u8)> = Vec::new();
        foreach_track(frame_type, &payload, |track| {
            let start = track.payload.as_ptr() as usize - payload.as_ptr() as usize;
            track_ranges.push((
                track.track_id,
                start,
                track.payload.len(),
                track.fourcc,
                track.packet_type,
                track.video_frame_type,
            ));
        });
        """
    ),
)

replace_once(
    "src/client/mod.rs",
    block(
        """
            for (track_id, start, len) in track_ranges {
                self.invoke_on_frame_cb(
                    cb,
                    frame_type,
                    timestamp,
                    track_id,
                    &payload[start..start + len],
                );
            }
        """
    ),
    block(
        """
            for (track_id, start, len, fourcc, packet_type, video_frame_type) in track_ranges {
                self.invoke_multitrack_on_frame_cb(
                    cb,
                    frame_type,
                    timestamp,
                    track_id,
                    fourcc,
                    packet_type,
                    video_frame_type,
                    &payload[start..start + len],
                );
            }
        """
    ),
)

insert_before_once(
    "src/client/mod.rs",
    "    fn invoke_on_frame_cb(\n",
    block(
        """
    fn invoke_multitrack_on_frame_cb(
        &mut self,
        cb: fn(&Frame),
        frame_type: FrameType,
        timestamp: u32,
        track_id: u8,
        fourcc: [u8; 4],
        packet_type: u8,
        video_frame_type: u8,
        payload: &[u8],
    ) {
        self.frame_cb_scratch.clear();
        self.frame_cb_scratch.extend_from_slice(payload);
        let mut frame = Frame {
            frame_type,
            timestamp,
            size: self.frame_cb_scratch.len() as u32,
            data: self.frame_cb_scratch.as_ptr(),
            track_id,
            ..Default::default()
        };
        populate_multitrack_frame(&mut frame, fourcc, packet_type, video_frame_type);
        cb(&frame);
    }

        """
    ),
)

insert_before_once(
    "src/ertmp/connect_amf.rs",
    "    #[test]\n    fn string_fourcc_value_is_read_after_consumed_marker() {\n",
    block(
        """
    #[test]
    fn string_fourcc_value_rejects_oversized_input() {
        let mut buf = Buffer::new();
        let oversized = "x".repeat(MAX_CAPS_BLOB_BYTES + 1);
        amf0::write_string(&mut buf, &oversized).unwrap();
        assert_eq!(read_fourcc_number(&mut buf), Err(ErrorCode::Amf));
    }

        """
    ),
)

insert_before_once(
    "src/media/init_cache.rs",
    "    #[test]\n    fn enhanced_hevc_sequence_start_is_cached() {\n",
    block(
        """
    #[test]
    fn multitrack_video_callback_metadata_uses_track_descriptor() {
        let mut frame = Frame {
            frame_type: FrameType::Video,
            ..Default::default()
        };
        populate_multitrack_frame(&mut frame, *b"hvc1", 1, 1);
        assert_eq!(&frame.video_fourcc.cc[..4], b"hvc1");
        assert_eq!(frame.video_codec, VideoCodec::H265);
        assert_eq!(frame.video_frame_type, 1);
    }

    #[test]
    fn multitrack_audio_callback_metadata_uses_track_descriptor() {
        let mut frame = Frame {
            frame_type: FrameType::Audio,
            ..Default::default()
        };
        populate_multitrack_frame(&mut frame, *b"Opus", 1, 0);
        assert_eq!(&frame.audio_fourcc.cc[..4], b"Opus");
        assert_eq!(frame.audio_codec, AudioCodec::Opus);
    }

        """
    ),
)

insert_before_once(
    "src/session/conn.rs",
    "    #[test]\n    fn relay_route_key_prefers_relay_key_over_rtmp_name() {\n",
    block(
        """
    #[test]
    fn relay_budget_counts_actual_retained_bytes() {
        let mut conn = Conn::new();
        conn.max_pending_relay_bytes = 6;

        assert!(
            conn.queue_relay_frame(FrameType::Video, 0, b"data", b"data")
                .is_ok()
        );
        assert_eq!(conn.pending_relay_bytes(), 4);
        assert!(conn.pending_relay[0].cache_payload.is_none());
        assert_eq!(conn.pending_relay[0].cache_payload(), b"data");

        assert!(
            conn.queue_relay_frame(FrameType::Video, 0, b"x", b"y")
                .is_ok()
        );
        assert_eq!(conn.pending_relay_bytes(), 6);
        assert!(
            conn.queue_relay_frame(FrameType::Video, 0, b"z", b"z")
                .is_err()
        );
    }

        """
    ),
)

Path(".github/workflows/apply-qodo-findings.yml").unlink()
Path(__file__).unlink()
