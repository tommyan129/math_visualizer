//! Source positions. UTF-8 byte offsets internally; `line:col` (col in chars) for display.

/// Half-open byte range into the request source. Server-only: never serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start: start as u32, end: end as u32 }
    }

    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }

    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

pub struct LineIndex<'a> {
    src: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(src: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        Self { src, line_starts }
    }

    /// 1-based line and 1-based column counted in characters.
    pub fn line_col(&self, offset: u32) -> (usize, usize) {
        let offset = (offset as usize).min(self.src.len());
        let line = self.line_starts.partition_point(|&s| s <= offset) - 1;
        let col = self.src[self.line_starts[line]..offset].chars().count();
        (line + 1, col + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_counts_chars_not_bytes() {
        let src = "a\n한글 x";
        let idx = LineIndex::new(src);
        let x = src.find('x').unwrap() as u32;
        assert_eq!(idx.line_col(x), (2, 4));
        assert_eq!(idx.line_col(0), (1, 1));
    }
}
