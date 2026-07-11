//! Outbound RTMP client
//!
//! Mirrors `src/client/client.h` and `src/client/client.c`.

use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::io::IntoRawFd;
use std::time::{Duration, Instant};

use crate::buffer::Buffer;
use crate::chunk::reader::{chunk_read, ChunkMessage};
use crate::chunk::state::{ChunkRegistry, DEFAULT_MAX_MSG_LENGTH};
use crate::chunk::writer::chunk_write;
use crate::handshake::{self, Handshake};
use crate::message::command;
use crate::message::control;
use crate::message::message as msg_dispatch;
use crate::net;
use crate::transport::Transport;
use crate::types::*;

/// Handshake payload size (mirrors `handshake::HANDSHAKE_SIZE`, which is private).
const HANDSHAKE_SIZE: usize = 1536;

/// Max time to wait for the peer to send more data before giving up.
const RECV_POLL_TIMEOUT_MS: i32 = 10_000;
/// Maximum frame payload accepted from FFI callers.
pub const MAX_CLIENT_FRAME_BYTES: usize = DEFAULT_MAX_MSG_LENGTH as usize;
/// Cap complete messages handled per `poll` recv pass.
const MAX_MESSAGES_PER_POLL: usize = 256;
/// Maximum inbound bytes drained from the socket per `poll` call. Without
/// this cap a malicious server can monopolize the embedder's event-loop thread
/// by keeping the kernel recv queue full across many `recv` syscalls in one
/// `poll()` invocation (mirrors the server-side fairness cap).
const MAX_RECV_BYTES_PER_POLL: usize = 256 * 1024;
/// Total inbound bytes allowed while waiting for AMF `_result` / `onStatus`
/// during `connect` / `publish` / `play`. Prevents a malicious server from
/// forcing the client through dozens of max-size junk commands before the
/// expected response.
const MAX_RECV_BYTES_PER_COMMAND_WAIT: usize = 256 * 1024;
/// Maximum time to wait for the initial TCP connect before failing.
const TCP_CONNECT_TIMEOUT_SECS: u64 = 10;

/// Client connection states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum ClientState {
    Disconnected = 0,
    Handshaking,
    Connected,
    AppConnected,
    StreamCreated,
    Publishing,
    Playing,
}

/// RTMP client object.
pub struct Client {
    pub client_fd: i32,
    pub transport: Option<Transport>,
    pub handshake: Handshake,
    pub state: ClientState,
    pub send_buffer: Buffer,
    pub recv_buffer: Buffer,
    pub chunk_reg: ChunkRegistry,
    pub stream_id: u32,
    pub app: String,
    pub stream_key: String,
    pub on_frame_cb: Option<fn(&Frame)>,
    /// Retains the last frame payload delivered through `on_frame_cb`.
    frame_cb_scratch: Vec<u8>,
    /// PEM CA bundle used to verify `rtmps://` servers, in addition to the
    /// system trust store. `None` uses the system trust store only.
    tls_ca_file: Option<String>,
    /// Skip TLS certificate verification for `rtmps://` connections.
    /// Only for testing against self-signed deployments.
    tls_insecure: bool,
}

impl Client {
    /// Create a new client.
    pub fn new() -> Self {
        Self {
            client_fd: -1,
            transport: None,
            handshake: Handshake::default(),
            state: ClientState::Disconnected,
            send_buffer: Buffer::new(),
            recv_buffer: Buffer::new(),
            chunk_reg: ChunkRegistry::new(),
            stream_id: 0,
            app: String::new(),
            stream_key: String::new(),
            on_frame_cb: None,
            frame_cb_scratch: Vec::new(),
            tls_ca_file: None,
            tls_insecure: false,
        }
    }

    /// Configure `rtmps://` verification for subsequent `connect()` calls.
    pub fn set_tls_client_config(&mut self, ca_file: Option<String>, insecure: bool) {
        self.tls_ca_file = ca_file;
        self.tls_insecure = insecure;
    }

