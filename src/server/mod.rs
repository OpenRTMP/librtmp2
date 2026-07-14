//! RTMP server listener
//!
//! Mirrors `src/server/server.h` and `src/server/server.c`.

use std::collections::HashMap;
use std::net::TcpListener;
use std::os::unix::io::{AsRawFd, IntoRawFd};
use std::sync::{Arc, Mutex};
#[cfg(feature = "tls")]
use std::time::{Duration, Instant};

use crate::chunk::state::DEFAULT_CHUNK_SIZE;
use crate::net;
use crate::session::conn::Conn;
use crate::session::publish_route::PublishRouteRegistry;
#[cfg(feature = "tls")]
use crate::transport::{PendingTlsAccept, TlsAcceptOutcome};
use crate::transport::{TlsCtx, Transport};
use crate::types::*;

/// Maximum distinct (app, stream_name) cache entries retained server-wide.
const MAX_STREAM_CACHE_ENTRIES: usize = 1024;

/// Maximum total bytes retained across all stream_cache entries server-wide.
const MAX_STREAM_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Maximum payload size cached as a codec header. Real AVC/AAC sequence
/// headers are small; larger payloads are relayed live but not stored for
/// init-frame replay.
const MAX_CACHED_INIT_FRAME_BYTES: usize = 64 * 1024;

/// Maximum payload size cached as a keyframe. Unlike codec headers, a
/// legitimate IDR frame (e.g. 1080p/4K H.264) can be several hundred KB to
/// a few MB, so keyframes get a separate, larger cap than codec headers.
/// Bounded by `DEFAULT_MAX_MSG_LENGTH` (the chunk layer's own hard ceiling
/// on a single message) rather than left unbounded.
const MAX_CACHED_KEYFRAME_BYTES: usize = 2 * 1024 * 1024;

/// Maximum number of incomplete TLS handshakes retained when `max_connections`
/// is unlimited. When `max_connections` is set, active connections and pending
/// handshakes share that configured cap instead.
#[cfg(feature = "tls")]
const MAX_PENDING_TLS_HANDSHAKES: usize = 128;

/// Drop TLS handshakes that do not complete within this overall budget.
#[cfg(feature = "tls")]
const TLS_HANDSHAKE_TIMEOUT_SECS: u64 = 10;

/// Maximum inbound bytes drained from one connection per `process_connections`
/// pass. Without this cap a peer that keeps the kernel recv buffer full can
/// monopolize the single-threaded poll loop and starve every other session
/// until its socket buffer is empty.
const MAX_RECV_BYTES_PER_CONN_PER_POLL: usize = 256 * 1024;

/// Maximum number of extra budget-only `recv(&[])` passes used to drain
/// already-buffered messages left over from `Conn`'s per-recv message cap.
/// Each pass affords another full message budget, so this bounds one
/// connection to this many extra budgets per poll tick -- any remainder
/// waits for the next `process_connections` pass instead of starving other
/// connections in this one.
const MAX_BUDGET_DRAIN_PASSES_PER_CONN_PER_POLL: usize = 3;

/// Cached codec headers and last keyframe for a (app, stream_name) pair.
/// Replayed to players that join after the publisher has already sent headers.
struct StreamCache {
    avc_header: Option<Vec<u8>>,
    aac_header: Option<Vec<u8>>,
    /// (timestamp, payload) of the most recent IDR keyframe.
    last_keyframe: Option<(u32, Vec<u8>)>,
}

/// One bound listener socket plus the TLS context (if any) new connections
/// accepted on it should use. A `Server` holds one of these per call to
/// [`Server::listen`] / [`Server::listen_tls`].
struct ListenerEntry {
    tcp: TcpListener,
    tls_ctx: Option<TlsCtx>,
}

#[cfg(feature = "tls")]
struct PendingTlsConnection {
    handshake: PendingTlsAccept,
    remote_addr: String,
    deadline: Instant,
}

/// Server object.
///
/// A single `Server` can bind more than one listener (see [`Server::listen`]
/// and [`Server::listen_tls`]) — e.g. plaintext RTMP on one port and RTMPS on
/// another. All listeners share the same `connections`, media relay, and
/// stream cache, so a publisher on one listener is relayed to players on any
/// other listener exactly as if they shared a port. Running two *separate*
/// `Server` instances instead would silently split the relay: each instance
/// only relays among its own `connections`.
pub struct Server {
    pub config: ServerConfig,
    pub resource_limits: ResourceLimits,
    pub running: bool,
    /// Identifies *one* bound listener (whichever was bound first) for
    /// diagnostics/backward compatibility. When more than one listener is
    /// bound, use [`Server::listener_fds`] to register every listener with an
    /// external readiness loop. [`Server::poll`] itself checks every bound
    /// listener internally.
    pub server_fd: i32,
    pub connections: Vec<Conn>,
    /// Fired for every audio/video frame on every connection.
    pub on_frame_cb: Option<fn(&Frame)>,
    /// Fired when a client completes the AMF `connect` exchange.
    pub on_connect_cb: Option<fn()>,
    /// When set, must return true to allow `publish`; false rejects the command.
    pub on_publish_cb: Option<fn(conn_id: u64, app: &str, stream_name: &str) -> bool>,
    /// When set, must return true to allow `play`; false rejects the command.
    pub on_play_cb: Option<fn(conn_id: u64, app: &str, stream_name: &str) -> bool>,
    /// When set, must return true before publisher media is queued for relay.
    pub on_media_cb: Option<fn(u64, FrameType, Option<&str>) -> bool>,
    /// TLS context built from `config` at construction time; used by
    /// [`Server::listen`] calls. This field stays public for Rust API
    /// compatibility so integrators that used to replace `server.tls_ctx`
    /// directly can continue to do so before calling `listen()`.
    pub tls_ctx: Option<TlsCtx>,
    listeners: Vec<ListenerEntry>,
    /// Listener index to try first on the next accept pass.
    next_listener_accept: usize,
    #[cfg(feature = "tls")]
    pending_tls: Vec<PendingTlsConnection>,
    stream_cache: HashMap<(String, String), StreamCache>,
    /// Cache keys created by each publisher connection (for teardown).
    publisher_cache_keys: HashMap<u64, Vec<(String, String)>>,
    next_conn_id: u64,
    /// Set once a connection ID has been handed out. This prevents resetting
    /// the counter later and reusing IDs after earlier connections were closed.
    conn_ids_issued: bool,
    /// Hold media relay until the integrator enables it per connection.
    pub defer_media_relay: bool,
    /// Active publish routes: (app, stream_name) -> owning conn_id.
    pub(crate) active_publish_routes: Arc<Mutex<HashMap<(String, String), u64>>>,
}

