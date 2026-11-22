//! Growable byte buffer
//!
//! Mirrors `src/core/buffer.h` and `src/core/buffer.c`.

use crate::types::ErrorCode;
use crate::types::Result;

/// Maximum buffer size: 64 MB
pub const BUFFER_MAX_SIZE: usize = 64 * 1024 * 1024;
/// Initial buffer capacity
pub const BUFFER_INITIAL_SIZE: usize = 4096;
/// Capacity retained after [`Buffer::reset`] so short-lived large messages
/// cannot leave multi-megabyte `Vec` allocations pinned for the connection.
pub const BUFFER_RESET_CAPACITY: usize = BUFFER_INITIAL_SIZE;
/// Capacity a receive buffer may keep between reads. It covers normal socket
/// reads (64 KiB on the server, up to 256 KiB per client poll) plus a partial
/// chunk, so only a buffer that grew for one large chunk is shrunk.
pub const RECV_BUFFER_KEEP_CAPACITY: usize = 512 * 1024;
/// Growth factor
const BUFFER_GROW_FACTOR: usize = 2;

/// A growable byte buffer with read/write cursors.

/// # Examples
///
/// ```
/// use librtmp2::buffer::Buffer;
///
/// let mut buf = Buffer::with_capacity(64);
/// buf.write(b"chunk").unwrap();
///
/// // Peeking does not move the read cursor; draining does.
/// assert_eq!(buf.peek(), b"chunk");
/// buf.drain(5);
/// assert_eq!(buf.available(), 0);
/// ```
#[derive(Debug)]
pub struct Buffer {
    data: Vec<u8>,
    /// bytes written
    size: usize,
    /// read cursor
    read_pos: usize,
    /// whether data is owned (heap-allocated)
    owned: bool,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    /// Create a new buffer with default initial capacity.
    ///
    /// The buffer starts empty but pre-allocates [`BUFFER_INITIAL_SIZE`] bytes so
    /// the first writes do not reallocate.
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let buf = Buffer::new();
    /// assert_eq!(buf.available(), 0);
    /// assert!(buf.space() >= 1);
    /// ```
    pub fn new() -> Self {
        Self {
            data: vec![0u8; BUFFER_INITIAL_SIZE],
            size: 0,
            read_pos: 0,
            owned: true,
        }
    }

