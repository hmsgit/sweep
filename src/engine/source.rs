/// Precomputed line-start offsets for byte-offset → (line, column) lookups.
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    /// 1-based (line, column) for a byte offset.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let line = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        (line + 1, offset - self.line_starts[line] + 1)
    }

    /// 1-based line number for a byte offset.
    pub fn line(&self, offset: usize) -> usize {
        self.line_col(offset).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_lookup() {
        let idx = LineIndex::new("ab\ncd\n");
        assert_eq!(idx.line_col(0), (1, 1));
        assert_eq!(idx.line_col(1), (1, 2));
        assert_eq!(idx.line_col(3), (2, 1));
        assert_eq!(idx.line_col(5), (2, 3));
    }
}
