//! Chunk reader
//!
//! Mirrors `src/chunk/chunk_reader.h` and `src/chunk/chunk_reader.c`.

use crate::buffer::Buffer;
use crate::chunk::state::{ChunkRegistry, ChunkStream, DEFAULT_CHUNK_SIZE};
use crate::bytes::{hton24, ntoh32};
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

    // Read extended timestamp if needed.
    // Use buf.available() here, not the stale `available` snapshot captured
    // before the header bytes were consumed.
    let final_timestamp = if ext_ts {
        if buf.available() < 4 { return Ok(0); }
        let mut ts_buf = [0u8; 4];
        buf.read(&mut ts_buf).map_err(|_| ErrorCode::Io)?;
        ntoh32(&ts_buf)
    } else {
        timestamp
    };

    // Get or create the chunk stream
    let stream = reg.get_or_create(csid)?;

    // Update stream state based on fmt
    match fmt {
        0 => {
            stream.type0_timestamp = final_timestamp;
            stream.type0_msg_length = msg_length;
            stream.type0_msg_type_id = msg_type_id;
            stream.type0_msg_stream_id = msg_stream_id;
            stream.type0_ext_ts = ext_ts;
        }
        1 => {
            stream.type0_timestamp = final_timestamp;
            stream.type0_msg_length = msg_length;
            stream.type0_msg_type_id = msg_type_id;
        }
        2 => {
            stream.type0_timestamp = final_timestamp;
        }
        _ => {}
    }

    let effective_ts = stream.type0_timestamp;
    let effective_length = stream.type0_msg_length;
    let effective_type_id = stream.type0_msg_type_id;
    let effective_stream_id = stream.type0_msg_stream_id;

    // Determine how many bytes to read for this chunk
    let chunk_size = stream.chunk_size as usize;
    let remaining = (effective_length as usize).saturating_sub(stream.reassembly_bytes_read as usize);
    let to_read = remaining.min(chunk_size);

    if buf.available() < to_read {
        return Ok(0);
    }

    // Read payload chunk
    let start = stream.reassembly_buf.available();
    let mut chunk_data = vec![0u8; to_read];
    buf.read(&mut chunk_data).map_err(|_| ErrorCode::Io)?;
    stream.reassembly_buf.write(&chunk_data).map_err(|_| ErrorCode::Chunk)?;
    stream.reassembly_bytes_read += to_read as u32;

    // Check if message is complete
    if stream.reassembly_bytes_read >= effective_length {
        msg.csid = csid;
        msg.fmt = fmt;
        msg.timestamp = effective_ts;
        msg.msg_length = effective_length;
        msg.msg_type_id = effective_type_id;
        msg.msg_stream_id = effective_stream_id;
        msg.is_complete = true;

        // Return a pointer to the reassembly buffer data.
        // This is safe because the caller consumes the payload before the next chunk_read.
        let data = stream.reassembly_buf.peek();
        *payload_len = data.len();
        *payload = data.as_ptr();

        // Reset for next message
        stream.reassembly_bytes_read = 0;
        stream.reassembly_buf.reset();

        Ok(1)
    } else {
        msg.is_complete = false;
        Ok(0)
    }
}