impl Server {
    /// Create a new server.
    pub fn new(config: ServerConfig) -> Result<Self> {
        let tls_ctx = if config.tls_enabled != 0 {
            if config.tls_cert_file.is_null() || config.tls_key_file.is_null() {
                return Err(ErrorCode::Internal);
            }
            let cert = unsafe {
                std::ffi::CStr::from_ptr(config.tls_cert_file as *const std::ffi::c_char)
            };
            let key =
                unsafe { std::ffi::CStr::from_ptr(config.tls_key_file as *const std::ffi::c_char) };
            let cert_str = cert.to_str().map_err(|_| ErrorCode::Internal)?;
            let key_str = key.to_str().map_err(|_| ErrorCode::Internal)?;
            if cert_str.is_empty() || key_str.is_empty() {
                return Err(ErrorCode::Internal);
            }
            Some(TlsCtx::new_server(cert_str, key_str)?)
        } else {
            None
        };

        Ok(Self {
            config,
            resource_limits: ResourceLimits::default(),
            running: false,
            server_fd: -1,
            connections: Vec::new(),
            on_frame_cb: None,
            on_connect_cb: None,
            on_publish_cb: None,
            on_play_cb: None,
            on_media_cb: None,
            tls_ctx,
            listeners: Vec::new(),
            next_listener_accept: 0,
            #[cfg(feature = "tls")]
            pending_tls: Vec::new(),
            stream_cache: HashMap::new(),
            publisher_cache_keys: HashMap::new(),
            next_conn_id: 1,
            conn_ids_issued: false,
            defer_media_relay: false,
            active_publish_routes: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn release_all_publish_routes(&self, conn_id: u64) {
        if let Ok(mut routes) = self.active_publish_routes.lock() {
            routes.retain(|_, owner| *owner != conn_id);
        }
    }

    /// Set the starting value used for auto-generated connection IDs.
    ///
    /// Only needed when integrating with something *outside* this crate that
    /// numbers connections independently and must not collide with this
    /// `Server`'s IDs. Prefer [`Server::listen_tls`] over running a second
    /// `Server` instance for a second listener — one `Server` with multiple
    /// listeners numbers all of its connections from one counter already, so
    /// this is unnecessary in that case. Call right after [`Server::new`].
    ///
    /// Panics if `base` is zero, cannot leave room for an increment, or if any
    /// connection ID has already been issued.
    pub fn set_conn_id_base(&mut self, base: u64) {
        assert!(base != 0, "conn_id base must be non-zero");
        assert!(
            base < u64::MAX,
            "conn_id base must leave room for at least one later connection ID"
        );
        assert!(
            !self.conn_ids_issued && self.connections.is_empty(),
            "set_conn_id_base must be called before accepting any connections"
        );
        #[cfg(feature = "tls")]
        assert!(
            self.pending_tls.is_empty(),
            "set_conn_id_base must be called before accepting any connections"
        );
        self.next_conn_id = base;
    }

    /// Resolve a "host:port" (default port 1935) string into a bindable address.
    fn resolve_bind_addr(bind_addr: &str) -> Result<String> {
        let mut host = String::new();
        let mut port = String::new();
        net::split_host_port(bind_addr, &mut host, &mut port, "1935")?;
        Ok(if host.is_empty() {
            format!("0.0.0.0:{port}")
        } else if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        })
    }

    fn bind_listener(&mut self, addr: &str) -> Result<TcpListener> {
        let listener = TcpListener::bind(addr).map_err(|_| ErrorCode::Io)?;
        listener.set_nonblocking(true).map_err(|_| ErrorCode::Io)?;
        if self.server_fd < 0 {
            self.server_fd = listener.as_raw_fd();
        }
        self.running = true;
        Ok(listener)
    }

    /// Return the file descriptor for every currently bound listener.
    ///
    /// Use this instead of the legacy [`Server::server_fd`] field when an
    /// external readiness loop needs to watch a multi-listener `Server`.
    pub fn listener_fds(&self) -> Vec<i32> {
        self.listeners
            .iter()
            .map(|listener| listener.tcp.as_raw_fd())
            .collect()
    }

    /// Start listening on the given address ("host:port", default port 1935).
    ///
    /// Uses the TLS/plaintext mode selected at construction time via
    /// [`ServerConfig::tls_enabled`]. Can be called more than once (e.g. to
    /// also bind an IPv6 address); every bound listener shares this `Server`'s
    /// connections, media relay, and stream cache. To add a listener with an
    /// *independent* TLS certificate — e.g. plaintext RTMP plus RTMPS on a
    /// second port — use [`Server::listen_tls`] instead.
    pub fn listen(&mut self, bind_addr: &str) -> Result<()> {
        let addr = Self::resolve_bind_addr(bind_addr)?;
        let tcp = self.bind_listener(&addr)?;
        self.listeners.push(ListenerEntry {
            tcp,
            tls_ctx: self.tls_ctx.clone(),
        });
        Ok(())
    }

    /// Start an additional RTMPS listener with its own certificate/key,
    /// independent of the TLS/plaintext mode passed to [`Server::new`].
    ///
    /// Connections accepted here land in the same `connections` list as every
    /// other listener on this `Server`, so a publisher here is relayed to
    /// players on any other listener (plaintext or TLS) and vice versa.
    pub fn listen_tls(&mut self, bind_addr: &str, cert_file: &str, key_file: &str) -> Result<()> {
        let ctx = TlsCtx::new_server(cert_file, key_file)?;
        let addr = Self::resolve_bind_addr(bind_addr)?;
        let tcp = self.bind_listener(&addr)?;
        self.listeners.push(ListenerEntry {
            tcp,
            tls_ctx: Some(ctx),
        });
        Ok(())
    }

    /// Poll for events (non-blocking).
    pub fn poll(&mut self, timeout_ms: i32) -> Result<()> {
        if !self.running {
            return Err(ErrorCode::Internal);
        }
        self.accept_new_connections();
        self.process_connections()?;
        if timeout_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(timeout_ms as u64));
        }
        Ok(())
    }

