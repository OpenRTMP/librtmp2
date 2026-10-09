//! Loopback integration tests for the socket backend, written to run
//! unchanged on every supported platform (Linux, macOS, Windows): plain RTMP
//! and RTMPS publish/play through a real `Server` and real `Client`s,
//! certificate verification, concurrent players, frame ordering, slow
//! consumers, resets and cleanup.

use std::cell::RefCell;
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use librtmp2::client::Client;
use librtmp2::server::Server;
use librtmp2::types::*;

fn plain_config() -> ServerConfig {
    ServerConfig {
        max_connections: 64,
        chunk_size: 4096,
        tls_enabled: 0,
        tls_cert_file: std::ptr::null(),
        tls_key_file: std::ptr::null(),
        tls_ca_file: std::ptr::null(),
        tls_insecure: 0,
        max_pending_tls_per_addr: 0,
        max_connections_per_addr: 64,
    }
}

/// FLV video body for frame `seq`: an AVC keyframe NALU header followed by
/// `seq` (big-endian) and a deterministic filler of `len` bytes in total.
fn video_body(seq: u32, len: usize) -> Vec<u8> {
    let mut body = vec![0x17, 0x01, 0x00, 0x00, 0x00];
    body.extend_from_slice(&seq.to_be_bytes());
    while body.len() < len {
        body.push((seq as usize + body.len()) as u8);
    }
    body
}

thread_local! {
    /// Bodies a player received, in callback order (one player per thread).
    static RECEIVED: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
}

fn record_frame(frame: &Frame) {
    if frame.frame_type != FrameType::Video || frame.data.is_null() || frame.size < 9 {
        return;
    }
    // SAFETY: the library guarantees `data` is valid for `size` bytes for
    // the duration of the callback.
    let body = unsafe { std::slice::from_raw_parts(frame.data, frame.size as usize) };
    RECEIVED.with(|r| r.borrow_mut().push(body.to_vec()));
}

fn received_seqs() -> Vec<u32> {
    RECEIVED.with(|r| {
        r.borrow()
            .iter()
            .map(|b| u32::from_be_bytes([b[5], b[6], b[7], b[8]]))
            .collect()
    })
}

/// Poll `server` until every client thread has reported back on `done`.
fn serve_until_done(
    server: &mut Server,
    done: &mpsc::Receiver<()>,
    threads: usize,
    limit: Duration,
) {
    let deadline = Instant::now() + limit;
    let mut finished = 0;
    while finished < threads {
        while done.try_recv().is_ok() {
            finished += 1;
        }
        assert!(Instant::now() < deadline, "clients did not finish in time");
        server.poll(1).unwrap();
    }
}

