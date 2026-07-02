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
            Some(TlsCtx::new_server(
                cert.to_str().unwrap_or(""),
                key.to_str().unwrap_or(""),
            )?)
        } else {
            None
        };

        Ok(Self {
            config,
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
        self.drain_pending_cache_evictions();

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
        for conn in self.connections.iter_mut() {
            if !conn.needs_init_frames {
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
                if let Some(ref hdr) = cache.avc_header.clone() {
                    let _ = conn.send_frame(FrameType::Video, 0, hdr);
                }
                if let Some(ref hdr) = cache.aac_header.clone() {
                    let _ = conn.send_frame(FrameType::Audio, 0, hdr);
                }
                if let Some((ts, ref kf)) = cache.last_keyframe.clone() {
                    let _ = conn.send_frame(FrameType::Video, ts, kf);
                }
            }
        }

        // Update per-stream cache and relay each frame in order so players
        // receive frames in the same sequence the publisher sent them.
        for frame in &relay_frames {
            self.cache_relay_frame(frame);
            for conn in self.connections.iter_mut() {
                let is_player = conn.relay_enabled
                    && conn
                        .current_stream
                        .as_ref()
                        .map(|s| s.is_playing && conn.relay_route_key() == frame.stream_name)
                        .unwrap_or(false);
                if !is_player || conn.app != frame.app {
                    continue;
                }
                let _ = conn.send_frame(frame.frame_type, frame.timestamp, &frame.payload);
            }
        }

        // Flush all connections.
        for (i, conn) in self.connections.iter_mut().enumerate() {
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
                    self.stream_cache.remove(&key);
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
        if projected_total > MAX_STREAM_CACHE_BYTES {
            let victims: Vec<_> = self
                .stream_cache
                .keys()
                .filter(|k| *k != &key)
                .cloned()
                .collect();
            for victim in victims {
                if projected_total <= MAX_STREAM_CACHE_BYTES {
                    break;
                }
                if let Some(cache) = self.stream_cache.get(&victim) {
                    projected_total -= Self::stream_cache_entry_bytes(cache);
                }
                self.evict_stream_cache_key(&victim);
            }
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

    fn drain_pending_cache_evictions(&mut self) {
        for conn in &mut self.connections {
            for key in conn.pending_cache_evictions.drain(..) {
                self.stream_cache.remove(&key);
                if let Some(keys) = self.publisher_cache_keys.get_mut(&conn.conn_id) {
                    keys.retain(|k| k != &key);
                }
            }
        }
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
    use crate::session::conn::RelayFrame;
    use crate::session::stream::Stream;
    use crate::types::FrameType;

    fn test_config() -> ServerConfig {
        ServerConfig {
            max_connections: 0,
            chunk_size: 0,
            tls_enabled: 0,
            tls_cert_file: std::ptr::null(),
            tls_key_file: std::ptr::null(),
            tls_ca_file: std::ptr::null(),
            tls_insecure: 0,
        }
    }

    fn relay_frame(
        publisher_conn_id: u64,
        app: &str,
        stream_name: &str,
        payload: &[u8],
    ) -> RelayFrame {
        RelayFrame {
            frame_type: FrameType::Video,
            timestamp: 0,
            payload: payload.to_vec(),
            app: app.to_string(),
            stream_name: stream_name.to_string(),
            publisher_conn_id,
        }
    }

    #[test]
    fn stream_cache_evicts_pending_publisher_renames() {
        let config = test_config();
        let mut server = Server::new(config).unwrap();
        let avc = [0x17u8, 0x00, 0x00, 0x00, 0x00];

        server
            .stream_cache
            .insert(("live".to_string(), "old".to_string()), StreamCache {
                avc_header: Some(avc.to_vec()),
                aac_header: None,
                last_keyframe: None,
            });

        let mut conn = Conn::new();
        conn.conn_id = 1;
        conn.pending_cache_evictions
            .push(("live".to_string(), "old".to_string()));
        server.connections.push(conn);

        server.drain_pending_cache_evictions();

        assert!(!server.stream_cache.contains_key(&("live".to_string(), "old".to_string())));
    }

    #[test]
    fn stream_cache_is_bounded() {
        let config = test_config();
        let mut server = Server::new(config).unwrap();
        let avc = [0x17u8, 0x00, 0x00, 0x00, 0x00];

        for i in 0..=MAX_STREAM_CACHE_ENTRIES {
            let name = format!("stream{i}");
            let frames = vec![relay_frame(1, "live", &name, &avc)];
            for frame in &frames {
                server.cache_relay_frame(frame);
            }
        }

        assert!(server.stream_cache.len() <= MAX_STREAM_CACHE_ENTRIES);
    }

    #[test]
    fn disconnect_after_publish_then_create_stream_still_clears_publisher_cache_keys() {
        let config = test_config();
        let mut server = Server::new(config).unwrap();
        let avc = [0x17u8, 0x00, 0x00, 0x00, 0x00];

        // Publisher creates a cache entry...
        server.cache_relay_frame(&relay_frame(1, "live", "old_name", &avc));
        assert!(server
            .stream_cache
            .contains_key(&("live".to_string(), "old_name".to_string())));
        assert!(server.publisher_cache_keys.contains_key(&1));

        // ...then issues another createStream, replacing current_stream and
        // resetting is_publishing to false, before disconnecting.
        let mut conn = Conn::new();
        conn.conn_id = 1;
        conn.current_stream = Some(Box::new(Stream::new(2)));
        // transport left None: the recv loop in process_connections()
        // marks this connection closed immediately (transport-less
        // simulates a dead/dropped socket).
        server.connections.push(conn);

        server.process_connections().unwrap();

        assert!(
            !server.publisher_cache_keys.contains_key(&1),
            "publisher_cache_keys must be cleared on disconnect even when \
             current_stream.is_publishing is false at teardown time"
        );
        assert!(!server
            .stream_cache
            .contains_key(&("live".to_string(), "old_name".to_string())));
        assert!(server.connections.is_empty());
    }

    #[test]
    fn cache_relay_frame_skips_uncacheable_frames() {
        let config = test_config();
        let mut server = Server::new(config).unwrap();

        // Neither an AVC/AAC sequence header nor a keyframe marker.
        let non_cacheable_video = [0x27u8, 0x01, 0x00, 0x00, 0x00];
        server.cache_relay_frame(&relay_frame(1, "live", "stream1", &non_cacheable_video));

        assert!(server.stream_cache.is_empty());
        assert!(server.publisher_cache_keys.is_empty());
    }

    #[test]
    fn stream_cache_respects_byte_budget() {
        let config = test_config();
        let mut server = Server::new(config).unwrap();

        // Each entry stores a ~2 MiB "keyframe"; enough entries to exceed
        // MAX_STREAM_CACHE_BYTES well before MAX_STREAM_CACHE_ENTRIES.
        let payload_len = 2 * 1024 * 1024;
        let mut payload = vec![0u8; payload_len];
        payload[0] = 0x17;
        payload[1] = 0x01;

        for i in 0..40 {
            let name = format!("stream{i}");
            server.cache_relay_frame(&relay_frame(1, "live", &name, &payload));
        }

        assert!(server.stream_cache_bytes() <= MAX_STREAM_CACHE_BYTES);
    }

    #[test]
    fn cache_evictions_from_current_batch_apply_before_init_frame_replay() {
        use crate::buffer::Buffer;
        use crate::chunk::reader::ChunkMessage;
        use crate::chunk::writer::chunk_write;
        use crate::message::command;
        use crate::transport::Transport;
        use std::os::unix::io::IntoRawFd;
        use std::os::unix::net::UnixStream;

        let config = test_config();
        let mut server = Server::new(config).unwrap();

        // Stale cache entry under the route key the publisher is about to
        // abandon by renaming.
        let stale_avc = vec![0x17u8, 0x00, 0xAA, 0xBB, 0xCC];
        server.stream_cache.insert(
            ("live".to_string(), "old_name".to_string()),
            StreamCache {
                avc_header: Some(stale_avc),
                aac_header: None,
                last_keyframe: None,
            },
        );

        // Wire up a real socket pair and pre-load it with a chunk-encoded
        // "publish" command, so process_connections()'s recv loop reads it
        // via a real recv(2) call within this single batch.
        let (pub_srv, mut pub_cli) = UnixStream::pair().unwrap();
        pub_srv.set_nonblocking(true).unwrap();

        let mut amf = Buffer::with_capacity(128);
        command::build_publish(&mut amf, "new_name", "live").unwrap();
        let mut wire = Buffer::with_capacity(256);
        let mut cmsg = ChunkMessage::default();
        cmsg.csid = 3;
        cmsg.fmt = 0;
        cmsg.msg_length = amf.as_slice().len() as u32;
        cmsg.msg_type_id = 0x14; // AMF0 command
        cmsg.msg_stream_id = 1;
        chunk_write(&mut wire, &cmsg, amf.as_slice(), amf.as_slice().len(), 128).unwrap();
        std::io::Write::write_all(&mut pub_cli, wire.as_slice()).unwrap();

        let mut publisher = Conn::new();
        publisher.conn_id = 1;
        publisher.app = "live".to_string();
        publisher.state = ConnState::Publishing;
        publisher.current_stream = Some(Box::new(Stream::new(1)));
        if let Some(ref mut s) = publisher.current_stream {
            s.is_publishing = true;
            s.name = "old_name".to_string();
        }
        let raw_fd = pub_srv.into_raw_fd();
        publisher.client_fd = raw_fd;
        publisher.transport = Some(Transport::new_plain(raw_fd));
        server.connections.push(publisher);

        server.process_connections().unwrap();

        // The rename to "new_name" happened inside this very call's recv
        // loop; the stale "old_name" entry must already be gone by the time
        // this same call returns, not merely on the next poll.
        assert!(!server
            .stream_cache
            .contains_key(&("live".to_string(), "old_name".to_string())));

        // Keep pub_cli alive until here so the socket isn't torn down mid-test.
        drop(pub_cli);
    }
}
