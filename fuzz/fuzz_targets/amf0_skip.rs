#![no_main]

use libfuzzer_sys::fuzz_target;
use librtmp2::amf::amf0;
use librtmp2::buffer::Buffer;

fuzz_target!(|data: &[u8]| {
    let mut buf = Buffer::new();
    if buf.write(data).is_err() {
        return;
    }

    for _ in 0..256 {
        if buf.available() == 0 {
            break;
        }
        if amf0::skip_value(&mut buf).is_err() {
            break;
        }
    }
});
