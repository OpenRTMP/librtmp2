//! Legacy RTMP handshake (C0/C1/C2 ↔ S0/S1/S2)
//!
//! Mirrors `src/handshake/handshake.h` and `src/handshake/handshake.c`.

use crate::buffer::Buffer;
use crate::bytes::ntoh32;
use crate::types::ErrorCode;
use crate::types::Result;

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

/// Counter mixed into the PRNG seed so calls made at the same timestamp still differ.
static RANDOM_SEED_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Fill `buf` with pseudo-random bytes using a seeded PRNG.
fn fill_random(buf: &mut [u8]) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let counter = RANDOM_SEED_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut state = now.as_secs()
        ^ (u64::from(now.subsec_nanos()) << 32)
        ^ counter.wrapping_mul(0xD1B5_4A32_D192_ED03)
        ^ (buf.len() as u64);

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

/* ── Adobe digest ("complex") handshake ── */

/// Size of the HMAC-SHA256 digest embedded in C1/S1/S2.
const DIGEST_LEN: usize = 32;

/// Shared 32-byte tail of the Flash Player / FMS handshake keys.
const HANDSHAKE_KEY_TAIL: [u8; 32] = [
    0xF0, 0xEE, 0xC2, 0x4A, 0x80, 0x68, 0xBE, 0xE8, 0x2E, 0x00, 0xD0, 0xD1, 0x02, 0x9E, 0x7E, 0x57,
    0x6E, 0xEC, 0x5D, 0x2D, 0x29, 0x80, 0x6F, 0xAB, 0x93, 0xB8, 0xE6, 0x36, 0xCF, 0xEB, 0x31, 0xAE,
];
/// Text prefix of the client (Flash Player) key; C1's digest is keyed with it alone.
const FP_KEY_TEXT: &[u8] = b"Genuine Adobe Flash Player 001";
/// Text prefix of the server (FMS) key; S1's digest is keyed with it alone.
const FMS_KEY_TEXT: &[u8] = b"Genuine Adobe Flash Media Server 001";

/// Version field the server advertises in a digest S1 (any non-zero value
/// tells the client the server speaks the digest handshake).
const SERVER_DIGEST_VERSION: [u8; 4] = [0x04, 0x05, 0x00, 0x01];

/// The digest block offsets of the two C1/S1 layouts: schema 0 puts the key
/// block first (digest block at 772), schema 1 the digest block first (at 8).
const DIGEST_BLOCK_BASES: [usize; 2] = [772, 8];

/// How the server answered a client's C1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeMode {
    /// Plain RTMP handshake: S1 is random, S2 echoes C1.
    Simple,
    /// C1 advertised a Flash version but carried no valid digest; answered
    /// with the plain handshake.
    SimpleFallback,
    /// Adobe digest handshake; the value is the digest block base (772 or 8).
    Digest(usize),
}

/// True when C1 advertises the Adobe digest/complex handshake schema.
fn c1_requests_complex_handshake(c1: &[u8]) -> bool {
    c1.len() >= HANDSHAKE_SIZE && (c1[4] != 0 || c1[5] != 0 || c1[6] != 0 || c1[7] != 0)
}

/// Offset of the 32-byte digest inside a 1536-byte C1/S1 whose digest block
/// starts at `base`: the first four bytes of the block pick the position.
fn digest_offset(buf: &[u8], base: usize) -> usize {
    let sum: usize = buf[base..base + 4].iter().map(|&b| b as usize).sum();
    base + 4 + sum % 728
}

/// HMAC-SHA256 over `buf` with the 32 bytes at `offset` left out.
fn digest_without(buf: &[u8], offset: usize, key: &[u8]) -> [u8; DIGEST_LEN] {
    crate::sha256::hmac_sha256(key, &[&buf[..offset], &buf[offset + DIGEST_LEN..]])
}

/// Find a valid client digest in C1. Returns the digest block base and the
/// digest itself.
fn find_c1_digest(c1: &[u8]) -> Option<(usize, [u8; DIGEST_LEN])> {
    DIGEST_BLOCK_BASES.into_iter().find_map(|base| {
        let off = digest_offset(c1, base);
        let expected = digest_without(c1, off, FP_KEY_TEXT);
        let mut found = [0u8; DIGEST_LEN];
        found.copy_from_slice(&c1[off..off + DIGEST_LEN]);
        (expected == found).then_some((base, found))
    })
}

