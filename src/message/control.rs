//! RTMP control message encoder/decoder
//!
//! Mirrors `src/message/control.h` and `src/message/control.c`.

use crate::buffer::Buffer;
use crate::bytes::ntoh32;
use crate::types::ErrorCode;
use crate::types::Result;

/// RTMP control message types.
///
/// These are the `msg_type_id` values of RTMP protocol control messages. Each
/// control message must be sent on chunk stream 2 with message stream 0, per the
/// RTMP specification.
///
/// SetChunkSize: changes the chunk payload size for subsequent chunks.
pub const CTRL_SET_CHUNK_SIZE: u8 = 0x01;
/// AbortMessage: discards a partially transmitted message on a chunk stream.
pub const CTRL_ABORT_MESSAGE: u8 = 0x02;
/// Acknowledgement: reports bytes received so far.
pub const CTRL_ACKNOWLEDGEMENT: u8 = 0x03;
/// UserControlMessage: carries a [`UCTRL_*`](UCTRL_STREAM_BEGIN) event.
pub const CTRL_USER_CONTROL: u8 = 0x04;
/// WindowAckSize: how many bytes may arrive before an Acknowledgement is due.
pub const CTRL_WINDOW_ACK_SIZE: u8 = 0x05;
/// SetPeerBandwidth: limits the peer's output bandwidth.
pub const CTRL_SET_PEER_BANDWIDTH: u8 = 0x06;

/// RTMP User Control event types.
///
/// The 16-bit event type that prefixes a [`CTRL_USER_CONTROL`] message.
///
/// StreamBegin: the stream is ready for playback.
pub const UCTRL_STREAM_BEGIN: u16 = 0x00;
/// StreamEOF: the stream ended.
pub const UCTRL_STREAM_EOF: u16 = 0x01;
/// StreamDry: the stream has no more data.
pub const UCTRL_STREAM_DRY: u16 = 0x02;
/// SetBufferLength: a player's buffer length in milliseconds.
pub const UCTRL_SET_BUFFER_LENGTH: u16 = 0x03;
/// StreamIsRecorded: the stream is being recorded.
pub const UCTRL_STREAM_IS_RECORDED: u16 = 0x04;
/// PingRequest: ask the peer to send a PingResponse.
pub const UCTRL_PING_REQUEST: u16 = 0x06;
/// PingResponse: answer to a PingRequest.
pub const UCTRL_PING_RESPONSE: u16 = 0x07;

/// Minimum inbound chunk size accepted from peers. Values below the RTMP default
/// (128) enable extreme per-byte allocation/parsing amplification during
/// message reassembly (a 4 MiB message becomes millions of chunk iterations).
const MIN_CHUNK_SIZE: u32 = 128;
const MAX_CHUNK_SIZE: u32 = 0xFFFFFF;
/// Upper bound for the chunk size a server announces to its peers.
pub const MAX_INBOUND_CHUNK_SIZE: u32 = 65536;
/// Largest chunk size that changes chunk parsing. RTMP allows peers to
/// announce up to 0x7FFFFFFF, but no message is longer than 0xFFFFFF bytes, so
/// any larger chunk size reads every message in a single chunk exactly like
/// this one does. Chunk size only bounds how much of an already
/// length-checked message is read per chunk; it sizes no buffer.
pub const MAX_PEER_CHUNK_SIZE: u32 = 0x00FF_FFFF;
/// Reject peer WindowAckSize values below this to prevent ACK-per-byte amplification.
pub const MIN_WINDOW_ACK_SIZE: u32 = 1024;

/* ── Encoder ── */