    /// Connect to an RTMP(S) server at `rtmp://host[:port]/app/streamKey` or
    /// `rtmps://host[:port]/app/streamKey`.
    ///
    /// Performs the real TCP connect (wrapped in a TLS client handshake for
    /// `rtmps://`, verified against the system trust store), the legacy
    /// C0/C1/C2 handshake, then the `connect` + `createStream` AMF0 command
    /// exchange.
    pub fn connect(&mut self, url: &str) -> Result<()> {
        let (use_tls, host, port, app, stream_key) = parse_rtmp_url(url)?;
        if use_tls && !crate::transport::tls_available() {
            return Err(ErrorCode::Unsupported);
        }
        self.reset_session_state();

        let addrs = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|_| ErrorCode::Io)?;
        let deadline = Instant::now() + Duration::from_secs(TCP_CONNECT_TIMEOUT_SECS);
        let mut last_err_was_timeout = false;
        let mut stream = None;
        for addr in addrs {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                last_err_was_timeout = true;
                break;
            }
            match TcpStream::connect_timeout(&addr, remaining) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => last_err_was_timeout = e.kind() == std::io::ErrorKind::TimedOut,
            }
        }
        let stream = stream.ok_or(if last_err_was_timeout {
            ErrorCode::Timeout
        } else {
            ErrorCode::Io
        })?;
        let mut transport = if use_tls {
            Transport::connect_tls(
                stream,
                &host,
                self.tls_ca_file.as_deref(),
                self.tls_insecure,
            )?
        } else {
            Transport::new_plain(stream.into_raw_fd())
        };

        self.state = ClientState::Handshaking;
        if let Err(e) = self.do_handshake(&mut transport) {
            // transport drops here, closing the fd via Transport::drop
            return Err(e);
        }

        self.client_fd = transport.fd();
        self.transport = Some(transport);
        self.app = app.clone();
        self.stream_key = stream_key;
        self.state = ClientState::Connected;

        if let Err(e) = self.do_amf_connect(&app, &host, port, use_tls) {
            self.reset_session_state();
            return Err(e);
        }
        Ok(())
    }

    /// Begin publishing.
    pub fn publish(&mut self) -> Result<()> {
        if self.state != ClientState::AppConnected {
            return Err(ErrorCode::Protocol);
        }
        let mut amf = Buffer::with_capacity(256);
        command::build_publish(&mut amf, &self.stream_key, "live")?;
        self.send_command_msg(self.stream_id, amf.as_slice())?;
        let mut status = self.wait_for_command("onStatus")?;
        command::read_onstatus(&mut status)?;
        self.state = ClientState::Publishing;
        Ok(())
    }

    /// Run the AMF connect + createStream exchange. Separated from `connect()`
    /// so the transport is already stored before we enter, letting the caller
    /// call `reset_session_state()` (which drops the transport) on any error.
    fn do_amf_connect(&mut self, app: &str, host: &str, port: u16, use_tls: bool) -> Result<()> {
        let scheme = if use_tls { "rtmps" } else { "rtmp" };
        let tc_url = format!("{scheme}://{host}:{port}/{app}");
        let mut connect_amf = Buffer::with_capacity(512);
        command::build_connect(&mut connect_amf, app, &tc_url, "", "", "FMLE/3.0", 0, 0)?;
        self.send_command_msg(0, connect_amf.as_slice())?;
        let mut result = self.wait_for_command("_result")?;
        command::read_connect_result(&mut result)?;

        let mut create_stream_amf = Buffer::with_capacity(64);
        command::build_create_stream(&mut create_stream_amf, 2.0)?;
        self.send_command_msg(0, create_stream_amf.as_slice())?;
        let mut create_result = self.wait_for_command("_result")?;
        let (_txn, stream_id) = command::read_create_stream_result(&mut create_result)?;
        self.stream_id = stream_id as u32;

        self.state = ClientState::AppConnected;
        Ok(())
    }

    /// Begin playing.
    pub fn play(&mut self) -> Result<()> {
        if self.state != ClientState::AppConnected {
            return Err(ErrorCode::Protocol);
        }
        let mut amf = Buffer::with_capacity(256);
        command::build_play(&mut amf, &self.stream_key)?;
        self.send_command_msg(self.stream_id, amf.as_slice())?;
        let mut status = self.wait_for_command("onStatus")?;
        command::read_onstatus(&mut status)?;
        self.state = ClientState::Playing;
        Ok(())
    }

    /// Send a frame while publishing.
    pub fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        if self.state != ClientState::Publishing {
            return Err(ErrorCode::Protocol);
        }
        let payload = self.frame_payload_slice(frame)?;
        self.send_frame_payload(frame.frame_type, frame.timestamp, payload)
    }

    /// Send a frame from an owned payload slice.
    pub fn send_frame_payload(
        &mut self,
        frame_type: FrameType,
        timestamp: u32,
        payload: &[u8],
    ) -> Result<()> {
        if self.state != ClientState::Publishing {
            return Err(ErrorCode::Protocol);
        }
        if payload.len() > MAX_CLIENT_FRAME_BYTES {
            return Err(ErrorCode::Protocol);
        }

        let mut cmsg = ChunkMessage::default();
        cmsg.timestamp = timestamp;
        cmsg.msg_length = payload.len() as u32;
        cmsg.msg_stream_id = self.stream_id;

        if frame_type == FrameType::Audio {
            cmsg.csid = 4;
            cmsg.msg_type_id = 0x08; // AUDIO
        } else {
            cmsg.csid = 6;
            cmsg.msg_type_id = 0x09; // VIDEO
        }
        cmsg.fmt = 0;

        chunk_write(
            &mut self.send_buffer,
            &cmsg,
            payload,
            payload.len(),
            128,
        )?;

        // Flush
        let data = self.send_buffer.peek().to_vec();
        if let Some(ref mut transport) = self.transport {
            transport.send(&data)?;
        }
        self.send_buffer.reset();

        Ok(())
    }

    /// Poll for incoming data while playing.
    pub fn poll(&mut self, timeout_ms: i32) -> Result<()> {
        if self.state != ClientState::Playing {
            return Err(ErrorCode::Protocol);
        }

        // Scope the mutable transport borrow to the recv phase only.
        let (poll_fd, has_buffered_tls_data) = {
            let Some(t) = self.transport.as_ref() else {
                return Err(ErrorCode::Internal);
            };
            (t.fd(), t.pending() > 0)
        };
        // A prior poll() call may have stopped draining at
        // MAX_RECV_BYTES_PER_POLL while OpenSSL still held decrypted
        // plaintext internally. The kernel socket can then have nothing left
        // to report ready, so blocking in poll(2) here would wait out the
        // full timeout even though data is already available via recv().
        if !has_buffered_tls_data {
            let mut pfd = libc::pollfd {
                fd: poll_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // poll(2) already treats a negative timeout as "block indefinitely"
            // (the POSIX idiom), so pass it through as-is instead of clamping to 0.
            unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        }

        let mut buf = [0u8; 65536];
        let mut bytes_drained = 0usize;
        loop {
            if bytes_drained >= MAX_RECV_BYTES_PER_POLL {
                break;
            }
            let (n, again) = {
                let Some(t) = self.transport.as_mut() else {
                    return Err(ErrorCode::Internal);
                };
                let mut again = 0i32;
                let n = t.recv(&mut buf, &mut again);
                (n, again)
            };
            if n > 0 {
                let chunk_len = n as usize;
                self.recv_buffer
                    .write(&buf[..chunk_len])
                    .map_err(|_| ErrorCode::Internal)?;
                bytes_drained += chunk_len;
            } else if n == 0 {
                return Err(ErrorCode::Io);
            } else if again == 2 {
                // TLS renegotiation can need write-readiness during a read;
                // the POLLIN wait above cannot detect that on its own. Wait
                // for POLLOUT once, bounded by the same timeout, then retry
                // the read instead of giving up on a writable socket.
                let mut wpfd = libc::pollfd {
                    fd: poll_fd,
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let rc = unsafe { libc::poll(&mut wpfd, 1, timeout_ms) };
                if rc <= 0 {
                    break;
                }
            } else {
                break;
            }
        }

        let mut messages_processed = 0usize;
        loop {
            if messages_processed >= MAX_MESSAGES_PER_POLL {
                break;
            }

            let mut msg = ChunkMessage::default();
            let mut payload_ptr: *const u8 = std::ptr::null();
            let mut payload_len = 0;
            match chunk_read(
                &mut self.recv_buffer,
                &mut self.chunk_reg,
                None,
                &mut msg,
                &mut payload_ptr,
                &mut payload_len,
            ) {
                Ok(1) if msg.is_complete => {
                    messages_processed += 1;
                    if msg.msg_type_id == msg_dispatch::RTMP_MSG_SET_CHUNK_SIZE {
                        let payload = if payload_ptr.is_null() || payload_len == 0 {
                            &[][..]
                        } else {
                            unsafe { std::slice::from_raw_parts(payload_ptr, payload_len) }
                        };
                        if let Ok(cs) = control::read_set_chunk_size(payload) {
                            self.chunk_reg.set_all_chunk_size(cs);
                        }
                    } else if msg.msg_type_id == msg_dispatch::RTMP_MSG_AUDIO
                        || msg.msg_type_id == msg_dispatch::RTMP_MSG_VIDEO
                    {
                        if let Some(ref cb) = self.on_frame_cb {
                            self.frame_cb_scratch.clear();
                            if !payload_ptr.is_null() && payload_len > 0 {
                                self.frame_cb_scratch.extend_from_slice(unsafe {
                                    std::slice::from_raw_parts(payload_ptr, payload_len)
                                });
                            }
                            let frame = Frame {
                                frame_type: if msg.msg_type_id == msg_dispatch::RTMP_MSG_AUDIO {
                                    FrameType::Audio
                                } else {
                                    FrameType::Video
                                },
                                timestamp: msg.timestamp,
                                size: self.frame_cb_scratch.len() as u32,
                                data: self.frame_cb_scratch.as_ptr(),
                                ..Default::default()
                            };
                            cb(&frame);
                        }
                    }
                }
                Ok(_) => break,
                Err(_) => return Err(ErrorCode::Chunk),
            }
        }

        Ok(())
    }

    // ── Internal helpers ──

    fn frame_payload_slice<'a>(&self, frame: &'a Frame) -> Result<&'a [u8]> {
        if frame.size == 0 {
            return Ok(&[]);
        }
        if frame.data.is_null() {
            return Err(ErrorCode::Internal);
        }
        let len = frame.size as usize;
        if len > MAX_CLIENT_FRAME_BYTES {
            return Err(ErrorCode::Protocol);
        }
        Ok(unsafe { std::slice::from_raw_parts(frame.data, len) })
    }

    /// Drop any prior socket and reset all protocol state before a new connect.
    /// Prevents stale recv/send buffers, chunk registry entries, and handshake
    /// state from a previous (failed) session polluting the next attempt.
    fn reset_session_state(&mut self) {
        // Drop transport first: it owns and closes the fd.
        self.transport = None;
        self.client_fd = -1;
        self.recv_buffer.reset();
        self.send_buffer.reset();
        self.chunk_reg.destroy();
        self.chunk_reg.init();
        handshake::client_init(&mut self.handshake);
        self.state = ClientState::Disconnected;
        self.stream_id = 0;
    }

    /// Drive the legacy C0/C1/C2 client handshake to completion over `transport`.
    fn do_handshake(&mut self, transport: &mut Transport) -> Result<()> {
        handshake::client_init(&mut self.handshake);
        handshake::client_generate_c0c1(&mut self.handshake)?;
        let c0c1 = self.handshake.out.peek().to_vec();
        transport.send(&c0c1)?;
        self.handshake.out.reset();

        let s0s1 = read_exact(transport, 1 + HANDSHAKE_SIZE)?;
        let mut buf = Buffer::new();
        buf.write(&s0s1).map_err(|_| ErrorCode::Internal)?;
        handshake::client_read_s0(&mut self.handshake, &mut buf)?;
        handshake::client_read_s1(&mut self.handshake, &mut buf)?;

        let c2 = self.handshake.out.peek().to_vec();
        transport.send(&c2)?;
        self.handshake.out.reset();

        let s2 = read_exact(transport, HANDSHAKE_SIZE)?;
        let mut buf2 = Buffer::new();
        buf2.write(&s2).map_err(|_| ErrorCode::Internal)?;
        handshake::client_read_s2(&mut self.handshake, &mut buf2)?;

        Ok(())
    }

    fn send_command_msg(&mut self, msg_stream_id: u32, amf_data: &[u8]) -> Result<()> {
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 3;
        cmsg.fmt = 0;
        cmsg.msg_length = amf_data.len() as u32;
        cmsg.msg_type_id = 0x14; // AMF0_COMMAND
        cmsg.msg_stream_id = msg_stream_id;
        chunk_write(&mut self.send_buffer, &cmsg, amf_data, amf_data.len(), 128)?;

        let data = self.send_buffer.peek().to_vec();
        if let Some(ref mut transport) = self.transport {
            transport.send(&data)?;
        }
        self.send_buffer.reset();
        Ok(())
    }

    /// Block until an AMF0 command named `want` is received, returning its payload buffer.
    fn wait_for_command(&mut self, want: &str) -> Result<Buffer> {
        let mut recv_budget = MAX_RECV_BYTES_PER_COMMAND_WAIT;
        for _ in 0..64 {
            let (msg, payload) = self.recv_message(&mut recv_budget)?;
            if msg.msg_type_id != msg_dispatch::RTMP_MSG_AMF0_COMMAND {
                continue;
            }
            let mut buf = Buffer::from_slice(&payload);
            let mut name_buf = [0u8; 64];
            if command::peek_name(&mut buf, &mut name_buf).is_err() {
                continue;
            }
            let name = std::str::from_utf8(&name_buf)
                .unwrap_or("")
                .trim_end_matches('\0');
            if name == want {
                return Ok(buf);
            }
        }
        Err(ErrorCode::Timeout)
    }

    /// Block until one fully-reassembled chunk message is available.
    fn recv_message(&mut self, recv_budget: &mut usize) -> Result<(ChunkMessage, Vec<u8>)> {
        loop {
            let mut msg = ChunkMessage::default();
            let mut payload_ptr: *const u8 = std::ptr::null();
            let mut payload_len = 0;
            match chunk_read(
                &mut self.recv_buffer,
                &mut self.chunk_reg,
                None,
                &mut msg,
                &mut payload_ptr,
                &mut payload_len,
            ) {
                Ok(1) if msg.is_complete => {
                    let payload = if payload_ptr.is_null() || payload_len == 0 {
                        Vec::new()
                    } else {
                        unsafe { std::slice::from_raw_parts(payload_ptr, payload_len) }.to_vec()
                    };
                    if msg.msg_type_id == msg_dispatch::RTMP_MSG_SET_CHUNK_SIZE {
                        if let Ok(cs) = control::read_set_chunk_size(&payload) {
                            self.chunk_reg.set_all_chunk_size(cs);
                        }
                        continue;
                    }
                    return Ok((msg, payload));
                }
                Ok(_) => {}
                Err(_) => return Err(ErrorCode::Chunk),
            }

            if *recv_budget == 0 {
                return Err(ErrorCode::Timeout);
            }

            // Scope mutable transport borrow tightly to avoid conflict with
            // other self fields (recv_buffer) used after the borrow ends.
            // Cap the read itself at the remaining budget rather than reading
            // a full 4096-byte chunk and discarding it after the fact -- a
            // discard here would drop bytes the peer already sent (which can
            // include the tail of the very command this call is waiting for)
            // instead of just deferring them to the next wait_for_command
            // call, desynchronizing this connection's view of the stream.
            let mut tmp = [0u8; 4096];
            let read_cap = tmp.len().min(*recv_budget);
            let (n, again, t_fd) = {
                let t = self.transport.as_mut().ok_or(ErrorCode::Internal)?;
                let mut again = 0i32;
                let n = t.recv(&mut tmp[..read_cap], &mut again);
                (n, again, t.fd())
            };
            if n > 0 {
                let chunk_len = n as usize;
                *recv_budget -= chunk_len;
                self.recv_buffer
                    .write(&tmp[..chunk_len])
                    .map_err(|_| ErrorCode::Internal)?;
            } else if n == 0 {
                return Err(ErrorCode::Io);
            } else if again != 0 {
                poll_for_transport_direction(t_fd, again, RECV_POLL_TIMEOUT_MS)?;
            } else {
                return Err(ErrorCode::Io);
            }
        }
    }
}

