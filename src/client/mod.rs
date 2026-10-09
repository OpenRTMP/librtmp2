//! Outbound RTMP client
//!
//! Mirrors `src/client/client.h` and `src/client/client.c`.

use std::collections::VecDeque;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::buffer::Buffer;
use crate::chunk::media_out::{MediaHeaderTracker, MediaMessageInfo, write_media_message};
use crate::chunk::reader::{ChunkMessage, chunk_read_owned};
use crate::chunk::state::{ChunkRegistry, DEFAULT_MAX_MSG_LENGTH};
use crate::chunk::writer::chunk_write;
use crate::ertmp::multitrack_media::{foreach_track, is_multitrack_container};
use crate::handshake::{self, Handshake};
use crate::media::{
    is_on_metadata_payload, normalize_modex_payload_with_frame_type, populate_av_frame,
    populate_multitrack_frame,
};
use crate::message::command;
use crate::message::control;
use crate::message::message as msg_dispatch;
use crate::net::{self, Interest, Wait};
use crate::transport::Transport;
use crate::types::*;

/// Cap aggregate sub-tags per message (matches server-side limit).
const MAX_AGGREGATE_SUBTAGS: usize = 4096;

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
/// Maximum time to wait for the initial TCP connect (and DNS resolution) before failing.
const TCP_CONNECT_TIMEOUT_SECS: u64 = 10;
/// `recv_buffer` holds raw wire bytes, not just message payloads, so the
/// staging cap must budget for chunk-header overhead on top of the payload
/// bytes it is meant to bound.
const MAX_RECV_BUFFER_PAYLOAD_BYTES: usize = 2 * DEFAULT_MAX_MSG_LENGTH as usize;
/// Smallest chunk size a peer can realistically negotiate down to via
/// `SetChunkSize` (128 is the RTMP default and the practical floor seen from
/// real encoders). At this size, worst-case continuation-chunk framing is a
/// 3-byte basic header (the extended 2-byte CSID form the reader accepts for
/// csid >= 320, RTMP spec 5.3.1.1) plus a 4-byte extended timestamp field
/// (RTMP spec 5.3.1.3) repeated on every chunk of a message.
const MIN_PRACTICAL_CHUNK_SIZE: usize = 128;
const MAX_CHUNK_HEADER_OVERHEAD_BYTES: usize = 7;
/// Extra header bytes a message's first (fmt=0) chunk carries over a fmt=3
/// continuation chunk: the 11-byte message header (timestamp + length +
/// type id + stream id) that continuations omit.
const FIRST_CHUNK_EXTRA_OVERHEAD_BYTES: usize = 11;
/// Cap incomplete wire data staged in `recv_buffer` between chunk reads.
/// Mirrors the server-side staging limit in `session::conn` so a malicious
/// peer cannot retain up to `BUFFER_MAX_SIZE` (64 MiB) per client connection
/// when message budgets defer draining. Includes headroom for chunk-header
/// overhead (plus the two max-size messages' larger first-chunk headers) so
/// two max-size messages at the minimum practical chunk size don't get
/// rejected before they can be reassembled.
const MAX_RECV_BUFFER_BYTES: usize = MAX_RECV_BUFFER_PAYLOAD_BYTES
    + (MAX_RECV_BUFFER_PAYLOAD_BYTES / MIN_PRACTICAL_CHUNK_SIZE + 1)
        * MAX_CHUNK_HEADER_OVERHEAD_BYTES
    + 2 * FIRST_CHUNK_EXTRA_OVERHEAD_BYTES;
/// Cap inbound Ping-Request reflections to prevent trivial outbound
/// bandwidth/CPU amplification from a malicious RTMP(S) server.
const MAX_INBOUND_PING_RESPONSES: usize = 8;
const INBOUND_PING_WINDOW: Duration = Duration::from_secs(1);

/// Max DNS jobs waiting on the shared resolver thread.
const MAX_DNS_QUEUE_DEPTH: usize = 32;

struct DnsJob {
    host: String,
    port: u16,
    reply: mpsc::Sender<std::result::Result<Vec<std::net::SocketAddr>, ()>>,
}

/// Bounded work queue for the shared DNS resolver thread. Producers block on
/// `not_full` when the queue is at `MAX_DNS_QUEUE_DEPTH` and are woken as
/// soon as the worker dequeues a job, rather than polling: with many
/// concurrent `Client::connect()` calls outrunning the queue, a poll loop
/// would wake every blocked caller on a fixed interval regardless of whether
/// room actually opened up, burning CPU in proportion to the number of
/// waiters instead of the number of state changes.
struct DnsQueue {
    jobs: Mutex<VecDeque<DnsJob>>,
    not_empty: Condvar,
    not_full: Condvar,
}

impl DnsQueue {
    fn new() -> Self {
        Self {
            jobs: Mutex::new(VecDeque::new()),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
        }
    }

    /// Block until `job` is enqueued or `deadline` elapses, without polling.
    fn enqueue(&self, job: DnsJob, deadline: Instant) -> std::result::Result<(), ErrorCode> {
        let mut guard = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // This wakeup may have been meant for another waiter (e.g.
                // our deadline elapsed right as a slot freed up). notify_one
                // wakes exactly one waiter per freed slot; if we're bailing
                // while a slot is actually free, relay the notification so
                // it isn't stranded on every other waiter, rather than
                // broadcasting to all waiters on every dequeue regardless of
                // whether anyone is bailing.
                if guard.len() < MAX_DNS_QUEUE_DEPTH {
                    self.not_full.notify_one();
                }
                return Err(ErrorCode::Timeout);
            }
            if guard.len() < MAX_DNS_QUEUE_DEPTH {
                guard.push_back(job);
                drop(guard);
                self.not_empty.notify_one();
                return Ok(());
            }
            guard = self
                .not_full
                .wait_timeout(guard, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Pop the next queued job, blocking (without polling) while empty, and
    /// notify one producer that a slot has freed up. Split out from `run()`
    /// so tests can exercise the real pop-and-notify path without paying for
    /// (or depending on the timing of) an actual DNS lookup.
    fn dequeue_one(&self) -> DnsJob {
        let mut guard = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let job = loop {
            if let Some(job) = guard.pop_front() {
                break job;
            }
            guard = self
                .not_empty
                .wait(guard)
                .unwrap_or_else(|e| e.into_inner());
        };
        drop(guard);
        self.not_full.notify_one();
        job
    }

    /// Drain jobs one at a time, blocking (without polling) while empty.
    fn run(&self) {
        loop {
            let job = self.dequeue_one();
            let result = (job.host.as_str(), job.port)
                .to_socket_addrs()
                .map(|iter| iter.collect::<Vec<_>>())
                .map_err(|_| ());
            let _ = job.reply.send(result);
        }
    }
}

/// Resolve `host:port` with a wall-clock deadline so DNS cannot block longer
/// than the TCP connect budget. Lookups run on a single shared worker thread
/// so timed-out requests do not spawn unbounded detached resolver threads.
fn resolve_socket_addrs(
    host: &str,
    port: u16,
    deadline: Instant,
) -> Result<Vec<std::net::SocketAddr>> {
    static DNS_QUEUE: Mutex<Option<Arc<DnsQueue>>> = Mutex::new(None);
    let queue = {
        let mut guard = DNS_QUEUE.lock().map_err(|_| ErrorCode::Internal)?;
        if let Some(queue) = guard.as_ref() {
            queue.clone()
        } else {
            let queue = Arc::new(DnsQueue::new());
            let worker = queue.clone();
            std::thread::Builder::new()
                .name("lrtmp2-dns".into())
                .spawn(move || worker.run())
                .map_err(|_| ErrorCode::Internal)?;
            *guard = Some(queue.clone());
            queue
        }
    };

    let (reply_tx, reply_rx) = mpsc::channel();
    queue.enqueue(
        DnsJob {
            host: host.to_string(),
            port,
            reply: reply_tx,
        },
        deadline,
    )?;

    let remaining = deadline.saturating_duration_since(Instant::now());
    match reply_rx.recv_timeout(remaining) {
        Ok(Ok(addrs)) if !addrs.is_empty() => Ok(addrs),
        Ok(Ok(_)) | Ok(Err(_)) => Err(ErrorCode::Io),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(ErrorCode::Timeout),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(ErrorCode::Io),
    }
}

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
    /// Copy of the connected transport's socket handle (`i32` file
    /// descriptor on Unix, `u64` Winsock `SOCKET` on Windows), or
    /// [`net::INVALID_SOCKET`] when not connected. The transport owns it.
    pub client_fd: net::RawSocket,
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
    /// Fired when the server sends `NetConnection.Connect.ReconnectRequest`
    /// (E-RTMP v2 reconnect mechanism). `tc_url` is `Some` when the server
    /// wants the client to reconnect to a different URL, `None` to reuse the
    /// current one. The library only delivers the event -- establishing the
    /// new connection and disconnecting from this one is left to the host
    /// application, which can keep streaming through the next media boundary
    /// before doing so.
    pub on_reconnect_request_cb: Option<fn(tc_url: Option<&str>, description: Option<&str>)>,
    /// Retains the last frame payload delivered through `on_frame_cb` so
    /// `Frame.data` stays valid until the next callback on this connection
    /// (mirrors `Conn::frame_cb_scratch` on the server side).
    frame_cb_scratch: Vec<u8>,
    /// PEM CA bundle used to verify `rtmps://` servers, in addition to the
    /// system trust store. `None` uses the system trust store only.
    tls_ca_file: Option<String>,
    /// Skip TLS certificate verification for `rtmps://` connections.
    /// Only for testing against self-signed deployments.
    tls_insecure: bool,
    /// Overall wall-clock budget for blocking client I/O during `connect()`,
    /// `publish()`, and `play()`. `None` uses `TCP_CONNECT_TIMEOUT_SECS`.
    connect_timeout: Option<Duration>,
    inbound_ping_window_start: Option<Instant>,
    inbound_ping_responses: usize,
    /// E-RTMP capabilities negotiated during `connect` (used for ModEx unwrap).
    negotiated_caps: NegotiatedCaps,
    /// Peer's WindowAckSize (`0` = disabled). Once this many inbound bytes
    /// arrive without an Acknowledgement, one is sent with the running count.
    window_ack_size: u32,
    /// Total socket bytes received on this connection. Kept as `u64` so the
    /// `u32` acknowledgement sequence number wrapping is expected, not a
    /// counter reset.
    bytes_received: u64,
    bytes_at_last_ack: u64,
    /// An Acknowledgement is already queued in `send_buffer` and not yet
    /// flushed; further window crossings coalesce into it instead of
    /// appending acknowledgements a stalled peer could make accumulate.
    ack_pending: bool,
    /// Chunk size this client announces with `SetChunkSize` when it starts
    /// publishing, and then uses for media. Larger chunks mean fewer chunk
    /// headers and fewer reassembly steps on the server for every frame.
    /// Defaults to [`DEFAULT_PUBLISH_CHUNK_SIZE`]; values outside
    /// `128..=MAX_OUTBOUND_CHUNK_SIZE` are clamped when publishing starts.
    pub publish_chunk_size: u32,
    /// Use fmt=1/2 (delta) first-chunk headers for media where possible.
    pub compact_media_headers: bool,
    /// Chunk size currently in effect for outbound messages.
    out_chunk_size: usize,
    media_out_headers: MediaHeaderTracker,
}

/// Default [`Client::publish_chunk_size`], matching what ffmpeg and OBS
/// announce.
pub const DEFAULT_PUBLISH_CHUNK_SIZE: u32 = 4096;

/// Largest chunk size [`Client::publish_chunk_size`] may request (the RTMP
/// spec caps it at 0x7FFFFFFF; servers commonly accept up to 64 KiB).
pub const MAX_OUTBOUND_CHUNK_SIZE: u32 = 65536;

/// RTMP chunk size in effect before either side sends `SetChunkSize`.
const RTMP_DEFAULT_CHUNK_SIZE: usize = 128;

