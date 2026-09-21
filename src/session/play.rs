//! Play flow
//!
//! Mirrors `src/session/play.h` and `src/session/play.c`.

use super::stream::Stream;
use crate::types::Result;

/// Begin playing on a stream.
pub fn play_begin(stream: &mut Stream, stream_name: &str) -> Result<()> {
    stream.is_playing = true;
    stream.name = stream_name.to_string();
    Ok(())
}
