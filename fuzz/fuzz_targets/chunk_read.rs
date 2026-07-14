#![no_main]

use libfuzzer_sys::fuzz_target;
use librtmp2::buffer::Buffer;
use librtmp2::chunk::reader::{ChunkMessage, chunk_read};
use librtmp2::chunk::state::ChunkRegistry;

fuzz_target!(|data: &[u8]| {
    let mut buf = Buffer::new();
    if buf.write(data).is_err() {
        return;
    }

    let mut reg = ChunkRegistry::new();
    reg.init();
    let mut msg = ChunkMessage::default();
    let mut ptr = std::ptr::null();
    let mut len = 0usize;

    for _ in 0..4096 {
        match chunk_read(&mut buf, &mut reg, None, &mut msg, &mut ptr, &mut len) {
            Ok(0) => break,
            Ok(1) => {}
            Ok(_) => break,
            Err(_) => break,
        }
    }
});
