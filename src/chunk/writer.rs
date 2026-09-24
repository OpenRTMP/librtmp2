//! Chunk writer
//!
//! Mirrors `src/chunk/chunk_writer.h`, `src/chunk/chunk_write.h`, and `src/chunk/chunk_writer.c`.

use crate::buffer::Buffer;
use crate::chunk::reader::ChunkMessage;
use crate::chunk::state::RTMP_WIRE_MAX_MSG_LENGTH;
use crate::types::ErrorCode;
use crate::types::Result;

/// Write a full message to `out`, fragmenting the payload into chunks of at most `chunk_size` bytes.
pub fn chunk_write(
    out: &mut Buffer,
    msg: &ChunkMessage,
    payload: &[u8],
    payload_len: usize,
    chunk_size: usize,
) -> Result<()> {
    if chunk_size == 0 {
        return chunk_write(out, msg, payload, payload_len, 128);
    }

    // Only fmt=0 is safe for this stateless writer:
    // - fmt=1/2 carry a timestamp *delta* on the wire (RTMP spec 5.3.1.1),
    //   which the reader adds to the running per-CSID timestamp; this writer
    //   has no per-CSID prior-timestamp state, so it cannot compute a delta.
    // - fmt=3 carries no message header at all; its extended-timestamp field is
    //   present only when the inherited per-CSID header used one, which this
    //   writer cannot know either.
    // Emitting `msg.timestamp` for any of these would desynchronize the far
    // end, so reject caller-supplied fmt 1/2/3. The continuation chunks
    // generated internally below are unaffected.
    if msg.fmt != 0 {
        return Err(ErrorCode::Internal);
    }

    // The 24-bit message-length field cannot represent values above
    // RTMP_WIRE_MAX_MSG_LENGTH; reject rather than silently truncating the
    // header while still writing the full payload.
    if msg.msg_length > RTMP_WIRE_MAX_MSG_LENGTH {
        return Err(ErrorCode::Internal);
    }

    if payload_len != msg.msg_length as usize {
        return Err(ErrorCode::Internal);
    }
    if payload_len > payload.len() {
        return Err(ErrorCode::Internal);
    }

    let ext_ts = msg.timestamp >= EXTENDED_TIMESTAMP_MARKER;
    out.reserve(
        first_header_len(msg.csid, 0, msg.timestamp)
            + chunk_body_len(msg.csid, payload_len, chunk_size, ext_ts),
    )
    .map_err(|_| ErrorCode::Internal)?;
    write_first_header(
        out,
        msg.csid,
        0,
        msg.timestamp,
        msg.msg_length,
        msg.msg_type_id,
        msg.msg_stream_id,
    )?;
    write_chunk_body(
        out,
        msg.csid,
        &payload[..payload_len],
        chunk_size,
        ext_ts.then_some(msg.timestamp),
    )
}

/// 24-bit timestamp field value that signals a trailing 4-byte extended
/// timestamp.
pub(crate) const EXTENDED_TIMESTAMP_MARKER: u32 = 0xFFFFFF;

/// Encoded size of a first-chunk header written by [`write_first_header`].
pub fn first_header_len(csid: u32, fmt: u8, ts_field: u32) -> usize {
    let (_, basic_len) = basic_header(csid, fmt);
    let msg_hdr = match fmt {
        0 => 11,
        1 => 7,
        2 => 3,
        _ => 0,
    };
    let ext = if fmt < 3 && ts_field >= EXTENDED_TIMESTAMP_MARKER {
        4
    } else {
        0
    };
    basic_len + msg_hdr + ext
}

