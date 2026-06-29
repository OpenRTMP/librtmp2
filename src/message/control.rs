//! RTMP control message encoder/decoder
//!
//! Mirrors `src/message/control.h` and `src/message/control.c`.

use crate::buffer::Buffer;
use crate::bytes::{byteswap16, hton32, ntoh32};
use crate::types::Result;
use crate::types::ErrorCode;

/* Control message types */
pub const CTRL_SET_CHUNK_SIZE: u8 = 0x01;
pub const CTRL_ABORT_MESSAGE: u8 = 0x02;
pub const CTRL_ACKNOWLEDGEMENT: u8 = 0x03;
pub const CTRL_USER_CONTROL: u8 = 0x04;
pub const CTRL_WINDOW_ACK_SIZE: u8 = 0x05;
pub const CTRL_SET_PEER_BANDWIDTH: u8 = 0x06;

/* User Control event types */
pub const UCTRL_STREAM_BEGIN: u16 = 0x00;
pub const UCTRL_STREAM_EOF: u16 = 0x01;
pub const UCTRL_STREAM_DRY: u16 = 0x02;
pub const UCTRL_SET_BUFFER_LENGTH: u16 = 0x03;
pub const UCTRL_STREAM_IS_RECORDED: u16 = 0x04;
pub const UCTRL_PING_REQUEST: u16 = 0x06;
pub const UCTRL_PING_RESPONSE: u16 = 0x07;

const MIN_CHUNK_SIZE: u32 = 1;
const MAX_CHUNK_SIZE: u32 = 0xFFFFFF;

/* ── Encoder ── */

/// Write a SetChunkSize control message.
pub fn write_set_chunk_size(buf: &mut Buffer, chunk_size: u32) -> Result<()> {
    buf.write(&[CTRL_SET_CHUNK_SIZE]).map_err(|_| ErrorCode::Internal)?;
    let net = hton32(chunk_size);
    buf.write(&net.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an AbortMessage control message.
pub fn write_abort_message(buf: &mut Buffer, csid: u32) -> Result<()> {
    buf.write(&[CTRL_ABORT_MESSAGE]).map_err(|_| ErrorCode::Internal)?;
    let net = hton32(csid);
    buf.write(&net.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an Acknowledgement control message.
pub fn write_acknowledgement(buf: &mut Buffer, sequence_number: u32) -> Result<()> {
    buf.write(&[CTRL_ACKNOWLEDGEMENT]).map_err(|_| ErrorCode::Internal)?;
    let net = hton32(sequence_number);
    buf.write(&net.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a WindowAckSize control message.
pub fn write_window_ack_size(buf: &mut Buffer, window_size: u32) -> Result<()> {
    buf.write(&[CTRL_WINDOW_ACK_SIZE]).map_err(|_| ErrorCode::Internal)?;
    let net = hton32(window_size);
    buf.write(&net.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a SetPeerBandwidth control message.
pub fn write_set_peer_bandwidth(buf: &mut Buffer, window_size: u32, limit_type: u8) -> Result<()> {
    buf.write(&[CTRL_SET_PEER_BANDWIDTH]).map_err(|_| ErrorCode::Internal)?;
    let net = hton32(window_size);
    buf.write(&net.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    buf.write(&[limit_type]).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a User Control Stream Begin event.
pub fn write_user_control_stream_begin(buf: &mut Buffer, stream_id: u32) -> Result<()> {
    let evt = byteswap16(UCTRL_STREAM_BEGIN);
    buf.write(&evt.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    let net = hton32(stream_id);
    buf.write(&net.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a User Control Stream EOF event.
pub fn write_user_control_stream_eof(buf: &mut Buffer, stream_id: u32) -> Result<()> {
    let evt = byteswap16(UCTRL_STREAM_EOF);
    buf.write(&evt.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    let net = hton32(stream_id);
    buf.write(&net.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a User Control SetBufferLength event.
pub fn write_user_control_set_buffer_length(buf: &mut Buffer, stream_id: u32, ms: u32) -> Result<()> {
    let evt = byteswap16(UCTRL_SET_BUFFER_LENGTH);
    buf.write(&evt.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    let net_sid = hton32(stream_id);
    buf.write(&net_sid.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    let net_ms = hton32(ms);
    buf.write(&net_ms.to_be_bytes()).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/* ── Decoder ── */

/// Read a SetChunkSize message.
pub fn read_set_chunk_size(data: &[u8]) -> Result<u32> {
    let cs = ntoh32(data);
    if cs < MIN_CHUNK_SIZE || cs > MAX_CHUNK_SIZE {
        return Err(ErrorCode::Protocol);
    }
    Ok(cs)
}

/// Read an AbortMessage.
pub fn read_abort_message(data: &[u8]) -> Result<u32> {
    Ok(ntoh32(data))
}

/// Read an Acknowledgement size.
pub fn read_acknowledgement_size(data: &[u8]) -> Result<u32> {
    Ok(ntoh32(data))
}

/// Read a WindowAckSize.
pub fn read_window_ack_size(data: &[u8]) -> Result<u32> {
    Ok(ntoh32(data))
}

/// Read a SetPeerBandwidth.
pub fn read_set_peer_bandwidth(data: &[u8]) -> Result<(u32, u8)> {
    Ok((ntoh32(data), data[4]))
}

/// Read a User Control event.
pub fn read_user_control(data: &[u8], param2: bool) -> Result<(u16, u32, Option<u32>)> {
    let event_type = ((data[0] as u16) << 8) | (data[1] as u16);
    let param1 = ntoh32(&data[2..]);
    let p2 = if param2 && data.len() >= 10 {
        Some(ntoh32(&data[6..]))
    } else {
        None
    };
    Ok((event_type, param1, p2))
}
