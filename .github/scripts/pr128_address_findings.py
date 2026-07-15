from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}")
    file_path.write_text(text.replace(old, new, 1))


replace_once(
    "src/session/conn.rs",
    """        let cache_payload = if cache_payload == payload {
            None
        } else {
            Some(cache_payload.to_vec())
        };
""",
    """        let cache_payload = if cache_payload.len() == payload.len()
            && std::ptr::eq(cache_payload.as_ptr(), payload.as_ptr())
        {
            None
        } else {
            Some(cache_payload.to_vec())
        };
""",
)

replace_once(
    "src/session/conn.rs",
    """        if let Some(cb) = self.on_frame_cb {
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
            if track_ranges.is_empty() {
                self.invoke_on_frame_cb(cb, frame_type, timestamp, u8::MAX, parse_payload);
            } else {
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
            }
        }
""",
    """        if let Some(cb) = self.on_frame_cb {
            let had_multitrack = foreach_track(frame_type, parse_payload, |track| {
                self.invoke_multitrack_on_frame_cb(
                    cb,
                    frame_type,
                    timestamp,
                    track.track_id,
                    track.fourcc,
                    track.packet_type,
                    track.video_frame_type,
                    track.payload,
                );
            });
            if !had_multitrack {
                self.invoke_on_frame_cb(cb, frame_type, timestamp, u8::MAX, parse_payload);
            }
        }
""",
)

replace_once(
    "src/client/mod.rs",
    """        let mut track_ranges: Vec<(u8, usize, usize, [u8; 4], u8, u8)> = Vec::new();
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
        if track_ranges.is_empty() {
            self.invoke_on_frame_cb(cb, frame_type, timestamp, u8::MAX, &payload);
        } else {
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
        }
""",
    """        let had_multitrack = foreach_track(frame_type, &payload, |track| {
            self.invoke_multitrack_on_frame_cb(
                cb,
                frame_type,
                timestamp,
                track.track_id,
                track.fourcc,
                track.packet_type,
                track.video_frame_type,
                track.payload,
            );
        });
        if !had_multitrack {
            self.invoke_on_frame_cb(cb, frame_type, timestamp, u8::MAX, &payload);
        }
""",
)