/// Write the first chunk's basic + message header for a new message.
///
/// `fmt` is 0, 1 or 2 (RTMP spec 5.3.1.2). For fmt=0 `ts_field` is the
/// absolute timestamp; for fmt=1/2 it is the delta from the previous message
/// on this chunk stream, which the caller must track. fmt=1 omits the stream
/// id; fmt=2 also omits length and type, so the caller must only pick them
/// when those match the previous message on `csid`.
pub fn write_first_header(
    out: &mut Buffer,
    csid: u32,
    fmt: u8,
    ts_field: u32,
    msg_length: u32,
    msg_type_id: u8,
    msg_stream_id: u32,
) -> Result<()> {
    if fmt > 2 {
        return Err(ErrorCode::Internal);
    }
    let ext_ts = ts_field >= EXTENDED_TIMESTAMP_MARKER;
    let mut hdr = [0u8; 3 + 11 + 4];
    let (basic, basic_len) = basic_header(csid, fmt);
    hdr[..basic_len].copy_from_slice(&basic[..basic_len]);
    let mut n = basic_len;
    let mut field = [0u8; 3];
    hton24(
        &mut field,
        if ext_ts {
            EXTENDED_TIMESTAMP_MARKER
        } else {
            ts_field
        },
    );
    hdr[n..n + 3].copy_from_slice(&field);
    n += 3;
    if fmt <= 1 {
        hton24(&mut field, msg_length);
        hdr[n..n + 3].copy_from_slice(&field);
        n += 3;
        hdr[n] = msg_type_id;
        n += 1;
    }
    if fmt == 0 {
        hdr[n..n + 4].copy_from_slice(&msg_stream_id.to_le_bytes());
        n += 4;
    }
    if ext_ts {
        hdr[n..n + 4].copy_from_slice(&ts_field.to_be_bytes());
        n += 4;
    }
    out.write(&hdr[..n]).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Encoded size of [`write_chunk_body`]'s output.
pub fn chunk_body_len(csid: u32, payload_len: usize, chunk_size: usize, ext_ts: bool) -> usize {
    let chunk_size = if chunk_size == 0 { 128 } else { chunk_size };
    let (_, cont_len) = basic_header(csid, 3);
    let continuation_chunks = payload_len.saturating_sub(1) / chunk_size;
    payload_len + continuation_chunks * (cont_len + if ext_ts { 4 } else { 0 })
}

/// Write a message payload after its first-chunk header, split into chunks
/// of at most `chunk_size` bytes with fmt=3 continuation headers in between.
///
/// `ext_ts` must be `Some(field)` exactly when the first header carried an
/// extended timestamp, with the same 4-byte value: continuation chunks
/// repeat it so the reader (which inherits `type0_ext_ts` for the CSID)
/// stays in sync. The output depends only on `csid`, `chunk_size`, `ext_ts`
/// and the payload, not on the first header's fmt, so a relay can encode it
/// once and reuse it behind a different first header per receiver.
pub fn write_chunk_body(
    out: &mut Buffer,
    csid: u32,
    payload: &[u8],
    chunk_size: usize,
    ext_ts: Option<u32>,
) -> Result<()> {
    let chunk_size = if chunk_size == 0 { 128 } else { chunk_size };
    out.reserve(chunk_body_len(
        csid,
        payload.len(),
        chunk_size,
        ext_ts.is_some(),
    ))
    .map_err(|_| ErrorCode::Internal)?;
    let mut chdr = [0u8; 3 + 4];
    let (cont, cont_len) = basic_header(csid, 3);
    chdr[..cont_len].copy_from_slice(&cont[..cont_len]);
    let mut chdr_len = cont_len;
    if let Some(ts) = ext_ts {
        chdr[chdr_len..chdr_len + 4].copy_from_slice(&ts.to_be_bytes());
        chdr_len += 4;
    }
    let mut chunks = payload.chunks(chunk_size).peekable();
    while let Some(chunk) = chunks.next() {
        out.write(chunk).map_err(|_| ErrorCode::Internal)?;
        if chunks.peek().is_some() {
            out.write(&chdr[..chdr_len])
                .map_err(|_| ErrorCode::Internal)?;
        }
    }
    Ok(())
}

/// Write an extended timestamp chunk (for protocol control messages).
pub fn chunk_write_extended_timestamp(out: &mut Buffer, timestamp: u32) -> Result<()> {
    // Basic header: fmt=3, csid=2 (protocol control)
    let hdr = (3u8 << 6) | 2;
    out.write(&[hdr]).map_err(|_| ErrorCode::Internal)?;

    // 4 bytes extended timestamp
    out.write(&timestamp.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;

    Ok(())
}

/// Build a basic header for the given csid and fmt.
///
/// Returns a fixed-size, stack-allocated buffer plus the number of leading
/// bytes that are valid (1-3, matching the RTMP basic-header encoding). This
/// runs once per outbound message and again per continuation chunk for a
/// fragmented message, so on a busy relay (many viewers, large frames split
/// across `chunk_size`) a heap allocation here would run per chunk per
/// viewer; a `[u8; 3]` avoids that entirely.
fn basic_header(csid: u32, fmt: u8) -> ([u8; 3], usize) {
    if csid < 64 {
        ([(fmt << 6) | (csid as u8), 0, 0], 1)
    } else if csid < 320 {
        ([fmt << 6, (csid - 64) as u8, 0], 2)
    } else {
        (
            [
                (fmt << 6) | 1,
                ((csid - 64) & 0xFF) as u8,
                (((csid - 64) >> 8) & 0xFF) as u8,
            ],
            3,
        )
    }
}

/// Write a 24-bit big-endian value.
fn hton24(buf: &mut [u8; 3], val: u32) {
    buf[0] = ((val >> 16) & 0xFF) as u8;
    buf[1] = ((val >> 8) & 0xFF) as u8;
    buf[2] = (val & 0xFF) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::reader::chunk_read;
    use crate::chunk::state::ChunkRegistry;

    #[test]
    fn single_chunk_round_trips_through_reader() {
        let payload = b"hello rtmp";
        let msg = ChunkMessage {
            csid: 3,
            fmt: 0,
            timestamp: 1234,
            msg_length: payload.len() as u32,
            msg_type_id: 0x14,
            msg_stream_id: 1,
            is_complete: false,
        };

        let mut wire = Buffer::new();
        chunk_write(&mut wire, &msg, payload, payload.len(), 128).unwrap();

        let mut reg = ChunkRegistry::new();
        let mut out_msg = ChunkMessage::default();
        let mut ptr = std::ptr::null();
        let mut len = 0usize;
        let rc = chunk_read(&mut wire, &mut reg, None, &mut out_msg, &mut ptr, &mut len).unwrap();

        assert_eq!(rc, 1);
        assert!(out_msg.is_complete);
        assert_eq!(out_msg.csid, 3);
        assert_eq!(out_msg.timestamp, 1234);
        assert_eq!(out_msg.msg_type_id, 0x14);
        assert_eq!(out_msg.msg_stream_id, 1);
        let received = unsafe { std::slice::from_raw_parts(ptr, len) };
        assert_eq!(received, payload);
    }

    #[test]
    fn fragmented_chunks_round_trip_through_reader() {
        let payload = vec![0xAB_u8; 300];
        let msg = ChunkMessage {
            csid: 4,
            fmt: 0,
            timestamp: 0,
            msg_length: payload.len() as u32,
            msg_type_id: 0x09,
            msg_stream_id: 1,
            is_complete: false,
        };

        let mut wire = Buffer::new();
        chunk_write(&mut wire, &msg, &payload, payload.len(), 128).unwrap();

        let mut reg = ChunkRegistry::new();
        let mut out_msg = ChunkMessage::default();
        let mut ptr = std::ptr::null();
        let mut len = 0usize;
        // chunk_write fragments the payload across multiple 128-byte
        // chunks; chunk_read consumes one chunk per call, so drive it
        // until the reassembled message is complete.
        let mut rc;
        loop {
            rc = chunk_read(&mut wire, &mut reg, None, &mut out_msg, &mut ptr, &mut len).unwrap();
            if rc == 1 || (rc == 0 && wire.available() == 0) {
                break;
            }
        }

        assert_eq!(rc, 1);
        let received = unsafe { std::slice::from_raw_parts(ptr, len) };
        assert_eq!(received, payload.as_slice());
    }

    #[test]
    fn extended_timestamp_round_trips_big_endian() {
        let payload = b"x";
        let msg = ChunkMessage {
            csid: 5,
            fmt: 0,
            timestamp: 0x0100_0000,
            msg_length: payload.len() as u32,
            msg_type_id: 0x09,
            msg_stream_id: 1,
            is_complete: false,
        };

        let mut wire = Buffer::new();
        chunk_write(&mut wire, &msg, payload, payload.len(), 128).unwrap();

        let mut reg = ChunkRegistry::new();
        let mut out_msg = ChunkMessage::default();
        let mut ptr = std::ptr::null();
        let mut len = 0usize;
        chunk_read(&mut wire, &mut reg, None, &mut out_msg, &mut ptr, &mut len).unwrap();

        assert_eq!(out_msg.timestamp, 0x0100_0000);
    }

    #[test]
    fn rejects_payload_length_mismatch_for_fmt0() {
        let payload = b"hello";
        let msg = ChunkMessage {
            csid: 3,
            fmt: 0,
            timestamp: 0,
            msg_length: 99,
            msg_type_id: 0x14,
            msg_stream_id: 1,
            is_complete: false,
        };
        let mut wire = Buffer::new();
        assert_eq!(
            chunk_write(&mut wire, &msg, payload, payload.len(), 128),
            Err(ErrorCode::Internal)
        );
    }

    #[test]
    fn fmt1_to_fmt3_are_rejected_by_stateless_writer() {
        let payload = b"x";
        for fmt in [1u8, 2u8, 3u8] {
            let msg = ChunkMessage {
                csid: 3,
                fmt,
                timestamp: 5,
                msg_length: payload.len() as u32,
                msg_type_id: 0x14,
                msg_stream_id: 1,
                is_complete: false,
            };
            let mut wire = Buffer::new();
            assert_eq!(
                chunk_write(&mut wire, &msg, payload, payload.len(), 128),
                Err(ErrorCode::Internal),
                "fmt={fmt} must be rejected (needs per-CSID state the writer lacks)"
            );
        }
    }

    #[test]
    fn rejects_msg_length_above_wire_max() {
        let msg = ChunkMessage {
            csid: 3,
            fmt: 0,
            timestamp: 0,
            msg_length: RTMP_WIRE_MAX_MSG_LENGTH + 1,
            msg_type_id: 0x14,
            msg_stream_id: 1,
            is_complete: false,
        };
        let mut wire = Buffer::new();
        assert_eq!(
            chunk_write(&mut wire, &msg, &[], 0, 128),
            Err(ErrorCode::Internal)
        );
    }

    #[test]
    fn fragmented_extended_timestamp_round_trips() {
        // Internally generated continuation chunks must carry the same 4-byte
        // extended timestamp the reader inherits for the CSID.
        let payload = vec![0xAB_u8; 300];
        let msg = ChunkMessage {
            csid: 6,
            fmt: 0,
            timestamp: 0x0100_0000,
            msg_length: payload.len() as u32,
            msg_type_id: 0x09,
            msg_stream_id: 1,
            is_complete: false,
        };
        let mut wire = Buffer::new();
        chunk_write(&mut wire, &msg, &payload, payload.len(), 128).unwrap();

        let mut reg = ChunkRegistry::new();
        let mut out_msg = ChunkMessage::default();
        let mut ptr = std::ptr::null();
        let mut len = 0usize;
        let mut rc;
        loop {
            rc = chunk_read(&mut wire, &mut reg, None, &mut out_msg, &mut ptr, &mut len).unwrap();
            if rc == 1 || (rc == 0 && wire.available() == 0) {
                break;
            }
        }
        assert_eq!(rc, 1);
        assert_eq!(out_msg.timestamp, 0x0100_0000);
        let received = unsafe { std::slice::from_raw_parts(ptr, len) };
        assert_eq!(received, payload.as_slice());
    }
}