impl Client {
    /// Create a new client.
    pub fn new() -> Self {
        Self {
            client_fd: net::INVALID_SOCKET,
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
            on_reconnect_request_cb: None,
            frame_cb_scratch: Vec::new(),
            tls_ca_file: None,
            tls_insecure: false,
            connect_timeout: None,
            inbound_ping_window_start: None,
            inbound_ping_responses: 0,
            publish_chunk_size: DEFAULT_PUBLISH_CHUNK_SIZE,
            compact_media_headers: true,
            out_chunk_size: RTMP_DEFAULT_CHUNK_SIZE,
            media_out_headers: MediaHeaderTracker::default(),
            negotiated_caps: NegotiatedCaps::default(),
            window_ack_size: 0,
            bytes_received: 0,
            bytes_at_last_ack: 0,
            ack_pending: false,
        }
    }

    /// Configure `rtmps://` verification for subsequent `connect()` calls.
    pub fn set_tls_client_config(&mut self, ca_file: Option<String>, insecure: bool) {
        self.tls_ca_file = ca_file;
        self.tls_insecure = insecure;
    }

    /// Override the overall wall-clock budget for subsequent blocking client
    /// calls (`connect()`, `publish()`, and `play()`). DNS resolution, TCP
    /// connect, TLS handshake, RTMP handshake, and each AMF command exchange
    /// share this budget within the call they belong to. Defaults to
    /// `TCP_CONNECT_TIMEOUT_SECS` when never called.
    pub fn set_connect_timeout(&mut self, timeout: Duration) {
        self.connect_timeout = Some(timeout);
    }

    /// Wall-clock deadline for a single blocking AMF command exchange.
    fn command_io_deadline(&self) -> Result<Instant> {
        let timeout = self
            .connect_timeout
            .unwrap_or(Duration::from_secs(TCP_CONNECT_TIMEOUT_SECS));
        Instant::now()
            .checked_add(timeout)
            .ok_or(ErrorCode::Internal)
    }

    /// Connect to an RTMP(S) server at `rtmp://host[:port]/app/streamKey` or
    /// `rtmps://host[:port]/app/streamKey`.
    ///
    /// Performs the real TCP connect (wrapped in a TLS client handshake for
    /// `rtmps://`, verified against the system trust store by default), the
    /// legacy C0/C1/C2 handshake, then the `connect` + `createStream` AMF0
    /// command exchange. Call [`Client::set_tls_client_config`] before
    /// `connect()` to trust an additional CA bundle or disable verification.
    pub fn connect(&mut self, url: &str) -> Result<()> {
        // Before any early return below: a failed connect must tear down the
        // previous session, or a later `publish()`/`play()` keeps streaming to
        // the old server.
        self.reset_session_state();
        let (use_tls, host, port, app, stream_key) = parse_rtmp_url(url)?;
        if use_tls && !crate::transport::tls_available() {
            return Err(ErrorCode::Unsupported);
        }

        let connect_timeout = self
            .connect_timeout
            .unwrap_or(Duration::from_secs(TCP_CONNECT_TIMEOUT_SECS));
        // A caller-supplied timeout could in principle be large enough that
        // adding it to `Instant::now()` overflows the clock's representable
        // range; `Instant::now() + timeout` would panic in that case, so use
        // `checked_add` and fail the connect instead of aborting the process.
        let deadline = Instant::now()
            .checked_add(connect_timeout)
            .ok_or(ErrorCode::Internal)?;
        let addrs = resolve_socket_addrs(&host, port, deadline)?;
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
        // RTMP's handshake and command exchange are many small,
        // latency-sensitive round trips; without TCP_NODELAY, Nagle's
        // algorithm here combined with the peer's delayed ACK can stall each
        // one by tens of ms. Best-effort: an unsupported/dead socket just
        // keeps Nagle's default behavior.
        let _ = stream.set_nodelay(true);
        let mut transport = if use_tls {
            let remaining = deadline.saturating_duration_since(Instant::now());
            Transport::connect_tls_with_timeout(
                stream,
                &host,
                self.tls_ca_file.as_deref(),
                self.tls_insecure,
                remaining,
            )?
        } else {
            Transport::from_tcp_stream(stream)
        };

        self.state = ClientState::Handshaking;
        if let Err(e) = self.do_handshake(&mut transport, deadline) {
            // transport drops here, closing the fd via Transport::drop
            return Err(e);
        }

        self.client_fd = transport.fd();
        self.transport = Some(transport);
        self.app = app.clone();
        self.stream_key = stream_key;
        self.state = ClientState::Connected;

        if let Err(e) = self.do_amf_connect(&app, &host, port, use_tls, deadline) {
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
        let deadline = self.command_io_deadline()?;
        let mut amf = Buffer::with_capacity(256);
        command::build_publish(&mut amf, &self.stream_key, "live")?;
        self.send_command_msg(self.stream_id, amf.as_slice(), Some(deadline))?;
        let mut recv_budget = MAX_RECV_BYTES_PER_COMMAND_WAIT;
        loop {
            let mut status =
                self.wait_for_command_with_budget("onStatus", Some(deadline), &mut recv_budget)?;
            if command::read_onstatus(&mut status, "NetStream.Publish.Start")? {
                break;
            }
        }
        self.state = ClientState::Publishing;
        self.queue_publish_chunk_size()?;
        Ok(())
    }

    /// Announce [`Self::publish_chunk_size`] ahead of the first media
    /// message. It is queued (and flushed with the first frame) in order
    /// with everything sent after it, so the server applies it before any
    /// chunk that relies on it.
    fn queue_publish_chunk_size(&mut self) -> Result<()> {
        let chunk_size = self
            .publish_chunk_size
            .clamp(RTMP_DEFAULT_CHUNK_SIZE as u32, MAX_OUTBOUND_CHUNK_SIZE);
        if chunk_size as usize == self.out_chunk_size {
            return Ok(());
        }
        let mut payload = Buffer::with_capacity(4);
        control::write_set_chunk_size(&mut payload, chunk_size)?;
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 2;
        cmsg.fmt = 0;
        cmsg.msg_length = 4;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_SET_CHUNK_SIZE;
        cmsg.msg_stream_id = 0;
        chunk_write(
            &mut self.send_buffer,
            &cmsg,
            payload.as_slice(),
            4,
            self.out_chunk_size,
        )?;
        self.out_chunk_size = chunk_size as usize;
        Ok(())
    }

    /// Run the AMF connect + createStream exchange. Separated from `connect()`
    /// so the transport is already stored before we enter, letting the caller
    /// call `reset_session_state()` (which drops the transport) on any error.
    fn do_amf_connect(
        &mut self,
        app: &str,
        host: &str,
        port: u16,
        use_tls: bool,
        deadline: Instant,
    ) -> Result<()> {
        let scheme = if use_tls { "rtmps" } else { "rtmp" };
        let tc_url = format!("{scheme}://{host}:{port}/{app}");
        // Only advertise reconnect support when the host has actually wired
        // up on_reconnect_request_cb -- otherwise a spec-compliant server
        // would have no reason to ever send a ReconnectRequest to this
        // client, but we also must not claim a capability nothing handles.
        // Both `has_caps_ex`/`caps_ex_mask` *and* `has_reconnect` must be set:
        // `negotiate_caps` on the server side only folds
        // `CAPS_EX_MASK_RECONNECT` into its response mask when the client's
        // parsed `ConnectInfo::has_reconnect` is also true (see
        // `ertmp::connect_amf::negotiate_caps`), which requires an actual
        // `reconnect` AMF value on the wire, not just the capsEx bit.
        let advertised_caps = self.on_reconnect_request_cb.map(|_| NegotiatedCaps {
            has_caps_ex: true,
            caps_ex_mask: CAPS_EX_MASK_RECONNECT,
            has_reconnect: true,
            reconnect: Reconnect::default(),
            ..Default::default()
        });
        let mut connect_amf = Buffer::with_capacity(512);
        command::build_connect(
            &mut connect_amf,
            app,
            &tc_url,
            "",
            "",
            "FMLE/3.0",
            0,
            0,
            advertised_caps.as_ref(),
        )?;
        self.send_command_msg(0, connect_amf.as_slice(), Some(deadline))?;
        let mut result = self.wait_for_command("_result", Some(deadline))?;
        command::read_connect_result_with_caps(&mut result, Some(&mut self.negotiated_caps))?;

        let mut create_stream_amf = Buffer::with_capacity(64);
        command::build_create_stream(&mut create_stream_amf, 2.0)?;
        self.send_command_msg(0, create_stream_amf.as_slice(), Some(deadline))?;
        let mut create_result = self.wait_for_command("_result", Some(deadline))?;
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
        let deadline = self.command_io_deadline()?;
        let mut amf = Buffer::with_capacity(256);
        command::build_play(&mut amf, &self.stream_key)?;
        self.send_command_msg(self.stream_id, amf.as_slice(), Some(deadline))?;
        let mut recv_budget = MAX_RECV_BYTES_PER_COMMAND_WAIT;
        loop {
            let mut status =
                self.wait_for_command_with_budget("onStatus", Some(deadline), &mut recv_budget)?;
            if command::read_onstatus(&mut status, "NetStream.Play.Start")? {
                break;
            }
        }
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
        self.try_flush_send_buffer()?;
        self.service_inbound(0)?;
        if payload.len() > MAX_CLIENT_FRAME_BYTES {
            return Err(ErrorCode::Protocol);
        }

        write_media_message(
            &mut self.send_buffer,
            &mut self.media_out_headers,
            self.compact_media_headers,
            MediaMessageInfo {
                frame_type,
                msg_stream_id: self.stream_id,
                timestamp,
            },
            payload,
            self.out_chunk_size,
        )?;

        // Non-blocking flush: a malicious server that stops reading must not
        // stall the embedder's thread for up to 10s per frame via blocking send.
        self.try_flush_send_buffer()?;

        Ok(())
    }

    /// Wait once for the socket to become writable, bounded by `timeout_ms`.
    /// Returns `true` when it is writable (so the read can be retried) and
    /// `false` on timeout or poll error.
    fn wait_writable_once(fd: net::RawSocket, timeout_ms: i32) -> bool {
        net::poll_one(fd, Interest::Write, timeout_ms) == Wait::Ready
    }

    /// Poll for incoming control traffic and flush queued outbound bytes.
    pub fn poll(&mut self, timeout_ms: i32) -> Result<()> {
        if self.state == ClientState::Publishing {
            let send_poll_again = self.try_flush_send_buffer()?;
            if self.send_buffer.available() > 0 {
                if let Some(t) = self.transport.as_ref() {
                    let again = send_poll_again.unwrap_or(2);
                    // A full send buffer just means "not writable yet"; treat
                    // the poll timeout as nothing-ready (as the Playing branch
                    // does) instead of aborting a healthy publish session.
                    match poll_for_transport_direction(t.fd(), again, timeout_ms) {
                        Ok(()) | Err(ErrorCode::Timeout) => {}
                        Err(e) => return Err(e),
                    }
                }
                self.try_flush_send_buffer()?;
            }
            // While outbound bytes remain queued (e.g. pong after EAGAIN), do
            // not block on POLLIN-only service_inbound — that can delay the
            // pong until the read timeout even after the socket is writable.
            let inbound_timeout = if self.send_buffer.available() > 0 {
                0
            } else {
                timeout_ms
            };
            self.service_inbound(inbound_timeout)?;
            self.try_flush_send_buffer()?;
            return Ok(());
        }
        if self.state != ClientState::Playing {
            return Err(ErrorCode::Protocol);
        }

        self.try_flush_send_buffer()?;
        // Scope the mutable transport borrow to the recv phase only.
        let (poll_fd, has_buffered_tls_data) = {
            let Some(t) = self.transport.as_ref() else {
                return Err(ErrorCode::Internal);
            };
            (t.fd(), t.pending() > 0)
        };

        let mut messages_processed = 0usize;
        // Drain any complete messages a prior poll() left staged in
        // recv_buffer (it may have stopped at MAX_MESSAGES_PER_POLL) before
        // reading more off the socket or blocking on socket readiness below.
        // Otherwise (a) the staging-cap check further down sees those
        // leftover bytes plus newly read bytes and can reject a read that
        // draining first would have made room for, and (b) a caller with a
        // long or infinite timeout would stall waiting on the socket instead
        // of getting complete messages that were already sitting in the
        // buffer.
        self.drain_ready_messages(&mut messages_processed)?;

        // A prior poll() call may have stopped draining at
        // MAX_RECV_BYTES_PER_POLL while OpenSSL still held decrypted
        // plaintext internally. The kernel socket can then have nothing left
        // to report ready, so blocking in poll(2) here would wait out the
        // full timeout even though data is already available via recv().
        // Likewise, skip the wait entirely if the drain above already made
        // progress -- there is no need to block on socket readiness when
        // complete messages were just delivered.
        if messages_processed == 0 && !has_buffered_tls_data {
            // poll(2) and WSAPoll both treat a negative timeout as "block
            // indefinitely", so pass it through as-is instead of clamping to 0.
            net::poll_one(poll_fd, Interest::Read, timeout_ms);
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
                if self.recv_buffer.available().saturating_add(chunk_len) > MAX_RECV_BUFFER_BYTES {
                    return Err(ErrorCode::Protocol);
                }
                self.bytes_received = self.bytes_received.saturating_add(chunk_len as u64);
                self.recv_buffer
                    .write(&buf[..chunk_len])
                    .map_err(|_| ErrorCode::Internal)?;
                bytes_drained += chunk_len;
            } else if n == 0 {
                return Err(ErrorCode::Io);
            } else if again == 2 && Self::wait_writable_once(poll_fd, timeout_ms) {
                // TLS renegotiation can need write-readiness during a read; the
                // POLLIN wait above cannot detect that on its own. Wait for
                // POLLOUT once, bounded by the same timeout, then retry the read
                // instead of giving up on a writable socket.
                continue;
            } else if again == 0 {
                // A fatal recv error that the transport reported without asking
                // for a retry: treating it like would-block would spin on a
                // reset socket forever, so surface it instead of looping.
                return Err(ErrorCode::Io);
            } else {
                break;
            }
        }

        self.drain_ready_messages(&mut messages_processed)?;

        self.maybe_send_window_ack()?;
        self.try_flush_send_buffer()?;
        Ok(())
    }