    /// Create a new buffer with a specific initial capacity.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// // Exact pre-allocation avoids reallocation on the first writes.
    /// let buf = Buffer::with_capacity(1024);
    /// assert_eq!(buf.available(), 0);
    /// assert!(buf.space() >= 1024);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: vec![0u8; capacity],
            size: 0,
            read_pos: 0,
            owned: true,
        }
    }

    /// Create an "unowned" buffer wrapping external data.
    ///
    /// The bytes are copied, and the buffer starts with its read cursor at the
    /// beginning, so the whole slice is immediately readable.
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::from_slice(&[0xAA, 0xBB, 0xCC]);
    /// assert_eq!(buf.available(), 3);
    /// assert_eq!(buf.peek(), &[0xAA, 0xBB, 0xCC]);
    ///
    /// let mut first = [0u8; 2];
    /// assert_eq!(buf.read(&mut first).unwrap(), 2);
    /// assert_eq!(first, [0xAA, 0xBB]);
    /// assert_eq!(buf.available(), 1);
    /// ```
    pub fn from_slice(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            size: data.len(),
            read_pos: 0,
            owned: true,
        }
    }

    /// Create an unowned view over existing data (for stack/static buffers).

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut raw = [1u8, 2, 3, 4];
    /// let mut view = Buffer::from_static(&mut raw);
    ///
    /// assert_eq!(view.available(), 4);
    /// let mut out = [0u8; 2];
    /// view.read(&mut out).unwrap();
    /// assert_eq!(out, [1, 2]);
    /// ```
    pub fn from_static(data: &mut [u8]) -> Buffer {
        let len = data.len();
        Buffer {
            data: data.to_vec(),
            size: len,
            read_pos: 0,
            owned: false,
        }
    }

    /// Number of bytes available to read.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::from_slice(&[0xAA, 0xBB]);
    /// assert_eq!(buf.available(), 2);
    ///
    /// let mut one = [0u8; 1];
    /// buf.read(&mut one).unwrap();
    /// assert_eq!(buf.available(), 1);
    /// ```
    pub fn available(&self) -> usize {
        self.size.saturating_sub(self.read_pos)
    }

    /// Remaining write space.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::with_capacity(8);
    /// let before = buf.space();
    /// buf.write(&[0u8; 4]).unwrap();
    /// // Four of the reserved bytes are now occupied.
    /// assert_eq!(buf.space(), before.saturating_sub(4));
    /// ```
    pub fn space(&self) -> usize {
        self.data.len().saturating_sub(self.size)
    }

    /// Write data to the buffer.
    ///
    /// Appends at the write cursor, growing the backing allocation if needed.
    /// Data already readable (before the read cursor) is untouched.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::Internal`] when the buffer cannot grow.
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::new();
    /// buf.write(&[1, 2, 3]).unwrap();
    /// // An empty write is a no-op.
    /// assert_eq!(buf.write(&[]).unwrap(), 0);
    /// assert_eq!(buf.peek(), &[1, 2, 3]);
    /// ```
    pub fn write(&mut self, data: &[u8]) -> Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }

        self.make_room(data.len())?;

        self.data[self.size..self.size + data.len()].copy_from_slice(data);
        self.size += data.len();
        Ok(data.len())
    }

    /// Ensure at least `additional` bytes can be written without another
    /// reallocation or compaction, so a multi-part write (e.g. a chunked
    /// RTMP message) either fits entirely or fails before writing anything.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::new();
    /// buf.reserve(4096).unwrap();
    /// // One contiguous 4096-byte write cannot fail part-way.
    /// buf.write(&[0u8; 4096]).unwrap();
    /// assert_eq!(buf.available(), 4096);
    /// ```
    pub fn reserve(&mut self, additional: usize) -> Result<()> {
        self.make_room(additional)
    }

    /// Read data from the buffer.
    ///
    /// Advances the read cursor by `out.len()` on success. All-or-nothing: if
    /// fewer bytes are available than `out` needs, the cursor does not move.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::Io`] when fewer than `out.len()` bytes are available.
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::from_slice(&[1, 2]);
    ///
    /// // Asking for more than is available fails without consuming anything.
    /// let mut too_big = [0u8; 3];
    /// assert!(buf.read(&mut too_big).is_err());
    /// assert_eq!(buf.available(), 2);
    ///
    /// let mut ok = [0u8; 2];
    /// assert_eq!(buf.read(&mut ok).unwrap(), 2);
    /// assert_eq!(buf.available(), 0);
    /// ```
    pub fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if self.read_pos + out.len() > self.size {
            return Err(ErrorCode::Io);
        }
        out.copy_from_slice(&self.data[self.read_pos..self.read_pos + out.len()]);
        self.read_pos += out.len();
        Ok(out.len())
    }

    /// Peek at available data without consuming it.
    ///
    /// Returns everything between the read cursor and the end of written data.
    /// This is the cheap way to inspect a complete message before deciding
    /// whether to consume it.
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let buf = Buffer::from_slice(b"RTMP");
    /// assert_eq!(buf.peek(), b"RTMP");
    /// // Peeking does not consume.
    /// assert_eq!(buf.peek(), b"RTMP");
    /// ```
    pub fn peek(&self) -> &[u8] {
        &self.data[self.read_pos..self.size]
    }

    /// Total allocated capacity (logical size plus spare write space).

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let buf = Buffer::with_capacity(128);
    /// assert!(buf.capacity() >= 128);
    /// // Nothing written yet, so all of it is spare.
    /// assert_eq!(buf.available(), 0);
    /// ```
    pub fn capacity(&self) -> usize {
        self.data.len()
    }

    /// Reset the buffer to empty.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::from_slice(&[1, 2, 3]);
    /// buf.reset();
    ///
    /// assert_eq!(buf.available(), 0);
    /// assert_eq!(buf.read_pos(), 0);
    /// ```
    pub fn reset(&mut self) {
        self.size = 0;
        self.read_pos = 0;
        if self.owned && self.data.len() > BUFFER_RESET_CAPACITY {
            self.data.truncate(BUFFER_RESET_CAPACITY);
            self.data.shrink_to_fit();
        }
    }

    /// Shrink an allocation that grew past `keep` bytes once most of it has
    /// been read. Unread bytes move to the front and are kept, so this is safe
    /// to call with a partial chunk still buffered.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::from_slice(&[1, 2, 3, 4]);
    /// // Read most of it back, then give the capacity back.
    /// let mut head = [0u8; 3];
    /// buf.read(&mut head).unwrap();
    /// buf.release_excess(8);
    ///
    /// // The unread byte survives the shrink.
    /// assert_eq!(buf.peek(), &[4]);
    /// ```
    pub fn release_excess(&mut self, keep: usize) {
        if !self.owned || self.data.len() <= keep || self.available() > keep / 2 {
            return;
        }
        self.compact();
        self.data.truncate(self.size.max(BUFFER_INITIAL_SIZE));
        self.data.shrink_to_fit();
    }

    /// Drain (skip) `len` bytes from the read position.
    ///
    /// Clamps to the available bytes: draining more than is present empties the
    /// buffer rather than panicking. Used to discard bytes already handled out of
    /// band, e.g. after sending from [`Buffer::peek`].
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::from_slice(&[1, 2, 3, 4]);
    /// buf.drain(2);
    /// assert_eq!(buf.peek(), &[3, 4]);
    ///
    /// // Over-draining clamps instead of panicking.
    /// buf.drain(99);
    /// assert_eq!(buf.available(), 0);
    /// ```
    pub fn drain(&mut self, len: usize) {
        let available = self.available();
        self.read_pos += len.min(available);
    }

    /// Take ownership of the unread bytes, leaving the buffer empty.
    ///
    /// Unlike `peek().to_vec()`, this avoids copying: when `read_pos` is 0
    /// (the common case for a buffer that is only ever appended to and never
    /// partially read, e.g. per-CSID chunk reassembly) it just truncates and
    /// moves out the backing `Vec` in O(1).
    ///
    /// The buffer is left empty with no retained capacity.
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::new();
    /// buf.write(&[7, 8, 9]).unwrap();
    ///
    /// let bytes = buf.take();
    /// assert_eq!(bytes, vec![7, 8, 9]);
    /// assert_eq!(buf.available(), 0);
    /// ```
    pub fn take(&mut self) -> Vec<u8> {
        let fixed_capacity = self.data.len();
        let mut out = std::mem::take(&mut self.data);
        out.truncate(self.size);
        if self.read_pos > 0 {
            out.drain(0..self.read_pos.min(out.len()));
        }
        self.size = 0;
        self.read_pos = 0;
        if !self.owned {
            // `ensure_capacity` never grows an unowned buffer, so leaving it
            // with the empty, zero-capacity `Vec` that `mem::take` put in
            // `self.data` above would permanently reject the next `write()`.
            // Restore its original fixed capacity instead.
            self.data = vec![0u8; fixed_capacity];
        }
        out
    }

    /// Get the raw data slice.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let buf = Buffer::from_slice(&[9, 8, 7]);
    /// // `as_slice` is the whole backing store, unlike `peek` (unread window).
    /// assert!(buf.as_slice().len() >= 3);
    /// assert_eq!(buf.peek(), &[9, 8, 7]);
    /// ```
    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.size]
    }

    /// Get the raw data slice mutably.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::with_capacity(4);
    /// buf.write(&[0; 4]).unwrap();
    /// // Patch a byte in place, then re-read it through `peek`.
    /// buf.as_mut_slice()[0] = 0xAB;
    /// assert_eq!(buf.peek()[0], 0xAB);
    /// ```
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data[..self.size]
    }

    /// Get the write position for direct writing.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::from_slice(&[1, 2]);
    /// // For a buffer that is only appended to, the write cursor sits at the end.
    /// assert!(buf.write_pos() >= 2);
    /// assert_eq!(buf.read_pos(), 0);
    /// ```
    pub fn write_pos(&self) -> usize {
        self.size
    }

    /// Set the size after direct writing.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::new();
    /// // Committing zero bytes leaves the buffer empty.
    /// let end = buf.write_pos();
    /// buf.set_size(end);
    /// assert_eq!(buf.available(), 0);
    /// ```
    pub fn set_size(&mut self, size: usize) {
        self.size = size.min(self.data.len());
    }

    /// Get the current read position.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::from_slice(&[1, 2, 3]);
    /// assert_eq!(buf.read_pos(), 0);
    ///
    /// let mut one = [0u8; 1];
    /// buf.read(&mut one).unwrap();
    /// assert_eq!(buf.read_pos(), 1);
    /// ```
    pub fn read_pos(&self) -> usize {
        self.read_pos
    }

    /// Set the read position.

    /// # Examples
    ///
    /// ```
    /// use librtmp2::buffer::Buffer;
    ///
    /// let mut buf = Buffer::from_slice(&[1, 2, 3]);
    /// let mut two = [0u8; 2];
    /// buf.read(&mut two).unwrap();
    ///
    /// // Rewind to re-read from the start.
    /// buf.set_read_pos(0);
    /// assert_eq!(buf.available(), 3);
    /// assert_eq!(buf.peek(), &[1, 2, 3]);
    /// ```
    pub fn set_read_pos(&mut self, pos: usize) {
        self.read_pos = pos.min(self.size);
    }

    /// Make room for `additional` more bytes after the write cursor.
    ///
    /// Unread data is only moved to the front when the tail lacks space.
    /// Compacting unconditionally on every `write()` cost a `memmove` of the
    /// entire unread backlog per call -- on a relay's per-player send buffer
    /// that is partially flushed every tick (a slow viewer, or any keyframe
    /// larger than the socket send buffer), that meant copying the whole
    /// backlog once per relayed message, per player. A fully drained buffer
    /// still rewinds for free.
    fn make_room(&mut self, additional: usize) -> Result<()> {
        if self.read_pos > 0 && self.read_pos >= self.size {
            self.size = 0;
            self.read_pos = 0;
        }
        // The cap applies to the unread bytes, before the fast path below:
        // a buffer allocated larger than BUFFER_MAX_SIZE must not hold more.
        if additional > BUFFER_MAX_SIZE || self.available() > BUFFER_MAX_SIZE - additional {
            return Err(ErrorCode::Internal);
        }
        if additional <= self.space() {
            return Ok(());
        }
        self.compact();
        self.ensure_capacity(self.size + additional)
    }

    /// Compact: move unread data to the front.
    fn compact(&mut self) {
        if self.read_pos == 0 {
            return;
        }
        if self.read_pos >= self.size {
            self.size = 0;
            self.read_pos = 0;
            return;
        }
        let avail = self.size - self.read_pos;
        self.data.copy_within(self.read_pos..self.size, 0);
        self.size = avail;
        self.read_pos = 0;
    }

    /// Ensure the buffer has at least `needed` capacity.
    fn ensure_capacity(&mut self, needed: usize) -> Result<()> {
        if needed <= self.data.len() {
            return Ok(());
        }
        if !self.owned {
            return Err(ErrorCode::Internal);
        }
        if needed > BUFFER_MAX_SIZE {
            return Err(ErrorCode::Internal);
        }
        let mut new_cap = self.data.len().max(BUFFER_INITIAL_SIZE);
        while new_cap < needed {
            new_cap = new_cap.saturating_mul(BUFFER_GROW_FACTOR);
            if new_cap > BUFFER_MAX_SIZE {
                new_cap = BUFFER_MAX_SIZE;
                break;
            }
        }
        self.data.resize(new_cap, 0);
        Ok(())
    }
}

