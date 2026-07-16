/// A single text replacement. `start == end` is an insertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl Edit {
    pub fn replace(start: usize, end: usize, text: impl Into<String>) -> Self {
        Self {
            start,
            end,
            text: text.into(),
        }
    }

    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Self {
            start: at,
            end: at,
            text: text.into(),
        }
    }

    pub fn delete(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            text: String::new(),
        }
    }

    fn conflicts_with(&self, other: &Edit) -> bool {
        if self == other {
            // Identical edits are merged, not conflicting.
            return false;
        }
        // Two insertions at the same offset are ambiguous in order.
        if self.start == self.end && other.start == other.end {
            return self.start == other.start;
        }
        self.start < other.end && other.start < self.end
    }
}

/// A set of edits that together fix one diagnostic. Edits within a fix
/// must not overlap each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub edits: Vec<Edit>,
}

impl Fix {
    pub fn new(edits: Vec<Edit>) -> Self {
        Self { edits }
    }
}

/// Apply as many fixes as possible without conflicts. Returns the new
/// source and how many fixes were applied. Skipped fixes get picked up
/// on the next runner iteration.
pub fn apply_fixes(source: &str, fixes: &[&Fix]) -> (String, usize) {
    let mut accepted: Vec<Edit> = Vec::new();
    let mut applied = 0usize;

    'fixes: for fix in fixes {
        for edit in &fix.edits {
            let duplicate = accepted.contains(edit);
            if !duplicate && accepted.iter().any(|a| a.conflicts_with(edit)) {
                continue 'fixes;
            }
        }
        for edit in &fix.edits {
            if !accepted.contains(edit) {
                accepted.push(edit.clone());
            }
        }
        applied += 1;
    }

    if applied == 0 {
        return (source.to_string(), 0);
    }

    accepted.sort_by(|a, b| b.start.cmp(&a.start).then(b.end.cmp(&a.end)));
    let mut result = source.to_string();
    for edit in &accepted {
        result.replace_range(edit.start..edit.end, &edit.text);
    }
    (result, applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_non_overlapping() {
        let src = "abcdef";
        let f1 = Fix::new(vec![Edit::replace(0, 1, "X")]);
        let f2 = Fix::new(vec![Edit::replace(3, 4, "Y")]);
        let (out, n) = apply_fixes(src, &[&f1, &f2]);
        assert_eq!(out, "XbcYef");
        assert_eq!(n, 2);
    }

    #[test]
    fn skips_conflicting() {
        let src = "abcdef";
        let f1 = Fix::new(vec![Edit::replace(0, 3, "X")]);
        let f2 = Fix::new(vec![Edit::replace(2, 4, "Y")]);
        let (out, n) = apply_fixes(src, &[&f1, &f2]);
        assert_eq!(out, "Xdef");
        assert_eq!(n, 1);
    }

    #[test]
    fn merges_identical_insertions() {
        let src = "body";
        let f1 = Fix::new(vec![Edit::insert(0, "H\n"), Edit::replace(0, 1, "B")]);
        let f2 = Fix::new(vec![Edit::insert(0, "H\n"), Edit::replace(2, 3, "D")]);
        let (out, n) = apply_fixes(src, &[&f1, &f2]);
        assert_eq!(out, "H\nBoDy");
        assert_eq!(n, 2);
    }

    #[test]
    fn distinct_insertions_at_same_offset_conflict() {
        let src = "body";
        let f1 = Fix::new(vec![Edit::insert(0, "A\n")]);
        let f2 = Fix::new(vec![Edit::insert(0, "B\n")]);
        let (out, n) = apply_fixes(src, &[&f1, &f2]);
        assert_eq!(out, "A\nbody");
        assert_eq!(n, 1);
    }
}
