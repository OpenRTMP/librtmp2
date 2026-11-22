//! Publish flow
//!
//! Mirrors `src/session/publish.h` and `src/session/publish.c`.

use super::stream::Stream;
use crate::types::Result;

/// Begin publishing on a stream.
///
/// # Examples
///
/// ```
/// use librtmp2::session::publish::publish_begin;
/// use librtmp2::session::stream::Stream;
///
/// let mut stream = Stream::new(1);
/// publish_begin(&mut stream, "cam1").unwrap();
/// assert!(stream.is_publishing);
/// assert_eq!(stream.name, "cam1");
/// ```
pub fn publish_begin(stream: &mut Stream, stream_key: &str) -> Result<()> {
    stream.is_publishing = true;
    stream.name = stream_key.to_string();
    Ok(())
}
