//! Outbound audio/video/data message encoding with per-chunk-stream header
//! compression, shared by server connections and the client.

use crate::buffer::Buffer;
use crate::chunk::state::RTMP_WIRE_MAX_MSG_LENGTH;
use crate::chunk::writer::{
    EXTENDED_TIMESTAMP_MARKER, chunk_body_len, first_header_len, write_chunk_body,
    write_first_header,
};
use crate::message::message::RTMP_MSG_AMF0_DATA;
use crate::types::{ErrorCode, FrameType, Result};

/// Chunk stream id, message type id and [`MediaHeaderTracker`] slot for a
/// media frame type.
pub(crate) fn media_chunk_stream(frame_type: FrameType) -> (u32, u8, usize) {
    match frame_type {
        FrameType::Audio => (4, 0x08, 0),
        FrameType::Script | FrameType::Metadata => (5, RTMP_MSG_AMF0_DATA, 1),
        FrameType::Video => (6, 0x09, 2),
    }
}

/// Last media message header sent on one outbound chunk stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MediaOutHeader {
    msg_stream_id: u32,
    timestamp: u32,
    msg_length: u32,
    msg_type_id: u8,
}

/// A chosen first-chunk header for one outbound media message.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MediaFirstHeader {
    csid: u32,
    fmt: u8,
    ts_field: u32,
    slot: usize,
    current: MediaOutHeader,
}

impl MediaFirstHeader {
    pub(crate) fn encoded_len(&self) -> usize {
        first_header_len(self.csid, self.fmt, self.ts_field)
    }

    pub(crate) fn write(&self, out: &mut Buffer) -> Result<()> {
        write_first_header(
            out,
            self.csid,
            self.fmt,
            self.ts_field,
            self.current.msg_length,
            self.current.msg_type_id,
            self.current.msg_stream_id,
        )
    }
}

/// Tracks the last header sent on the audio (csid 4), data (csid 5) and
/// video (csid 6) chunk streams so later messages can use fmt=1/2 headers.
#[derive(Debug, Clone, Default)]
pub(crate) struct MediaHeaderTracker {
    last: [Option<MediaOutHeader>; 3],
}

impl MediaHeaderTracker {
    /// Choose the smallest first-chunk header the peer can decode for a new
    /// media message, given the last one sent on the same chunk stream.
    ///
    /// fmt=1/2 carry a timestamp *delta*, so they are only used when the
    /// previous message on this chunk stream went to the same message
    /// stream, time did not go backwards, and neither timestamp needs the
    /// extended field (keeping extended-timestamp handling, which peers
    /// implement inconsistently for deltas, on the fmt=0 path only).
    pub(crate) fn choose(
        &self,
        compact: bool,
        frame_type: FrameType,
        msg_stream_id: u32,
        timestamp: u32,
        payload_len: usize,
    ) -> Result<MediaFirstHeader> {
        if payload_len > RTMP_WIRE_MAX_MSG_LENGTH as usize {
            return Err(ErrorCode::Internal);
        }
        let (csid, msg_type_id, slot) = media_chunk_stream(frame_type);
        let current = MediaOutHeader {
            msg_stream_id,
            timestamp,
            msg_length: payload_len as u32,
            msg_type_id,
        };
        let (fmt, ts_field) = match self.last[slot] {
            Some(prev)
                if compact
                    && prev.msg_stream_id == msg_stream_id
                    && prev.timestamp <= timestamp
                    && timestamp < EXTENDED_TIMESTAMP_MARKER =>
            {
                let fmt =
                    if prev.msg_length == current.msg_length && prev.msg_type_id == msg_type_id {
                        2
                    } else {
                        1
                    };
                (fmt, timestamp - prev.timestamp)
            }
            _ => (0, timestamp),
        };
        Ok(MediaFirstHeader {
            csid,
            fmt,
            ts_field,
            slot,
            current,
        })
    }

    /// Record `header` as sent; call only once it is fully queued.
    pub(crate) fn commit(&mut self, header: MediaFirstHeader) {
        self.last[header.slot] = Some(header.current);
    }
}

/// Chunk a media message's payload (everything after its first-chunk
/// header) onto `out`. The result depends only on the frame type,
/// timestamp, payload and `chunk_size`, so a relay can encode it once and
/// put a different [`MediaFirstHeader`] in front of it per receiver.
pub(crate) fn encode_media_body(
    out: &mut Buffer,
    frame_type: FrameType,
    timestamp: u32,
    payload: &[u8],
    chunk_size: usize,
) -> Result<()> {
    let (csid, _, _) = media_chunk_stream(frame_type);
    let ext_ts = (timestamp >= EXTENDED_TIMESTAMP_MARKER).then_some(timestamp);
    write_chunk_body(out, csid, payload, chunk_size, ext_ts)
}

/// Queue a complete media message (first header + chunk body) on `out`,
/// using and updating `tracker` for header compression. Nothing is written
/// if the message does not fit.
pub(crate) fn write_media_message(
    out: &mut Buffer,
    tracker: &mut MediaHeaderTracker,
    compact: bool,
    frame_type: FrameType,
    msg_stream_id: u32,
    timestamp: u32,
    payload: &[u8],
    chunk_size: usize,
) -> Result<()> {
    let header = tracker.choose(compact, frame_type, msg_stream_id, timestamp, payload.len())?;
    let (csid, _, _) = media_chunk_stream(frame_type);
    let ext = timestamp >= EXTENDED_TIMESTAMP_MARKER;
    out.reserve(header.encoded_len() + chunk_body_len(csid, payload.len(), chunk_size, ext))?;
    header.write(out)?;
    encode_media_body(out, frame_type, timestamp, payload, chunk_size)?;
    tracker.commit(header);
    Ok(())
}

/// Queue a media message whose chunk body [`encode_media_body`] already
/// produced for the same frame and chunk size.
pub(crate) fn write_media_message_with_body(
    out: &mut Buffer,
    tracker: &mut MediaHeaderTracker,
    compact: bool,
    frame_type: FrameType,
    msg_stream_id: u32,
    timestamp: u32,
    payload_len: usize,
    body: &[u8],
) -> Result<()> {
    let header = tracker.choose(compact, frame_type, msg_stream_id, timestamp, payload_len)?;
    out.reserve(header.encoded_len() + body.len())?;
    header.write(out)?;
    out.write(body)?;
    tracker.commit(header);
    Ok(())
}
