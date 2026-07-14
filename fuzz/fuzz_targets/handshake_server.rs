#![no_main]

use libfuzzer_sys::fuzz_target;
use librtmp2::buffer::Buffer;
use librtmp2::handshake::{Handshake, server_init, server_read_c0, server_read_c1, server_read_c2};

fuzz_target!(|data: &[u8]| {
    let mut hs = Handshake::default();
    server_init(&mut hs);

    let mut buf = Buffer::new();
    if buf.write(data).is_err() {
        return;
    }

    let _ = server_read_c0(&mut hs, &mut buf);
    let _ = server_read_c1(&mut hs, &mut buf);
    let _ = server_read_c2(&mut hs, &mut buf);
});