    /// Stop the server.
    pub fn stop(&mut self) {
        self.running = false;
        self.listeners.clear();
        self.next_listener_accept = 0;
        #[cfg(feature = "tls")]
        self.pending_tls.clear();
        // bind_listener() only assigns server_fd when it's negative, so a
        // later listen() call after stop() must see it reset here or it
        // would keep exposing the now-closed fd from before this stop().
        self.server_fd = -1;
    }

    fn max_connections_reached(&self) -> bool {
        self.config.max_connections > 0
            && self.connections.len() >= self.config.max_connections as usize
    }

    #[cfg(feature = "tls")]
    fn tls_handshake_deadline() -> Instant {
        Instant::now() + Duration::from_secs(TLS_HANDSHAKE_TIMEOUT_SECS)
    }

    #[cfg(feature = "tls")]
    fn pending_tls_limit_reached(&self) -> bool {
        let pending = self.pending_tls.len();
        if self.config.max_connections > 0 {
            self.connections.len() + pending >= self.config.max_connections as usize
        } else {
            pending >= MAX_PENDING_TLS_HANDSHAKES
        }
    }

    fn allocate_conn_id(&mut self) -> Option<u64> {
        let conn_id = self.next_conn_id;
        if conn_id == 0 || conn_id == u64::MAX {
            return None;
        }
        self.next_conn_id = conn_id + 1;
        self.conn_ids_issued = true;
        Some(conn_id)
    }

    fn add_connection(&mut self, transport: Transport, remote_addr: String) -> bool {
        if self.max_connections_reached() {
            return false;
        }
        let Some(conn_id) = self.allocate_conn_id() else {
            return false;
        };

        let conn_fd = transport.fd();
        let mut conn = Conn::new();
        conn.chunk_reg.max_reassembly_bytes = self.resource_limits.max_reassembly_bytes;
        conn.max_pending_relay_bytes = self.resource_limits.max_pending_relay_bytes;
        // Outbound chunk size only: peers start sending at the RTMP
        // default (128) until SetChunkSize is negotiated.
        conn.chunk_size = if self.config.chunk_size > 0 {
            self.config.chunk_size as u32
        } else {
            DEFAULT_CHUNK_SIZE
        };
        conn.client_fd = conn_fd;
        conn.conn_id = conn_id;
        conn.remote_addr = remote_addr;
        conn.defer_media_relay = self.defer_media_relay;
        conn.transport = Some(transport);
        conn.on_frame_cb = self.on_frame_cb;
        conn.on_media_cb = self.on_media_cb;
        conn.on_connect_cb = self.on_connect_cb;
        conn.on_publish_cb = self.on_publish_cb;
        conn.on_play_cb = self.on_play_cb;
        conn.publish_routes = Some(PublishRouteRegistry::new(Arc::clone(
            &self.active_publish_routes,
        )));
        self.connections.push(conn);
        true
    }

    #[cfg(feature = "tls")]
    fn progress_pending_tls(&mut self) {
        let pending = std::mem::take(&mut self.pending_tls);
        let now = Instant::now();
        for pending_conn in pending {
            if now >= pending_conn.deadline {
                continue;
            }
            if self.max_connections_reached() {
                self.pending_tls.push(pending_conn);
                continue;
            }

            match pending_conn.handshake.progress() {
                Ok(TlsAcceptOutcome::Complete(transport)) => {
                    self.add_connection(transport, pending_conn.remote_addr);
                }
                Ok(TlsAcceptOutcome::WouldBlock(handshake)) => {
                    if Instant::now() < pending_conn.deadline {
                        self.pending_tls.push(PendingTlsConnection {
                            handshake,
                            remote_addr: pending_conn.remote_addr,
                            deadline: pending_conn.deadline,
                        });
                    }
                }
                Err(_) => {}
            }
        }
    }

    #[cfg(not(feature = "tls"))]
    fn progress_pending_tls(&mut self) {}