impl Clone for Buffer {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            size: self.size,
            read_pos: self.read_pos,
            owned: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_round_trips() {
        let mut buf = Buffer::new();
        buf.write(b"hello world").unwrap();
        assert_eq!(buf.available(), 11);
        let mut out = [0u8; 5];
        buf.read(&mut out).unwrap();
        assert_eq!(&out, b"hello");
        assert_eq!(buf.available(), 6);
        assert_eq!(buf.peek(), b" world");
    }

    #[test]
    fn write_after_partial_drain_keeps_unread_bytes_in_order() {
        let mut buf = Buffer::with_capacity(8);
        buf.write(b"abcdef").unwrap();
        buf.drain(4);
        // Fits in the tail: no compaction needed.
        buf.write(b"gh").unwrap();
        assert_eq!(buf.peek(), b"efgh");
        // Does not fit in the tail: compacts first, then fits without growing.
        buf.write(b"ijkl").unwrap();
        assert_eq!(buf.peek(), b"efghijkl");
        assert_eq!(buf.capacity(), 8);
        // Needs growth beyond compaction.
        buf.write(b"mn").unwrap();
        assert_eq!(buf.peek(), b"efghijklmn");
    }

    #[test]
    fn write_after_full_drain_rewinds_without_growing() {
        let mut buf = Buffer::with_capacity(8);
        buf.write(b"abcdefgh").unwrap();
        buf.drain(8);
        buf.write(b"12345678").unwrap();
        assert_eq!(buf.peek(), b"12345678");
        assert_eq!(buf.capacity(), 8);
    }

