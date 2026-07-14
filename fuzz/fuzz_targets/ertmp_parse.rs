#![no_main]

use libfuzzer_sys::fuzz_target;
use librtmp2::ertmp::{exaudio, exvideo, modex, multitrack, reconnect};
use librtmp2::types::{AudioHeader, Modex, Multitrack, Reconnect, VideoHeader};

fuzz_target!(|data: &[u8]| {
    let mut video = VideoHeader::default();
    let _ = exvideo::exvideo_parse(data, &mut video);

    let mut audio = AudioHeader::default();
    let _ = exaudio::exaudio_parse(data, &mut audio);

    let mut modex = Modex::default();
    let _ = modex::modex_parse(&mut modex, data);

    let mut multitrack = Multitrack::default();
    let _ = multitrack::multitrack_parse(&mut multitrack, data);

    let mut reconnect = Reconnect::default();
    let _ = reconnect::reconnect_parse(&mut reconnect, data);
});