    /// Accept any pending inbound connections on every bound listener.
    ///
    /// Both TCP accept and TLS handshakes are driven non-blockingly. TLS
    /// handshakes that need more bytes are retried on later `poll()` calls so a
    /// stalled RTMPS peer cannot freeze other listeners or active sessions.
    fn accept_new_connections(&mut self) {
        self.progress_pending_tls();

        let listener_count = self.listeners.len();
        if listener_count == 0 {
            self.next_listener_accept = 0;
            return;
        }
        self.next_listener_accept %= listener_count;

        loop {
            let mut accepted_any = false;

            for offset in 0..listener_count {
                if self.max_connections_reached() {
                    return;
                }
                let i = (self.next_listener_accept + offset) % listener_count;
                match self.listeners[i].tcp.accept() {
                    Ok((stream, addr)) => {
                        accepted_any = true;
                        self.next_listener_accept = (i + 1) % listener_count;
                        let remote_addr = addr.to_string();
                        let tls_ctx = self.listeners[i].tls_ctx.clone();
                        if let Some(ctx) = tls_ctx.as_ref() {
                            #[cfg(feature = "tls")]
                            {
                                match ctx.accept_nonblocking(stream.into_raw_fd()) {
                                    Ok(TlsAcceptOutcome::Complete(transport)) => {
                                        self.add_connection(transport, remote_addr);
                                    }
                                    Ok(TlsAcceptOutcome::WouldBlock(handshake)) => {
                                        if !self.pending_tls_limit_reached() {
                                            self.pending_tls.push(PendingTlsConnection {
                                                handshake,
                                                remote_addr,
                                                deadline: Self::tls_handshake_deadline(),
                                            });
                                        }
                                    }
                                    Err(_) => {}
                                }
                            }
                            #[cfg(not(feature = "tls"))]
                            {
                                let _ = ctx;
                                drop(stream);
                            }
                        } else {
                            let _ = stream.set_nonblocking(true);
                            let transport = Transport::new_plain(stream.into_raw_fd());
                            self.add_connection(transport, remote_addr);
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => {}
                }
            }

            if !accepted_any {
                break;
            }
        }
    }

    /// Process all active connections: drain readable bytes, drive the
    /// protocol state machine, relay frames from publishers to players,
    /// flush pending writes, and reap closed peers.
    pub fn process_connections(&mut self) -> Result<()> {
        let mut buf = [0u8; 65536];
        let mut closed = Vec::new();

        // Drive recv/processing for every connection.
        self.drain_pending_cache_evictions();
        // Cloned so a closed connection's publish route can be released
        // immediately (below) without conflicting with the mutable borrow
        // of `self.connections` this loop holds via `iter_mut()`.
        let active_publish_routes = Arc::clone(&self.active_publish_routes);
        for (i, conn) in self.connections.iter_mut().enumerate() {
            let mut conn_closed_this_iteration = false;
            if conn.session_setup_timed_out() {
                conn.disconnect_transport();
                closed.push(i);
                conn_closed_this_iteration = true;
            }
            let mut bytes_drained = 0usize;
            while !conn_closed_this_iteration {
                if bytes_drained >= MAX_RECV_BYTES_PER_CONN_PER_POLL {
                    break;
                }
                let Some(transport) = conn.transport.as_mut() else {
                    closed.push(i);
                    conn_closed_this_iteration = true;
                    break;
                };
                let mut again = 0i32;
                let n = transport.recv(&mut buf, &mut again);
                if n > 0 {
                    let chunk_len = n as usize;
                    if conn.recv(&buf[..chunk_len]).is_err() {
                        closed.push(i);
                        conn_closed_this_iteration = true;
                        break;
                    }
                    bytes_drained += chunk_len;
                } else if n == 0 {
                    closed.push(i);
                    conn_closed_this_iteration = true;
                    break;
                } else if again != 0 {
                    break;
                } else {
                    closed.push(i);
                    conn_closed_this_iteration = true;
                    break;
                }
            }
            // A batch larger than the per-`recv` message budget leaves
            // complete messages buffered but unprocessed; keep draining them
            // with no new bytes instead of waiting on the peer to send more,
            // which may never happen if it's waiting on our response. Capped
            // so one connection with a huge batch can't monopolize this poll
            // tick -- any remainder is picked up on the next poll() call.
            for _ in 0..MAX_BUDGET_DRAIN_PASSES_PER_CONN_PER_POLL {
                if conn_closed_this_iteration || !conn.has_buffered_messages() {
                    break;
                }
                if conn.recv(&[]).is_err() {
                    closed.push(i);
                    conn_closed_this_iteration = true;
                    break;
                }
            }
            // Release this connection's claimed publish route(s) right away
            // rather than deferring to the end-of-batch cleanup below: a
            // later connection processed in this same loop may try to
            // publish the same (app, stream) route, and PublishRouteRegistry
            // must not see this now-closed connection as the stale owner.
            if conn_closed_this_iteration {
                if let Ok(mut routes) = active_publish_routes.lock() {
                    routes.retain(|_, owner| *owner != conn.conn_id);
                }
            }
        }

        // Ordering within this function is deliberate: evictions from
        // renames processed by the recv loop just above are applied here,
        // *after* that recv loop but *before* both init-frame replay and
        // caching of this batch's freshly-relayed frames below. This means
        // init-frame replay never sees a cache entry under a route key its
        // publisher abandoned earlier in this same batch, and a same-batch
        // rename can't leave a stale entry alive until the next poll() call.
        //
        // The returned set also guards the caching step below: a frame
        // queued (via pending_relay) under the old route *before* its
        // publisher's rename was processed in this same recv loop must not
        // resurrect the entry we just evicted. It's keyed by (app, name,
        // conn_id) -- not just (app, name) -- so this only suppresses
        // caching for the connection that actually abandoned the route;
        // a *different* publisher's legitimate same-batch frame for an
        // identical (app, name) is unaffected.
        let abandoned_this_batch = self.drain_pending_cache_evictions();

        // Collect all frames queued by publishers, then relay them to players
        // on the same (app, stream_name) pair.
        let relay_frames: Vec<_> = self
            .connections
            .iter_mut()
            .flat_map(|c| c.pending_relay.drain(..))
            .collect();

        // Replay cached codec headers and last keyframe to newly-joined players
        // using the pre-batch cache state, so init frames always precede live
        // frames from the current batch.
        for (i, conn) in self.connections.iter_mut().enumerate() {
            if conn.transport.is_none() || !conn.needs_init_frames {
                continue;
            }
            let Some(ref stream) = conn.current_stream else {
                continue;
            };
            if !stream.is_playing || !conn.relay_enabled {
                continue;
            }
            conn.needs_init_frames = false;
            let key = (conn.app.clone(), conn.relay_route_key());
            if let Some(cache) = self.stream_cache.get(&key) {
                let mut send_failed = false;
                if let Some(ref hdr) = cache.avc_header.clone() {
                    send_failed |= conn.send_frame(FrameType::Video, 0, hdr).is_err();
                }
                if !send_failed {
                    if let Some(ref hdr) = cache.aac_header.clone() {
                        send_failed |= conn.send_frame(FrameType::Audio, 0, hdr).is_err();
                    }
                }
                if !send_failed {
                    if let Some((ts, ref kf)) = cache.last_keyframe.clone() {
                        send_failed |= conn.send_frame(FrameType::Video, ts, kf).is_err();
                    }
                }
                if send_failed {
                    // Cached init-frame replay filled the send buffer for a
                    // slow player. Close immediately just like live relay sends.
                    conn.relay_enabled = false;
                    conn.needs_init_frames = false;
                    conn.disconnect_transport();
                    closed.push(i);
                }
            }
        }

        // Update per-stream cache and relay each frame in order so players
        // receive frames in the same sequence the publisher sent them.
        for frame in &relay_frames {
            let abandon_key = (
                frame.app.clone(),
                frame.stream_name.clone(),
                frame.publisher_conn_id,
            );
            if !abandoned_this_batch.contains(&abandon_key) {
                self.cache_relay_frame(frame);
            }
            for (i, conn) in self.connections.iter_mut().enumerate() {
                let is_player = conn.relay_enabled
                    && conn.transport.is_some()
                    && conn
                        .current_stream
                        .as_ref()
                        .map(|s| s.is_playing && conn.relay_route_key() == frame.stream_name)
                        .unwrap_or(false);
                if !is_player || conn.app != frame.app {
                    continue;
                }
                if conn
                    .send_frame(frame.frame_type, frame.timestamp, &frame.payload)
                    .is_err()
                {
                    // Player stopped reading; outbound send_buffer is full.
                    // Drop the connection immediately so later relay frames in
                    // this poll batch skip it and no more socket work is done.
                    conn.relay_enabled = false;
                    conn.needs_init_frames = false;
                    conn.disconnect_transport();
                    closed.push(i);
                }
            }
        }

        // Flush all connections.
        for (i, conn) in self.connections.iter_mut().enumerate() {
            if conn.transport.is_none() {
                closed.push(i);
                continue;
            }
            if conn.maybe_send_ping().is_err() {
                closed.push(i);
                continue;
            }
            if conn.flush().is_err() {
                closed.push(i);
            }
        }

        // A connection that errors on both recv and flush gets pushed twice.
        // Sort then dedup so each index is removed exactly once.
        closed.sort_unstable();
        closed.dedup();
        for i in closed.into_iter().rev() {
            let conn = &self.connections[i];
            self.release_all_publish_routes(conn.conn_id);
            // Tracking must be cleared unconditionally: a publisher can issue
            // another createStream after publishing, replacing current_stream
            // and leaving is_publishing false even though this conn_id still
            // owns cache entries.
            if let Some(keys) = self.publisher_cache_keys.remove(&conn.conn_id) {
                for key in keys {
                    // Two connections can end up tracking the same (app,
                    // stream_name) key (e.g. a stale entry left behind by a
                    // publisher that renamed via createStream, then a
                    // different connection republished under that same
                    // name). Only actually drop the cache entry once no
                    // other tracked publisher still claims it, or we'd wipe
                    // out a still-active publisher's cached headers.
                    let still_owned = self.publisher_cache_keys.values().any(|v| v.contains(&key));
                    if !still_owned {
                        self.stream_cache.remove(&key);
                    }
                }
            }
            self.connections.remove(i);
        }
        Ok(())
    }

    /// Bytes retained by a single stream_cache entry.
    fn stream_cache_entry_bytes(cache: &StreamCache) -> usize {
        cache.avc_header.as_ref().map(|v| v.len()).unwrap_or(0)
            + cache.aac_header.as_ref().map(|v| v.len()).unwrap_or(0)
            + cache
                .last_keyframe
                .as_ref()
                .map(|(_, v)| v.len())
                .unwrap_or(0)
    }

    /// Total bytes currently retained across all stream_cache entries.
    fn stream_cache_bytes(&self) -> usize {
        self.stream_cache
            .values()
            .map(Self::stream_cache_entry_bytes)
            .sum()
    }

    fn evict_stream_cache_key(&mut self, key: &(String, String)) {
        self.stream_cache.remove(key);
        for keys in self.publisher_cache_keys.values_mut() {
            keys.retain(|k| k != key);
        }
    }

    fn cache_relay_frame(&mut self, frame: &crate::session::conn::RelayFrame) {
        let is_avc_header = frame.frame_type == FrameType::Video
            && frame.payload.len() >= 2
            && frame.payload[0] == 0x17
            && frame.payload[1] == 0x00;
        let is_keyframe = frame.frame_type == FrameType::Video
            && frame.payload.len() >= 2
            && frame.payload[0] == 0x17
            && frame.payload[1] == 0x01;
        let is_aac_header = frame.frame_type == FrameType::Audio
            && frame.payload.len() >= 2
            && (frame.payload[0] & 0xF0) == 0xA0
            && frame.payload[1] == 0x00;

        // Frames that are neither a codec header nor a keyframe are relayed
        // live but never cached; don't create an empty entry for them.
        if !is_avc_header && !is_keyframe && !is_aac_header {
            return;
        }

        let key = (frame.app.clone(), frame.stream_name.clone());

        let cap = if is_keyframe {
            MAX_CACHED_KEYFRAME_BYTES
        } else {
            MAX_CACHED_INIT_FRAME_BYTES
        };
        if frame.payload.len() > cap {
            // An oversized replacement still means the previously cached
            // copy for this field is stale (the publisher has moved past
            // it) -- drop it rather than keep replaying it to late joiners.
            if let Some(cache) = self.stream_cache.get_mut(&key) {
                if is_avc_header {
                    cache.avc_header = None;
                } else if is_keyframe {
                    cache.last_keyframe = None;
                } else {
                    cache.aac_header = None;
                }
            }
            if self.stream_cache.get(&key).is_some_and(|c| {
                c.avc_header.is_none() && c.aac_header.is_none() && c.last_keyframe.is_none()
            }) {
                self.evict_stream_cache_key(&key);
            }
            return;
        }

        let publisher_keys = self
            .publisher_cache_keys
            .entry(frame.publisher_conn_id)
            .or_default();
        if !publisher_keys.iter().any(|k| k == &key) {
            publisher_keys.push(key.clone());
        }
        if self.stream_cache.len() >= MAX_STREAM_CACHE_ENTRIES
            && !self.stream_cache.contains_key(&key)
        {
            if let Some(evict) = self.stream_cache.keys().find(|k| *k != &key).cloned() {
                self.evict_stream_cache_key(&evict);
            }
        }

        let existing_field_len = self
            .stream_cache
            .get(&key)
            .map(|cache| {
                if is_avc_header {
                    cache.avc_header.as_ref().map(|v| v.len()).unwrap_or(0)
                } else if is_keyframe {
                    cache
                        .last_keyframe
                        .as_ref()
                        .map(|(_, v)| v.len())
                        .unwrap_or(0)
                } else {
                    cache.aac_header.as_ref().map(|v| v.len()).unwrap_or(0)
                }
            })
            .unwrap_or(0);
        let incoming_len = frame.payload.len();
        // Track the running total locally instead of recomputing
        // stream_cache_bytes() (an O(n) scan) on every eviction, which would
        // make this O(n^2) under sustained cache churn.
        let mut projected_total = self.stream_cache_bytes() + incoming_len - existing_field_len;
        let max_cache_bytes = self.resource_limits.max_stream_cache_bytes;
        if projected_total > max_cache_bytes {
            let victims: Vec<_> = self
                .stream_cache
                .keys()
                .filter(|k| *k != &key)
                .cloned()
                .collect();
            for victim in victims {
                if projected_total <= max_cache_bytes {
                    break;
                }
                if let Some(cache) = self.stream_cache.get(&victim) {
                    projected_total -= Self::stream_cache_entry_bytes(cache);
                }
                self.evict_stream_cache_key(&victim);
            }
        }

        // Evicting every other entry still isn't enough when this single
        // payload alone exceeds the budget -- don't cache it at all rather
        // than let the server-wide total blow past the configured cache cap.
        if projected_total > max_cache_bytes {
            return;
        }

        let cache = self.stream_cache.entry(key).or_insert(StreamCache {
            avc_header: None,
            aac_header: None,
            last_keyframe: None,
        });
        if is_avc_header {
            cache.avc_header = Some(frame.payload.clone());
        } else if is_keyframe {
            cache.last_keyframe = Some((frame.timestamp, frame.payload.clone()));
        } else if is_aac_header {
            cache.aac_header = Some(frame.payload.clone());
        }
    }

    /// Drain every connection's queued rename evictions, returning the set of
    /// keys actually removed from `stream_cache`.
    ///
    /// A `pending_cache_evictions` entry only reflects what the connection
    /// *believes* it was routing under before the rename -- it has no
    /// visibility into `publisher_cache_keys`. Two independent connections
    /// can publish under the same (app, stream_name) (accidentally, or via a
    /// hostile client racing a legitimate publisher's route name), so an
    /// eviction is only honored when this conn_id is a confirmed owner of
    /// the key in `publisher_cache_keys`; otherwise it would delete a cache
    /// entry that belongs to a different, still-active publisher.
    fn drain_pending_cache_evictions(
        &mut self,
    ) -> std::collections::HashSet<(String, String, u64)> {
        let mut abandoned = std::collections::HashSet::new();
        for conn in &mut self.connections {
            for key in conn.pending_cache_evictions.drain(..) {
                // Record the raw request regardless of ownership outcome
                // below: a connection's own frame, queued under this key
                // earlier in the same batch (before this rename/abandon was
                // processed), must never resurrect the entry -- whether or
                // not publisher_cache_keys already reflects ownership yet.
                // Scoped by conn_id so this doesn't suppress a *different*
                // publisher's frame for the same (app, name).
                abandoned.insert((key.0.clone(), key.1.clone(), conn.conn_id));

                let owns_key = self
                    .publisher_cache_keys
                    .get(&conn.conn_id)
                    .map(|keys| keys.contains(&key))
                    .unwrap_or(false);
                if !owns_key {
                    continue;
                }
                if let Some(keys) = self.publisher_cache_keys.get_mut(&conn.conn_id) {
                    keys.retain(|k| k != &key);
                }
                // A different conn_id can still be tracking this exact
                // (app, stream_name) (two publishers sharing a route name).
                // Only actually drop the cache entry once no other tracked
                // publisher claims it, or renaming away would wipe out a
                // still-active publisher's cached headers/keyframe.
                let still_owned = self
                    .publisher_cache_keys
                    .values()
                    .any(|keys| keys.contains(&key));
                if !still_owned {
                    self.stream_cache.remove(&key);
                }
            }
        }
        abandoned
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recv_budget_is_at_least_one_socket_read() {
        assert!(MAX_RECV_BYTES_PER_CONN_PER_POLL >= 65536);
    }

    #[test]
    fn recv_budget_is_small_enough_for_fairness_across_connections() {
        assert!(MAX_RECV_BYTES_PER_CONN_PER_POLL <= 1024 * 1024);
    }

    fn test_server() -> Server {
        Server::new(ServerConfig {
            max_connections: 4,
            chunk_size: 128,
            tls_enabled: 0,
            tls_cert_file: std::ptr::null(),
            tls_key_file: std::ptr::null(),
            tls_ca_file: std::ptr::null(),
            tls_insecure: 0,
        })
        .unwrap()
    }

    fn relay_frame(frame_type: FrameType, payload: Vec<u8>) -> crate::session::conn::RelayFrame {
        crate::session::conn::RelayFrame {
            app: "live".to_string(),
            stream_name: "stream".to_string(),
            publisher_conn_id: 1,
            frame_type,
            timestamp: 0,
            payload,
        }
    }

    #[test]
    fn oversized_codec_header_is_not_cached() {
        let mut server = test_server();

        let mut payload = vec![0x17, 0x00];
        payload.resize(MAX_CACHED_INIT_FRAME_BYTES + 1, 0xAA);
        server.cache_relay_frame(&relay_frame(FrameType::Video, payload));
        assert!(server.stream_cache.is_empty());
    }

    #[test]
    fn oversized_keyframe_is_not_cached() {
        let mut server = test_server();

        let mut payload = vec![0x17, 0x01];
        payload.resize(MAX_CACHED_KEYFRAME_BYTES + 1, 0xAA);
        server.cache_relay_frame(&relay_frame(FrameType::Video, payload));
        assert!(server.stream_cache.is_empty());
    }

    #[test]
    fn keyframe_within_larger_cap_is_still_cached() {
        // A real IDR frame can comfortably exceed the codec-header cap while
        // staying under the dedicated (larger) keyframe cap.
        let mut server = test_server();

        let mut payload = vec![0x17, 0x01];
        payload.resize(MAX_CACHED_INIT_FRAME_BYTES + 1, 0xAA);
        assert!(payload.len() <= MAX_CACHED_KEYFRAME_BYTES);
        server.cache_relay_frame(&relay_frame(FrameType::Video, payload));

        let key = ("live".to_string(), "stream".to_string());
        assert!(
            server
                .stream_cache
                .get(&key)
                .unwrap()
                .last_keyframe
                .is_some()
        );
    }

    #[test]
    fn oversized_replacement_clears_stale_cached_header() {
        let mut server = test_server();
        let key = ("live".to_string(), "stream".to_string());

        // Cache a normal-sized AVC header first.
        server.cache_relay_frame(&relay_frame(FrameType::Video, vec![0x17, 0x00, 0xAA]));
        assert!(server.stream_cache.get(&key).unwrap().avc_header.is_some());

        // A later oversized "header" replacement must drop the stale cached
        // copy instead of leaving late joiners with outdated codec data.
        let mut oversized = vec![0x17, 0x00];
        oversized.resize(MAX_CACHED_INIT_FRAME_BYTES + 1, 0xBB);
        server.cache_relay_frame(&relay_frame(FrameType::Video, oversized));

        // The whole entry is gone since it had no other cached fields.
        assert!(server.stream_cache.get(&key).is_none());
    }

    #[test]
    fn max_connections_limit_is_enforced_when_configured() {
        let config = ServerConfig {
            max_connections: 2,
            chunk_size: 128,
            tls_enabled: 0,
            tls_cert_file: std::ptr::null(),
            tls_key_file: std::ptr::null(),
            tls_ca_file: std::ptr::null(),
            tls_insecure: 0,
        };
        let mut server = Server::new(config).unwrap();
        server.listen("127.0.0.1:0").unwrap();

        let port = {
            let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
            let rc = unsafe {
                libc::getsockname(
                    server.server_fd,
                    &mut addr as *mut _ as *mut libc::sockaddr,
                    &mut len,
                )
            };
            assert_eq!(rc, 0);
            u16::from_be(addr.sin_port)
        };
        let addr = format!("127.0.0.1:{port}");

        let mut streams = Vec::new();
        for _ in 0..2 {
            streams.push(std::net::TcpStream::connect(&addr).unwrap());
        }
        server.accept_new_connections();
        assert_eq!(server.connections.len(), 2);

        let _third = std::net::TcpStream::connect(&addr).unwrap();
        server.accept_new_connections();
        assert_eq!(server.connections.len(), 2);
    }

    #[test]
    fn stale_pre_connect_sessions_are_closed_during_poll() {
        use std::time::{Duration, Instant};

        let config = ServerConfig {
            max_connections: 4,
            chunk_size: 128,
            tls_enabled: 0,
            tls_cert_file: std::ptr::null(),
            tls_key_file: std::ptr::null(),
            tls_ca_file: std::ptr::null(),
            tls_insecure: 0,
        };
        let mut server = Server::new(config).unwrap();
        server.listen("127.0.0.1:0").unwrap();

        let port = {
            let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
            let rc = unsafe {
                libc::getsockname(
                    server.server_fd,
                    &mut addr as *mut _ as *mut libc::sockaddr,
                    &mut len,
                )
            };
            assert_eq!(rc, 0);
            u16::from_be(addr.sin_port)
        };
        let addr = format!("127.0.0.1:{port}");

        let _stream = std::net::TcpStream::connect(&addr).unwrap();
        server.accept_new_connections();
        assert_eq!(server.connections.len(), 1);

        server.connections[0].state = ConnState::Handshake;
        server.connections[0]
            .set_session_setup_started_for_test(Instant::now() - Duration::from_secs(11));

        server.process_connections().unwrap();
        assert_eq!(server.connections.len(), 0);
    }

    #[test]
    fn second_publisher_on_same_route_is_rejected() {
        let server = test_server();

        let mut first = Conn::new();
        first.conn_id = 1;
        first.app = "live".to_string();
        first.current_stream = Some(Box::new(crate::session::stream::Stream {
            stream_id: 1,
            name: String::new(),
            is_publishing: false,
            is_playing: false,
        }));
        first.publish_routes = Some(PublishRouteRegistry::new(Arc::clone(
            &server.active_publish_routes,
        )));

        let mut second = Conn::new();
        second.conn_id = 2;
        second.app = "live".to_string();
        second.current_stream = Some(Box::new(crate::session::stream::Stream {
            stream_id: 1,
            name: String::new(),
            is_publishing: false,
            is_playing: false,
        }));
        second.publish_routes = Some(PublishRouteRegistry::new(Arc::clone(
            &server.active_publish_routes,
        )));

        let mut buf = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_create_stream(&mut buf, 1.0).unwrap();
        first.handle_command(buf.as_slice()).unwrap();
        second.handle_command(buf.as_slice()).unwrap();

        let mut publish_a = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_publish(&mut publish_a, "victim", "live").unwrap();
        first.handle_command(publish_a.as_slice()).unwrap();
        assert!(first.relay_enabled);

        let mut publish_b = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_publish(&mut publish_b, "victim", "live").unwrap();
        second.handle_command(publish_b.as_slice()).unwrap();
        assert!(
            !second.relay_enabled,
            "second publisher on the same route must be rejected"
        );
    }

    #[test]
    fn publish_rename_onto_occupied_route_keeps_old_route_claimed() {
        let server = test_server();

        let mut first = Conn::new();
        first.conn_id = 1;
        first.app = "live".to_string();
        first.current_stream = Some(Box::new(crate::session::stream::Stream {
            stream_id: 1,
            name: String::new(),
            is_publishing: false,
            is_playing: false,
        }));
        first.publish_routes = Some(PublishRouteRegistry::new(Arc::clone(
            &server.active_publish_routes,
        )));

        let mut second = Conn::new();
        second.conn_id = 2;
        second.app = "live".to_string();
        second.current_stream = Some(Box::new(crate::session::stream::Stream {
            stream_id: 1,
            name: String::new(),
            is_publishing: false,
            is_playing: false,
        }));
        second.publish_routes = Some(PublishRouteRegistry::new(Arc::clone(
            &server.active_publish_routes,
        )));

        let mut buf = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_create_stream(&mut buf, 1.0).unwrap();
        first.handle_command(buf.as_slice()).unwrap();
        second.handle_command(buf.as_slice()).unwrap();

        // `first` claims "a", `second` claims "b".
        let mut publish_a = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_publish(&mut publish_a, "a", "live").unwrap();
        first.handle_command(publish_a.as_slice()).unwrap();
        assert!(first.relay_enabled);

        let mut publish_b = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_publish(&mut publish_b, "b", "live").unwrap();
        second.handle_command(publish_b.as_slice()).unwrap();
        assert!(second.relay_enabled);

        // `first` tries to re-publish onto "b", which `second` already owns.
        // The rename must be rejected, and "a" must remain claimed by `first`
        // so a third connection cannot hijack it.
        let mut publish_rename = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_publish(&mut publish_rename, "b", "live").unwrap();
        first.handle_command(publish_rename.as_slice()).unwrap();

        let mut third = Conn::new();
        third.conn_id = 3;
        third.app = "live".to_string();
        third.current_stream = Some(Box::new(crate::session::stream::Stream {
            stream_id: 1,
            name: String::new(),
            is_publishing: false,
            is_playing: false,
        }));
        third.publish_routes = Some(PublishRouteRegistry::new(Arc::clone(
            &server.active_publish_routes,
        )));
        let mut create_third = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_create_stream(&mut create_third, 1.0).unwrap();
        third.handle_command(create_third.as_slice()).unwrap();

        let mut publish_hijack = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_publish(&mut publish_hijack, "a", "live").unwrap();
        third.handle_command(publish_hijack.as_slice()).unwrap();
        assert!(
            !third.relay_enabled,
            "route \"a\" must stay claimed by the original publisher after a failed rename"
        );
    }

    #[test]
    fn delete_stream_releases_publish_route_for_next_publisher() {
        let server = test_server();

        let mut first = Conn::new();
        first.conn_id = 1;
        first.app = "live".to_string();
        first.current_stream = Some(Box::new(crate::session::stream::Stream {
            stream_id: 1,
            name: String::new(),
            is_publishing: false,
            is_playing: false,
        }));
        first.publish_routes = Some(PublishRouteRegistry::new(Arc::clone(
            &server.active_publish_routes,
        )));

        let mut create_first = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_create_stream(&mut create_first, 1.0).unwrap();
        first.handle_command(create_first.as_slice()).unwrap();

        let mut publish_a = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_publish(&mut publish_a, "victim", "live").unwrap();
        first.handle_command(publish_a.as_slice()).unwrap();
        assert!(first.relay_enabled);

        // Client unpublishes but keeps the TCP connection open.
        let mut delete_stream = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_deletestream(&mut delete_stream, 1.0, 1).unwrap();
        first.handle_command(delete_stream.as_slice()).unwrap();
        assert!(
            !first.relay_enabled,
            "relay_enabled must be cleared along with the publish role, so a later \
             publish under defer_media_relay can't relay before being re-authorized"
        );

        let mut second = Conn::new();
        second.conn_id = 2;
        second.app = "live".to_string();
        second.current_stream = Some(Box::new(crate::session::stream::Stream {
            stream_id: 1,
            name: String::new(),
            is_publishing: false,
            is_playing: false,
        }));
        second.publish_routes = Some(PublishRouteRegistry::new(Arc::clone(
            &server.active_publish_routes,
        )));
        let mut create_second = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_create_stream(&mut create_second, 1.0).unwrap();
        second.handle_command(create_second.as_slice()).unwrap();

        let mut publish_b = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_publish(&mut publish_b, "victim", "live").unwrap();
        second.handle_command(publish_b.as_slice()).unwrap();
        assert!(
            second.relay_enabled,
            "route must be free for a new publisher after deleteStream"
        );
    }

    #[test]
    fn closed_connections_release_routes_before_later_publish_in_same_batch() {
        use crate::chunk::reader::ChunkMessage;
        use crate::chunk::writer::chunk_write;
        use crate::message::message::RTMP_MSG_AMF0_COMMAND;
        use crate::session::stream::Stream;
        use crate::types::ConnState;
        use crate::transport::Transport;
        use std::os::unix::io::IntoRawFd;
        use std::os::unix::net::UnixStream;

        let mut server = test_server();

        // Connection A already owns the "victim" route and is about to be
        // detected as closed (peer hung up) during this same
        // process_connections() pass.
        let mut conn_a = Conn::new();
        conn_a.conn_id = 1;
        conn_a.app = "live".to_string();
        conn_a.current_stream = Some(Box::new(Stream {
            stream_id: 1,
            name: "victim".to_string(),
            is_publishing: true,
            is_playing: false,
        }));
        conn_a.publish_routes = Some(PublishRouteRegistry::new(Arc::clone(
            &server.active_publish_routes,
        )));
        server
            .active_publish_routes
            .lock()
            .unwrap()
            .insert(("live".to_string(), "victim".to_string()), conn_a.conn_id);
        let (a_end, a_peer) = UnixStream::pair().unwrap();
        a_end.set_nonblocking(true).unwrap();
        conn_a.transport = Some(Transport::new_plain(a_end.into_raw_fd()));
        // Drop the peer side so conn_a's transport.recv() observes EOF (n == 0)
        // when process_connections() reads it, simulating the publisher
        // disconnecting.
        drop(a_peer);

        // Connection B is a second, already-connected client publishing the
        // same route later in the same connections vector.
        let mut conn_b = Conn::new();
        conn_b.conn_id = 2;
        conn_b.app = "live".to_string();
        conn_b.state = ConnState::StreamCreated;
        conn_b.current_stream = Some(Box::new(Stream {
            stream_id: 1,
            name: String::new(),
            is_publishing: false,
            is_playing: false,
        }));
        conn_b.publish_routes = Some(PublishRouteRegistry::new(Arc::clone(
            &server.active_publish_routes,
        )));
        let (b_end, mut b_peer) = UnixStream::pair().unwrap();
        b_end.set_nonblocking(true).unwrap();
        conn_b.transport = Some(Transport::new_plain(b_end.into_raw_fd()));

        let mut publish_cmd = crate::buffer::Buffer::with_capacity(128);
        crate::message::command::build_publish(&mut publish_cmd, "victim", "live").unwrap();
        let payload_len = publish_cmd.available();
        let mut wire = crate::buffer::Buffer::new();
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 3;
        cmsg.fmt = 0;
        cmsg.msg_length = payload_len as u32;
        cmsg.msg_type_id = RTMP_MSG_AMF0_COMMAND;
        cmsg.msg_stream_id = 1;
        chunk_write(&mut wire, &cmsg, publish_cmd.as_slice(), payload_len, 128).unwrap();
        use std::io::Write;
        b_peer.write_all(wire.peek()).unwrap();

        server.connections = vec![conn_a, conn_b];
        server.process_connections().unwrap();

        assert_eq!(server.connections.len(), 1, "conn_a should have been removed");
        assert!(
            server.connections[0].relay_enabled,
            "conn_b's publish must succeed in the same batch conn_a's route was freed in"
        );
    }
}
