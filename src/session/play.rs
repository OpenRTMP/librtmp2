//! Play flow
//!
//! Mirrors `src/session/play.h` and `src/session/play.c`.

use super::stream::Stream;
use crate::types::Result;

/// Begin playing on a stream.
///
/// # Examples
///
/// ```
/// use librtmp2::session::play::play_begin;
/// use librtmp2::session::stream::Stream;
///
/// let mut stream = Stream::new(1);
/// play_begin(&mut stream, "cam1").unwrap();
/// assert!(stream.is_playing);
/// assert_eq!(stream.name, "cam1");
/// ```
pub fn play_begin(stream: &mut Stream, stream_name: &str) -> Result<()> {
    stream.is_playing = true;
    stream.name = stream_name.to_string();
    Ok(())
}