/// Wait for the readiness direction `Transport::recv`/`send` reported via
/// `again` (1 = readable, 2 = writable — e.g. TLS renegotiation needing a
/// write during a read), bounded by `timeout_ms`.
///
/// A signal delivered during the wait (`EINTR`) is transient, same as
/// `Transport::recv`/`try_send` already treat it — retry rather than
/// surfacing it as a hard I/O error and aborting the caller's read/handshake.
fn poll_for_transport_direction(fd: i32, again: i32, timeout_ms: i32) -> Result<()> {
    let events = if again == 2 {
        libc::POLLOUT
    } else {
        libc::POLLIN
    };
    loop {
        let mut pfd = libc::pollfd { fd, events, revents: 0 };
        let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if rc == 0 {
            return Err(ErrorCode::Timeout);
        }
        if rc < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(ErrorCode::Io);
        }
        return Ok(());
    }
}

/// Block until exactly `n` bytes have been read from `transport`.
fn read_exact(transport: &mut Transport, n: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; n];
    let mut got = 0;
    while got < n {
        let mut again = 0i32;
        let r = transport.recv(&mut out[got..], &mut again);
        if r > 0 {
            got += r as usize;
        } else if r == 0 {
            return Err(ErrorCode::Io);
        } else if again != 0 {
            poll_for_transport_direction(transport.fd(), again, RECV_POLL_TIMEOUT_MS)?;
        } else {
            return Err(ErrorCode::Io);
        }
    }
    Ok(out)
}

