#![no_main]

use libfuzzer_sys::fuzz_target;
use librtmp2::message::control;

fuzz_target!(|data: &[u8]| {
    let _ = control::read_set_chunk_size(data);
    let _ = control::read_abort_message(data);
    let _ = control::read_acknowledgement_size(data);
    let _ = control::read_window_ack_size(data);
    let _ = control::read_set_peer_bandwidth(data);
    let _ = control::read_user_control(data, false);
    let _ = control::read_user_control(data, true);
});