    #[test]
    fn read_past_available_errors() {
        let mut buf = Buffer::new();
        buf.write(b"ab").unwrap();
        let mut out = [0u8; 3];
        assert!(buf.read(&mut out).is_err());
    }

    #[test]
    fn drain_skips_unread_bytes() {
        let mut buf = Buffer::new();
        buf.write(b"abcdef").unwrap();
        buf.drain(3);
        assert_eq!(buf.peek(), b"def");
    }

    #[test]
    fn grows_past_initial_capacity() {
        let mut buf = Buffer::new();
        let chunk = vec![0xAB_u8; BUFFER_INITIAL_SIZE * 3];
        buf.write(&chunk).unwrap();
        assert_eq!(buf.available(), chunk.len());
        assert_eq!(buf.peek(), chunk.as_slice());
    }

    #[test]
    fn reset_clears_buffer() {
        let mut buf = Buffer::new();
        buf.write(b"data").unwrap();
        buf.reset();
        assert_eq!(buf.available(), 0);
        assert_eq!(buf.peek(), b"");
    }

    #[test]
    fn reset_shrinks_oversized_capacity() {
        let mut buf = Buffer::new();
        buf.write(&vec![0u8; 1024 * 1024]).unwrap();
        assert!(buf.capacity() >= 1024 * 1024);
        buf.reset();
        assert_eq!(buf.available(), 0);
        assert!(buf.capacity() <= BUFFER_RESET_CAPACITY);
    }

