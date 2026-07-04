//! E2E: publisher and player on the same listener relay frames to each other.

mod common;

use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use librtmp2::client::Client;
use librtmp2::server::Server;
use librtmp2::types::ErrorCode;

use common::{
    allow_play, allow_publish, make_video_frame, on_video_frame, plain_config, poll_until,
    run_client_with_server, FRAMES_RECEIVED, SENT_FRAME_BYTE, SENT_FRAME_LEN,
};

#[test]
fn publisher_and_player_relay_on_same_listener() {
    FRAMES_RECEIVED.store(0, Ordering::SeqCst);

    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19665").unwrap();
    server.on_publish_cb = Some(allow_publish);
    server.on_play_cb = Some(allow_play);

    run_client_with_server(&mut server, || {
        let mut publisher = Client::new();
        publisher.connect("rtmp://127.0.0.1:19665/live/relay-same")?;
        publisher.publish()?;

        let mut player = Client::new();
        player.on_frame_cb = Some(on_video_frame);
        player.connect("rtmp://127.0.0.1:19665/live/relay-same")?;
        player.play()?;

        thread::sleep(Duration::from_millis(100));

        let (_data, frame) = make_video_frame(0, SENT_FRAME_BYTE);
        publisher.send_frame(&frame)?;

        let deadline = Instant::now() + Duration::from_secs(5);
        while FRAMES_RECEIVED.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            player.poll(50)?;
        }
        Ok::<(), ErrorCode>(())
    });

    poll_until(
        &mut server,
        Instant::now() + Duration::from_millis(500),
        || FRAMES_RECEIVED.load(Ordering::SeqCst) > 0,
    );

    assert!(
        FRAMES_RECEIVED.load(Ordering::SeqCst) > 0,
        "player never received the publisher's frame on the same listener"
    );
}
