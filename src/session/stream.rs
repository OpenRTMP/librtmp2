//! Stream management within a connection
//!
//! Mirrors `src/session/stream.h` and `src/session/stream.c`.
//!
//! An RTMP connection may allocate several message streams, but at most one is
//! "current" (the target of `publish`/`play`). [`Stream`] holds that stream's
//! role and its pause/A/V-receive flags, which the server relay consults before
//! queueing media for the player.

use std::sync::atomic::{AtomicU32, Ordering};

/// Per-stream state.
///
/// Created by [`Stream::new`] when a `createStream` is acknowledged, and turned
/// into a publishing/playing stream by [`crate::session::publish::publish_begin`]
/// or [`crate::session::play::play_begin`].
///
/// # Examples
///
/// ```
/// use librtmp2::session::stream::Stream;
///
/// let stream = Stream::new(1);
/// assert_eq!(stream.stream_id, 1);
/// assert!(!stream.is_publishing);
/// assert!(stream.receive_audio && stream.receive_video);
/// ```
#[derive(Debug)]
pub struct Stream {
    /// Message stream id assigned by `createStream`.
    pub stream_id: u32,
    /// `true` while the peer is publishing on this stream.
    pub is_publishing: bool,
    /// `true` while the peer is playing this stream.
    pub is_playing: bool,
    /// Stream name from the `publish` or `play` command.
    pub name: String,
    /// Set by the RTMP `pause` command.
    pub paused: bool,
    /// Honored by server relay when forwarding to this player.
    pub receive_audio: bool,
    /// Honored by server relay when forwarding to this player.
    pub receive_video: bool,
}

impl Stream {
    /// Create a new stream.
    ///
    /// The stream starts neither publishing nor playing, with both audio and
    /// video enabled.
    ///
    /// ```
    /// use librtmp2::session::stream::Stream;
    ///
    /// let stream = Stream::new(1);
    /// assert_eq!(stream.stream_id, 1);
    /// assert!(!stream.is_publishing && !stream.is_playing);
    /// assert!(stream.receive_audio && stream.receive_video);
    /// ```
    pub fn new(stream_id: u32) -> Self {
        Self {
            stream_id,
            is_publishing: false,
            is_playing: false,
            name: String::new(),
            paused: false,
            receive_audio: true,
            receive_video: true,
        }
    }
}

/// Begin publishing on this stream.
///
/// # Examples
///
/// ```
/// use librtmp2::session::stream::{publish_begin, Stream};
///
/// let mut stream = Stream::new(1);
/// // Only flips the role flag; the name is set by
/// // `session::publish::publish_begin`.
/// publish_begin(&mut stream, "cam1");
/// assert!(stream.is_publishing);
/// assert!(stream.name.is_empty());
/// ```
pub fn publish_begin(stream: &mut Stream, _stream_key: &str) {
    stream.is_publishing = true;
}

/// Begin playing on this stream.
///
/// # Examples
///
/// ```
/// use librtmp2::session::stream::{play_begin, Stream};
///
/// let mut stream = Stream::new(1);
/// // Only flips the role flag; use `session::play::play_begin` to record the name.
/// play_begin(&mut stream);
/// assert!(stream.is_playing);
/// assert!(stream.name.is_empty());
/// ```
pub fn play_begin(stream: &mut Stream) {
    stream.is_playing = true;
}
