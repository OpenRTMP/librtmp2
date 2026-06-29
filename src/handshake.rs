//! Legacy RTMP handshake (C0/C1/C2 ↔ S0/S1/S2)
//!
//! Mirrors `src/handshake/handshake.h` and `src/handshake/handshake.c`.

use crate::buffer::Buffer;
use crate::bytes::{hton32, ntoh32};
use crate::types::Result;
use crate::types::ErrorCode;

/// RTMP protocol version byte
const RTMP_VERSION: u8 = 0x03;
/// Handshake payload size
const HANDSHAKE_SIZE: usize = 1536;

/// Handshake state machine states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum HandshakeState {
    ServerWaitC0 = 0,
    ServerWaitC1,
    ServerWaitC2,
    ClientWaitS0,
    ClientWaitS1,
    ClientWaitS2,
    Done,
}

/// Handshake context.
#[derive(Debug)]
pub struct Handshake {
    pub state: HandshakeState,
    pub version: u8,
    pub peer_time: u32,
    /// queued output bytes
    pub out: Buffer,
}

impl Default for Handshake {
    fn default() -> Self {
        Self {
            state: HandshakeState::ServerWaitC0,
            version: 0,
            peer_time: 0,
            out: Buffer::new(),
        }
    }
}

impl Handshake {
    /// Check if the handshake is complete.
    pub fn is_complete(&self) -> bool {
        self.state == HandshakeState::Done
    }

    /// Release the internal output buffer.
    pub fn cleanup(&mut self) {
        self.out.reset();
    }
}

/* ── SplitMix64 PRNG ── */

/// SplitMix64: a small, fast PRNG used to fill the handshake's random payload.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Fill `buf` with pseudo-random bytes using a seeded PRNG.
fn fill_random(buf: &mut [u8]) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut state = now ^ (buf.as_ptr() as u64);

    let mut i = 0;
    while i < buf.len() {
        let r = splitmix64(&mut state);
        for b in 0..8 {
            if i >= buf.len() {
                break;
            }
            buf[i] = (r >> (b * 8)) as u8;
            i += 1;
        }
    }
}

/// Get the current time as a u32.
fn get_time() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32
}

/* ── Server-side handshake ── */

/// Initialize a server-side handshake.
pub fn server_init(hs: &mut Handshake) {
    hs.state = HandshakeState::ServerWaitC0;
    hs.version = 0;
    hs.peer_time = 0;
}

/// Read C0 (version byte) from the client.
pub fn server_read_c0(hs: &mut Handshake, buf: &mut Buffer) -> Result<()> {
    let mut ver = [0u8; 1];
    buf.read(&mut ver).map_err(|_| ErrorCode::Io)?;

    if ver[0] != RTMP_VERSION {
        return Err(ErrorCode::Handshake);
    }

    hs.version = ver[0];
    hs.state = HandshakeState::ServerWaitC1;
    Ok(())
}

/// Read C1 (1536-byte handshake) from the client and queue S1+S2.
pub fn server_read_c1(hs: &mut Handshake, buf: &mut Buffer) -> Result<()> {
    if buf.available() < HANDSHAKE_SIZE {
        return Err(ErrorCode::Io);
    }

    let mut c1 = vec![0u8; HANDSHAKE_SIZE];
    buf.read(&mut c1).map_err(|_| ErrorCode::Io)?;

    hs.peer_time = ntoh24(&c1[..4]);

    // Build S1
    let mut s1 = vec![0u8; HANDSHAKE_SIZE];
    let server_time = get_time();
    let net_time = hton32(server_time);
    s1[..4].copy_from_slice(&net_time.to_be_bytes());
    // bytes 4-7 = 0
    fill_random(&mut s1[8..]);

    // S2 echoes C1 with time2 replaced
    let mut s2 = c1.clone();
    s2[..4].copy_from_slice(&net_time.to_be_bytes());

    hs.out.reset();
    hs.out.write(&s1).map_err(|_| ErrorCode::Internal)?;
    hs.out.write(&s2).map_err(|_| ErrorCode::Internal)?;

    hs.state = HandshakeState::ServerWaitC2;
    Ok(())
}

/// Read C2 from the client, completing the handshake.
pub fn server_read_c2(hs: &mut Handshake, buf: &mut Buffer) -> Result<()> {
    if buf.available() < HANDSHAKE_SIZE {
        return Err(ErrorCode::Io);
    }

    let mut c2 = vec![0u8; HANDSHAKE_SIZE];
    buf.read(&mut c2).map_err(|_| ErrorCode::Io)?;

    hs.state = HandshakeState::Done;
    Ok(())
}

/* ── Client-side handshake ── */

/// Initialize a client-side handshake.
pub fn client_init(hs: &mut Handshake) {
    hs.state = HandshakeState::ClientWaitS0;
    hs.version = 0;
    hs.peer_time = 0;
}

/// Generate C0+C1 to send to the server.
pub fn client_generate_c0c1(hs: &mut Handshake) -> Result<()> {
    let client_time = get_time();
    hs.peer_time = client_time;
    let net_time = hton32(client_time);

    let mut c0c1 = vec![0u8; 1 + HANDSHAKE_SIZE];
    c0c1[0] = RTMP_VERSION;
    c0c1[1..5].copy_from_slice(&net_time.to_be_bytes());
    // bytes 5-8 = 0
    fill_random(&mut c0c1[9..]);

    hs.out.reset();
    hs.out.write(&c0c1).map_err(|_| ErrorCode::Internal)?;
    hs.state = HandshakeState::ClientWaitS1;
    Ok(())
}

/// Read S0 (version byte) from the server.
pub fn client_read_s0(hs: &mut Handshake, buf: &mut Buffer) -> Result<()> {
    let mut ver = [0u8; 1];
    buf.read(&mut ver).map_err(|_| ErrorCode::Io)?;

    if ver[0] != RTMP_VERSION {
        return Err(ErrorCode::Handshake);
    }

    hs.version = ver[0];
    hs.state = HandshakeState::ClientWaitS1;
    Ok(())
}

/// Read S1 from the server and queue C2.
pub fn client_read_s1(hs: &mut Handshake, buf: &mut Buffer) -> Result<()> {
    if buf.available() < HANDSHAKE_SIZE {
        return Err(ErrorCode::Io);
    }

    let mut s1 = vec![0u8; HANDSHAKE_SIZE];
    buf.read(&mut s1).map_err(|_| ErrorCode::Io)?;

    hs.peer_time = ntoh32(&s1[..4]);

    // C2 echoes S1 with time2 replaced
    let mut c2 = s1.clone();
    let c2_time2 = hton32(get_time());
    c2[..4].copy_from_slice(&c2_time2.to_be_bytes());

    hs.out.reset();
    hs.out.write(&c2).map_err(|_| ErrorCode::Internal)?;
    hs.state = HandshakeState::ClientWaitS2;
    Ok(())
}

/// Read S2 from the server, completing the handshake.
pub fn client_read_s2(hs: &mut Handshake, buf: &mut Buffer) -> Result<()> {
    if buf.available() < HANDSHAKE_SIZE {
        return Err(ErrorCode::Io);
    }

    let mut s2 = vec![0u8; HANDSHAKE_SIZE];
    buf.read(&mut s2).map_err(|_| ErrorCode::Io)?;

    hs.state = HandshakeState::Done;
    Ok(())
}

// Fix the s1 building in server_read_c1
fn ntoh24(buf: &[u8]) -> u32 {
    ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32)
}