/// One publisher and `players` concurrent players exchange `frames`
/// frames over `url`; every player must get every frame, intact and in
/// order. `configure` sets up each client (e.g. TLS trust) before connect.
fn relay_round_trip(
    server: &mut Server,
    url: &'static str,
    players: usize,
    frames: u32,
    frame_len: usize,
    configure: fn(&mut Client),
) {
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (publisher_ready_tx, publisher_ready_rx) = mpsc::channel::<()>();

    let publisher = {
        let done_tx = done_tx.clone();
        thread::spawn(move || {
            let mut client = Client::new();
            configure(&mut client);
            client.connect(url).expect("publisher connect");
            client.publish().expect("publish");
            publisher_ready_tx.send(()).unwrap();
            go_rx.recv().unwrap();
            for seq in 0..frames {
                let body = video_body(seq, frame_len);
                client
                    .send_frame_payload(FrameType::Video, seq, &body)
                    .expect("send_frame");
                // Keep draining the send buffer so a full socket never
                // turns into an unbounded local backlog.
                client.poll(0).expect("publisher poll");
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while client.send_buffer.available() > 0 && Instant::now() < deadline {
                client.poll(10).expect("publisher flush");
            }
            assert_eq!(client.send_buffer.available(), 0, "publisher never flushed");
            done_tx.send(()).unwrap();
            // Stay connected until the players are done.
            client
        })
    };

    let mut player_threads = Vec::new();
    for _ in 0..players {
        let done_tx = done_tx.clone();
        let ready_tx = ready_tx.clone();
        player_threads.push(thread::spawn(move || {
            let mut client = Client::new();
            configure(&mut client);
            client.on_frame_cb = Some(record_frame);
            client.connect(url).expect("player connect");
            client.play().expect("play");
            ready_tx.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            while received_seqs().len() < frames as usize && Instant::now() < deadline {
                client.poll(20).expect("player poll");
            }
            let bodies = RECEIVED.with(std::cell::RefCell::take);
            done_tx.send(()).unwrap();
            bodies
        }));
    }

    // Publisher first, then players, then the frames.
    let setup_deadline = Instant::now() + Duration::from_secs(15);
    loop {
        server.poll(1).unwrap();
        if publisher_ready_rx.try_recv().is_ok() {
            break;
        }
        assert!(Instant::now() < setup_deadline, "publisher setup timed out");
    }
    let mut ready = 0;
    while ready < players {
        server.poll(1).unwrap();
        while ready_rx.try_recv().is_ok() {
            ready += 1;
        }
        assert!(Instant::now() < setup_deadline, "player setup timed out");
    }
    // Let the server finish enabling relay for the last player.
    let settle = Instant::now() + Duration::from_millis(100);
    while Instant::now() < settle {
        server.poll(1).unwrap();
    }
    go_tx.send(()).unwrap();
    serve_until_done(server, &done_rx, players + 1, Duration::from_secs(30));

    for player in player_threads {
        let bodies = player.join().expect("player thread");
        assert_eq!(bodies.len(), frames as usize, "player lost frames");
        for (seq, body) in bodies.iter().enumerate() {
            assert_eq!(
                body,
                &video_body(seq as u32, frame_len),
                "frame {seq} differs"
            );
        }
    }
    drop(publisher.join().expect("publisher thread"));
}

#[test]
fn plain_rtmp_relay_delivers_every_frame_in_order_to_concurrent_players() {
    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19681").unwrap();
    relay_round_trip(
        &mut server,
        "rtmp://127.0.0.1:19681/live/xplat",
        6,
        400,
        3000,
        |_| {},
    );
}

#[test]
fn closed_and_reset_connections_are_reaped() {
    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19682").unwrap();

    // A clean close mid-handshake...
    let clean = TcpStream::connect("127.0.0.1:19682").unwrap();
    // ...and an abortive one (RST via SO_LINGER=0) after a partial C0/C1.
    let reset = TcpStream::connect("127.0.0.1:19682").unwrap();
    {
        use std::io::Write;
        let mut r = &reset;
        r.write_all(&[0x03, 0, 0, 0, 0]).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.connections.len() < 2 {
        assert!(Instant::now() < deadline, "server never accepted both");
        server.poll(1).unwrap();
    }
    drop(clean);
    socket2::SockRef::from(&reset)
        .set_linger(Some(Duration::ZERO))
        .unwrap();
    drop(reset);

    // A full client session that then disconnects.
    let (done_tx, done_rx) = mpsc::channel();
    let client = thread::spawn(move || {
        let mut c = Client::new();
        c.connect("rtmp://127.0.0.1:19682/live/reap").unwrap();
        c.publish().unwrap();
        done_tx.send(()).unwrap();
        drop(c);
    });
    serve_until_done(&mut server, &done_rx, 1, Duration::from_secs(10));
    client.join().unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while !server.connections.is_empty() {
        assert!(
            Instant::now() < deadline,
            "{} connection(s) were never reaped",
            server.connections.len()
        );
        server.poll(5).unwrap();
    }
    // The listener keeps working after the churn.
    let again = TcpStream::connect("127.0.0.1:19682").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.connections.is_empty() {
        assert!(Instant::now() < deadline, "listener stopped accepting");
        server.poll(1).unwrap();
    }
    drop(again);
}

#[test]
fn a_stalled_player_does_not_block_a_fast_player() {
    // One player stops reading entirely; the publisher keeps pushing far
    // more than the stalled player's socket buffers can hold. The fast
    // player must still get the newest frame, in order, and the server
    // must stay responsive throughout.
    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19683").unwrap();
    const URL: &str = "rtmp://127.0.0.1:19683/live/slow";
    const FRAMES: u32 = 600;
    const LEN: usize = 32 * 1024;

    let (done_tx, done_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let publisher = {
        let ready_tx = ready_tx.clone();
        let done_tx = done_tx.clone();
        thread::spawn(move || {
            let mut c = Client::new();
            c.connect(URL).unwrap();
            c.publish().unwrap();
            ready_tx.send(()).unwrap();
            go_rx.recv().unwrap();
            for seq in 0..FRAMES {
                c.send_frame_payload(FrameType::Video, seq, &video_body(seq, LEN))
                    .unwrap();
                c.poll(0).unwrap();
            }
            let deadline = Instant::now() + Duration::from_secs(20);
            while c.send_buffer.available() > 0 && Instant::now() < deadline {
                c.poll(10).unwrap();
            }
            done_tx.send(()).unwrap();
            let _ = stop_rx.recv();
        })
    };
    let stalled = {
        let ready_tx = ready_tx.clone();
        thread::spawn(move || {
            let mut c = Client::new();
            c.connect(URL).unwrap();
            c.play().unwrap();
            ready_tx.send(()).unwrap();
            // Never poll again: the server's sends to us back up.
            c
        })
    };
    let fast = thread::spawn(move || {
        let mut c = Client::new();
        c.on_frame_cb = Some(record_frame);
        c.connect(URL).unwrap();
        c.play().unwrap();
        ready_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while received_seqs().last() != Some(&(FRAMES - 1)) && Instant::now() < deadline {
            c.poll(20).unwrap();
        }
        let seqs = received_seqs();
        done_tx.send(()).unwrap();
        seqs
    });

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut ready = 0;
    while ready < 3 {
        server.poll(1).unwrap();
        while ready_rx.try_recv().is_ok() {
            ready += 1;
        }
        assert!(Instant::now() < deadline, "setup timed out");
    }
    let settle = Instant::now() + Duration::from_millis(100);
    while Instant::now() < settle {
        server.poll(1).unwrap();
    }
    go_tx.send(()).unwrap();
    serve_until_done(&mut server, &done_rx, 2, Duration::from_secs(40));
    stop_tx.send(()).unwrap();

    let seqs = fast.join().unwrap();
    assert_eq!(
        seqs.last(),
        Some(&(FRAMES - 1)),
        "fast player missed the newest frame"
    );
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "frames arrived out of order"
    );
    publisher.join().unwrap();
    drop(stalled.join().unwrap());
}

#[cfg(feature = "tls")]
mod rtmps {
    use super::*;
    use openssl::asn1::Asn1Time;
    use openssl::bn::{BigNum, MsbOption};
    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;
    use openssl::x509::extension::{BasicConstraints, SubjectAlternativeName};
    use openssl::x509::{X509, X509NameBuilder};
    use std::path::PathBuf;
    use std::sync::OnceLock;

    /// A self-signed CA-capable certificate valid for `127.0.0.1` (IP SAN)
    /// and `localhost`, written once per test binary.
    fn cert_files() -> &'static (PathBuf, PathBuf) {
        static FILES: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
        FILES.get_or_init(|| {
            let pkey = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
            let mut name = X509NameBuilder::new().unwrap();
            name.append_entry_by_text("CN", "localhost").unwrap();
            let name = name.build();
            let mut b = X509::builder().unwrap();
            b.set_version(2).unwrap();
            let mut sn = BigNum::new().unwrap();
            sn.rand(64, MsbOption::MAYBE_ZERO, false).unwrap();
            b.set_serial_number(&sn.to_asn1_integer().unwrap()).unwrap();
            b.set_subject_name(&name).unwrap();
            b.set_issuer_name(&name).unwrap();
            b.set_pubkey(&pkey).unwrap();
            b.set_not_before(&Asn1Time::days_from_now(0).unwrap())
                .unwrap();
            b.set_not_after(&Asn1Time::days_from_now(1).unwrap())
                .unwrap();
            b.append_extension(BasicConstraints::new().critical().ca().build().unwrap())
                .unwrap();
            let san = SubjectAlternativeName::new()
                .dns("localhost")
                .ip("127.0.0.1")
                .build(&b.x509v3_context(None, None))
                .unwrap();
            b.append_extension(san).unwrap();
            b.sign(&pkey, MessageDigest::sha256()).unwrap();
            let cert = b.build();
            let base = std::env::temp_dir().join(format!("librtmp2-xplat-{}", std::process::id()));
            let cert_path = base.with_extension("cert.pem");
            let key_path = base.with_extension("key.pem");
            std::fs::write(&cert_path, cert.to_pem().unwrap()).unwrap();
            std::fs::write(&key_path, pkey.private_key_to_pem_pkcs8().unwrap()).unwrap();
            (cert_path, key_path)
        })
    }

    fn tls_server(port: u16) -> Server {
        let (cert, key) = cert_files();
        let mut server = Server::new(plain_config()).unwrap();
        server
            .listen_tls(
                &format!("127.0.0.1:{port}"),
                cert.to_str().unwrap(),
                key.to_str().unwrap(),
            )
            .unwrap();
        server
    }

    fn trust_test_ca(client: &mut Client) {
        let (cert, _) = cert_files();
        client.set_tls_client_config(Some(cert.to_str().unwrap().to_string()), false);
    }

    #[test]
    fn rtmps_relay_with_verified_certificate_delivers_every_frame_in_order() {
        let mut server = tls_server(19691);
        relay_round_trip(
            &mut server,
            "rtmps://127.0.0.1:19691/live/xplat-tls",
            3,
            300,
            3000,
            trust_test_ca,
        );
    }

    /// Run `connect` against a polled TLS server and return its result.
    fn connect_result(
        server: &mut Server,
        configure: fn(&mut Client),
        url: &'static str,
    ) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let t = thread::spawn(move || {
            let mut c = Client::new();
            c.set_connect_timeout(Duration::from_secs(5));
            configure(&mut c);
            let r = c.connect(url);
            tx.send(r).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let result = loop {
            if let Ok(r) = rx.try_recv() {
                break r;
            }
            assert!(Instant::now() < deadline, "connect never returned");
            server.poll(1).unwrap();
        };
        t.join().unwrap();
        result
    }

    #[test]
    fn rtmps_rejects_an_untrusted_certificate_by_default() {
        let mut server = tls_server(19692);
        let r = connect_result(&mut server, |_| {}, "rtmps://127.0.0.1:19692/live/x");
        assert_eq!(r, Err(ErrorCode::Handshake));
    }

    #[test]
    fn rtmps_rejects_a_hostname_the_certificate_does_not_cover() {
        // Trusted CA, wrong name: 127.0.0.2 is not in the certificate's
        // SANs, so hostname/IP verification must fail.
        let (cert, key) = cert_files();
        let mut server = Server::new(plain_config()).unwrap();
        server
            .listen_tls(
                "0.0.0.0:19693",
                cert.to_str().unwrap(),
                key.to_str().unwrap(),
            )
            .unwrap();
        // 127.0.0.2 is loopback on Linux and Windows but not routed by
        // default on macOS; skip only the connect there.
        if TcpStream::connect_timeout(&"127.0.0.2:19693".parse().unwrap(), Duration::from_secs(1))
            .is_err()
        {
            return;
        }
        let r = connect_result(&mut server, trust_test_ca, "rtmps://127.0.0.2:19693/live/x");
        assert_eq!(r, Err(ErrorCode::Handshake));
    }

    #[test]
    fn rtmps_insecure_mode_skips_verification() {
        let mut server = tls_server(19694);
        let r = connect_result(
            &mut server,
            |c| c.set_tls_client_config(None, true),
            "rtmps://127.0.0.1:19694/live/x",
        );
        assert_eq!(r, Ok(()));
    }

    #[test]
    fn a_silent_peer_on_the_tls_listener_does_not_block_other_clients() {
        // A TCP peer that never sends a ClientHello must not stall the
        // accept loop: a real RTMPS client connects while it sits there.
        let mut server = tls_server(19695);
        let silent = TcpStream::connect("127.0.0.1:19695").unwrap();
        let r = connect_result(&mut server, trust_test_ca, "rtmps://127.0.0.1:19695/live/x");
        assert_eq!(r, Ok(()));
        drop(silent);
    }
}
