//! What a run keeps of one captured output stream (spec 263).
//!
//! The head and the tail share the one bound, and one JSON line marks the gap between them. Both capture paths (the
//! process reader and the container demux) use this type, so the rule lives here once. The head is where claude's
//! `system/init` line is; the tail is where its `result` line is. Memory is bounded by the bound itself: the head is at
//! most [`OUTPUT_HEAD_BYTES`] and the tail is a fixed ring of what fits after it.

use super::coder_runtime::OUTPUT_KEEP_BYTES;

/// The most of a stream's first bytes kept as the head, cut back to its last newline.
pub(crate) const OUTPUT_HEAD_BYTES: usize = 64 * 1024;

/// The marker line that stands in for the dropped middle. It is a JSON object with a `type` no reader knows, so every
/// line-based reader skips it.
fn marker(dropped: usize) -> String {
    format!("{{\"type\":\"pii_output_gap\",\"dropped_bytes\":{dropped}}}\n")
}

/// One stream's captured text, bounded to [`OUTPUT_KEEP_BYTES`] however long the stream is.
#[derive(Default)]
pub(crate) struct OutputKeep {
    /// The first bytes of the stream. Once decided, the head cut back to its last newline.
    head: Vec<u8>,
    /// The bytes after the head, the last of them once the stream outgrows the bound.
    tail: Ring,
    /// Whether the head has been decided (the first [`OUTPUT_HEAD_BYTES`] arrived and were cut).
    decided: bool,
    /// Every byte the stream carried, kept or not.
    total: usize,
}

impl OutputKeep {
    /// Takes the next bytes of the stream. What `finish` returns depends on the bytes alone, not on how they came.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len();
        let mut bytes = bytes;
        if !self.decided {
            let take = (OUTPUT_HEAD_BYTES - self.head.len()).min(bytes.len());
            self.head
                .extend_from_slice(bytes.get(..take).unwrap_or_default());
            bytes = bytes.get(take..).unwrap_or_default();
            if self.head.len() < OUTPUT_HEAD_BYTES {
                return;
            }
            self.decide();
        }
        self.tail.write(bytes);
    }

    /// Cuts the head back to its last newline; the bytes past the cut start the tail.
    fn decide(&mut self) {
        self.decided = true;
        let keep = self
            .head
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(self.head.len(), |i| i + 1);
        let cut = self.head.split_off(keep);
        self.tail = Ring::new(OUTPUT_KEEP_BYTES - keep);
        self.tail.write(&cut);
    }

    /// The captured text. A stream within the bound comes back whole. A longer one is its head, one marker line that
    /// counts what was dropped, and its tail cut forward to the first newline after its start.
    #[must_use]
    pub(crate) fn finish(&self) -> String {
        if self.total <= OUTPUT_KEEP_BYTES {
            let mut all = self.head.clone();
            all.extend_from_slice(&self.tail.bytes());
            return String::from_utf8_lossy(&all).into_owned();
        }
        // A head with no newline is kept whole, so the marker needs its own line in front of it.
        let sep: &[u8] = if self.head.last().is_some_and(|&b| b != b'\n') {
            b"\n"
        } else {
            b""
        };
        // The widest marker this stream can need: the dropped count never exceeds the stream's length.
        let budget = OUTPUT_KEEP_BYTES - self.head.len() - sep.len() - marker(self.total).len();
        let kept = self.tail.bytes();
        let mut tail = kept
            .get(kept.len().saturating_sub(budget)..)
            .unwrap_or_default()
            .to_vec();
        if let Some(i) = tail.iter().position(|&b| b == b'\n') {
            tail.drain(..=i);
        }
        let dropped = self.total - self.head.len() - tail.len();
        let mut out = self.head.clone();
        out.extend_from_slice(sep);
        out.extend_from_slice(marker(dropped).as_bytes());
        out.extend_from_slice(&tail);
        String::from_utf8_lossy(&out).into_owned()
    }
}

/// The last bytes of a stream, in a buffer of fixed size. A write copies at most two slices, never a byte at a time,
/// so a huge stream costs time proportional to its size and memory proportional to the buffer.
#[derive(Default)]
struct Ring {
    buf: Vec<u8>,
    /// Where the next byte goes.
    next: usize,
    /// How many bytes of the buffer hold the stream, at most `buf.len()`.
    len: usize,
}

impl Ring {
    fn new(cap: usize) -> Self {
        Self {
            buf: vec![0; cap],
            next: 0,
            len: 0,
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        let cap = self.buf.len();
        if cap == 0 {
            return;
        }
        // Only the last `cap` bytes of a long write can survive it; the rest still advance the write position.
        let skip = bytes.len().saturating_sub(cap);
        let at = (self.next + skip) % cap;
        let kept = bytes.get(skip..).unwrap_or_default();
        let (first, second) = kept.split_at(kept.len().min(cap - at));
        self.put(at, first);
        self.put(0, second);
        self.next = (self.next + bytes.len()) % cap;
        self.len = (self.len + bytes.len()).min(cap);
    }

    fn put(&mut self, at: usize, src: &[u8]) {
        if let Some(dst) = self.buf.get_mut(at..at + src.len()) {
            dst.copy_from_slice(src);
        }
    }

    /// The stream's last bytes in order, oldest first.
    fn bytes(&self) -> Vec<u8> {
        let cap = self.buf.len();
        if cap == 0 {
            return Vec::new();
        }
        let start = (self.next + cap - self.len) % cap;
        let mut out = self.buf.get(start..).unwrap_or_default().to_vec();
        out.extend_from_slice(self.buf.get(..start).unwrap_or_default());
        out.truncate(self.len);
        out
    }
}
