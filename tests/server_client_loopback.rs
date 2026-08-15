//! End-to-end smoke test for the real TCP socket glue: a `Server` listens on
//! loopback, a `Client` connects, performs the RTMP handshake + connect +
//! createStream + publish exchange over real sockets, and sends one video
//! frame that the server's `on_frame_cb` should observe.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use librtmp2::client::Client;
use librtmp2::server::Server;
use librtmp2::types::*;

static FRAMES_RECEIVED: AtomicUsize = AtomicUsize::new(0);

const SENT_FRAME_BYTE: u8 = 0xAB;
const SENT_FRAME_LEN: usize = 32;

fn on_frame(frame: &Frame) {
    if frame.size as usize == SENT_FRAME_LEN {
        FRAMES_RECEIVED.fetch_add(1, Ordering::SeqCst);
    }
}

fn allow_publish(_conn_id: u64, _app: &str, _stream_name: &str) -> bool {
    true
}

fn plain_config() -> ServerConfig {
    ServerConfig {
        max_connections: 8,
        chunk_size: 128,
        tls_enabled: 0,
        tls_cert_file: std::ptr::null(),
        tls_key_file: std::ptr::null(),
        tls_ca_file: std::ptr::null(),
        tls_insecure: 0,
        max_pending_tls_per_addr: 0,
        max_connections_per_addr: 0,
    }
}

