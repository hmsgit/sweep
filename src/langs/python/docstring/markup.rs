//! Inline-markup checks inside docstrings. The project convention for
//! reST docstrings is single-backtick spans (`code`); reST-literal
//! ``double backticks`` are flagged and fixed down to single. Roles
//! like :func:`name` already use single backticks and are unaffected.

use crate::engine::config::DocStyle;

pub struct MarkupIssue {
    /// Byte range of the span *relative to the docstring content*.
    pub start: usize,
    pub end: usize,
    pub replacement: String,
    pub message: String,
}

pub fn markup_issues(content: &str, style: DocStyle) -> Vec<MarkupIssue> {
    if style != DocStyle::Rest {
        // No markup convention enforced for Google/NumPy projects.
        return Vec::new();
    }

    let mut issues = Vec::new();
    let mut line_start = 0;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        // Skip doctest lines; backticks there are code, not markup.
        if !(trimmed.starts_with(">>>") || trimmed.starts_with("... ")) {
            scan_line(line, line_start, &mut issues);
        }
        line_start += line.len();
    }
    issues
}

fn scan_line(line: &str, line_offset: usize, issues: &mut Vec<MarkupIssue>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        // Double backtick: flag the span and fix it to single backticks.
        if bytes.get(i + 1) == Some(&b'`') {
            let Some(close) = find(bytes, i + 2, b"``") else {
                i += 2;
                continue;
            };
            let span = &line[i + 2..close];
            if !span.is_empty() && !span.contains('`') {
                issues.push(MarkupIssue {
                    start: line_offset + i,
                    end: line_offset + close + 2,
                    replacement: format!("`{span}`"),
                    message: format!(
                        "``{span}`` uses double backticks; the convention is single-backtick `{span}`"
                    ),
                });
            }
            i = close + 2;
            continue;
        }
        // Single backtick span: correct — skip past it so its contents
        // aren't misread as new spans.
        match find(bytes, i + 1, b"`") {
            Some(close) => i = close + 1,
            None => i += 1,
        }
    }
}

fn find(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_double_backticks_for_rest() {
        let issues = markup_issues("Read ``scope`` here.", DocStyle::Rest);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].replacement, "`scope`");
    }

    #[test]
    fn keeps_singles_and_roles() {
        assert!(markup_issues("Use `foo` and :func:`bar`.", DocStyle::Rest).is_empty());
    }

    #[test]
    fn skips_doctests() {
        assert!(markup_issues(">>> x = ``weird``\n", DocStyle::Rest).is_empty());
    }

    #[test]
    fn google_style_is_left_alone() {
        assert!(markup_issues("Read ``scope`` here.", DocStyle::Google).is_empty());
    }
}
