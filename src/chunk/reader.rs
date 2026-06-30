//! Chunk reader
//!
//! Mirrors `src/chunk/chunk_reader.h` and `src/chunk/chunk_reader.c`.

use crate::buffer::Buffer;
use crate::chunk::state::{ChunkRegistry, ChunkStream, MAX_REASSEMBLY_BYTES_PER_CONN, MAX_CHUNK_STREAMS};
use crate::bytes::ntoh32;
use crate::types::Result;
use crate::types::ErrorCode;

/// A chunk message (resembled from one or more chunk reads).
#[derive(Debug, Clone, Default)]
pub struct ChunkMessage {
    pub csid: u32,
    pub fmt: u8,
    pub timestamp: u32,
    pub msg_length: u32,
    pub msg_type_id: u8,
    pub msg_stream_id: u32,
    pub is_complete: bool,
}

/// Read a single chunk from the buffer, reassembling into complete messages.
///
/// Returns:
/// - Ok(1) with is_complete=true when a full message is ready
/// - Ok(0) when more data is needed
/// - Err on protocol errors
pub fn chunk_read(
    buf: &mut Buffer,
    reg: &mut ChunkRegistry,
    _unused: Option<&()>,
    msg: &mut ChunkMessage,
    payload: &mut *const u8,
    payload_len: &mut usize,
) -> Result<i32> {
    let available = buf.available();
    if available < 1 {
        return Ok(0);
    }

    // Peek at the first byte to determine fmt and csid
    let first = buf.peek()[0];
    let fmt = first >> 6;
    let csid_low = (first & 0x3F) as u32;

    let csid = match csid_low {
        0 => {
            if available < 2 { return Ok(0); }
            buf.peek()[1] as u32 + 64
        }
        1 => {
            if available < 3 { return Ok(0); }
            let peek = buf.peek();
            ((peek[1] as u32) | ((peek[2] as u32) << 8)) + 64
        }
        n => n,
    };

    // Determine header size based on fmt
    let header_size = match csid {
        0 => 2,
        1 => 3,
        _ => 1,
    };

    let msg_header_size = match fmt {
        0 => 11 + header_size, // timestamp(3) + length(3) + typeid(1) + streamid(4)
        1 => 7 + header_size,  // timestamp(3) + length(3) + typeid(1)
        2 => 3 + header_size,  // timestamp(3)
        3 => header_size,      // nothing
        _ => return Err(ErrorCode::Chunk),
    };

    if available < msg_header_size {
        return Ok(0);
    }

    // Consume basic header
    let mut hdr = vec![0u8; header_size];
    buf.read(&mut hdr).map_err(|_| ErrorCode::Io)?;

    // Read message header based on fmt
    let timestamp: u32;
    let msg_length: u32;
    let msg_type_id: u8;
    let msg_stream_id: u32;
    let ext_ts: bool;

    match fmt {
        0 => {
            let mut mh = [0u8; 11];
            buf.read(&mut mh).map_err(|_| ErrorCode::Io)?;
            timestamp = ((mh[0] as u32) << 16) | ((mh[1] as u32) << 8) | (mh[2] as u32);
            msg_length = ((mh[3] as u32) << 16) | ((mh[4] as u32) << 8) | (mh[5] as u32);
            msg_type_id = mh[6];
            msg_stream_id = (mh[7] as u32) | ((mh[8] as u32) << 8) | ((mh[9] as u32) << 16) | ((mh[10] as u32) << 24);
            ext_ts = timestamp >= 0xFFFFFF;
        }
        1 => {
            let mut mh = [0u8; 7];
            buf.read(&mut mh).map_err(|_| ErrorCode::Io)?;
            timestamp = ((mh[0] as u32) << 16) | ((mh[1] as u32) << 8) | (mh[2] as u32);
            msg_length = ((mh[3] as u32) << 16) | ((mh[4] as u32) << 8) | (mh[5] as u32);
            msg_type_id = mh[6];
            msg_stream_id = 0; // carried from previous
            ext_ts = timestamp >= 0xFFFFFF;
        }
        2 => {
            let mut mh = [0u8; 3];
            buf.read(&mut mh).map_err(|_| ErrorCode::Io)?;
            timestamp = ((mh[0] as u32) << 16) | ((mh[1] as u32) << 8) | (mh[2] as u32);
            msg_length = 0;
            msg_type_id = 0;
            msg_stream_id = 0;
            ext_ts = timestamp >= 0xFFFFFF;
        }
        3 => {
            timestamp = 0;
            msg_length = 0;
            msg_type_id = 0;
            msg_stream_id = 0;
            ext_ts = false;
        }
        _ => return Err(ErrorCode::Chunk),
    };

    // Resolve chunk stream index (index access avoids overlapping borrows later).
    let idx = match reg.streams.iter().position(|s| s.csid == csid && s.in_use) {
        Some(i) => i,
        None => {
            if reg.streams.len() >= MAX_CHUNK_STREAMS {
                return Err(ErrorCode::Chunk);
            }
            reg.streams.push(ChunkStream {
                csid,
                in_use: true,
                chunk_size: reg.default_chunk_size,
                ..ChunkStream::default()
            });
            reg.streams.len() - 1
        }
    };

    // fmt 0/1/2: ext ts follows the message header when the 24-bit field is 0xFFFFFF.
    // fmt 3: continuation chunks repeat ext ts when the message started with one.
    let needs_ext_ts = match fmt {
        0 | 1 | 2 => ext_ts,
        3 => reg.streams[idx].type0_ext_ts,
        _ => false,
    };

    let final_timestamp = if needs_ext_ts {
        if buf.available() < 4 {
            return Ok(0);
        }
        let mut ts_buf = [0u8; 4];
        buf.read(&mut ts_buf).map_err(|_| ErrorCode::Io)?;
        match fmt {
            3 => reg.streams[idx].type0_timestamp,
            _ => ntoh32(&ts_buf),
        }
    } else if fmt == 3 {
        reg.streams[idx].type0_timestamp
    } else {
        timestamp
    };

    // fmt 0/1 start a new message; discard any partial reassembly from the prior one.
    if fmt == 0 || fmt == 1 {
        reg.streams[idx].reassembly_bytes_read = 0;
        reg.streams[idx].reassembly_buf.reset();
    }

    // Update stream state based on fmt
    match fmt {
        0 => {
            reg.streams[idx].type0_timestamp = final_timestamp;
            reg.streams[idx].type0_msg_length = msg_length;
            reg.streams[idx].type0_msg_type_id = msg_type_id;
            reg.streams[idx].type0_msg_stream_id = msg_stream_id;
            reg.streams[idx].type0_ext_ts = ext_ts;
        }
        1 => {
            reg.streams[idx].type0_timestamp = final_timestamp;
            reg.streams[idx].type0_msg_length = msg_length;
            reg.streams[idx].type0_msg_type_id = msg_type_id;
        }
        2 => {
            reg.streams[idx].type0_timestamp = final_timestamp;
        }
        _ => {}
    }

    let effective_ts = reg.streams[idx].type0_timestamp;
    let effective_length = reg.streams[idx].type0_msg_length;
    let effective_type_id = reg.streams[idx].type0_msg_type_id;
    let effective_stream_id = reg.streams[idx].type0_msg_stream_id;

    // Determine how many bytes to read for this chunk
    let chunk_size = reg.streams[idx].chunk_size as usize;
    let remaining = (effective_length as usize)
        .saturating_sub(reg.streams[idx].reassembly_bytes_read as usize);
    let to_read = remaining.min(chunk_size);

    if buf.available() < to_read {
        return Ok(0);
    }

    if to_read > 0 {
        let total: usize = reg.streams.iter()
            .filter(|s| s.in_use)
            .map(|s| s.reassembly_buf.available())
            .sum();
        if total + to_read > MAX_REASSEMBLY_BYTES_PER_CONN {
            return Err(ErrorCode::Chunk);
        }
    }

    // Read payload chunk
    let mut chunk_data = vec![0u8; to_read];
    buf.read(&mut chunk_data).map_err(|_| ErrorCode::Io)?;
    reg.streams[idx].reassembly_buf.write(&chunk_data).map_err(|_| ErrorCode::Chunk)?;
    reg.streams[idx].reassembly_bytes_read += to_read as u32;

    // Check if message is complete
    if reg.streams[idx].reassembly_bytes_read >= effective_length {
        msg.csid = csid;
        msg.fmt = fmt;
        msg.timestamp = effective_ts;
        msg.msg_length = effective_length;
        msg.msg_type_id = effective_type_id;
        msg.msg_stream_id = effective_stream_id;
        msg.is_complete = true;

        // Return a pointer to the reassembly buffer data.
        // This is safe because the caller consumes the payload before the next chunk_read.
        let data = reg.streams[idx].reassembly_buf.peek();
        *payload_len = data.len();
        *payload = data.as_ptr();

        // Reset for next message
        reg.streams[idx].reassembly_bytes_read = 0;
        reg.streams[idx].reassembly_buf.reset();

        Ok(1)
    } else {
        msg.is_complete = false;
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::writer::chunk_write;

    #[test]
    fn partial_extended_timestamp_waits_for_more_data() {
        let payload = b"x";
        let msg = ChunkMessage {
            csid: 3,
            fmt: 0,
            timestamp: 0x0100_0000,
            msg_length: payload.len() as u32,
            msg_type_id: 0x09,
            msg_stream_id: 1,
            is_complete: false,
        };

        let mut wire = Buffer::new();
        chunk_write(&mut wire, &msg, payload, payload.len(), 128).unwrap();

        // Stop before the 4-byte extended timestamp field.
        let partial = wire.peek()[..12].to_vec();
        let mut buf = Buffer::new();
        buf.write(&partial).unwrap();

        let mut reg = ChunkRegistry::new();
        let mut out_msg = ChunkMessage::default();
        let mut ptr = std::ptr::null();
        let mut len = 0usize;

        assert_eq!(
            chunk_read(&mut buf, &mut reg, None, &mut out_msg, &mut ptr, &mut len).unwrap(),
            0
        );
        assert!(!out_msg.is_complete);
    }

    #[test]
    fn fragmented_extended_timestamp_round_trips() {
        let payload = vec![0xCD_u8; 300];
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

    #[test]
    fn fmt0_new_message_resets_partial_reassembly() {
        let mut reg = ChunkRegistry::new();
        let mut wire = Buffer::new();

        // fmt=0, length 200 — only the first 128-byte chunk arrives.
        let first = ChunkMessage {
            csid: 7,
            fmt: 0,
            timestamp: 1,
            msg_length: 200,
            msg_type_id: 0x09,
            msg_stream_id: 1,
            is_complete: false,
        };
        chunk_write(&mut wire, &first, &vec![0x11; 200], 200, 128).unwrap();
        // First RTMP chunk only: 1-byte basic hdr + 11-byte msg hdr + 128-byte payload.
        let first_chunk_bytes = 1 + 11 + 128;
        let mut buf = Buffer::new();
        buf.write(&wire.peek()[..first_chunk_bytes]).unwrap();

        let mut out_msg = ChunkMessage::default();
        let mut ptr = std::ptr::null();
        let mut len = 0usize;
        assert_eq!(
            chunk_read(&mut buf, &mut reg, None, &mut out_msg, &mut ptr, &mut len).unwrap(),
            0
        );

        // Peer starts a new fmt=0 message on the same csid before finishing the first.
        let second = ChunkMessage {
            csid: 7,
            fmt: 0,
            timestamp: 2,
            msg_length: 4,
            msg_type_id: 0x14,
            msg_stream_id: 1,
            is_complete: false,
        };
        chunk_write(&mut buf, &second, b"done", 4, 128).unwrap();

        let rc = chunk_read(&mut buf, &mut reg, None, &mut out_msg, &mut ptr, &mut len).unwrap();
        assert_eq!(rc, 1);
        assert_eq!(out_msg.msg_length, 4);
        let received = unsafe { std::slice::from_raw_parts(ptr, len) };
        assert_eq!(received, b"done");
    }
}