/// Full FMS key (text + shared tail), used to derive the S2 signing key.
fn fms_full_key() -> [u8; 68] {
    let mut key = [0u8; 68];
    key[..FMS_KEY_TEXT.len()].copy_from_slice(FMS_KEY_TEXT);
    key[FMS_KEY_TEXT.len()..].copy_from_slice(&HANDSHAKE_KEY_TAIL);
    key
}

/// Build the digest S1 and S2 answering a C1 whose digest block sits at
/// `base` and whose digest is `c1_digest`.
fn build_digest_s1s2(
    base: usize,
    c1_digest: &[u8; DIGEST_LEN],
    server_time: u32,
) -> (Vec<u8>, Vec<u8>) {
    let mut s1 = vec![0u8; HANDSHAKE_SIZE];
    fill_random(&mut s1[8..]);
    s1[..4].copy_from_slice(&server_time.to_be_bytes());
    s1[4..8].copy_from_slice(&SERVER_DIGEST_VERSION);
    let off = digest_offset(&s1, base);
    let digest = digest_without(&s1, off, FMS_KEY_TEXT);
    s1[off..off + DIGEST_LEN].copy_from_slice(&digest);

    let mut s2 = vec![0u8; HANDSHAKE_SIZE];
    fill_random(&mut s2);
    let sign_key = crate::sha256::hmac_sha256(&fms_full_key(), &[c1_digest]);
    let sig_at = HANDSHAKE_SIZE - DIGEST_LEN;
    let signature = crate::sha256::hmac_sha256(&sign_key, &[&s2[..sig_at]]);
    s2[sig_at..].copy_from_slice(&signature);
    (s1, s2)
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
    server_read_c1_mode(hs, buf).map(|_| ())
}