    /// Process fully-reassembled messages already staged in `recv_buffer`,
    /// up to `MAX_MESSAGES_PER_POLL` total across calls sharing
    /// `messages_processed`. Stops as soon as the next message is incomplete.
    fn drain_ready_messages(&mut self, messages_processed: &mut usize) -> Result<()> {
        loop {
            if *messages_processed >= MAX_MESSAGES_PER_POLL {
                break;
            }

            let before = self.recv_buffer.available();
            let mut msg = ChunkMessage::default();
            match chunk_read_owned(&mut self.recv_buffer, &mut self.chunk_reg, &mut msg) {
                Ok((1, payload)) if msg.is_complete => {
                    if msg.msg_type_id == msg_dispatch::RTMP_MSG_AGGREGATE {
                        self.handle_aggregate_message(msg.timestamp, &payload, messages_processed)?;
                    } else {
                        if *messages_processed >= MAX_MESSAGES_PER_POLL {
                            break;
                        }
                        *messages_processed += 1;
                        if msg.msg_type_id == msg_dispatch::RTMP_MSG_SET_CHUNK_SIZE {
                            let cs = control::read_set_chunk_size(&payload)?;
                            self.chunk_reg.set_all_chunk_size(cs);
                        } else if msg.msg_type_id == msg_dispatch::RTMP_MSG_ABORT_MESSAGE {
                            // Discard only the partial reassembly on the
                            // aborted CSID; the header-compression context is
                            // retained for a following fmt=1/2/3 chunk.
                            if let Ok(csid) = control::read_abort_message(&payload) {
                                self.chunk_reg.abort_stream(csid);
                            }
                        } else if msg.msg_type_id == msg_dispatch::RTMP_MSG_WINDOW_ACK_SIZE {
                            if let Ok(win) = control::read_window_ack_size(&payload) {
                                self.window_ack_size = win;
                            }
                        } else if msg.msg_type_id == msg_dispatch::RTMP_MSG_USER_CONTROL {
                            self.handle_user_control(&payload)?;
                        } else if msg.msg_type_id == msg_dispatch::RTMP_MSG_AUDIO
                            || msg.msg_type_id == msg_dispatch::RTMP_MSG_VIDEO
                        {
                            if let Some(cb) = self.on_frame_cb {
                                let frame_type = if msg.msg_type_id == msg_dispatch::RTMP_MSG_AUDIO
                                {
                                    FrameType::Audio
                                } else {
                                    FrameType::Video
                                };
                                self.deliver_av_frame_cb(
                                    cb,
                                    frame_type,
                                    msg.timestamp,
                                    &payload,
                                    messages_processed,
                                )?;
                            }
                        } else if msg.msg_type_id == msg_dispatch::RTMP_MSG_AMF0_DATA
                            || msg.msg_type_id == msg_dispatch::RTMP_MSG_AMF3_DATA
                        {
                            let data_payload: &[u8] = if msg.msg_type_id
                                == msg_dispatch::RTMP_MSG_AMF3_DATA
                                && !payload.is_empty()
                                && payload[0] == 0x00
                            {
                                &payload[1..]
                            } else {
                                &payload
                            };
                            if let Some(cb) = self.on_frame_cb {
                                self.deliver_script_frame_cb(cb, msg.timestamp, data_payload);
                            }
                        } else if msg.msg_type_id == msg_dispatch::RTMP_MSG_AMF0_COMMAND {
                            self.handle_command_message(&payload);
                        } else if msg.msg_type_id == msg_dispatch::RTMP_MSG_AMF3_COMMAND {
                            let data: &[u8] = if !payload.is_empty() && payload[0] == 0x00 {
                                &payload[1..]
                            } else {
                                &payload
                            };
                            self.handle_command_message(data);
                        }
                    }
                }
                Ok(_) => {
                    // Ok(0) means both "need more bytes" and "consumed a
                    // non-final fragment". Only stop when the cursor did not
                    // advance — leftover continuation chunks may already be
                    // in recv_buffer.
                    if self.recv_buffer.available() >= before {
                        break;
                    }
                }
                Err(_) => return Err(ErrorCode::Chunk),
            }
        }
        Ok(())
    }

    /// Handle an AMF command received outside the blocking connect/publish/play
    /// handshakes (e.g. a server-initiated `onStatus` mid-session). Unknown or
    /// malformed commands are ignored -- this path only reacts to events the
    /// client understands.
    fn handle_command_message(&mut self, payload: &[u8]) {
        let mut buf = Buffer::from_slice(payload);
        if let Ok(Some(req)) = command::read_reconnect_request(&mut buf) {
            if let Some(cb) = self.on_reconnect_request_cb {
                cb(req.tc_url.as_deref(), req.description.as_deref());
            }
        }
    }

    /// Unpack aggregate A/V/script sub-tags for play-side frame callbacks.
    fn handle_aggregate_message(
        &mut self,
        base_timestamp: u32,
        payload: &[u8],
        messages_processed: &mut usize,
    ) -> Result<()> {
        let mut pos = 0usize;
        let mut have_base = false;
        let mut sub_base_ts: u32 = 0;
        let mut subtags = 0usize;

        while pos + 11 <= payload.len() {
            if subtags >= MAX_AGGREGATE_SUBTAGS {
                return Err(ErrorCode::Protocol);
            }
            subtags += 1;

            let tag_type = payload[pos];
            let data_size = ((payload[pos + 1] as u32) << 16)
                | ((payload[pos + 2] as u32) << 8)
                | (payload[pos + 3] as u32);
            let ts = ((payload[pos + 4] as u32) << 16)
                | ((payload[pos + 5] as u32) << 8)
                | (payload[pos + 6] as u32)
                | ((payload[pos + 7] as u32) << 24);
            let body = pos + 11;
            let data_size = data_size as usize;
            if body + data_size > payload.len() {
                return Err(ErrorCode::Protocol);
            }
            if data_size == 0 {
                return Err(ErrorCode::Protocol);
            }
            if *messages_processed >= MAX_MESSAGES_PER_POLL {
                break;
            }
            *messages_processed += 1;
            if !have_base {
                sub_base_ts = ts;
                have_base = true;
            }
            let out_ts = base_timestamp.wrapping_add(ts.wrapping_sub(sub_base_ts));
            let tag_payload = &payload[body..body + data_size];

            if let Some(cb) = self.on_frame_cb {
                match tag_type {
                    msg_dispatch::RTMP_MSG_AUDIO => {
                        self.deliver_av_frame_cb(
                            cb,
                            FrameType::Audio,
                            out_ts,
                            tag_payload,
                            messages_processed,
                        )?;
                    }
                    msg_dispatch::RTMP_MSG_VIDEO => {
                        self.deliver_av_frame_cb(
                            cb,
                            FrameType::Video,
                            out_ts,
                            tag_payload,
                            messages_processed,
                        )?;
                    }
                    msg_dispatch::RTMP_MSG_AMF0_DATA => {
                        self.deliver_script_frame_cb(cb, out_ts, tag_payload);
                    }
                    _ => {
                        pos = body + data_size + 4;
                        continue;
                    }
                }
            } else {
                match tag_type {
                    msg_dispatch::RTMP_MSG_AUDIO
                    | msg_dispatch::RTMP_MSG_VIDEO
                    | msg_dispatch::RTMP_MSG_AMF0_DATA => {}
                    _ => {
                        pos = body + data_size + 4;
                        continue;
                    }
                }
            }

            pos = body + data_size + 4;
        }
        Ok(())
    }

    // ── Internal helpers ──