    #[test]
    fn release_excess_shrinks_a_mostly_read_buffer_and_keeps_unread_bytes() {
        let mut buf = Buffer::new();
        buf.write(&vec![7u8; 4 * 1024 * 1024]).unwrap();
        buf.write(b"tail").unwrap();
        buf.drain(4 * 1024 * 1024);
        buf.release_excess(RECV_BUFFER_KEEP_CAPACITY);
        assert_eq!(buf.peek(), b"tail");
        assert!(buf.capacity() <= BUFFER_INITIAL_SIZE);
        buf.write(b"more").unwrap();
        assert_eq!(buf.peek(), b"tailmore");
    }

    #[test]
    fn release_excess_leaves_small_or_still_full_buffers_alone() {
        let mut small = Buffer::new();
        small.write(&vec![0u8; 100 * 1024]).unwrap();
        small.drain(100 * 1024);
        let cap = small.capacity();
        small.release_excess(RECV_BUFFER_KEEP_CAPACITY);
        assert_eq!(small.capacity(), cap);

        let mut full = Buffer::new();
        full.write(&vec![0u8; 2 * 1024 * 1024]).unwrap();
        let cap = full.capacity();
        full.release_excess(RECV_BUFFER_KEEP_CAPACITY);
        assert_eq!(full.capacity(), cap);
        assert_eq!(full.available(), 2 * 1024 * 1024);
    }

    #[test]
    fn write_over_max_size_errors() {
        let mut buf = Buffer::new();
        assert!(buf.write(&vec![0u8; BUFFER_MAX_SIZE + 1]).is_err());
    }

    #[test]
    fn take_returns_written_bytes_and_leaves_buffer_writable() {
        let mut buf = Buffer::new();
        buf.write(b"hello world").unwrap();
        assert_eq!(buf.take(), b"hello world");
        assert_eq!(buf.available(), 0);
        // Owned buffers reallocate on demand, so this must still work.
        buf.write(b"more").unwrap();
        assert_eq!(buf.peek(), b"more");
    }

    #[test]
    fn preallocated_tail_cannot_exceed_the_max_size() {
        let mut buf = Buffer::with_capacity(BUFFER_MAX_SIZE + 16);
        assert!(buf.reserve(BUFFER_MAX_SIZE + 1).is_err());
        buf.write(&[0u8; 8]).unwrap();
        assert!(buf.reserve(BUFFER_MAX_SIZE - 7).is_err());
        assert!(buf.write(&vec![0u8; BUFFER_MAX_SIZE - 7]).is_err());
        assert_eq!(buf.available(), 8, "a rejected write leaves nothing behind");
        assert!(buf.reserve(BUFFER_MAX_SIZE - 8).is_ok());
    }

    #[test]
    fn take_respects_unread_prefix() {
        let mut buf = Buffer::new();
        buf.write(b"abcdef").unwrap();
        let mut out = [0u8; 2];
        buf.read(&mut out).unwrap();
        assert_eq!(buf.take(), b"cdef");
    }

    #[test]
    fn take_on_unowned_buffer_leaves_it_writable() {
        // Regression test: `take()` used to leave an unowned (`from_static`)
        // buffer with zero capacity, permanently rejecting any further
        // write since `ensure_capacity` never grows an unowned buffer.
        // `from_static` treats the whole slice as already written, so
        // `take()` is what empties it out for reuse.
        let mut backing = *b"abcdefgh";
        let mut buf = Buffer::from_static(&mut backing);
        assert_eq!(buf.take(), b"abcdefgh");
        buf.write(b"ef").unwrap();
        assert_eq!(buf.peek(), b"ef");
    }
}
