//! Byte offset ↔ (line, column) conversion. The only place position encodings are handled.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Default)]
pub struct LineIndex {
    line_starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineIndex {
            line_starts,
            len: text.len(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte offset where `line` starts, clamped to the end of the text.
    pub fn line_start(&self, line: u32) -> usize {
        self.line_starts
            .get(line as usize)
            .copied()
            .unwrap_or(self.len)
    }

    fn line_end(&self, text: &str, line: u32) -> usize {
        let next = self
            .line_starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(self.len);
        let bytes = text.as_bytes();
        let mut end = next;
        while end > self.line_start(line) && matches!(bytes.get(end - 1), Some(b'\n' | b'\r')) {
            end -= 1;
        }
        end
    }

    pub fn line_of(&self, offset: usize) -> u32 {
        match self.line_starts.binary_search(&offset) {
            Ok(i) => i as u32,
            Err(i) => i.saturating_sub(1) as u32,
        }
    }

    /// Byte offset → position. Offsets inside a multi-byte character snap to its start.
    pub fn position(&self, text: &str, offset: usize, enc: Encoding) -> LineCol {
        let offset = offset.min(self.len);
        let line = self.line_of(offset);
        let start = self.line_start(line);
        let mut end = offset;
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        let slice = text.get(start..end).unwrap_or("");
        let col = match enc {
            Encoding::Utf8 => slice.len(),
            Encoding::Utf16 => slice.chars().map(char::len_utf16).sum(),
        };
        LineCol {
            line,
            col: col as u32,
        }
    }

    /// Position → byte offset. Columns past the end of the line clamp to the line end.
    pub fn offset(&self, text: &str, pos: LineCol, enc: Encoding) -> usize {
        if pos.line as usize >= self.line_starts.len() {
            return self.len;
        }
        let start = self.line_start(pos.line);
        let end = self.line_end(text, pos.line);
        let line = text.get(start..end).unwrap_or("");
        let mut remaining = pos.col as usize;
        for (i, ch) in line.char_indices() {
            let w = match enc {
                Encoding::Utf8 => ch.len_utf8(),
                Encoding::Utf16 => ch.len_utf16(),
            };
            if remaining < w {
                return start + i;
            }
            remaining -= w;
        }
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "a\n    const-string v0, \"héllo 😀\" # ü\r\nend";

    #[test]
    fn round_trip_every_char_boundary() {
        let idx = LineIndex::new(TEXT);
        for enc in [Encoding::Utf8, Encoding::Utf16] {
            // Offsets inside a line terminator have no LSP position of their own.
            for (off, _) in TEXT.char_indices().filter(|(_, c)| *c != '\n') {
                let pos = idx.position(TEXT, off, enc);
                assert_eq!(idx.offset(TEXT, pos, enc), off, "{enc:?} {off}");
            }
        }
    }

    #[test]
    fn utf16_counts_surrogate_pairs() {
        let idx = LineIndex::new(TEXT);
        let emoji = TEXT.find('😀').unwrap_or(0);
        let after = emoji + '😀'.len_utf8();
        let a = idx.position(TEXT, emoji, Encoding::Utf16);
        let b = idx.position(TEXT, after, Encoding::Utf16);
        assert_eq!(b.col - a.col, 2);
        let c = idx.position(TEXT, after, Encoding::Utf8);
        assert_eq!(c.col - idx.position(TEXT, emoji, Encoding::Utf8).col, 4);
        assert_eq!(a.line, 1);
    }

    #[test]
    fn crlf_and_clamping() {
        let idx = LineIndex::new(TEXT);
        let p = LineCol { line: 1, col: 999 };
        let off = idx.offset(TEXT, p, Encoding::Utf16);
        assert_eq!(&TEXT[off..off + 2], "\r\n");
        assert_eq!(
            idx.offset(TEXT, LineCol { line: 9, col: 0 }, Encoding::Utf8),
            TEXT.len()
        );
        assert_eq!(
            idx.position(TEXT, TEXT.len(), Encoding::Utf8),
            LineCol { line: 2, col: 3 }
        );
    }
}