#[test]
fn server_client_publish_over_real_sockets() {
    FRAMES_RECEIVED.store(0, Ordering::SeqCst);

    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19661").unwrap();
    server.on_frame_cb = Some(on_frame);
    server.on_publish_cb = Some(allow_publish);

    let (setup_tx, setup_rx) = std::sync::mpsc::channel();
    let client_thread = thread::spawn(move || {
        let mut client = Client::new();
        let result = (|| -> std::result::Result<(), librtmp2::types::ErrorCode> {
            client.connect("rtmp://127.0.0.1:19661/live/stream1")?;
            client.publish()?;

            let data = [SENT_FRAME_BYTE; SENT_FRAME_LEN];
            let frame = Frame {
                frame_type: FrameType::Video,
                timestamp: 0,
                composition_time: 0,
                size: data.len() as u32,
                data: data.as_ptr(),
                audio_codec: AudioCodec::default(),
                audio_sample_rate: 0,
                audio_channels: 0,
                audio_bit_depth: 0,
                audio_fourcc: FourCc::default(),
                video_codec: VideoCodec::H264,
                video_fourcc: FourCc::default(),
                video_frame_type: 1,
                is_metadata: 0,
                track_id: u8::MAX,
            };
            client.send_frame(&frame)?;
            Ok(())
        })();
        let _ = setup_tx.send(result.is_ok());
        result.unwrap();
        thread::sleep(Duration::from_millis(200));
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(setup_ok) = setup_rx.try_recv() {
            assert!(setup_ok, "client setup failed");
        }
        if FRAMES_RECEIVED.load(Ordering::SeqCst) > 0 || Instant::now() >= deadline {
            break;
        }
        server.poll(20).unwrap();
    }

    client_thread.join().unwrap();
    assert!(
        FRAMES_RECEIVED.load(Ordering::SeqCst) > 0,
        "server never observed the published frame"
    );
}

static OBSERVED_CONN_ID: AtomicU64 = AtomicU64::new(0);

fn record_conn_id(conn_id: u64, _app: &str, _stream_name: &str) -> bool {
    OBSERVED_CONN_ID.store(conn_id, Ordering::SeqCst);
    true
}

/// `Server::set_conn_id_base` is what lets an integrator run two `Server`
/// instances (e.g. plaintext RTMP + RTMPS) in one process without their
/// auto-assigned `conn_id`s colliding. Verify the first connection accepted
/// after calling it actually gets the configured base rather than 1.
#[test]
fn set_conn_id_base_offsets_first_assigned_conn_id() {
    OBSERVED_CONN_ID.store(0, Ordering::SeqCst);
    const BASE: u64 = 1 << 40;

    let mut server = Server::new(plain_config()).unwrap();
    server.set_conn_id_base(BASE);
    server.listen("127.0.0.1:19662").unwrap();
    server.on_publish_cb = Some(record_conn_id);

    let (setup_tx, setup_rx) = std::sync::mpsc::channel();
    let client_thread = thread::spawn(move || {
        let mut client = Client::new();
        let result = (|| -> std::result::Result<(), librtmp2::types::ErrorCode> {
            client.connect("rtmp://127.0.0.1:19662/live/stream1")?;
            client.publish()?;
            Ok(())
        })();
        let _ = setup_tx.send(result.is_ok());
        result.unwrap();
        thread::sleep(Duration::from_millis(200));
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(setup_ok) = setup_rx.try_recv() {
            assert!(setup_ok, "client setup failed");
        }
        if OBSERVED_CONN_ID.load(Ordering::SeqCst) != 0 || Instant::now() >= deadline {
            break;
        }
        server.poll(20).unwrap();
    }

    client_thread.join().unwrap();
    assert_eq!(
        OBSERVED_CONN_ID.load(Ordering::SeqCst),
        BASE,
        "first accepted connection should have conn_id == configured base"
    );
}

#[test]
#[should_panic(expected = "conn_id base must be non-zero")]
fn set_conn_id_base_rejects_zero() {
    let mut server = Server::new(plain_config()).unwrap();
    server.set_conn_id_base(0);
}

#[test]
#[should_panic(expected = "external publisher id range")]
fn set_conn_id_base_rejects_high_bit() {
    let mut server = Server::new(plain_config()).unwrap();
    server.set_conn_id_base(1u64 << 63);
}

#[test]
fn listener_fds_exposes_every_bound_listener_and_stop_clears_them() {
    let mut server = Server::new(plain_config()).unwrap();
    assert!(server.tls_ctx.is_none());

    server.listen("127.0.0.1:0").unwrap();
    server.listen("127.0.0.1:0").unwrap();

    let fds = server.listener_fds();
    assert_eq!(fds.len(), 2, "each listen() call should expose its fd");
    assert!(fds.iter().all(|fd| *fd >= 0));
    assert_ne!(fds[0], fds[1], "listeners must expose distinct fds");
    assert_eq!(server.server_fd, fds[0]);

    server.stop();
    assert_eq!(server.server_fd, -1);
    assert!(server.listener_fds().is_empty());
}

static PLAYER_FRAMES_RECEIVED: AtomicUsize = AtomicUsize::new(0);

fn on_player_frame(frame: &Frame) {
    if frame.size as usize == SENT_FRAME_LEN {
        PLAYER_FRAMES_RECEIVED.fetch_add(1, Ordering::SeqCst);
    }
}

/// A publisher on one listener and a player on a *different* listener of the
/// same `Server` must still relay to each other — the whole point of binding
/// multiple listeners (e.g. plaintext RTMP + RTMPS) on one `Server` instead
/// of running two separate `Server`s is that they share one relay/connection
/// list. Running two separate `Server`s instead would silently drop this
/// cross-listener case, since each would only relay within its own
/// `connections`.
#[test]
fn publisher_and_player_relay_across_different_listeners() {
    PLAYER_FRAMES_RECEIVED.store(0, Ordering::SeqCst);

    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19663").unwrap();
    server.listen("127.0.0.1:19664").unwrap();

    let (setup_tx, setup_rx) = std::sync::mpsc::channel();
    let client_thread = thread::spawn(move || {
        let result = (|| -> std::result::Result<(), librtmp2::types::ErrorCode> {
            let mut publisher = Client::new();
            publisher.connect("rtmp://127.0.0.1:19663/live/relaytest")?;
            publisher.publish()?;

            let mut player = Client::new();
            player.on_frame_cb = Some(on_player_frame);
            player.connect("rtmp://127.0.0.1:19664/live/relaytest")?;
            player.play()?;

            // Give the server a moment to process the play authorization and
            // enable relay for this connection before the frame is sent.
            thread::sleep(Duration::from_millis(100));

            let data = [SENT_FRAME_BYTE; SENT_FRAME_LEN];
            let frame = Frame {
                frame_type: FrameType::Video,
                timestamp: 0,
                composition_time: 0,
                size: data.len() as u32,
                data: data.as_ptr(),
                audio_codec: AudioCodec::default(),
                audio_sample_rate: 0,
                audio_channels: 0,
                audio_bit_depth: 0,
                audio_fourcc: FourCc::default(),
                video_codec: VideoCodec::H264,
                video_fourcc: FourCc::default(),
                video_frame_type: 1,
                is_metadata: 0,
                track_id: u8::MAX,
            };
            publisher.send_frame(&frame)?;

            let deadline = Instant::now() + Duration::from_secs(5);
            while PLAYER_FRAMES_RECEIVED.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
                player.poll(50)?;
            }
            Ok(())
        })();
        let _ = setup_tx.send(result.is_ok());
        result.unwrap();
    });

    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        if let Ok(setup_ok) = setup_rx.try_recv() {
            assert!(setup_ok, "publisher/player setup failed");
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        server.poll(20).unwrap();
    }

    client_thread.join().unwrap();
    assert!(
        PLAYER_FRAMES_RECEIVED.load(Ordering::SeqCst) > 0,
        "player on a different listener never received the publisher's frame \
         — relay must be shared across every listener on one Server"
    );
}

static RECONNECT_CONN_ID: AtomicU64 = AtomicU64::new(0);
static RECONNECT_RECEIVED: AtomicUsize = AtomicUsize::new(0);
static RECONNECT_TC_URL: Mutex<Option<String>> = Mutex::new(None);
static RECONNECT_DESCRIPTION: Mutex<Option<String>> = Mutex::new(None);

fn record_play_conn_id(conn_id: u64, _app: &str, _stream_name: &str) -> bool {
    RECONNECT_CONN_ID.store(conn_id, Ordering::SeqCst);
    true
}

fn on_reconnect_request(tc_url: Option<&str>, description: Option<&str>) {
    *RECONNECT_TC_URL.lock().unwrap() = tc_url.map(str::to_string);
    *RECONNECT_DESCRIPTION.lock().unwrap() = description.map(str::to_string);
    RECONNECT_RECEIVED.fetch_add(1, Ordering::SeqCst);
}

/// `Server::request_reconnect` must reach the client as a
/// `NetConnection.Connect.ReconnectRequest` `onStatus` and fire
/// `on_reconnect_request_cb` with the `tcUrl` the server sent.
#[test]
fn server_request_reconnect_reaches_client_callback() {
    RECONNECT_CONN_ID.store(0, Ordering::SeqCst);
    RECONNECT_RECEIVED.store(0, Ordering::SeqCst);
    *RECONNECT_TC_URL.lock().unwrap() = None;
    *RECONNECT_DESCRIPTION.lock().unwrap() = None;

    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19665").unwrap();
    server.on_play_cb = Some(record_play_conn_id);

    let (setup_tx, setup_rx) = std::sync::mpsc::channel();
    let client_thread = thread::spawn(move || {
        let result = (|| -> std::result::Result<(), librtmp2::types::ErrorCode> {
            let mut player = Client::new();
            player.on_reconnect_request_cb = Some(on_reconnect_request);
            player.connect("rtmp://127.0.0.1:19665/live/reconnecttest")?;
            player.play()?;

            let deadline = Instant::now() + Duration::from_secs(5);
            while RECONNECT_RECEIVED.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
                player.poll(50)?;
            }
            Ok(())
        })();
        let _ = setup_tx.send(result.is_ok());
        result.unwrap();
    });

    let mut reconnect_sent = false;
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        if let Ok(setup_ok) = setup_rx.try_recv() {
            assert!(setup_ok, "player setup failed");
            break;
        }
        if !reconnect_sent && RECONNECT_CONN_ID.load(Ordering::SeqCst) != 0 {
            server
                .request_reconnect(
                    RECONNECT_CONN_ID.load(Ordering::SeqCst),
                    Some("rtmp://backup.example.com/live"),
                    Some("moving you to a healthier node"),
                )
                .unwrap();
            reconnect_sent = true;
        }
        if Instant::now() >= deadline {
            break;
        }
        server.poll(20).unwrap();
    }

    client_thread.join().unwrap();
    assert!(
        RECONNECT_RECEIVED.load(Ordering::SeqCst) > 0,
        "client never observed the server's reconnect request"
    );
    assert_eq!(
        RECONNECT_TC_URL.lock().unwrap().as_deref(),
        Some("rtmp://backup.example.com/live")
    );
    assert_eq!(
        RECONNECT_DESCRIPTION.lock().unwrap().as_deref(),
        Some("moving you to a healthier node")
    );
}