/// Parse `rtmp://host[:port]/app/streamKey` or `rtmps://host[:port]/app/streamKey`
/// into (use_tls, host, port, app, stream_key). `rtmps://` defaults to port 443
/// (the conventional RTMPS port) when no port is given; `rtmp://` defaults to 1935.
fn parse_rtmp_url(url: &str) -> Result<(bool, String, u16, String, String)> {
    let (use_tls, rest, default_port) = if let Some(rest) = url.strip_prefix("rtmps://") {
        (true, rest, "443")
    } else if let Some(rest) = url.strip_prefix("rtmp://") {
        (false, rest, "1935")
    } else {
        return Err(ErrorCode::Internal);
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };

    let mut host = String::new();
    let mut port_str = String::new();
    net::split_host_port(authority, &mut host, &mut port_str, default_port)?;
    let port: u16 = port_str.parse().map_err(|_| ErrorCode::Internal)?;

    let mut parts = path.splitn(2, '/');
    let app = parts.next().unwrap_or("").to_string();
    let stream_key = parts.next().unwrap_or("").to_string();

    if app.is_empty() || stream_key.is_empty() {
        return Err(ErrorCode::Internal);
    }

    Ok((use_tls, host, port, app, stream_key))
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // The Transport owns the fd when set; only close directly if there is
        // no transport (e.g. the fd was set but connecting failed before the
        // transport was stored, which cannot currently happen — this guard is
        // here for correctness if the two ever diverge).
        if self.transport.is_none() && self.client_fd >= 0 {
            unsafe {
                libc::close(self.client_fd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recv_budget_is_at_least_one_socket_read() {
        assert!(MAX_RECV_BYTES_PER_POLL >= 65536);
    }

    #[test]
    fn command_wait_recv_budget_bounds_connect_handshake_amplification() {
        // 64 max-size AMF commands would be 256 MiB without a byte cap.
        assert!(MAX_RECV_BYTES_PER_COMMAND_WAIT < 64 * 4 * 1024 * 1024);
        assert!(MAX_RECV_BYTES_PER_COMMAND_WAIT >= 65536);
    }

    #[test]
    fn tcp_connect_timeout_is_bounded() {
        assert!(TCP_CONNECT_TIMEOUT_SECS > 0);
        assert!(TCP_CONNECT_TIMEOUT_SECS <= 30);
    }

    #[test]
    fn tls_client_config_defaults_to_verified() {
        let client = Client::new();
        assert_eq!(client.tls_ca_file, None);
        assert!(!client.tls_insecure);
    }

    #[test]
    fn tls_client_config_is_stored() {
        let mut client = Client::new();
        client.set_tls_client_config(Some("/etc/ca.pem".to_string()), true);
        assert_eq!(client.tls_ca_file.as_deref(), Some("/etc/ca.pem"));
        assert!(client.tls_insecure);
    }

    #[test]
    fn connect_refused_reports_io_not_timeout() {
        // Bind then immediately drop a listener to get a port nobody accepts
        // on, so the OS replies with ECONNREFUSED rather than timing out.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let mut client = Client::new();
        let err = client
            .connect(&format!("rtmp://127.0.0.1:{port}/live/stream"))
            .unwrap_err();
        assert_eq!(err, ErrorCode::Io);
    }

    #[test]
    fn parse_rtmp_url_defaults_to_plaintext_and_port_1935() {
        let (use_tls, host, port, app, stream_key) =
            parse_rtmp_url("rtmp://example.com/live/streamkey").unwrap();
        assert!(!use_tls);
        assert_eq!(host, "example.com");
        assert_eq!(port, 1935);
        assert_eq!(app, "live");
        assert_eq!(stream_key, "streamkey");
    }

    #[test]
    fn parse_rtmp_url_rtmps_defaults_to_tls_and_port_443() {
        let (use_tls, host, port, app, stream_key) =
            parse_rtmp_url("rtmps://example.com/live/streamkey").unwrap();
        assert!(use_tls);
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
        assert_eq!(app, "live");
        assert_eq!(stream_key, "streamkey");
    }

    #[test]
    fn parse_rtmp_url_rtmps_respects_explicit_port() {
        let (use_tls, host, port, _app, _stream_key) =
            parse_rtmp_url("rtmps://example.com:1935/live/streamkey").unwrap();
        assert!(use_tls);
        assert_eq!(host, "example.com");
        assert_eq!(port, 1935);
    }

    #[test]
    fn parse_rtmp_url_rejects_unknown_scheme() {
        assert_eq!(
            parse_rtmp_url("http://example.com/live/streamkey"),
            Err(ErrorCode::Internal)
        );
    }

    #[test]
    fn parse_rtmp_url_rejects_missing_stream_key() {
        assert_eq!(
            parse_rtmp_url("rtmp://example.com/live"),
            Err(ErrorCode::Internal)
        );
    }
}