/// Write a SetChunkSize control message.
///
/// Emits the bare 4-byte chunk size only. The RTMP control message type
/// (`CTRL_SET_CHUNK_SIZE`) lives in the message header, not the payload, so it
/// must NOT be prepended here (the matching decoder reads a bare value).
///
/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
/// use librtmp2::message::control;
///
/// let mut buf = Buffer::new();
/// control::write_set_chunk_size(&mut buf, 4096).unwrap();
/// assert_eq!(buf.peek(), &0x0000_1000u32.to_be_bytes());
/// ```
pub fn write_set_chunk_size(buf: &mut Buffer, chunk_size: u32) -> Result<()> {
    buf.write(&chunk_size.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an AbortMessage control message.
///
/// Bare 4-byte chunk-stream id; the control type is carried by the message
/// header, not the payload.

/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
/// use librtmp2::message::control;
///
/// let mut buf = Buffer::new();
/// control::write_abort_message(&mut buf, 4).unwrap();
///
/// // Round-trips through the matching reader.
/// assert_eq!(control::read_abort_message(buf.peek()).unwrap(), 4);
/// ```
pub fn write_abort_message(buf: &mut Buffer, csid: u32) -> Result<()> {
    buf.write(&csid.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write an Acknowledgement control message.
///
/// Bare 4-byte sequence number; the control type is carried by the message
/// header, not the payload.

/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
/// use librtmp2::message::control;
///
/// let mut buf = Buffer::new();
/// control::write_acknowledgement(&mut buf, 123_456).unwrap();
/// assert_eq!(control::read_acknowledgement_size(buf.peek()).unwrap(), 123_456);
/// ```
pub fn write_acknowledgement(buf: &mut Buffer, sequence_number: u32) -> Result<()> {
    buf.write(&sequence_number.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a WindowAckSize control message.
///
/// Bare 4-byte window size; the control type is carried by the message header,
/// not the payload.

/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
/// use librtmp2::message::control;
///
/// let mut buf = Buffer::new();
/// control::write_window_ack_size(&mut buf, 2_500_000).unwrap();
/// assert_eq!(buf.peek(), &2_500_000u32.to_be_bytes());
/// ```
pub fn write_window_ack_size(buf: &mut Buffer, window_size: u32) -> Result<()> {
    buf.write(&window_size.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a SetPeerBandwidth control message.
///
/// Bare 4-byte window size followed by the 1-byte limit type; the control type
/// is carried by the message header, not the payload.

/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
/// use librtmp2::message::control;
///
/// let mut buf = Buffer::new();
/// control::write_set_peer_bandwidth(&mut buf, 3_125_000, 2).unwrap();
///
/// let (window, limit_type) = control::read_set_peer_bandwidth(buf.peek()).unwrap();
/// assert_eq!(window, 3_125_000);
/// assert_eq!(limit_type, 2);
/// ```
pub fn write_set_peer_bandwidth(buf: &mut Buffer, window_size: u32, limit_type: u8) -> Result<()> {
    buf.write(&window_size.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&[limit_type]).map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a User Control Stream Begin event.

/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
/// use librtmp2::message::control;
///
/// let mut buf = Buffer::new();
/// control::write_user_control_stream_begin(&mut buf, 1).unwrap();
///
/// let (event, stream_id, _) = control::read_user_control(buf.peek(), false).unwrap();
/// assert_eq!(event, control::UCTRL_STREAM_BEGIN);
/// assert_eq!(stream_id, 1);
/// ```
pub fn write_user_control_stream_begin(buf: &mut Buffer, stream_id: u32) -> Result<()> {
    buf.write(&UCTRL_STREAM_BEGIN.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&stream_id.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a User Control Stream EOF event.

/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
/// use librtmp2::message::control;
///
/// let mut buf = Buffer::new();
/// control::write_user_control_stream_eof(&mut buf, 1).unwrap();
///
/// let (event, stream_id, _) = control::read_user_control(buf.peek(), false).unwrap();
/// assert_eq!(event, control::UCTRL_STREAM_EOF);
/// assert_eq!(stream_id, 1);
/// ```
pub fn write_user_control_stream_eof(buf: &mut Buffer, stream_id: u32) -> Result<()> {
    buf.write(&UCTRL_STREAM_EOF.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&stream_id.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a User Control SetBufferLength event.

/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
/// use librtmp2::message::control;
///
/// let mut buf = Buffer::new();
/// // Stream 1, 3000 ms of buffer.
/// control::write_user_control_set_buffer_length(&mut buf, 1, 3000).unwrap();
///
/// let (event, stream_id, len) = control::read_user_control(buf.peek(), true).unwrap();
/// assert_eq!(event, control::UCTRL_SET_BUFFER_LENGTH);
/// assert_eq!(stream_id, 1);
/// assert_eq!(len, Some(3000));
/// ```
pub fn write_user_control_set_buffer_length(
    buf: &mut Buffer,
    stream_id: u32,
    ms: u32,
) -> Result<()> {
    buf.write(&UCTRL_SET_BUFFER_LENGTH.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&stream_id.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&ms.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a User Control Ping Request event (4-byte opaque timestamp/token).

/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
/// use librtmp2::message::control;
///
/// let mut buf = Buffer::new();
/// control::write_user_control_ping_request(&mut buf, 0xDEAD_BEEF).unwrap();
///
/// // A PingRequest payload is 6 bytes: the event type plus the opaque token,
/// // so the token arrives as `param1` and there is no second parameter.
/// assert_eq!(buf.available(), 6);
/// let (event, token, extra) = control::read_user_control(buf.peek(), true).unwrap();
/// assert_eq!(event, control::UCTRL_PING_REQUEST);
/// assert_eq!(token, 0xDEAD_BEEF);
/// assert_eq!(extra, None);
/// ```
pub fn write_user_control_ping_request(buf: &mut Buffer, timestamp: u32) -> Result<()> {
    buf.write(&UCTRL_PING_REQUEST.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&timestamp.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/// Write a User Control Ping Response event (echoes the request timestamp/token).

/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
/// use librtmp2::message::control;
///
/// let mut buf = Buffer::new();
/// control::write_user_control_ping_response(&mut buf, 0xDEAD_BEEF).unwrap();
///
/// let (event, _, _) = control::read_user_control(buf.peek(), true).unwrap();
/// assert_eq!(event, control::UCTRL_PING_RESPONSE);
/// ```
pub fn write_user_control_ping_response(buf: &mut Buffer, timestamp: u32) -> Result<()> {
    buf.write(&UCTRL_PING_RESPONSE.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    buf.write(&timestamp.to_be_bytes())
        .map_err(|_| ErrorCode::Internal)?;
    Ok(())
}

/* ── Decoder ── */

/// Read a SetChunkSize message.

/// # Examples
///
/// ```
/// use librtmp2::message::control;
///
/// // Bare 4-byte payload: the control type lives in the message header.
/// let payload = 4096u32.to_be_bytes();
/// assert_eq!(control::read_set_chunk_size(&payload).unwrap(), 4096);
///
/// // Sizes below the RTMP default (128) are refused as amplification.
/// let tiny = 64u32.to_be_bytes();
/// assert!(control::read_set_chunk_size(&tiny).is_err());
///
/// // Truncated payloads are rejected rather than read out of bounds.
/// assert!(control::read_set_chunk_size(&[0, 0]).is_err());
/// ```
pub fn read_set_chunk_size(data: &[u8]) -> Result<u32> {
    if data.len() < 4 {
        return Err(ErrorCode::Protocol);
    }
    let cs = ntoh32(data);
    // The top bit is reserved and must be zero. Sizes under 128 are refused:
    // they only multiply per-chunk work, and no real encoder sends them.
    if cs < MIN_CHUNK_SIZE || cs & 0x8000_0000 != 0 {
        return Err(ErrorCode::Protocol);
    }
    Ok(cs.min(MAX_PEER_CHUNK_SIZE))
}

/// Read an AbortMessage.

/// # Examples
///
/// ```
/// use librtmp2::message::control;
///
/// let payload = 4u32.to_be_bytes();
/// assert_eq!(control::read_abort_message(&payload).unwrap(), 4);
/// assert!(control::read_abort_message(&[]).is_err());
/// ```
pub fn read_abort_message(data: &[u8]) -> Result<u32> {
    if data.len() < 4 {
        return Err(ErrorCode::Protocol);
    }
    Ok(ntoh32(data))
}

/// Read an Acknowledgement size.

/// # Examples
///
/// ```
/// use librtmp2::message::control;
///
/// let payload = 1_000_000u32.to_be_bytes();
/// assert_eq!(control::read_acknowledgement_size(&payload).unwrap(), 1_000_000);
/// ```
pub fn read_acknowledgement_size(data: &[u8]) -> Result<u32> {
    if data.len() < 4 {
        return Err(ErrorCode::Protocol);
    }
    Ok(ntoh32(data))
}

/// Read a WindowAckSize.

/// # Examples
///
/// ```
/// use librtmp2::message::control;
///
/// let ok = 2500000u32.to_be_bytes();
/// assert_eq!(control::read_window_ack_size(&ok).unwrap(), 2_500_000);
///
/// // A non-zero window below MIN_WINDOW_ACK_SIZE would force ACK-per-byte.
/// let too_small = 16u32.to_be_bytes();
/// assert!(control::read_window_ack_size(&too_small).is_err());
///
/// // Zero disables acknowledgements and is accepted.
/// assert_eq!(control::read_window_ack_size(&[0, 0, 0, 0]).unwrap(), 0);
/// ```
pub fn read_window_ack_size(data: &[u8]) -> Result<u32> {
    if data.len() < 4 {
        return Err(ErrorCode::Protocol);
    }
    let win = ntoh32(data);
    if win > 0 && win < MIN_WINDOW_ACK_SIZE {
        return Err(ErrorCode::Protocol);
    }
    Ok(win)
}

/// Read a SetPeerBandwidth.

/// # Examples
///
/// ```
/// use librtmp2::message::control;
///
/// // Four-byte window followed by a one-byte limit type.
/// let payload = [0x00, 0x2F, 0xAF, 0x08, 0x02];
/// let (window, limit_type) = control::read_set_peer_bandwidth(&payload).unwrap();
/// assert_eq!(window, 3_125_000);
/// assert_eq!(limit_type, 2);
/// ```
pub fn read_set_peer_bandwidth(data: &[u8]) -> Result<(u32, u8)> {
    if data.len() < 5 {
        return Err(ErrorCode::Protocol);
    }
    Ok((ntoh32(data), data[4]))
}

/// Read a User Control event.

/// # Examples
///
/// ```
/// use librtmp2::message::control;
///
/// // PingRequest (0x0006) carrying an opaque token, plus the 4-byte payload.
/// let mut payload = vec![0x00, 0x06];
/// payload.extend_from_slice(&7u32.to_be_bytes());
/// payload.extend_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
///
/// let (event, param1, param2) = control::read_user_control(&payload, true).unwrap();
/// assert_eq!(event, librtmp2::message::control::UCTRL_PING_REQUEST);
/// assert_eq!(param1, 7);
/// assert_eq!(param2, Some(0xDEAD_BEEF));
///
/// // With `param2 = false` the trailing word is not read.
/// let (_, _, none) = control::read_user_control(&payload, false).unwrap();
/// assert_eq!(none, None);
/// ```
pub fn read_user_control(data: &[u8], param2: bool) -> Result<(u16, u32, Option<u32>)> {
    if data.len() < 6 {
        return Err(ErrorCode::Protocol);
    }
    let event_type = ((data[0] as u16) << 8) | (data[1] as u16);
    let param1 = ntoh32(&data[2..]);
    let p2 = if param2 && data.len() >= 10 {
        Some(ntoh32(&data[6..]))
    } else {
        None
    };
    Ok((event_type, param1, p2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_chunk_size_round_trips_and_is_big_endian() {
        let mut buf = Buffer::new();
        write_set_chunk_size(&mut buf, 4096).unwrap();
        assert_eq!(buf.peek(), &[0x00, 0x00, 0x10, 0x00]);
        assert_eq!(read_set_chunk_size(buf.peek()).unwrap(), 4096);
    }

    #[test]
    fn set_chunk_size_rejects_out_of_range_values() {
        assert!(read_set_chunk_size(&[0, 0, 0, 0]).is_err());
        assert!(read_set_chunk_size(&1u32.to_be_bytes()).is_err());
        assert_eq!(read_set_chunk_size(&128u32.to_be_bytes()).unwrap(), 128);
        assert!(read_set_chunk_size(&[0xFF, 0xFF, 0xFF, 0xFF]).is_err());
        // Spec-legal sizes above 64 KiB are accepted (some encoders announce
        // huge sizes); anything past the max message length is equivalent.
        assert_eq!(
            read_set_chunk_size(&1_000_000u32.to_be_bytes()).unwrap(),
            1_000_000
        );
        assert_eq!(
            read_set_chunk_size(&0x7FFF_FFFFu32.to_be_bytes()).unwrap(),
            MAX_PEER_CHUNK_SIZE
        );
        assert_eq!(
            read_set_chunk_size(&MAX_INBOUND_CHUNK_SIZE.to_be_bytes()).unwrap(),
            MAX_INBOUND_CHUNK_SIZE
        );
    }

    #[test]
    fn window_ack_size_round_trips() {
        let mut buf = Buffer::new();
        write_window_ack_size(&mut buf, 2_500_000).unwrap();
        assert_eq!(read_window_ack_size(buf.peek()).unwrap(), 2_500_000);
    }

    #[test]
    fn window_ack_size_rejects_amplification_values() {
        assert!(read_window_ack_size(&1u32.to_be_bytes()).is_err());
        assert!(read_window_ack_size(&1023u32.to_be_bytes()).is_err());
        assert_eq!(read_window_ack_size(&0u32.to_be_bytes()).unwrap(), 0);
        assert_eq!(read_window_ack_size(&1024u32.to_be_bytes()).unwrap(), 1024);
    }

    #[test]
    fn abort_message_rejects_short_input() {
        // A truncated AbortMessage (fewer than 4 bytes) must be rejected by
        // the decoder itself rather than relying on callers to pre-check the
        // length — `ntoh32()` indexes data[0..4] directly and panics on
        // shorter slices, so this was a reachable DoS if any caller forwarded
        // the payload without its own length guard.
        assert!(read_abort_message(&[]).is_err());
        assert!(read_abort_message(&[0x00]).is_err());
        assert!(read_abort_message(&[0x00, 0x00, 0x00]).is_err());
        assert_eq!(read_abort_message(&7u32.to_be_bytes()).unwrap(), 7);
    }

    #[test]
    fn acknowledgement_size_rejects_short_input() {
        assert!(read_acknowledgement_size(&[]).is_err());
        assert!(read_acknowledgement_size(&[0x00, 0x00]).is_err());
        assert_eq!(
            read_acknowledgement_size(&123u32.to_be_bytes()).unwrap(),
            123
        );
    }

    #[test]
    fn window_ack_size_rejects_short_input() {
        // Same landmine as abort/acknowledgement above: ntoh32() indexes
        // data[0..4] directly, so this decoder must reject short input itself
        // rather than relying on every caller to pre-check the length.
        assert!(read_window_ack_size(&[]).is_err());
        assert!(read_window_ack_size(&[0x00, 0x00, 0x00]).is_err());
        assert_eq!(
            read_window_ack_size(&(MIN_WINDOW_ACK_SIZE).to_be_bytes()).unwrap(),
            MIN_WINDOW_ACK_SIZE
        );
    }

    #[test]
    fn set_peer_bandwidth_round_trips() {
        let mut buf = Buffer::new();
        write_set_peer_bandwidth(&mut buf, 2_500_000, 2).unwrap();
        let (window, limit_type) = read_set_peer_bandwidth(buf.peek()).unwrap();
        assert_eq!(window, 2_500_000);
        assert_eq!(limit_type, 2);
    }

    #[test]
    fn user_control_stream_begin_wire_format_is_big_endian() {
        let mut buf = Buffer::new();
        write_user_control_stream_begin(&mut buf, 1).unwrap();
        assert_eq!(buf.peek(), &[0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);
        let (event_type, stream_id, _) = read_user_control(buf.peek(), false).unwrap();
        assert_eq!(event_type, UCTRL_STREAM_BEGIN);
        assert_eq!(stream_id, 1);
    }

    #[test]
    fn user_control_set_buffer_length_round_trips_both_params() {
        let mut buf = Buffer::new();
        write_user_control_set_buffer_length(&mut buf, 1, 3000).unwrap();
        let (event_type, stream_id, ms) = read_user_control(buf.peek(), true).unwrap();
        assert_eq!(event_type, UCTRL_SET_BUFFER_LENGTH);
        assert_eq!(stream_id, 1);
        assert_eq!(ms, Some(3000));
    }

    #[test]
    fn user_control_ping_round_trips() {
        let mut buf = Buffer::new();
        write_user_control_ping_request(&mut buf, 0x1234_5678).unwrap();
        let (event_type, token, _) = read_user_control(buf.peek(), false).unwrap();
        assert_eq!(event_type, UCTRL_PING_REQUEST);
        assert_eq!(token, 0x1234_5678);

        buf.reset();
        write_user_control_ping_response(&mut buf, 0x1234_5678).unwrap();
        let (event_type, token, _) = read_user_control(buf.peek(), false).unwrap();
        assert_eq!(event_type, UCTRL_PING_RESPONSE);
        assert_eq!(token, 0x1234_5678);
    }
}
