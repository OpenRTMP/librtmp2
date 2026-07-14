//! E2E: server publish callback can reject unauthorized publishers.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use librtmp2::client::Client;
use librtmp2::server::Server;

use common::{SENT_FRAME_LEN, deny_publish, plain_config, poll_until};

static SERVER_FRAMES: AtomicUsize = AtomicUsize::new(0);

fn count_server_frames(frame: &librtmp2::types::Frame) {
    if frame.size as usize == SENT_FRAME_LEN {
        SERVER_FRAMES.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn publish_rejected_by_callback_does_not_deliver_frames() {
    SERVER_FRAMES.store(0, Ordering::SeqCst);

    let mut server = Server::new(plain_config()).unwrap();
    server.listen("127.0.0.1:19666").unwrap();
    server.on_publish_cb = Some(deny_publish);
    server.on_frame_cb = Some(count_server_frames);

    let publish_result = thread::spawn(|| {
        let mut client = Client::new();
        client.connect("rtmp://127.0.0.1:19666/live/denied")?;
        client.publish()
    })
    .join()
    .unwrap();

    assert!(
        publish_result.is_err(),
        "publish should fail when on_publish_cb returns false"
    );

    poll_until(&mut server, Instant::now() + Duration::from_secs(2), || {
        false
    });

    assert_eq!(
        SERVER_FRAMES.load(Ordering::SeqCst),
        0,
        "rejected publisher must not produce server-side frames"
    );
}
