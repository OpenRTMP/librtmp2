//! E2E: multiple sequential video frames are observed by the server callback.

mod common;

use std::sync::atomic::Ordering;

use librtmp2::client::Client;
use librtmp2::server::Server;

use common::{
    allow_publish, make_video_frame, on_video_frame, plain_config, poll_until, run_client_with_server,
    FRAMES_RECEIVED,
};

#[test]
fn server_observes_multiple_published_video_frames() {
    FRAMES_RECEIVED.store(0, Ordering::SeqCst);

    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19667").unwrap();
    server.on_frame_cb = Some(on_video_frame);
    server.on_publish_cb = Some(allow_publish);

    run_client_with_server(&mut server, || {
        let mut client = Client::new();
        client.connect("rtmp://127.0.0.1:19667/live/multiframe")?;
        client.publish()?;

        let mut frames = Vec::with_capacity(5);
        for i in 0..5u8 {
            frames.push(make_video_frame(u32::from(i) * 40, i));
        }
        for (data, frame) in &frames {
            let _ = data;
            client.send_frame(frame)?;
        }
        Ok(())
    });

    poll_until(
        &mut server,
        std::time::Instant::now() + std::time::Duration::from_secs(2),
        || FRAMES_RECEIVED.load(Ordering::SeqCst) >= 5,
    );

    assert!(
        FRAMES_RECEIVED.load(Ordering::SeqCst) >= 5,
        "server should observe all published frames"
    );
}
