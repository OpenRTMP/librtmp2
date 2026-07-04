//! E2E: audio frames published over real sockets reach the server callback.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use librtmp2::client::Client;
use librtmp2::server::Server;
use librtmp2::types::FrameType;

use common::{allow_publish, make_audio_frame, plain_config, poll_until, run_client_with_server};

static AUDIO_FRAMES: AtomicUsize = AtomicUsize::new(0);

fn on_audio_frame(frame: &librtmp2::types::Frame) {
    if frame.frame_type == FrameType::Audio && frame.size > 0 {
        AUDIO_FRAMES.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn server_observes_published_audio_frames() {
    AUDIO_FRAMES.store(0, Ordering::SeqCst);

    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19668").unwrap();
    server.on_frame_cb = Some(on_audio_frame);
    server.on_publish_cb = Some(allow_publish);

    run_client_with_server(&mut server, || {
        let mut client = Client::new();
        client.connect("rtmp://127.0.0.1:19668/live/audiotest")?;
        client.publish()?;

        for ts in [0u32, 23, 46] {
            let (_data, frame) = make_audio_frame(ts);
            client.send_frame(&frame)?;
        }
        Ok(())
    });

    poll_until(
        &mut server,
        std::time::Instant::now() + std::time::Duration::from_secs(2),
        || AUDIO_FRAMES.load(Ordering::SeqCst) >= 3,
    );

    assert!(
        AUDIO_FRAMES.load(Ordering::SeqCst) >= 3,
        "server should observe published audio frames"
    );
}
