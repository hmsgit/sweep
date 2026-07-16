//! Inline-markup checks inside docstrings. When the project convention
//! is reST, a single-backtick span like `code` is (almost always) a
//! Markdown habit — reST literals need ``double backticks``. Spans
//! preceded by a role (:func:`name`) are correct reST and left alone.

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
        // Google/NumPy docstrings still use reST inline markup under
        // Sphinx+napoleon, so there is nothing safe to flag.
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
        // Double backtick: a correct reST literal — skip the whole span.
        if bytes.get(i + 1) == Some(&b'`') {
            match find(bytes, i + 2, b"``") {
                Some(close) => i = close + 2,
                None => i += 2,
            }
            continue;
        }
        // Single backtick: find the closing one on this line.
        let Some(close) = find(bytes, i + 1, b"`") else {
            i += 1;
            continue;
        };
        let span = &line[i + 1..close];
        if !preceded_by_role(&line[..i]) && !span.is_empty() && !span.contains('`') {
            issues.push(MarkupIssue {
                start: line_offset + i,
                end: line_offset + close + 1,
                replacement: format!("``{span}``"),
                message: format!("`{span}` is Markdown-style; reST literals use ``{span}``"),
            });
        }
        i = close + 1;
    }
}

/// True when `prefix` ends with a reST role like `:func:` or `:py:meth:`.
fn preceded_by_role(prefix: &str) -> bool {
    let Some(rest) = prefix.strip_suffix(':') else {
        return false;
    };
    let Some(colon) = rest.rfind(':') else {
        return false;
    };
    let word = &rest[colon + 1..];
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '.' || c == '_')
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
    fn flags_single_backticks_for_rest() {
        let issues = markup_issues("Use `foo` here.", DocStyle::Rest);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].replacement, "``foo``");
    }

    #[test]
    fn keeps_roles_and_doubles() {
        assert!(markup_issues("See :func:`foo` and ``bar``.", DocStyle::Rest).is_empty());
    }

    #[test]
    fn skips_doctests() {
        assert!(markup_issues(">>> x = `weird`\n", DocStyle::Rest).is_empty());
    }

    #[test]
    fn google_style_is_left_alone() {
        assert!(markup_issues("Use `foo` here.", DocStyle::Google).is_empty());
    }
}