/// [`server_read_c1`], also reporting which handshake flavour was answered.
///
/// A C1 that carries a valid Adobe digest gets a digest S1/S2 (some hardware
/// encoders verify the server's digest and hang up on a plain S1); any other
/// C1 gets the plain handshake.
pub fn server_read_c1_mode(hs: &mut Handshake, buf: &mut Buffer) -> Result<HandshakeMode> {
    if buf.available() < HANDSHAKE_SIZE {
        return Err(ErrorCode::Io);
    }

    let mut c1 = vec![0u8; HANDSHAKE_SIZE];
    buf.read(&mut c1).map_err(|_| ErrorCode::Io)?;

    hs.peer_time = ntoh32(&c1[..4]);
    let server_time = get_time();

    let complex = c1_requests_complex_handshake(&c1);
    let (s1, s2, mode) = match complex.then(|| find_c1_digest(&c1)).flatten() {
        Some((base, c1_digest)) => {
            let (s1, s2) = build_digest_s1s2(base, &c1_digest, server_time);
            (s1, s2, HandshakeMode::Digest(base))
        }
        None => {
            let mut s1 = vec![0u8; HANDSHAKE_SIZE];
            s1[..4].copy_from_slice(&server_time.to_be_bytes());
            // bytes 4-7 = 0
            fill_random(&mut s1[8..]);
            // S2 echoes C1's time1 (bytes 0..4) and random; time2 (bytes 4..8)
            // is the server's own time, per RTMP spec 5.2.4.
            let mut s2 = c1;
            s2[4..8].copy_from_slice(&server_time.to_be_bytes());
            let mode = if complex {
                HandshakeMode::SimpleFallback
            } else {
                HandshakeMode::Simple
            };
            (s1, s2, mode)
        }
    };

    hs.out.reset();
    hs.out.write(&s1).map_err(|_| ErrorCode::Internal)?;
    hs.out.write(&s2).map_err(|_| ErrorCode::Internal)?;

    hs.state = HandshakeState::ServerWaitC2;
    Ok(mode)
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

    let mut c0c1 = vec![0u8; 1 + HANDSHAKE_SIZE];
    c0c1[0] = RTMP_VERSION;
    c0c1[1..5].copy_from_slice(&client_time.to_be_bytes());
    // bytes 5-8 = 0
    fill_random(&mut c0c1[9..]);

    hs.out.reset();
    hs.out.write(&c0c1).map_err(|_| ErrorCode::Internal)?;
    // The client has only *sent* C0+C1: the server's S0 has not been read
    // yet, so the state stays ClientWaitS0 until `client_read_s0` consumes
    // the version byte. Advancing it here would make a state-driven caller
    // skip S0 and desynchronize the handshake by one byte.
    hs.state = HandshakeState::ClientWaitS0;
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

    // C2 echoes S1's time1 (bytes 0..4) and random; time2 (bytes 4..8) is the
    // client's own time, per RTMP spec 5.2.5.
    let mut c2 = s1.clone();
    c2[4..8].copy_from_slice(&get_time().to_be_bytes());

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_client_server_handshake_completes() {
        let mut client = Handshake::default();
        let mut server = Handshake::default();
        client_init(&mut client);
        server_init(&mut server);

        // Client sends C0+C1
        client_generate_c0c1(&mut client).unwrap();
        let c0c1 = client.out.peek().to_vec();
        client.out.reset();

        // Server reads C0+C1, generates S0+S1+S2
        let mut server_in = Buffer::new();
        server_in.write(&c0c1).unwrap();
        server_read_c0(&mut server, &mut server_in).unwrap();
        server_read_c1(&mut server, &mut server_in).unwrap();
        let s1s2 = server.out.peek().to_vec();

        assert_eq!(server.state, HandshakeState::ServerWaitC2);
        assert_eq!(s1s2.len(), HANDSHAKE_SIZE * 2);

        // Client reads S0 (we only sent S1+S2 above; prepend version byte)
        let mut s0s1s2 = vec![RTMP_VERSION];
        s0s1s2.extend_from_slice(&s1s2);
        let mut client_in = Buffer::new();
        client_in.write(&s0s1s2).unwrap();
        client_read_s0(&mut client, &mut client_in).unwrap();
        client_read_s1(&mut client, &mut client_in).unwrap();
        let c2 = client.out.peek().to_vec();
        assert_eq!(client.state, HandshakeState::ClientWaitS2);

        // Server reads C2
        let mut server_in2 = Buffer::new();
        server_in2.write(&c2).unwrap();
        server_read_c2(&mut server, &mut server_in2).unwrap();
        assert!(server.is_complete());

        // Client reads S2 (echo of C1, any 1536 bytes)
        let mut client_in2 = Buffer::new();
        client_in2.write(&vec![0u8; HANDSHAKE_SIZE]).unwrap();
        client_read_s2(&mut client, &mut client_in2).unwrap();
        assert!(client.is_complete());
    }

    #[test]
    fn server_read_c0_rejects_wrong_version() {
        let mut hs = Handshake::default();
        let mut buf = Buffer::new();
        buf.write(&[0x99]).unwrap();
        assert!(server_read_c0(&mut hs, &mut buf).is_err());
    }

    #[test]
    fn s1_time_field_matches_wall_clock_in_big_endian() {
        let mut hs = Handshake::default();
        server_init(&mut hs);
        let c1 = vec![0u8; HANDSHAKE_SIZE];
        let mut buf = Buffer::new();
        buf.write(&c1).unwrap();
        let before = get_time();
        server_read_c1(&mut hs, &mut buf).unwrap();
        let after = get_time();

        let s1 = &hs.out.peek()[..HANDSHAKE_SIZE];
        let time_from_s1 = u32::from_be_bytes([s1[0], s1[1], s1[2], s1[3]]);
        assert!(time_from_s1 >= before && time_from_s1 <= after);
    }

    #[test]
    fn s2_echoes_c1_time1_and_carries_server_time_in_time2() {
        let mut hs = Handshake::default();
        server_init(&mut hs);
        let mut c1 = vec![0u8; HANDSHAKE_SIZE];
        c1[..4].copy_from_slice(&0x1234_5678u32.to_be_bytes());
        c1[8..].copy_from_slice(&[0xAB; HANDSHAKE_SIZE - 8]);
        let mut buf = Buffer::new();
        buf.write(&c1).unwrap();

        let before = get_time();
        server_read_c1(&mut hs, &mut buf).unwrap();
        let after = get_time();

        let out = hs.out.peek();
        let s2 = &out[HANDSHAKE_SIZE..HANDSHAKE_SIZE * 2];
        let time1 = u32::from_be_bytes([s2[0], s2[1], s2[2], s2[3]]);
        let time2 = u32::from_be_bytes([s2[4], s2[5], s2[6], s2[7]]);
        assert_eq!(time1, 0x1234_5678, "S2 time1 must echo C1's time1");
        assert!(
            time2 >= before && time2 <= after,
            "S2 time2 must be the server's own time"
        );
        assert_eq!(
            &s2[8..],
            &c1[8..],
            "S2 random bytes must echo C1's random bytes"
        );
    }

    #[test]
    fn complex_c1_without_valid_digest_falls_back_to_simple() {
        let mut hs = Handshake::default();
        server_init(&mut hs);
        let mut c1 = vec![0u8; HANDSHAKE_SIZE];
        c1[..4].copy_from_slice(&0x1234_5678u32.to_be_bytes());
        c1[4..8].copy_from_slice(&1u32.to_be_bytes());
        let mut buf = Buffer::new();
        buf.write(&c1).unwrap();
        let mode = server_read_c1_mode(&mut hs, &mut buf).unwrap();
        assert_eq!(mode, HandshakeMode::SimpleFallback);
        assert_eq!(hs.state, HandshakeState::ServerWaitC2);
        let out = hs.out.peek();
        assert_eq!(out.len(), HANDSHAKE_SIZE * 2);
        assert_eq!(&out[HANDSHAKE_SIZE + 8..], &c1[8..], "S2 echoes C1");
    }

    /// Build a C1 the way a Flash-style client does: version set, digest
    /// keyed with the Flash Player key at the schema's offset.
    fn digest_c1(base: usize) -> Vec<u8> {
        let mut c1 = vec![0u8; HANDSHAKE_SIZE];
        fill_random(&mut c1);
        c1[..4].copy_from_slice(&0u32.to_be_bytes());
        c1[4..8].copy_from_slice(&[0x80, 0x00, 0x07, 0x02]);
        let off = digest_offset(&c1, base);
        let d = digest_without(&c1, off, FP_KEY_TEXT);
        c1[off..off + DIGEST_LEN].copy_from_slice(&d);
        c1
    }

    #[test]
    fn digest_c1_gets_verifiable_digest_s1_and_signed_s2() {
        for base in DIGEST_BLOCK_BASES {
            let c1 = digest_c1(base);
            let (_, c1_digest) = find_c1_digest(&c1).expect("client digest found");
            let mut hs = Handshake::default();
            server_init(&mut hs);
            let mut buf = Buffer::new();
            buf.write(&c1).unwrap();
            let mode = server_read_c1_mode(&mut hs, &mut buf).unwrap();
            assert_eq!(mode, HandshakeMode::Digest(base));
            let out = hs.out.peek().to_vec();
            let (s1, s2) = out.split_at(HANDSHAKE_SIZE);

            // Client-side S1 check: digest keyed with the FMS key text.
            assert_ne!(&s1[4..8], &[0, 0, 0, 0]);
            let off = digest_offset(s1, base);
            assert_eq!(
                &s1[off..off + DIGEST_LEN],
                &digest_without(s1, off, FMS_KEY_TEXT)
            );

            // Client-side S2 check: last 32 bytes signed with a key derived
            // from the client's own C1 digest.
            let key = crate::sha256::hmac_sha256(&fms_full_key(), &[&c1_digest]);
            let sig_at = HANDSHAKE_SIZE - DIGEST_LEN;
            assert_eq!(
                &s2[sig_at..],
                &crate::sha256::hmac_sha256(&key, &[&s2[..sig_at]])
            );
        }
    }

    #[test]
    fn digest_handshake_is_not_used_for_zero_version_c1() {
        let mut c1 = digest_c1(8);
        c1[4..8].copy_from_slice(&[0, 0, 0, 0]);
        let mut hs = Handshake::default();
        server_init(&mut hs);
        let mut buf = Buffer::new();
        buf.write(&c1).unwrap();
        assert_eq!(
            server_read_c1_mode(&mut hs, &mut buf).unwrap(),
            HandshakeMode::Simple
        );
    }

    #[test]
    fn c2_echoes_s1_time1_and_carries_client_time_in_time2() {
        let mut client = Handshake::default();
        client_init(&mut client);
        let mut s1 = vec![0u8; HANDSHAKE_SIZE];
        s1[..4].copy_from_slice(&0x0BAD_F00Du32.to_be_bytes());
        s1[8..].copy_from_slice(&[0xCD; HANDSHAKE_SIZE - 8]);
        let mut buf = Buffer::new();
        buf.write(&s1).unwrap();

        let before = get_time();
        client_read_s1(&mut client, &mut buf).unwrap();
        let after = get_time();

        let c2 = client.out.peek();
        let time1 = u32::from_be_bytes([c2[0], c2[1], c2[2], c2[3]]);
        let time2 = u32::from_be_bytes([c2[4], c2[5], c2[6], c2[7]]);
        assert_eq!(time1, 0x0BAD_F00D, "C2 time1 must echo S1's time1");
        assert!(
            time2 >= before && time2 <= after,
            "C2 time2 must be the client's own time"
        );
        assert_eq!(
            &c2[8..],
            &s1[8..],
            "C2 random bytes must echo S1's random bytes"
        );
    }
}