fn noop_reconnect_request(_tc_url: Option<&str>, _description: Option<&str>) {}

fn connect_and_wait(
    server: &mut Server,
    port: u16,
    app_stream: &str,
    with_reconnect_cb: bool,
) -> NegotiatedCaps {
    let (setup_tx, setup_rx) = std::sync::mpsc::channel();
    let url = format!("rtmp://127.0.0.1:{port}/{app_stream}");
    // Generous: on a contended CI runner (shared with every other test
    // binary's threads under `cargo test`'s default full parallelism) even
    // a purely-local loopback connect + createStream round trip can take
    // several seconds of wall-clock time despite doing very little real
    // work, so this needs real margin over the "should be instant" case the
    // other, older tests in this file were written against.
    let client_timeout = Duration::from_secs(20);
    let client_thread = thread::spawn(move || {
        // `client` must stay alive (and its transport connected) past the
        // point where the main thread observes success and inspects
        // `server.connections` -- dropping it right after `connect()`
        // returns closes the TCP connection immediately, racing the
        // server's own bookkeeping and intermittently making the
        // connection vanish from `server.connections` before the caller
        // ever gets to look at it.
        let mut client = Client::new();
        client.set_connect_timeout(client_timeout);
        if with_reconnect_cb {
            client.on_reconnect_request_cb = Some(noop_reconnect_request);
        }
        let result = client.connect(&url);
        let _ = setup_tx.send(result.is_ok());
        result.unwrap();
        thread::sleep(Duration::from_millis(200));
    });

    // Keep servicing the server side until the client thread itself reports
    // it finished `connect()` (the authoritative signal -- not an inferred
    // server-side connection state, which can lag behind or race what the
    // client is actually waiting on) or clearly hung past its own timeout.
    let deadline = Instant::now() + client_timeout + Duration::from_secs(5);
    loop {
        if let Ok(setup_ok) = setup_rx.try_recv() {
            assert!(setup_ok, "client setup failed");
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        server.poll(20).unwrap();
    }

    client_thread.join().unwrap();
    assert_eq!(server.connections.len(), 1);
    server.connections[0].negotiated_caps.clone()
}

/// `Client::connect` must advertise E-RTMP v2 reconnect support (`capsEx`
/// reconnect bit + a `reconnect` value) when the host has registered
/// `on_reconnect_request_cb` -- otherwise a spec-compliant server would have
/// no reason to ever send it a `ReconnectRequest`.
#[test]
fn client_with_reconnect_callback_advertises_reconnect_capability() {
    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19666").unwrap();
    let caps = connect_and_wait(&mut server, 19666, "live/with_cb", true);
    assert!(caps.has_reconnect);
    assert!(
        caps.caps_ex_mask & CAPS_EX_MASK_RECONNECT != 0,
        "the negotiated caps_ex_mask must include the reconnect bit"
    );
}

/// A client that never wired up `on_reconnect_request_cb` must not claim it
/// can handle a reconnect request.
#[test]
fn client_without_reconnect_callback_does_not_advertise_reconnect_capability() {
    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19667").unwrap();
    let caps = connect_and_wait(&mut server, 19667, "live/without_cb", false);
    assert!(
        !caps.has_reconnect,
        "a client without on_reconnect_request_cb must not advertise reconnect support"
    );
}
