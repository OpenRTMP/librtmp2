from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}")
    file_path.write_text(text.replace(old, new, 1))


replace_once(
    "src/server/mod.rs",
    """/// Cap total relay sends issued while fanning publisher media out to
/// players in one `process_connections` pass. Without this, a full
/// `pending_relay` batch multiplied by every playing connection can
/// monopolize the single-threaded poll loop.
const MAX_RELAY_SENDS_PER_POLL: usize = 4096;
""",
    """/// Default cap on relay sends issued while fanning publisher media out to
/// players in one `process_connections` pass. Integrators can tune the active
/// value through [`Server::max_relay_sends_per_poll`].
pub const DEFAULT_MAX_RELAY_SENDS_PER_POLL: usize = 4096;
""",
)

replace_once(
    "src/server/mod.rs",
    """pub struct Server {
    pub config: ServerConfig,
    pub resource_limits: ResourceLimits,
    pub running: bool,
""",
    """pub struct Server {
    pub config: ServerConfig,
    pub resource_limits: ResourceLimits,
    /// Maximum relay send operations allowed in one `process_connections` pass.
    ///
    /// The first eligible frame in a pass is always relayed even when its fan-out
    /// exceeds this value, which guarantees forward progress instead of
    /// perpetually re-queueing an oversized frame. Later frames are deferred once
    /// the accumulated send count would exceed the configured budget.
    pub max_relay_sends_per_poll: usize,
    pub running: bool,
""",
)

replace_once(
    "src/server/mod.rs",
    """        Ok(Self {
            config,
            resource_limits: ResourceLimits::default(),
            running: false,
""",
    """        Ok(Self {
            config,
            resource_limits: ResourceLimits::default(),
            max_relay_sends_per_poll: DEFAULT_MAX_RELAY_SENDS_PER_POLL,
            running: false,
""",
)

replace_once(
    "src/server/mod.rs",
    """            if player_count > 0
                && relay_sends.saturating_add(player_count) > MAX_RELAY_SENDS_PER_POLL
            {
                break;
            }
""",
    """            if player_count > 0
                && relay_sends > 0
                && relay_sends.saturating_add(player_count) > self.max_relay_sends_per_poll
            {
                break;
            }
""",
)

replace_once(
    "src/server/mod.rs",
    """    #[test]
    fn relay_send_budget_limits_worst_case_player_fan_out() {
        // One publisher can queue 1024 frames; with 256 connections that
        // could be 261_120 sends per poll without a relay budget.
        let worst_case = 1024 * 256;
        assert!(
            MAX_RELAY_SENDS_PER_POLL < worst_case / 10,
            "relay budget should be well below unbounded fan-out"
        );
    }

""",
    """    #[test]
    fn relay_send_budget_limits_worst_case_player_fan_out() {
        // One publisher can queue 1024 frames; with 256 connections that
        // could be 261_120 sends per poll without a relay budget.
        let worst_case = 1024 * 256;
        let server = test_server();
        assert_eq!(
            server.max_relay_sends_per_poll,
            DEFAULT_MAX_RELAY_SENDS_PER_POLL
        );
        assert!(
            server.max_relay_sends_per_poll < worst_case / 10,
            "relay budget should be well below unbounded fan-out"
        );
    }

    #[test]
    fn oversized_relay_fan_out_still_makes_progress_each_poll() {
        use crate::session::stream::Stream;
        use crate::transport::Transport;
        use std::os::unix::io::IntoRawFd;
        use std::os::unix::net::UnixStream;

        fn attached_conn(conn_id: u64, publishing: bool) -> (Conn, UnixStream) {
            let (server_end, peer_end) = UnixStream::pair().unwrap();
            server_end.set_nonblocking(true).unwrap();
            peer_end.set_nonblocking(true).unwrap();

            let mut conn = Conn::new();
            conn.conn_id = conn_id;
            conn.app = "live".to_string();
            conn.relay_enabled = true;
            conn.transport = Some(Transport::new_plain(server_end.into_raw_fd()));
            conn.current_stream = Some(Box::new(Stream {
                stream_id: 1,
                name: "stream".to_string(),
                is_publishing: publishing,
                is_playing: !publishing,
                paused: false,
                receive_audio: true,
                receive_video: true,
            }));
            (conn, peer_end)
        }

        let mut server = test_server();
        server.max_relay_sends_per_poll = 1;

        let (mut publisher, _publisher_peer) = attached_conn(1, true);
        publisher
            .pending_relay
            .push(relay_frame(FrameType::Video, vec![0x17, 0x01, 0xAA]));
        publisher
            .pending_relay
            .push(relay_frame(FrameType::Video, vec![0x27, 0x01, 0xBB]));
        let (player_a, _player_a_peer) = attached_conn(2, false);
        let (player_b, _player_b_peer) = attached_conn(3, false);
        server.connections = vec![publisher, player_a, player_b];

        server.process_connections().unwrap();
        assert_eq!(
            server.connections[0].pending_relay.len(),
            1,
            "the first oversized fan-out frame must be relayed instead of re-queuing the whole batch"
        );

        server.process_connections().unwrap();
        assert!(
            server.connections[0].pending_relay.is_empty(),
            "the deferred frame must make progress on the next poll"
        );
    }

""",
)

replace_once(
    "CHANGELOG.md",
    """### Changed
- Connect AMF helpers now parse and write E-RTMP v2 capability representations,
""",
    """### Changed
- The built-in relay fan-out budget is configurable through
  `Server::max_relay_sends_per_poll` (default: 4096 sends per poll). The first
  eligible frame in a poll is always processed even when its audience exceeds
  the budget, preventing an oversized fan-out frame from being re-queued forever.
- Connect AMF helpers now parse and write E-RTMP v2 capability representations,
""",
)
