//! RTMP server listener
//!
//! Mirrors `src/server/server.h` and `src/server/server.c`.

use std::collections::HashMap;
use std::net::TcpListener;
use std::os::unix::io::{AsRawFd, IntoRawFd};

use crate::chunk::state::DEFAULT_CHUNK_SIZE;
use crate::net;
use crate::session::conn::Conn;
use crate::transport::{TlsCtx, Transport};
use crate::types::*;

/// Maximum distinct (app, stream_name) cache entries retained server-wide.
const MAX_STREAM_CACHE_ENTRIES: usize = 1024;

/// Maximum total bytes retained across all stream_cache entries server-wide.
const MAX_STREAM_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Cached codec headers and last keyframe for a (app, stream_name) pair.
/// Replayed to players that join after the publisher has already sent headers.
struct StreamCache {
    avc_header: Option<Vec<u8>>,
    aac_header: Option<Vec<u8>>,
    /// (timestamp, payload) of the most recent IDR keyframe.
    last_keyframe: Option<(u32, Vec<u8>)>,
}

/// Server object.
pub struct Server {
    pub config: ServerConfig,
    pub resource_limits: ResourceLimits,
    pub running: bool,
    pub server_fd: i32,
    pub connections: Vec<Conn>,
    pub tls_ctx: Option<TlsCtx>,
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
    listener: Option<TcpListener>,
    stream_cache: HashMap<(String, String), StreamCache>,
    /// Cache keys created by each publisher connection (for teardown).
    publisher_cache_keys: HashMap<u64, Vec<(String, String)>>,
    next_conn_id: u64,
    /// Hold media relay until the integrator enables it per connection.
    pub defer_media_relay: bool,
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
            tls_ctx,
            on_frame_cb: None,
            on_connect_cb: None,
            on_publish_cb: None,
            on_play_cb: None,
            on_media_cb: None,
            listener: None,
            stream_cache: HashMap::new(),
            publisher_cache_keys: HashMap::new(),
            next_conn_id: 1,
            defer_media_relay: false,
        })
    }

    /// Set the starting value used for auto-generated connection IDs.
    ///
    /// Each `Server` instance numbers its own connections starting at 1, so
    /// two instances running side by side in the same process (e.g. a
    /// plaintext RTMP listener and an RTMPS listener sharing one integrator
    /// callback/state layer keyed by `conn_id`) would otherwise hand out
    /// colliding IDs. Call this right after [`Server::new`] with disjoint
    /// ranges per instance to keep `conn_id` globally unique.
    pub fn set_conn_id_base(&mut self, base: u64) {
        debug_assert!(
            self.connections.is_empty(),
            "set_conn_id_base should be called before accepting any connections"
        );
        debug_assert!(
            base != 0,
            "set_conn_id_base(0) would collide with Conn::new's unset conn_id sentinel"
        );
        self.next_conn_id = base;
    }

    /// Start listening on the given address ("host:port", default port 1935).
    pub fn listen(&mut self, bind_addr: &str) -> Result<()> {
        let mut host = String::new();
        let mut port = String::new();
        net::split_host_port(bind_addr, &mut host, &mut port, "1935")?;
        let addr = if host.is_empty() {
            format!("0.0.0.0:{port}")
        } else if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };

        let listener = TcpListener::bind(&addr).map_err(|_| ErrorCode::Io)?;
        listener.set_nonblocking(true).map_err(|_| ErrorCode::Io)?;

        self.server_fd = listener.as_raw_fd();
        self.listener = Some(listener);
        self.running = true;
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
        self.listener = None;
    }

    /// Accept any pending inbound connections (non-blocking).
    fn accept_new_connections(&mut self) {
        let Some(listener) = self.listener.as_ref() else {
            return;
        };
        loop {
            if self.config.max_connections > 0
                && self.connections.len() >= self.config.max_connections as usize
            {
                break;
            }
            match listener.accept() {
                Ok((stream, addr)) => {
                    let transport = if let Some(ref ctx) = self.tls_ctx {
                        // TlsCtx::accept() takes ownership of the fd, sets the socket
                        // to blocking for the handshake, then restores non-blocking.
                        // On error the fd is already closed inside accept(); skip the conn.
                        match ctx.accept(stream.into_raw_fd()) {
                            Ok(t) => t,
                            Err(_) => continue,
                        }
                    } else {
                        let _ = stream.set_nonblocking(true);
                        Transport::new_plain(stream.into_raw_fd())
                    };
                    let conn_fd = transport.fd();
                    let mut conn = Conn::new();
                    conn.chunk_reg.max_reassembly_bytes =
                        self.resource_limits.max_reassembly_bytes;
                    conn.max_pending_relay_bytes = self.resource_limits.max_pending_relay_bytes;
                    // Outbound chunk size only: peers start sending at the RTMP
                    // default (128) until SetChunkSize is negotiated.
                    conn.chunk_size = if self.config.chunk_size > 0 {
                        self.config.chunk_size as u32
                    } else {
                        DEFAULT_CHUNK_SIZE
                    };
                    conn.client_fd = conn_fd;
                    conn.conn_id = self.next_conn_id;
                    self.next_conn_id = self.next_conn_id.saturating_add(1);
                    conn.remote_addr = addr.to_string();
                    conn.defer_media_relay = self.defer_media_relay;
                    conn.transport = Some(transport);
                    conn.on_frame_cb = self.on_frame_cb;
                    conn.on_media_cb = self.on_media_cb;
                    conn.on_connect_cb = self.on_connect_cb;
                    conn.on_publish_cb = self.on_publish_cb;
                    conn.on_play_cb = self.on_play_cb;
                    self.connections.push(conn);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
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
        for (i, conn) in self.connections.iter_mut().enumerate() {
            loop {
                let Some(transport) = conn.transport.as_mut() else {
                    closed.push(i);
                    break;
                };
                let mut again = 0i32;
                let n = transport.recv(&mut buf, &mut again);
                if n > 0 {
                    if conn.recv(&buf[..n as usize]).is_err() {
                        closed.push(i);
                        break;
                    }
                } else if n == 0 {
                    closed.push(i);
                    break;
                } else if again != 0 {
                    break;
                } else {
                    closed.push(i);
                    break;
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
                    conn.transport = None;
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
                if conn.send_frame(frame.frame_type, frame.timestamp, &frame.payload).is_err() {
                    // Player stopped reading; outbound send_buffer is full.
                    // Drop the connection immediately so later relay frames in
                    // this poll batch skip it and no more socket work is done.
                    conn.relay_enabled = false;
                    conn.needs_init_frames = false;
                    conn.transport = None;
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
    fn drain_pending_cache_evictions(&mut self) -> std::collections::HashSet<(String, String, u64)> {
        let mut abandoned = std::collections::HashSet::new();
        for conn in &mut self.connections {
            for key in conn.pending_cache_evictions.drain(..) {
                // Record the raw request regardless of ownership outcome
                // below: a connection's own frame, queued under this key
                // earlier in the same batch (before this rename/abandon was
                // processed), must never resurrect the entry -- whether or
                // not publisher_cache_keys already reflects ownership yet.
                // Scoped by conn_id so this doesn't suppress a *different*
                // publisher's legitimate frame for the same (app, name).
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
