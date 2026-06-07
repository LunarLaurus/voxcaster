// These items are public API consumed by PtySession in a later task.
#![allow(dead_code)]

use std::collections::VecDeque;

/// Result of a buffer read: a contiguous slice of lines plus addressing metadata.
pub struct ReadSlice {
    pub lines: Vec<String>,
    pub offset: usize,
    pub total: usize,
    pub truncated: bool,
}

/// Bounded, line-oriented scrollback. Stores raw lines; drops oldest on overflow.
pub struct RingBuffer {
    lines: VecDeque<String>,
    pending: String,
    max_lines: usize,
    max_bytes: usize,
    bytes: usize,
    dropped: bool,
}

impl RingBuffer {
    /// Create a new `RingBuffer` capped at `max_lines` lines and `max_bytes` bytes.
    pub fn new(max_lines: usize, max_bytes: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            pending: String::new(),
            max_lines,
            max_bytes,
            bytes: 0,
            dropped: false,
        }
    }

    /// Append a chunk of raw PTY output, splitting on `\n`. A trailing partial
    /// line is held in `pending` until its newline arrives.
    pub fn append(&mut self, chunk: &str) {
        self.pending.push_str(chunk);
        while let Some(idx) = self.pending.find('\n') {
            let mut line: String = self.pending.drain(..=idx).collect();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            self.push_line(line);
        }
    }

    /// Force any held partial line into the buffer. Call on process exit.
    pub fn flush(&mut self) {
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.push_line(line);
        }
    }

    fn push_line(&mut self, line: String) {
        self.bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > self.max_lines || self.bytes > self.max_bytes {
            if let Some(old) = self.lines.pop_front() {
                self.bytes -= old.len();
                self.dropped = true;
            } else {
                break;
            }
        }
    }

    /// Read a line range starting at `offset`, up to `limit` lines.
    /// `raw = false` strips ANSI escape sequences before returning.
    pub fn read(&self, offset: usize, limit: Option<usize>, raw: bool) -> ReadSlice {
        let total = self.lines.len();
        let end = match limit {
            Some(l) => (offset + l).min(total),
            None => total,
        };
        let lines = self
            .lines
            .iter()
            .skip(offset)
            .take(end.saturating_sub(offset))
            .map(|l| if raw { l.clone() } else { strip_ansi(l) })
            .collect();
        ReadSlice {
            lines,
            offset,
            total,
            truncated: self.dropped,
        }
    }

    /// Returns the number of lines currently held in the buffer.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Returns `true` if the buffer holds no lines.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

fn strip_ansi(s: &str) -> String {
    strip_ansi_escapes::strip_str(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_reads_lines_by_range() {
        let mut b = RingBuffer::new(100, 64 * 1024);
        b.append("line one\nline two\nline three\n");
        let r = b.read(0, Some(2), false);
        assert_eq!(r.lines, vec!["line one", "line two"]);
        assert_eq!(r.total, 3);
        assert!(!r.truncated);
    }

    #[test]
    fn strips_ansi_by_default_keeps_raw_on_request() {
        let mut b = RingBuffer::new(100, 64 * 1024);
        b.append("\x1b[31mred\x1b[0m\n");
        assert_eq!(b.read(0, None, false).lines, vec!["red"]);
        assert_eq!(b.read(0, None, true).lines, vec!["\x1b[31mred\x1b[0m"]);
    }

    #[test]
    fn drops_oldest_lines_past_cap() {
        let mut b = RingBuffer::new(2, 64 * 1024);
        b.append("a\nb\nc\n");
        let r = b.read(0, None, false);
        assert_eq!(r.lines, vec!["b", "c"]);
        assert_eq!(r.total, 2);
    }
}