    fn deliver_av_frame_cb(
        &mut self,
        cb: fn(&Frame),
        frame_type: FrameType,
        timestamp: u32,
        payload: &[u8],
        messages_processed: &mut usize,
    ) -> Result<()> {
        // Always peel ModEx wrappers for frame metadata, as `handle_media_frame`
        // does on the server side: gating on the negotiated `caps_ex_mask`
        // (which never carries the MODEX bit) would report the wrapper's
        // extension bytes at 1..5 as the codec FourCC.
        let normalized =
            normalize_modex_payload_with_frame_type(payload, CAPS_EX_MASK_MODEX, frame_type);
        let parse_payload = normalized.as_ref();
        let is_multitrack = is_multitrack_container(frame_type, parse_payload);
        let mut track_index = 0usize;
        let parsed_multitrack = foreach_track(frame_type, parse_payload, |track| {
            if track_index > 0 {
                if *messages_processed >= MAX_MESSAGES_PER_POLL {
                    return;
                }
                *messages_processed += 1;
            }
            track_index += 1;
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
        if is_multitrack && !parsed_multitrack {
            return Err(ErrorCode::Protocol);
        }
        if !is_multitrack {
            self.invoke_on_frame_cb(cb, frame_type, timestamp, u8::MAX, parse_payload);
        }
        Ok(())
    }

    fn invoke_multitrack_on_frame_cb(
        &mut self,
        cb: fn(&Frame),
        frame_type: FrameType,
        timestamp: u32,
        track_id: u8,
        fourcc: [u8; 4],
        packet_type: u8,
        video_frame_type: u8,
        payload: &[u8],
    ) {
        self.frame_cb_scratch.clear();
        self.frame_cb_scratch.extend_from_slice(payload);
        let mut frame = Frame {
            frame_type,
            timestamp,
            size: self.frame_cb_scratch.len() as u32,
            data: self.frame_cb_scratch.as_ptr(),
            track_id,
            ..Default::default()
        };
        populate_multitrack_frame(&mut frame, fourcc, packet_type, video_frame_type);
        cb(&frame);
    }
    fn invoke_on_frame_cb(
        &mut self,
        cb: fn(&Frame),
        frame_type: FrameType,
        timestamp: u32,
        track_id: u8,
        payload: &[u8],
    ) {
        self.frame_cb_scratch.clear();
        self.frame_cb_scratch.extend_from_slice(payload);
        let mut frame = Frame {
            frame_type,
            timestamp,
            size: self.frame_cb_scratch.len() as u32,
            data: self.frame_cb_scratch.as_ptr(),
            track_id,
            ..Default::default()
        };
        populate_av_frame(&mut frame, &self.frame_cb_scratch);
        cb(&frame);
    }

    fn deliver_script_frame_cb(&mut self, cb: fn(&Frame), timestamp: u32, payload: &[u8]) {
        let is_metadata = u8::from(is_on_metadata_payload(payload));
        self.frame_cb_scratch.clear();
        self.frame_cb_scratch.extend_from_slice(payload);
        let frame = Frame {
            frame_type: FrameType::Script,
            timestamp,
            size: self.frame_cb_scratch.len() as u32,
            data: self.frame_cb_scratch.as_ptr(),
            is_metadata,
            ..Default::default()
        };
        cb(&frame);
    }

    fn queue_control_message(&mut self, msg_type_id: u8, payload: &[u8]) -> Result<()> {
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 2;
        cmsg.fmt = 0;
        cmsg.msg_length = payload.len() as u32;
        cmsg.msg_type_id = msg_type_id;
        cmsg.msg_stream_id = 0;
        chunk_write(
            &mut self.send_buffer,
            &cmsg,
            payload,
            payload.len(),
            self.out_chunk_size,
        )?;
        Ok(())
    }

    fn queue_user_control_message(&mut self, payload: &[u8]) -> Result<()> {
        self.queue_control_message(msg_dispatch::RTMP_MSG_USER_CONTROL, payload)
    }

    /// Queue an Acknowledgement once `window_ack_size` inbound bytes have
    /// arrived since the last one. The `u64` counters let the on-wire `u32`
    /// sequence number wrap without resetting the byte accounting.
    ///
    /// At most one acknowledgement is queued at a time: while one is still
    /// waiting to be flushed, further crossings are coalesced into it, so a
    /// peer that streams without reading its socket direction cannot grow
    /// `send_buffer` by driving acknowledgement after acknowledgement.
    fn maybe_send_window_ack(&mut self) -> Result<()> {
        if self.ack_pending {
            return Ok(());
        }
        if self.window_ack_size > 0
            && self.bytes_received.saturating_sub(self.bytes_at_last_ack)
                >= self.window_ack_size as u64
        {
            let mut payload = Buffer::with_capacity(4);
            control::write_acknowledgement(&mut payload, self.bytes_received as u32)?;
            self.queue_control_message(msg_dispatch::RTMP_MSG_ACKNOWLEDGEMENT, payload.as_slice())?;
            self.bytes_at_last_ack = self.bytes_received;
            self.ack_pending = true;
        }
        Ok(())
    }

    /// Flush queued outbound control bytes (e.g. a due Acknowledgement)
    /// before the caller blocks on more inbound data. A peer enforcing its
    /// advertised window stops sending until the acknowledgement arrives, so
    /// leaving it merely queued would stall the exchange until the caller's
    /// deadline. Bounded by `deadline` when one is supplied.
    fn flush_pending_outbound(&mut self, deadline: Option<Instant>) -> Result<()> {
        if self.send_buffer.available() == 0 {
            return Ok(());
        }
        let data = self.send_buffer.peek().to_vec();
        if let Some(ref mut transport) = self.transport {
            match deadline {
                Some(deadline) => send_bounded(transport, &data, deadline)?,
                None => transport.send(&data)?,
            }
        }
        self.send_buffer.reset();
        self.ack_pending = false;
        Ok(())
    }

    /// Flush queued outbound bytes without blocking.
    ///
    /// Returns the poll direction reported by the last `try_send` when bytes
    /// remain queued (1 = `POLLIN` for TLS WANT_READ, 2 = `POLLOUT`).
    fn try_flush_send_buffer(&mut self) -> Result<Option<i32>> {
        let mut poll_again = None;
        while self.send_buffer.available() > 0 {
            let Some(ref mut transport) = self.transport else {
                break;
            };
            let pending = self.send_buffer.peek();
            let mut again = 0i32;
            let n = transport.try_send(pending, &mut again)?;
            if n == 0 {
                if again != 0 {
                    poll_again = Some(again);
                }
                break;
            }
            self.send_buffer.drain(n);
        }
        if self.send_buffer.available() > 0 {
            Ok(poll_again)
        } else {
            // Fully drained: shrink a send_buffer that grew for a large
            // frame (e.g. a multi-megabyte keyframe) back down instead of
            // pinning that allocation for the rest of the connection.
            self.send_buffer.reset();
            self.ack_pending = false;
            Ok(None)
        }
    }

    fn send_user_control_message(&mut self, payload: &[u8]) -> Result<()> {
        self.queue_user_control_message(payload)?;
        let data = self.send_buffer.peek().to_vec();
        if let Some(ref mut transport) = self.transport {
            transport.send(&data)?;
        }
        self.send_buffer.reset();
        self.ack_pending = false;
        Ok(())
    }

    fn send_user_control_message_nonblocking(&mut self, payload: &[u8]) -> Result<()> {
        self.queue_user_control_message(payload)?;
        self.try_flush_send_buffer()?;
        Ok(())
    }

    fn handle_user_control(&mut self, payload: &[u8]) -> Result<()> {
        if payload.len() < 6 {
            return Ok(());
        }
        let event_type = ((payload[0] as u16) << 8) | (payload[1] as u16);
        let (event_type, param1, param2) = if event_type == control::UCTRL_SET_BUFFER_LENGTH {
            control::read_user_control(payload, true)?
        } else {
            let (ty, p1, _) = control::read_user_control(payload, false)?;
            (ty, p1, None)
        };
        match event_type {
            control::UCTRL_PING_REQUEST => {
                let now = Instant::now();
                if let Some(start) = self.inbound_ping_window_start {
                    if now.duration_since(start) >= INBOUND_PING_WINDOW {
                        self.inbound_ping_window_start = Some(now);
                        self.inbound_ping_responses = 0;
                    }
                } else {
                    self.inbound_ping_window_start = Some(now);
                }
                if self.inbound_ping_responses >= MAX_INBOUND_PING_RESPONSES {
                    return Err(ErrorCode::Protocol);
                }
                self.inbound_ping_responses += 1;
                let mut buf = Buffer::with_capacity(6);
                control::write_user_control_ping_response(&mut buf, param1)?;
                self.send_user_control_message_nonblocking(buf.as_slice())?;
            }
            control::UCTRL_STREAM_BEGIN | control::UCTRL_STREAM_EOF => {}
            control::UCTRL_SET_BUFFER_LENGTH => {
                let _ = param2;
            }
            _ => {}
        }
        Ok(())
    }

    /// Drain inbound RTMP control messages (pings, chunk-size).
    fn service_inbound(&mut self, timeout_ms: i32) -> Result<()> {
        let Some(t) = self.transport.as_ref() else {
            return Ok(());
        };
        let poll_fd = t.fd();
        let has_buffered_tls_data = t.pending() > 0;
        let mut messages_processed = 0usize;
        self.drain_ready_messages(&mut messages_processed)?;

        if messages_processed == 0
            && !has_buffered_tls_data
            && net::poll_one(poll_fd, Interest::Read, timeout_ms) != Wait::Ready
        {
            return Ok(());
        }

        let mut buf = [0u8; 4096];
        let mut bytes_drained = 0usize;
        loop {
            if bytes_drained >= MAX_RECV_BYTES_PER_POLL {
                break;
            }
            if messages_processed >= MAX_MESSAGES_PER_POLL {
                break;
            }
            let (n, again) = {
                let Some(t) = self.transport.as_mut() else {
                    return Ok(());
                };
                let mut again = 0i32;
                let n = t.recv(&mut buf, &mut again);
                (n, again)
            };
            if n > 0 {
                let chunk_len = n as usize;
                if self.recv_buffer.available().saturating_add(chunk_len) > MAX_RECV_BUFFER_BYTES {
                    return Err(ErrorCode::Protocol);
                }
                self.bytes_received = self.bytes_received.saturating_add(chunk_len as u64);
                self.recv_buffer
                    .write(&buf[..chunk_len])
                    .map_err(|_| ErrorCode::Internal)?;
                bytes_drained += chunk_len;
                self.drain_ready_messages(&mut messages_processed)?;
            } else if n == 0 {
                return Err(ErrorCode::Io);
            } else if again == 0 {
                // A fatal recv error with no retry requested: report it instead
                // of spinning. Same shape as the arm in `poll`.
                return Err(ErrorCode::Io);
            } else {
                break;
            }
        }
        self.maybe_send_window_ack()?;
        Ok(())
    }

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
        self.client_fd = net::INVALID_SOCKET;
        self.recv_buffer.reset();
        self.send_buffer.reset();
        self.chunk_reg.destroy();
        self.chunk_reg.init();
        handshake::client_init(&mut self.handshake);
        self.state = ClientState::Disconnected;
        self.stream_id = 0;
        self.inbound_ping_window_start = None;
        self.inbound_ping_responses = 0;
        self.negotiated_caps = NegotiatedCaps::default();
        self.window_ack_size = 0;
        self.bytes_received = 0;
        self.bytes_at_last_ack = 0;
        self.ack_pending = false;
        self.out_chunk_size = RTMP_DEFAULT_CHUNK_SIZE;
        self.media_out_headers = MediaHeaderTracker::default();
    }

    /// Drive the legacy C0/C1/C2 client handshake to completion over `transport`.
    fn do_handshake(&mut self, transport: &mut Transport, deadline: Instant) -> Result<()> {
        handshake::client_init(&mut self.handshake);
        handshake::client_generate_c0c1(&mut self.handshake)?;
        let c0c1 = self.handshake.out.peek().to_vec();
        send_bounded(transport, &c0c1, deadline)?;
        self.handshake.out.reset();

        let s0s1 = read_exact_bounded(transport, 1 + HANDSHAKE_SIZE, deadline)?;
        let mut buf = Buffer::new();
        buf.write(&s0s1).map_err(|_| ErrorCode::Internal)?;
        handshake::client_read_s0(&mut self.handshake, &mut buf)?;
        handshake::client_read_s1(&mut self.handshake, &mut buf)?;

        let c2 = self.handshake.out.peek().to_vec();
        send_bounded(transport, &c2, deadline)?;
        self.handshake.out.reset();

        let s2 = read_exact_bounded(transport, HANDSHAKE_SIZE, deadline)?;
        let mut buf2 = Buffer::new();
        buf2.write(&s2).map_err(|_| ErrorCode::Internal)?;
        handshake::client_read_s2(&mut self.handshake, &mut buf2)?;

        Ok(())
    }

    fn send_command_msg(
        &mut self,
        msg_stream_id: u32,
        amf_data: &[u8],
        deadline: Option<Instant>,
    ) -> Result<()> {
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 3;
        cmsg.fmt = 0;
        cmsg.msg_length = amf_data.len() as u32;
        cmsg.msg_type_id = 0x14; // AMF0_COMMAND
        cmsg.msg_stream_id = msg_stream_id;
        chunk_write(
            &mut self.send_buffer,
            &cmsg,
            amf_data,
            amf_data.len(),
            self.out_chunk_size,
        )?;

        let data = self.send_buffer.peek().to_vec();
        if let Some(ref mut transport) = self.transport {
            match deadline {
                Some(deadline) => send_bounded(transport, &data, deadline)?,
                None => transport.send(&data)?,
            }
        }
        self.send_buffer.reset();
        self.ack_pending = false;
        Ok(())
    }

    /// Block until an AMF0 command named `want` is received, returning its payload buffer.
    ///
    /// Allocates a fresh [`MAX_RECV_BYTES_PER_COMMAND_WAIT`] budget for this
    /// single call. Callers that loop (waiting for a specific status among
    /// several `onStatus` messages) must use
    /// [`Self::wait_for_command_with_budget`] with one shared budget across
    /// the whole loop instead -- otherwise each retry gets its own fresh
    /// budget and a peer can multiply total inbound bytes processed per
    /// command exchange by sending many non-matching messages.
    fn wait_for_command(&mut self, want: &str, deadline: Option<Instant>) -> Result<Buffer> {
        let mut recv_budget = MAX_RECV_BYTES_PER_COMMAND_WAIT;
        self.wait_for_command_with_budget(want, deadline, &mut recv_budget)
    }

    /// Like [`Self::wait_for_command`], but draws from a caller-supplied,
    /// caller-owned byte budget so repeated calls (e.g. skipping transitional
    /// `onStatus` messages while waiting for a terminal one) share a single
    /// per-exchange cap instead of each getting a fresh one.
    fn wait_for_command_with_budget(
        &mut self,
        want: &str,
        deadline: Option<Instant>,
        recv_budget: &mut usize,
    ) -> Result<Buffer> {
        for _ in 0..64 {
            let (msg, payload) = self.recv_message(recv_budget, deadline)?;
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
    fn recv_message(
        &mut self,
        recv_budget: &mut usize,
        deadline: Option<Instant>,
    ) -> Result<(ChunkMessage, Vec<u8>)> {
        loop {
            // Parse whatever a prior iteration already staged in recv_buffer
            // before consulting the deadline: a reply that arrived in full
            // before the budget ran out must be delivered rather than
            // abandoned in favour of Timeout. Mirrors poll()'s
            // drain-staged-first ordering.
            let mut msg = ChunkMessage::default();
            match chunk_read_owned(&mut self.recv_buffer, &mut self.chunk_reg, &mut msg) {
                Ok((1, payload)) if msg.is_complete => {
                    if self.consume_protocol_control_message(&msg, &payload)? {
                        self.flush_pending_outbound(deadline)?;
                        continue;
                    }
                    return Ok((msg, payload));
                }
                Ok(_) => {}
                Err(_) => return Err(ErrorCode::Chunk),
            }

            if let Some(deadline) = deadline {
                if Instant::now() >= deadline {
                    return Err(ErrorCode::Timeout);
                }
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
                if self.recv_buffer.available().saturating_add(chunk_len) > MAX_RECV_BUFFER_BYTES {
                    return Err(ErrorCode::Protocol);
                }
                *recv_budget -= chunk_len;
                self.bytes_received = self.bytes_received.saturating_add(chunk_len as u64);
                self.recv_buffer
                    .write(&tmp[..chunk_len])
                    .map_err(|_| ErrorCode::Internal)?;
                self.maybe_send_window_ack()?;
                self.flush_pending_outbound(deadline)?;
            } else if n == 0 {
                return Err(ErrorCode::Io);
            } else if again != 0 {
                match deadline {
                    Some(deadline) => poll_until_deadline(t_fd, again, deadline)?,
                    None => poll_for_transport_direction(t_fd, again, RECV_POLL_TIMEOUT_MS)?,
                }
            } else {
                return Err(ErrorCode::Io);
            }
        }
    }

    /// Applies a fully-reassembled protocol-control message the client handles
    /// internally (`SetChunkSize`, `Abort`, `WindowAckSize`, `UserControl`).
    ///
    /// Returns `true` when the message was consumed and the caller must keep
    /// waiting instead of delivering it, `false` for messages that belong to
    /// the caller.
    fn consume_protocol_control_message(
        &mut self,
        msg: &ChunkMessage,
        payload: &[u8],
    ) -> Result<bool> {
        match msg.msg_type_id {
            msg_dispatch::RTMP_MSG_SET_CHUNK_SIZE => {
                let cs = control::read_set_chunk_size(payload)?;
                self.chunk_reg.set_all_chunk_size(cs);
                Ok(true)
            }
            msg_dispatch::RTMP_MSG_ABORT_MESSAGE => {
                // Discard only the partial reassembly on the aborted CSID;
                // the header-compression context is retained for a following
                // fmt=1/2/3 chunk.
                if let Ok(csid) = control::read_abort_message(payload) {
                    self.chunk_reg.abort_stream(csid);
                }
                Ok(true)
            }
            msg_dispatch::RTMP_MSG_WINDOW_ACK_SIZE => {
                if let Ok(win) = control::read_window_ack_size(payload) {
                    self.window_ack_size = win;
                    // Bytes counted before the window was learned may already
                    // have crossed it; re-evaluate now that it is active.
                    self.maybe_send_window_ack()?;
                }
                Ok(true)
            }
            msg_dispatch::RTMP_MSG_USER_CONTROL => {
                self.handle_user_control(payload)?;
                Ok(true)
            }
            _ => Ok(false),
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
fn poll_for_transport_direction(fd: net::RawSocket, again: i32, timeout_ms: i32) -> Result<()> {
    let interest = Interest::from_again(again);
    loop {
        match net::poll_one(fd, interest, timeout_ms) {
            Wait::Ready => return Ok(()),
            Wait::TimedOut => return Err(ErrorCode::Timeout),
            Wait::Interrupted => continue,
            Wait::Failed => return Err(ErrorCode::Io),
        }
    }
}

/// Block until exactly `n` bytes have been read from `transport`, or `deadline`.
fn read_exact_bounded(transport: &mut Transport, n: usize, deadline: Instant) -> Result<Vec<u8>> {
    let mut out = vec![0u8; n];
    let mut got = 0;
    while got < n {
        if Instant::now() >= deadline {
            return Err(ErrorCode::Timeout);
        }
        let mut again = 0i32;
        let r = transport.recv(&mut out[got..], &mut again);
        if r > 0 {
            got += r as usize;
        } else if r == 0 {
            return Err(ErrorCode::Io);
        } else if again != 0 {
            poll_until_deadline(transport.fd(), again, deadline)?;
        } else {
            return Err(ErrorCode::Io);
        }
    }
    Ok(out)
}

/// Send all bytes before `deadline`, using non-blocking I/O with poll retries.
fn send_bounded(transport: &mut Transport, data: &[u8], deadline: Instant) -> Result<()> {
    let mut sent = 0;
    while sent < data.len() {
        if Instant::now() >= deadline {
            return Err(ErrorCode::Timeout);
        }
        let mut again = 0i32;
        let n = transport.try_send(&data[sent..], &mut again)?;
        if n == 0 {
            let direction = if again == 0 { 2 } else { again };
            poll_until_deadline(transport.fd(), direction, deadline)?;
            continue;
        }
        sent += n;
    }
    Ok(())
}

/// Like `poll_for_transport_direction`, but bounded by an absolute `deadline`
/// rather than a fixed timeout. Unlike that function, the `EINTR` retry loop
/// recomputes the remaining time on every iteration — a signal arriving near
/// the deadline must not restart a full poll interval and blow through the
/// caller's wall-clock budget.
fn poll_until_deadline(fd: net::RawSocket, again: i32, deadline: Instant) -> Result<()> {
    let interest = Interest::from_again(again);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        // `poll(2)`'s granularity is milliseconds, so a sub-millisecond
        // remainder can't be represented faithfully; round it down to an
        // expired deadline rather than up to a full 1ms wait, which would
        // let the caller's absolute deadline be overshot.
        if remaining.as_millis() == 0 {
            return Err(ErrorCode::Timeout);
        }
        // `poll(2)`'s timeout is a 32-bit millisecond count, so a remaining
        // budget past ~24.8 days must be clamped; `rc == 0` then only means
        // "this clamped wait expired", not "the real deadline passed" — loop
        // and recheck instead of timing out early.
        let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
        match net::poll_one(fd, interest, timeout_ms) {
            Wait::Ready => return Ok(()),
            Wait::TimedOut => {
                if Instant::now() >= deadline {
                    return Err(ErrorCode::Timeout);
                }
                continue;
            }
            Wait::Interrupted => continue,
            Wait::Failed => return Err(ErrorCode::Io),
        }
    }
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
        if self.transport.is_none() && self.client_fd != net::INVALID_SOCKET {
            net::close(self.client_fd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dns_job(host: &str) -> DnsJob {
        let (reply, _rx) = mpsc::channel();
        DnsJob {
            host: host.to_string(),
            port: 0,
            reply,
        }
    }

    #[test]
    fn dns_queue_enqueue_wakes_promptly_on_notify_instead_of_polling_to_deadline() {
        let queue = Arc::new(DnsQueue::new());
        for i in 0..MAX_DNS_QUEUE_DEPTH {
            queue
                .enqueue(
                    dns_job(&format!("filler{i}")),
                    Instant::now() + Duration::from_secs(5),
                )
                .unwrap();
        }

        let freer = queue.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            freer.jobs.lock().unwrap().pop_front();
            freer.not_full.notify_one();
        });

        let start = Instant::now();
        let result = queue.enqueue(dns_job("waiter"), start + Duration::from_secs(5));
        assert!(result.is_ok());
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "enqueue should be woken by the freed-slot notification almost immediately, \
             not only discover it near the 5s deadline (took {:?})",
            start.elapsed()
        );
    }

    #[test]
    fn dns_queue_enqueue_times_out_at_deadline_when_no_slot_frees() {
        let queue = DnsQueue::new();
        for i in 0..MAX_DNS_QUEUE_DEPTH {
            queue
                .enqueue(
                    dns_job(&format!("filler{i}")),
                    Instant::now() + Duration::from_secs(5),
                )
                .unwrap();
        }

        let start = Instant::now();
        let deadline = start + Duration::from_millis(100);
        let result = queue.enqueue(dns_job("waiter"), deadline);
        assert!(matches!(result, Err(ErrorCode::Timeout)));
        assert!(start.elapsed() >= Duration::from_millis(90));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn dns_queue_enqueue_rejects_already_expired_deadline_even_with_room() {
        // An already-expired deadline must not admit the job, even though the
        // queue has plenty of free capacity: doing so would waste a worker
        // resolution and a queue slot on a caller that has already given up.
        let queue = DnsQueue::new();
        let expired = Instant::now() - Duration::from_millis(1);
        let result = queue.enqueue(dns_job("waiter"), expired);
        assert!(matches!(result, Err(ErrorCode::Timeout)));
        assert!(queue.jobs.lock().unwrap().is_empty());
    }

    #[test]
    fn dns_queue_dequeue_one_wakes_the_sole_waiter_via_the_real_production_path() {
        // Exercises the real production pop-and-notify path (not a
        // hand-rolled notify in the test): if `run()`/`dequeue_one()`
        // regresses to not notifying at all, this test catches it, without
        // paying for (or depending on the timing of) a real DNS lookup.
        let queue = Arc::new(DnsQueue::new());
        for i in 0..MAX_DNS_QUEUE_DEPTH {
            queue
                .enqueue(
                    dns_job(&format!("filler{i}")),
                    Instant::now() + Duration::from_secs(5),
                )
                .unwrap();
        }

        let waiter_queue = queue.clone();
        let waiter = std::thread::spawn(move || {
            let start = Instant::now();
            let result = waiter_queue.enqueue(dns_job("waiter"), start + Duration::from_secs(5));
            (result, start.elapsed())
        });
        std::thread::sleep(Duration::from_millis(50));

        let job = queue.dequeue_one();
        assert_eq!(job.host, "filler0");

        let (result, elapsed) = waiter.join().unwrap();
        assert!(
            result.is_ok(),
            "the waiter must be woken by dequeue_one()'s real notify"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "admission should happen promptly via the notification, not be \
             stranded until the waiter's own 5s deadline (took {elapsed:?})"
        );
    }

    #[test]
    fn dns_queue_enqueue_relays_freed_slot_when_bailing_on_an_already_expired_deadline() {
        // Regression: notify_one() wakes exactly one waiter per freed slot.
        // If that waiter's own deadline has already elapsed, it must relay
        // the notification onward (since a slot really is free) instead of
        // silently dropping it — otherwise a different waiter with plenty of
        // time left can be stranded asleep despite the free slot.
        //
        // This can't be pinned down by racing two real deadlines against
        // wall-clock sleeps: Condvar::wait_timeout self-wakes independently
        // once *its own* deadline elapses, so a short-deadline waiter
        // reliably times out on its own long before an unrelated later
        // dequeue event, never actually contending for that dequeue's
        // notify_one() at all (confirmed by trying exactly that approach:
        // it kept passing even with the relay deliberately reverted).
        // Instead, drive the exact "woke up and immediately bailed" moment
        // directly and deterministically: free a slot without notifying
        // anyone (simulating the instant right after a dequeue, before any
        // wakeup is delivered), then call `enqueue()` with an already-past
        // deadline — the same state a woken-but-expired waiter observes.
        let queue = Arc::new(DnsQueue::new());
        for i in 0..MAX_DNS_QUEUE_DEPTH {
            queue
                .enqueue(
                    dns_job(&format!("filler{i}")),
                    Instant::now() + Duration::from_secs(5),
                )
                .unwrap();
        }

        let patient_queue = queue.clone();
        let patient = std::thread::spawn(move || {
            let start = Instant::now();
            let result =
                patient_queue.enqueue(dns_job("patient"), start + Duration::from_millis(500));
            (result, start.elapsed())
        });
        // Let patient actually park on `not_full` (queue is still full here).
        std::thread::sleep(Duration::from_millis(50));

        queue.jobs.lock().unwrap().pop_front();
        let already_expired = Instant::now() - Duration::from_millis(1);
        let expiring_result = queue.enqueue(dns_job("expiring"), already_expired);
        assert!(matches!(expiring_result, Err(ErrorCode::Timeout)));

        let (result, elapsed) = patient.join().unwrap();
        assert!(
            result.is_ok(),
            "the patient waiter must be admitted via the relay, even though the \
             other caller's deadline had already expired when the slot freed"
        );
        assert!(
            elapsed < Duration::from_millis(400),
            "admission should happen promptly via the relay, not be stranded \
             until the patient waiter's own 500ms deadline (took {elapsed:?})"
        );
    }

    fn rtmp_user_control_ping_chunk(token: u32) -> Vec<u8> {
        let mut payload = Buffer::with_capacity(6);
        control::write_user_control_ping_request(&mut payload, token).unwrap();
        let payload_len = payload.available();
        let mut wire = Buffer::new();
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 2;
        cmsg.fmt = 0;
        cmsg.msg_length = payload_len as u32;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_USER_CONTROL;
        cmsg.msg_stream_id = 0;
        chunk_write(&mut wire, &cmsg, payload.as_slice(), payload_len, 128).unwrap();
        wire.peek().to_vec()
    }

    #[test]
    fn recv_budget_is_at_least_one_socket_read() {
        assert!(MAX_RECV_BYTES_PER_POLL >= 65536);
    }

    #[test]
    fn recv_buffer_staging_cap_covers_two_max_messages_at_min_chunk_size() {
        // recv_buffer holds raw wire bytes, so the cap must have headroom for
        // chunk-header overhead on top of two max-size message payloads, even
        // at the smallest chunk size a peer can realistically negotiate.
        let payload = 2 * DEFAULT_MAX_MSG_LENGTH as usize;
        let chunks = payload.div_ceil(MIN_PRACTICAL_CHUNK_SIZE);
        // Each message's first chunk (fmt=0) carries the larger message
        // header on top of the shared continuation-chunk overhead.
        let worst_case_wire_bytes = payload
            + chunks * MAX_CHUNK_HEADER_OVERHEAD_BYTES
            + 2 * FIRST_CHUNK_EXTRA_OVERHEAD_BYTES;
        assert!(MAX_RECV_BUFFER_BYTES >= worst_case_wire_bytes);
    }

    #[test]
    fn poll_rejects_recv_buffer_growth_past_staging_cap() {
        use crate::net::testing::{PairStream, stream_pair};
        use std::io::Write;

        let (client_end, mut peer) = stream_pair().unwrap();
        client_end.set_nonblocking(true).unwrap();
        peer.set_nonblocking(true).unwrap();
        peer.write_all(&[0x01, 0x02, 0x03]).unwrap();

        let mut client = Client::new();
        client.state = ClientState::Playing;
        client.transport = Some(crate::net::testing::transport_from(client_end));
        // More than one poll() worth of drain budget (MAX_MESSAGES_PER_POLL
        // trivial 13-byte complete messages = 3328 bytes) can clear, so the
        // cap must still reject growth once the budgeted drain isn't enough.
        client
            .recv_buffer
            .write(&vec![0u8; MAX_RECV_BUFFER_BYTES * 2])
            .unwrap();

        assert_eq!(client.poll(0), Err(ErrorCode::Protocol));
    }

    #[test]
    fn try_flush_send_buffer_shrinks_after_full_drain() {
        use crate::buffer::BUFFER_RESET_CAPACITY;
        use crate::net::testing::stream_pair;

        let (client_end, _peer) = stream_pair().unwrap();

        // The default unix-domain socket buffer is much smaller on macOS
        // (~8 KiB) than on Linux, so a write past it would only partially
        // drain in one non-blocking call. Grow both ends past the test
        // payload explicitly so the "fully drains in one write" assumption
        // below holds on every platform this runs on.
        let big_len = BUFFER_RESET_CAPACITY * 4;
        let wanted_buf_size = big_len + BUFFER_RESET_CAPACITY;
        for end in [&client_end, &_peer] {
            let sock = socket2::SockRef::from(end);
            sock.set_send_buffer_size(wanted_buf_size)
                .expect("set SO_SNDBUF");
            sock.set_recv_buffer_size(wanted_buf_size)
                .expect("set SO_RCVBUF");
        }

        // The kernel may clamp the requested size below what was asked for
        // (e.g. a sandboxed CI runner with a low net.core.wmem_max /
        // kern.ipc.maxsockbuf ceiling), so confirm the *effective* buffer
        // rather than trusting the setsockopt call above blindly - otherwise
        // this test would fail with the same cryptic assertion it exists to
        // avoid, just on a different platform.
        let effective_sndbuf = socket2::SockRef::from(&client_end)
            .send_buffer_size()
            .expect("get SO_SNDBUF");
        assert!(
            effective_sndbuf >= wanted_buf_size,
            "environment's effective SO_SNDBUF ({effective_sndbuf}) is below what this test needs ({wanted_buf_size}); the full-drain-in-one-write assumption below won't hold here"
        );

        client_end.set_nonblocking(true).unwrap();

        let mut client = Client::new();
        client.transport = Some(crate::net::testing::transport_from(client_end));

        // Simulate a large keyframe having grown send_buffer well past its
        // reset capacity. This is well within the unix socket's send buffer,
        // so try_flush_send_buffer can fully drain it in one non-blocking
        // write without the peer needing to read concurrently.
        let big = vec![0u8; big_len];
        client.send_buffer.write(&big).unwrap();
        assert!(client.send_buffer.capacity() > BUFFER_RESET_CAPACITY);

        client.try_flush_send_buffer().unwrap();

        assert_eq!(client.send_buffer.available(), 0);
        assert!(
            client.send_buffer.capacity() <= BUFFER_RESET_CAPACITY,
            "send_buffer should shrink back to {BUFFER_RESET_CAPACITY} after a full flush, got {}",
            client.send_buffer.capacity()
        );
    }

    #[test]
    fn frame_cb_scratch_retains_payload_after_delivery() {
        let mut client = Client::new();

        let video_payload = [0x17u8, 0x01, 0x02, 0x03];
        let mut wire = Buffer::new();
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 6;
        cmsg.fmt = 0;
        cmsg.msg_length = video_payload.len() as u32;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_VIDEO;
        cmsg.msg_stream_id = 1;
        chunk_write(&mut wire, &cmsg, &video_payload, video_payload.len(), 128).unwrap();
        client.recv_buffer.write(wire.peek()).unwrap();

        client.on_frame_cb = Some(|_| {});
        let mut messages_processed = 0;
        client
            .drain_ready_messages(&mut messages_processed)
            .unwrap();

        // Frame.data must still be valid (i.e. frame_cb_scratch must still
        // hold the delivered payload) after the callback has returned, not
        // just for the duration of the call itself -- matching the
        // server-side Conn::frame_cb_scratch contract.
        assert_eq!(client.frame_cb_scratch.as_slice(), &video_payload[..]);
    }

    #[test]
    fn amf3_data_single_marker_byte_is_stripped() {
        use std::sync::{LazyLock, Mutex};

        static SIZES: LazyLock<Mutex<Vec<u32>>> = LazyLock::new(|| Mutex::new(Vec::new()));

        let mut client = Client::new();
        SIZES.lock().unwrap().clear();

        let payload = [0x00u8];
        let mut wire = Buffer::new();
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 6;
        cmsg.fmt = 0;
        cmsg.msg_length = payload.len() as u32;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_AMF3_DATA;
        cmsg.msg_stream_id = 1;
        chunk_write(&mut wire, &cmsg, &payload, payload.len(), 128).unwrap();
        client.recv_buffer.write(wire.peek()).unwrap();

        client.on_frame_cb = Some(|frame| SIZES.lock().unwrap().push(frame.size));
        let mut messages_processed = 0;
        client
            .drain_ready_messages(&mut messages_processed)
            .unwrap();

        // The AMF0-in-AMF3 marker must be stripped even when it is the whole
        // body, matching the AMF3_COMMAND arm.
        assert_eq!(*SIZES.lock().unwrap(), vec![0]);
    }

    #[test]
    fn script_callbacks_only_mark_on_metadata_events() {
        use std::sync::{LazyLock, Mutex};

        static FLAGS: LazyLock<Mutex<Vec<u8>>> = LazyLock::new(|| Mutex::new(Vec::new()));

        let mut client = Client::new();
        FLAGS.lock().unwrap().clear();

        let mut cue_point = Buffer::new();
        crate::amf::amf0::write_string(&mut cue_point, "onCuePoint").unwrap();
        client.deliver_script_frame_cb(
            |frame| FLAGS.lock().unwrap().push(frame.is_metadata),
            10,
            cue_point.as_slice(),
        );

        let mut metadata = Buffer::new();
        crate::amf::amf0::write_string(&mut metadata, "@setDataFrame").unwrap();
        crate::amf::amf0::write_string(&mut metadata, "onMetaData").unwrap();
        client.deliver_script_frame_cb(
            |frame| FLAGS.lock().unwrap().push(frame.is_metadata),
            20,
            metadata.as_slice(),
        );

        assert_eq!(*FLAGS.lock().unwrap(), vec![0, 1]);
    }

    #[test]
    fn aggregate_subtags_stop_at_max_messages_per_poll() {
        use std::sync::{LazyLock, Mutex};

        static CALLBACKS: LazyLock<Mutex<usize>> = LazyLock::new(|| Mutex::new(0));

        let audio_payload = vec![0xAF, 0x01];
        let mut aggregate = Vec::new();
        for i in 0..(MAX_MESSAGES_PER_POLL + 8) {
            aggregate.push(0x08);
            let len = audio_payload.len() as u32;
            aggregate.extend_from_slice(&[(len >> 16) as u8, (len >> 8) as u8, len as u8]);
            aggregate.extend_from_slice(&[
                (i >> 16) as u8,
                (i >> 8) as u8,
                i as u8,
                (i >> 24) as u8,
            ]);
            aggregate.extend_from_slice(&[0, 0, 0]);
            aggregate.extend_from_slice(&audio_payload);
            let prev_tag_size = (11 + audio_payload.len()) as u32;
            aggregate.extend_from_slice(&prev_tag_size.to_be_bytes());
        }

        let mut wire = Buffer::new();
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 6;
        cmsg.fmt = 0;
        cmsg.msg_length = aggregate.len() as u32;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_AGGREGATE;
        cmsg.msg_stream_id = 1;
        chunk_write(&mut wire, &cmsg, &aggregate, aggregate.len(), 128).unwrap();

        let mut client = Client::new();
        client.recv_buffer.write(wire.peek()).unwrap();
        *CALLBACKS.lock().unwrap() = 0;
        client.on_frame_cb = Some(|_| {
            *CALLBACKS.lock().unwrap() += 1;
        });

        let mut messages_processed = 0;
        client
            .drain_ready_messages(&mut messages_processed)
            .unwrap();

        assert_eq!(messages_processed, MAX_MESSAGES_PER_POLL);
        assert_eq!(*CALLBACKS.lock().unwrap(), MAX_MESSAGES_PER_POLL);
    }

    #[test]
    fn aggregate_unknown_subtags_consume_message_budget() {
        let filler = [0x00];
        let mut aggregate = Vec::new();
        for i in 0..(MAX_MESSAGES_PER_POLL + 8) {
            aggregate.push(0x01);
            aggregate.push(0x00);
            aggregate.push(0x00);
            aggregate.push(0x01);
            aggregate.extend_from_slice(&[
                (i >> 16) as u8,
                (i >> 8) as u8,
                i as u8,
                (i >> 24) as u8,
            ]);
            aggregate.extend_from_slice(&[0, 0, 0]);
            aggregate.push(filler[0]);
            let prev_tag_size = 12u32;
            aggregate.extend_from_slice(&prev_tag_size.to_be_bytes());
        }

        let mut wire = Buffer::new();
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 6;
        cmsg.fmt = 0;
        cmsg.msg_length = aggregate.len() as u32;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_AGGREGATE;
        cmsg.msg_stream_id = 1;
        chunk_write(&mut wire, &cmsg, &aggregate, aggregate.len(), 128).unwrap();

        let mut client = Client::new();
        client.recv_buffer.write(wire.peek()).unwrap();
        let mut messages_processed = 0;
        client
            .drain_ready_messages(&mut messages_processed)
            .unwrap();

        assert_eq!(messages_processed, MAX_MESSAGES_PER_POLL);
    }

    #[test]
    fn drain_ready_messages_splits_multitrack_video() {
        use std::sync::{LazyLock, Mutex};

        static SEEN: LazyLock<Mutex<Vec<(u8, Vec<u8>)>>> = LazyLock::new(|| Mutex::new(Vec::new()));

        let payload = vec![
            0x86, 0x10, b'a', b'v', b'c', b'1', 0x00, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC, 0x01,
            0x00, 0x00, 0x02, 0xDD, 0xEE,
        ];
        let mut wire = Buffer::new();
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 6;
        cmsg.fmt = 0;
        cmsg.msg_length = payload.len() as u32;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_VIDEO;
        cmsg.msg_stream_id = 1;
        chunk_write(&mut wire, &cmsg, &payload, payload.len(), 128).unwrap();

        let mut client = Client::new();
        client.recv_buffer.write(wire.peek()).unwrap();
        SEEN.lock().unwrap().clear();
        client.on_frame_cb = Some(|frame| {
            let data =
                unsafe { std::slice::from_raw_parts(frame.data, frame.size as usize).to_vec() };
            SEEN.lock().unwrap().push((frame.track_id, data));
        });

        let mut messages_processed = 0;
        client
            .drain_ready_messages(&mut messages_processed)
            .unwrap();

        let seen = SEEN.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0, 0);
        assert_eq!(seen[0].1, vec![0xAA, 0xBB, 0xCC]);
        assert_eq!(seen[1].0, 1);
        assert_eq!(seen[1].1, vec![0xDD, 0xEE]);
    }

    #[test]
    fn drain_ready_messages_rejects_oversized_multitrack_video() {
        let mut payload = vec![0x86, 0x10, b'a', b'v', b'c', b'1'];
        for id in 0..=crate::ertmp::multitrack_media::MAX_MULTITRACK_SUBTRACKS {
            payload.push(id as u8);
            payload.extend_from_slice(&[0x00, 0x00, 0x00]);
        }

        let mut wire = Buffer::new();
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 6;
        cmsg.fmt = 0;
        cmsg.msg_length = payload.len() as u32;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_VIDEO;
        cmsg.msg_stream_id = 1;
        let chunk_size = payload.len();
        chunk_write(&mut wire, &cmsg, &payload, payload.len(), chunk_size).unwrap();

        let mut client = Client::new();
        client.chunk_reg.set_all_chunk_size(chunk_size as u32);
        client.recv_buffer.write(wire.peek()).unwrap();
        client.on_frame_cb = Some(|_| panic!("invalid multitrack must not reach callback"));

        let mut messages_processed = 0;
        assert_eq!(
            client.drain_ready_messages(&mut messages_processed),
            Err(ErrorCode::Protocol)
        );
        assert!(client.frame_cb_scratch.is_empty());
    }

    #[test]
    fn poll_drains_leftover_messages_before_enforcing_staging_cap() {
        use crate::net::testing::{PairStream, stream_pair};
        use std::io::Write;

        let (client_end, mut peer) = stream_pair().unwrap();
        client_end.set_nonblocking(true).unwrap();
        peer.set_nonblocking(true).unwrap();
        peer.write_all(&[0x01, 0x02, 0x03]).unwrap();

        let mut client = Client::new();
        client.state = ClientState::Playing;
        client.transport = Some(crate::net::testing::transport_from(client_end));
        // Simulate a prior poll() that stopped at MAX_MESSAGES_PER_POLL:
        // recv_buffer is staged right up to the cap with trivial 13-byte
        // complete messages (2-byte extended-csid basic header + 11-byte
        // zeroed fmt=0 message header, msg_length=0). Draining the first
        // MAX_MESSAGES_PER_POLL of them frees enough room that the 3 new
        // bytes read below should NOT be rejected -- rejecting them would
        // mean the cap was checked before leftover messages were drained.
        let msg_count = MAX_RECV_BUFFER_BYTES / 13;
        client
            .recv_buffer
            .write(&vec![0u8; msg_count * 13])
            .unwrap();

        assert_eq!(client.poll(0), Ok(()));
    }

    #[test]
    fn poll_does_not_block_on_socket_readiness_when_messages_already_staged() {
        use crate::net::testing::{PairStream, stream_pair};
        use std::io::Write;
        use std::time::Instant;

        let (client_end, peer) = stream_pair().unwrap();
        client_end.set_nonblocking(true).unwrap();
        // Keep `peer` alive but never send anything further, so the socket
        // never becomes readable -- if poll() waited on readiness before
        // draining, this call would block for the full timeout below.
        let _peer = peer;

        let mut client = Client::new();
        client.state = ClientState::Playing;
        client.transport = Some(crate::net::testing::transport_from(client_end));
        // One complete trivial message already staged: 2-byte extended-csid
        // basic header + 11-byte zeroed fmt=0 message header (msg_length=0).
        client.recv_buffer.write(&[0u8; 13]).unwrap();

        let start = Instant::now();
        assert_eq!(client.poll(5_000), Ok(()));
        assert!(
            start.elapsed() < Duration::from_millis(1_000),
            "poll() blocked on socket readiness instead of draining the staged message first"
        );
    }

    #[test]
    fn command_wait_recv_budget_bounds_connect_handshake_amplification() {
        // 64 max-size AMF commands would be 256 MiB without a byte cap.
        assert!(MAX_RECV_BYTES_PER_COMMAND_WAIT < 64 * 4 * 1024 * 1024);
        assert!(MAX_RECV_BYTES_PER_COMMAND_WAIT >= 65536);
    }

    #[test]
    fn publish_and_play_honor_command_io_deadline() {
        use crate::net::testing::{PairStream, stream_pair};
        use std::time::Duration;

        let (client_end, _peer) = stream_pair().unwrap();
        client_end.set_nonblocking(true).unwrap();

        let mut client = Client::new();
        client.set_connect_timeout(Duration::from_millis(200));
        client.state = ClientState::AppConnected;
        client.stream_id = 1;
        client.stream_key = "stream".to_string();
        client.transport = Some(crate::net::testing::transport_from(client_end));

        let started = Instant::now();
        assert_eq!(client.publish().unwrap_err(), ErrorCode::Timeout);
        let publish_elapsed = started.elapsed();
        assert!(
            publish_elapsed < Duration::from_secs(2),
            "publish should time out near the configured deadline, took {:?}",
            publish_elapsed
        );

        client.state = ClientState::AppConnected;
        let started = Instant::now();
        assert_eq!(client.play().unwrap_err(), ErrorCode::Timeout);
        let play_elapsed = started.elapsed();
        assert!(
            play_elapsed < Duration::from_secs(2),
            "play should time out near the configured deadline, took {:?}",
            play_elapsed
        );
    }

    #[test]
    fn play_succeeds_after_transitional_status_before_start() {
        // A real RTMP server commonly sends a transitional onStatus (e.g.
        // `NetStream.Play.Reset`) before the terminal `NetStream.Play.Start`.
        // That must not be treated as a failure -- play() should keep
        // waiting for the expected code instead of aborting.
        use crate::net::testing::{PairStream, stream_pair};
        use std::io::Write;

        fn onstatus_chunk(level: &str, code: &str) -> Vec<u8> {
            let mut payload = Buffer::new();
            command::build_onstatus(&mut payload, level, code, "").unwrap();
            let payload_len = payload.available();
            let mut wire = Buffer::new();
            let mut cmsg = ChunkMessage::default();
            cmsg.csid = 3;
            cmsg.fmt = 0;
            cmsg.msg_length = payload_len as u32;
            cmsg.msg_type_id = msg_dispatch::RTMP_MSG_AMF0_COMMAND;
            cmsg.msg_stream_id = 1;
            chunk_write(&mut wire, &cmsg, payload.as_slice(), payload_len, 128).unwrap();
            wire.peek().to_vec()
        }

        let (client_end, mut peer) = stream_pair().unwrap();
        client_end.set_nonblocking(true).unwrap();

        peer.write_all(&onstatus_chunk("status", "NetStream.Play.Reset"))
            .unwrap();
        peer.write_all(&onstatus_chunk("status", "NetStream.Play.Start"))
            .unwrap();

        let mut client = Client::new();
        client.state = ClientState::AppConnected;
        client.stream_id = 1;
        client.stream_key = "stream".to_string();
        client.transport = Some(crate::net::testing::transport_from(client_end));

        client.play().unwrap();
        assert_eq!(client.state, ClientState::Playing);
    }

    #[test]
    fn play_fails_fast_when_transitional_status_flood_exceeds_recv_budget() {
        // A peer that keeps sending non-terminal `status`-level onStatus
        // messages (never the terminal NetStream.Play.Start) must not let
        // play()'s retry loop hand out a fresh MAX_RECV_BYTES_PER_COMMAND_WAIT
        // budget on every iteration -- that would let it process unbounded
        // inbound bytes for a single play() call. The shared budget should
        // exhaust and fail well before the command I/O deadline.
        use crate::net::testing::{PairStream, stream_pair};
        use std::io::Write;
        use std::time::Duration;

        fn onstatus_chunk(level: &str, code: &str) -> Vec<u8> {
            let mut payload = Buffer::new();
            command::build_onstatus(&mut payload, level, code, "").unwrap();
            let payload_len = payload.available();
            let mut wire = Buffer::new();
            let mut cmsg = ChunkMessage::default();
            cmsg.csid = 3;
            cmsg.fmt = 0;
            cmsg.msg_length = payload_len as u32;
            cmsg.msg_type_id = msg_dispatch::RTMP_MSG_AMF0_COMMAND;
            cmsg.msg_stream_id = 1;
            chunk_write(&mut wire, &cmsg, payload.as_slice(), payload_len, 128).unwrap();
            wire.peek().to_vec()
        }

        let (client_end, mut peer) = stream_pair().unwrap();
        client_end.set_nonblocking(true).unwrap();
        // Bound the writer's blocking write_all() calls: once play() stops
        // draining (budget exhausted), nothing reads the socket anymore, so
        // an unbounded write_all() could block the writer thread -- and this
        // test's join() -- forever.
        peer.set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        let reset_chunk = onstatus_chunk("status", "NetStream.Play.Reset");
        let flood_bytes = MAX_RECV_BYTES_PER_COMMAND_WAIT + reset_chunk.len() * 4;
        let writer = std::thread::spawn(move || {
            let mut sent = 0usize;
            while sent < flood_bytes {
                if peer.write_all(&reset_chunk).is_err() {
                    break;
                }
                sent += reset_chunk.len();
            }
            // Hand the socket back instead of dropping it here: closing it
            // early would let a slow play() see EOF (Io) before exhausting
            // its budget (Timeout), which is not what this test is about.
            peer
        });

        let mut client = Client::new();
        client.state = ClientState::AppConnected;
        client.stream_id = 1;
        client.stream_key = "stream".to_string();
        client.transport = Some(crate::net::testing::transport_from(client_end));

        let started = Instant::now();
        let err = client.play().unwrap_err();
        assert_eq!(err, ErrorCode::Timeout);
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "should fail once the shared recv budget is exhausted, not wait for the \
             command I/O deadline, took {:?}",
            elapsed
        );

        let _ = writer.join();
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
    fn inbound_ping_rate_limit_rejects_flood() {
        use crate::net::testing::{PairStream, stream_pair};

        let (client_end, _peer) = stream_pair().unwrap();
        client_end.set_nonblocking(true).unwrap();

        let mut client = Client::new();
        client.state = ClientState::AppConnected;
        client.transport = Some(crate::net::testing::transport_from(client_end));

        let mut ping = Buffer::with_capacity(6);
        for i in 0..MAX_INBOUND_PING_RESPONSES {
            ping.reset();
            control::write_user_control_ping_request(&mut ping, i as u32).unwrap();
            client.handle_user_control(ping.as_slice()).unwrap();
        }
        ping.reset();
        control::write_user_control_ping_request(&mut ping, 99).unwrap();
        assert_eq!(
            client.handle_user_control(ping.as_slice()).unwrap_err(),
            ErrorCode::Protocol
        );
    }

    #[test]
    fn inbound_ping_requests_are_answered() {
        use crate::net::testing::{PairStream, stream_pair};
        use std::io::Read;

        let (client_end, mut peer) = stream_pair().unwrap();
        client_end.set_nonblocking(true).unwrap();
        peer.set_nonblocking(true).unwrap();

        let mut client = Client::new();
        client.state = ClientState::AppConnected;
        client.transport = Some(crate::net::testing::transport_from(client_end));

        let mut ping = Buffer::with_capacity(6);
        control::write_user_control_ping_request(&mut ping, 99).unwrap();
        client.handle_user_control(ping.as_slice()).unwrap();

        let mut out = [0u8; 256];
        let n = peer.read(&mut out).unwrap();
        assert!(n > 0);
        let ping_response = control::UCTRL_PING_RESPONSE.to_be_bytes();
        assert!(
            out[..n].windows(2).any(|w| w == ping_response),
            "peer should receive a UserControl ping response"
        );
    }

    #[test]
    fn window_ack_size_message_stores_peer_window() {
        let mut payload = Buffer::new();
        control::write_window_ack_size(&mut payload, 2_500_000).unwrap();
        let mut wire = Buffer::new();
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 2;
        cmsg.fmt = 0;
        cmsg.msg_length = 4;
        cmsg.msg_type_id = msg_dispatch::RTMP_MSG_WINDOW_ACK_SIZE;
        cmsg.msg_stream_id = 0;
        chunk_write(&mut wire, &cmsg, payload.as_slice(), 4, 128).unwrap();

        let mut client = Client::new();
        client.recv_buffer.write(wire.peek()).unwrap();
        let mut messages_processed = 0;
        client
            .drain_ready_messages(&mut messages_processed)
            .unwrap();
        assert_eq!(client.window_ack_size, 2_500_000);
    }

    #[test]
    fn abort_message_discards_partial_reassembly_before_fmt3_chunk() {
        let mut client = Client::new();

        // fmt=0 chunk for a 200-byte message on csid 3, but only the first
        // 128-byte chunk arrives: csid 3 is left reassembling.
        let mut wire = Buffer::new();
        wire.write(&[0x03, 0, 0, 0, 0, 0, 0xC8, 0x08, 1, 0, 0, 0])
            .unwrap();
        wire.write(&[0xAA; 128]).unwrap();

        // AbortMessage (0x02) for csid 3 on the control chunk stream (csid 2).
        let mut abort_payload = Buffer::new();
        abort_payload.write(&3u32.to_be_bytes()).unwrap();
        let mut abort_msg = ChunkMessage::default();
        abort_msg.csid = 2;
        abort_msg.fmt = 0;
        abort_msg.msg_length = 4;
        abort_msg.msg_type_id = msg_dispatch::RTMP_MSG_ABORT_MESSAGE;
        abort_msg.msg_stream_id = 0;
        let mut abort_wire = Buffer::new();
        chunk_write(
            &mut abort_wire,
            &abort_msg,
            abort_payload.as_slice(),
            4,
            128,
        )
        .unwrap();
        wire.write(abort_wire.peek()).unwrap();

        client.recv_buffer.write(wire.peek()).unwrap();
        let mut messages_processed = 0;
        client
            .drain_ready_messages(&mut messages_processed)
            .unwrap();

        let stream = client.chunk_reg.get(3).expect("csid 3 stays registered");
        assert_eq!(stream.reassembly_bytes_read, 0);
        assert_eq!(stream.type0_msg_length, 200);
        assert!(!stream.reassembling);

        // A compressed fmt=3 chunk immediately after the abort starts a new
        // message that reuses the retained header context (length, type,
        // stream id) instead of being appended to the discarded one.
        let mut fmt3 = Buffer::new();
        fmt3.write(&[0xC3]).unwrap();
        fmt3.write(&[0xBB; 128]).unwrap();
        let mut out = ChunkMessage::default();
        assert_eq!(
            chunk_read_owned(&mut fmt3, &mut client.chunk_reg, &mut out)
                .unwrap()
                .0,
            0,
            "the first fmt=3 chunk starts the inherited 200-byte message"
        );
        let mut fmt3_tail = Buffer::new();
        fmt3_tail.write(&[0xC3]).unwrap();
        fmt3_tail.write(&[0xBB; 72]).unwrap();
        assert_eq!(
            chunk_read_owned(&mut fmt3_tail, &mut client.chunk_reg, &mut out)
                .unwrap()
                .0,
            1
        );
        assert_eq!(out.msg_length, 200);
        assert_eq!(out.msg_type_id, 0x08);
        assert_eq!(out.msg_stream_id, 1);

        // The CSID stays usable: a fresh fmt=0 message re-establishes the
        // header, and a following fmt=3 new message decodes with the
        // inherited header and timestamp.
        let payload = b"hello";
        let mut reestablished = ChunkMessage {
            csid: 3,
            fmt: 0,
            timestamp: 100,
            msg_length: payload.len() as u32,
            msg_type_id: 0x08,
            msg_stream_id: 1,
            is_complete: false,
        };
        let mut first = Buffer::new();
        chunk_write(&mut first, &reestablished, payload, payload.len(), 128).unwrap();
        assert_eq!(
            chunk_read_owned(&mut first, &mut client.chunk_reg, &mut reestablished)
                .unwrap()
                .0,
            1
        );
        let mut next = Buffer::new();
        next.write(&[0xC3]).unwrap();
        next.write(b"again").unwrap();
        assert_eq!(
            chunk_read_owned(&mut next, &mut client.chunk_reg, &mut reestablished)
                .unwrap()
                .0,
            1
        );
        assert_eq!(reestablished.timestamp, 200);
    }

    #[test]
    fn abort_message_resets_stream_in_blocking_consume_path() {
        let mut client = Client::new();
        {
            let cs = client.chunk_reg.get_or_create(3).unwrap();
            cs.type0_msg_length = 200;
            cs.reassembly_bytes_read = 128;
            cs.reassembling = true;
        }

        let mut msg = ChunkMessage::default();
        msg.msg_type_id = msg_dispatch::RTMP_MSG_ABORT_MESSAGE;
        let payload = 3u32.to_be_bytes();
        assert!(
            client
                .consume_protocol_control_message(&msg, &payload)
                .unwrap(),
            "an AbortMessage must be consumed by the blocking path"
        );

        let cs = client.chunk_reg.get(3).expect("csid 3 stays registered");
        // The header-compression context survives; only the partial
        // reassembly is discarded.
        assert_eq!(cs.type0_msg_length, 200);
        assert_eq!(cs.reassembly_bytes_read, 0);
        assert!(!cs.reassembling);
    }

    #[test]
    fn bytes_received_ack_uses_u64_after_u32_wrap() {
        let mut client = Client::new();
        client.window_ack_size = 1024;
        client.bytes_received = u32::MAX as u64;
        client.bytes_at_last_ack = u32::MAX as u64;
        client.bytes_received = client.bytes_received.saturating_add(2048);
        client.maybe_send_window_ack().unwrap();
        assert_eq!(client.bytes_received, u32::MAX as u64 + 2048);
        assert_eq!(client.bytes_at_last_ack, client.bytes_received);
        assert!(
            client.send_buffer.available() > 0,
            "an acknowledgement must be queued once the window is exceeded"
        );
    }

    #[test]
    fn learning_window_ack_size_rechecks_accumulated_bytes() {
        let mut client = Client::new();
        client.bytes_received = 5000;

        let mut msg = ChunkMessage::default();
        msg.msg_type_id = msg_dispatch::RTMP_MSG_WINDOW_ACK_SIZE;
        let mut payload = Buffer::new();
        control::write_window_ack_size(&mut payload, 1024).unwrap();

        assert!(
            client
                .consume_protocol_control_message(&msg, payload.as_slice())
                .unwrap()
        );
        assert_eq!(client.window_ack_size, 1024);
        assert!(
            client.send_buffer.available() > 0,
            "bytes received before the window was learned must trigger the acknowledgement"
        );
        assert!(client.ack_pending);
    }

    #[test]
    fn pending_acknowledgement_coalesces_until_flushed() {
        let mut client = Client::new();
        client.window_ack_size = 1024;
        client.bytes_received = 2048;
        client.maybe_send_window_ack().unwrap();
        let queued = client.send_buffer.available();
        assert!(queued > 0);

        client.bytes_received = 10_000;
        client.maybe_send_window_ack().unwrap();
        assert_eq!(
            client.send_buffer.available(),
            queued,
            "a pending acknowledgement must not append more messages"
        );

        client.flush_pending_outbound(None).unwrap();
        assert!(!client.ack_pending);
        assert_eq!(client.send_buffer.available(), 0);

        client.maybe_send_window_ack().unwrap();
        assert!(
            client.send_buffer.available() > 0,
            "the next window crossing after a flush must queue again"
        );
    }

    #[test]
    fn send_frame_payload_services_inbound_pings() {
        use crate::net::testing::{PairStream, stream_pair};
        use std::io::{Read, Write};

        let (client_end, mut peer) = stream_pair().unwrap();
        client_end.set_nonblocking(true).unwrap();

        let mut client = Client::new();
        client.chunk_reg.init();
        client.state = ClientState::Publishing;
        client.stream_id = 1;
        client.transport = Some(crate::net::testing::transport_from(client_end));

        peer.write_all(&rtmp_user_control_ping_chunk(77)).unwrap();

        client
            .send_frame_payload(FrameType::Video, 0, &[0x17, 0x00])
            .unwrap();

        let mut out = [0u8; 512];
        let n = peer.read(&mut out).unwrap();
        assert!(n > 0);
        let ping_response = control::UCTRL_PING_RESPONSE.to_be_bytes();
        assert!(
            out[..n].windows(2).any(|w| w == ping_response),
            "send_frame_payload should answer inbound pings before sending media"
        );
    }

    #[test]
    fn publishing_poll_services_inbound_pings() {
        use crate::net::testing::{PairStream, stream_pair};
        use std::io::{Read, Write};

        let (client_end, mut peer) = stream_pair().unwrap();
        client_end.set_nonblocking(true).unwrap();

        let mut client = Client::new();
        client.chunk_reg.init();
        client.state = ClientState::Publishing;
        client.stream_id = 1;
        client.transport = Some(crate::net::testing::transport_from(client_end));

        peer.write_all(&rtmp_user_control_ping_chunk(88)).unwrap();

        client.poll(0).unwrap();

        let mut out = [0u8; 512];
        let n = peer.read(&mut out).unwrap();
        assert!(n > 0);
        let ping_response = control::UCTRL_PING_RESPONSE.to_be_bytes();
        assert!(
            out[..n].windows(2).any(|w| w == ping_response),
            "publishing poll should answer inbound pings for idle publishers"
        );
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
