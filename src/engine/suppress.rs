use std::collections::{HashMap, HashSet};
use std::ops::Range;

use tree_sitter::Node;

use crate::engine::context::walk_tree;
use crate::engine::diagnostic::{Diagnostic, Severity};
use crate::engine::source::LineIndex;

/// Which rules a directive applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleFilter {
    All,
    Named(Vec<String>),
}

impl RuleFilter {
    fn matches(&self, rule: &str) -> bool {
        match self {
            RuleFilter::All => true,
            RuleFilter::Named(rules) => rules.iter().any(|r| r == rule),
        }
    }
}

/// Suppression directives, parsed from comments. Scope is explicit in
/// the directive name — placement never silently changes meaning:
///
/// - `# sweep: ignore[rules] reason` — this line or the line below.
/// - `# sweep: ignore-block[rules]` — on a def/class header line (or
///   the line above it): everything inside that definition.
/// - `# sweep: ignore-start[rules] reason` … `# sweep: ignore-end` —
///   everything between the pair, both lines included. A bare
///   ignore-end closes the most recent open start; ignore-end[rules]
///   closes the most recent start with the same rule list, so regions
///   for different rules may overlap. An unclosed start suppresses to
///   the end of the file but is itself an error, as is an ignore-end
///   with no matching start (self-cleaning, like expect).
/// - `# sweep: ignore-file[rules]` — in the file header region (before
///   the first real statement): the whole file.
/// - `# sweep: expect[rules]` — like ignore, but it is an error when
///   no matching finding was actually suppressed (self-cleaning).
/// - `# sweep: avoid-cycle reason` — imports-ban-local shorthand.
///
/// Misplaced ignore-file/ignore-block directives degrade to line scope.
/// Bare `# noqa` and `# type: ignore` (no codes) also suppress, on
/// their own line only, matching flake8/mypy semantics.
#[derive(Debug, Default)]
pub struct Suppressions {
    file: Vec<RuleFilter>,
    blocks: Vec<(Range<usize>, RuleFilter)>,
    /// Line ranges from ignore-start/ignore-end pairs.
    regions: Vec<(Range<usize>, RuleFilter)>,
    lines: HashMap<usize, Vec<RuleFilter>>,
    blanket_lines: HashSet<usize>,
    expects: Vec<Expect>,
    /// Malformed region pairing, reported as error diagnostics.
    region_errors: Vec<RegionError>,
}

#[derive(Debug)]
struct RegionError {
    rule: &'static str,
    message: &'static str,
    span: Range<usize>,
}

#[derive(Debug)]
struct Expect {
    line: usize,
    filter: RuleFilter,
    span: Range<usize>,
}

#[derive(Debug, PartialEq)]
enum Parsed {
    Ignore(RuleFilter),
    IgnoreFile(RuleFilter),
    IgnoreBlock(RuleFilter),
    IgnoreStart(RuleFilter),
    IgnoreEnd(RuleFilter),
    Expect(RuleFilter),
    Blanket,
}

impl Suppressions {
    pub fn from_tree(root: Node, source: &str, line_index: &LineIndex) -> Self {
        let header_end = file_header_end(root);

        let mut suppressions = Suppressions::default();
        // Open ignore-start directives, awaiting their ignore-end.
        let mut open_regions: Vec<(usize, Range<usize>, RuleFilter)> = Vec::new();
        walk_tree(root, &mut |node| {
            if node.kind() != "comment" {
                return;
            }
            let text = &source[node.byte_range()];
            let line = line_index.line(node.start_byte());
            for parsed in parse_comment(text) {
                match parsed {
                    Parsed::Ignore(filter) => {
                        suppressions.lines.entry(line).or_default().push(filter);
                    }
                    Parsed::IgnoreStart(filter) => {
                        open_regions.push((line, node.byte_range(), filter));
                    }
                    Parsed::IgnoreEnd(filter) => {
                        // A bare end closes the most recent open start; a
                        // rule-listed end closes the most recent start with
                        // the same list (regions may overlap).
                        let matched = open_regions
                            .iter()
                            .rposition(|(_, _, open)| filter == RuleFilter::All || *open == filter);
                        match matched {
                            Some(i) => {
                                let (start_line, _, open_filter) = open_regions.remove(i);
                                suppressions
                                    .regions
                                    .push((start_line..line + 1, open_filter));
                            }
                            None => suppressions.region_errors.push(RegionError {
                                rule: "ignore-end",
                                message: "no matching `# sweep: ignore-start` above; \
                                          remove this directive",
                                span: node.byte_range(),
                            }),
                        }
                    }
                    Parsed::IgnoreFile(filter) => {
                        if node.start_byte() < header_end {
                            suppressions.file.push(filter);
                        } else {
                            suppressions.lines.entry(line).or_default().push(filter);
                        }
                    }
                    Parsed::IgnoreBlock(filter) => match block_range(root, line, line_index) {
                        Some(range) => suppressions.blocks.push((range, filter)),
                        None => {
                            suppressions.lines.entry(line).or_default().push(filter);
                        }
                    },
                    Parsed::Expect(filter) => suppressions.expects.push(Expect {
                        line,
                        filter,
                        span: node.byte_range(),
                    }),
                    Parsed::Blanket => {
                        suppressions.blanket_lines.insert(line);
                    }
                }
            }
        });
        for (start_line, span, filter) in open_regions {
            // Still honor the author's intent (suppress to end of file)
            // but demand the missing ignore-end.
            suppressions.regions.push((start_line..usize::MAX, filter));
            suppressions.region_errors.push(RegionError {
                rule: "ignore-start",
                message: "unclosed region; add `# sweep: ignore-end`",
                span,
            });
        }
        suppressions
    }

    /// Filter suppressed diagnostics and report unfulfilled expects.
    /// `active_rules` are the rules that actually ran: an expect for a
    /// rule excluded via --select/--ignore is not reported as stale.
    pub fn apply(
        &self,
        mut diagnostics: Vec<Diagnostic>,
        line_index: &LineIndex,
        active_rules: &[&str],
    ) -> Vec<Diagnostic> {
        let mut hits = vec![false; self.expects.len()];
        diagnostics.retain(|d| {
            let line = line_index.line(d.start);
            let mut expected = false;
            for (i, expect) in self.expects.iter().enumerate() {
                if (expect.line == line || expect.line + 1 == line) && expect.filter.matches(d.rule)
                {
                    hits[i] = true;
                    expected = true;
                }
            }
            if expected {
                return false;
            }
            !(self.blanket_lines.contains(&line)
                || self.file.iter().any(|f| f.matches(d.rule))
                || self
                    .blocks
                    .iter()
                    .any(|(range, f)| range.contains(&d.start) && f.matches(d.rule))
                || self
                    .regions
                    .iter()
                    .any(|(range, f)| range.contains(&line) && f.matches(d.rule))
                || [line, line.saturating_sub(1)]
                    .iter()
                    .filter_map(|l| self.lines.get(l))
                    .flatten()
                    .any(|f| f.matches(d.rule)))
        });

        for error in &self.region_errors {
            diagnostics.push(
                Diagnostic::new(
                    error.rule,
                    error.message.to_string(),
                    error.span.start,
                    error.span.end,
                )
                .with_severity(Severity::Error),
            );
        }

        for (expect, hit) in self.expects.iter().zip(hits) {
            let relevant = match &expect.filter {
                RuleFilter::All => true,
                RuleFilter::Named(rules) => {
                    rules.iter().any(|r| active_rules.contains(&r.as_str()))
                }
            };
            if !hit && relevant {
                diagnostics.push(
                    Diagnostic::new(
                        "expect",
                        "expected finding was not produced; remove this directive".to_string(),
                        expect.span.start,
                        expect.span.end,
                    )
                    .with_severity(Severity::Error),
                );
            }
        }
        diagnostics
    }
}

/// Byte offset where the file header region ends: the first root child
/// that is neither a comment nor the module docstring.
fn file_header_end(root: Node) -> usize {
    let mut cursor = root.walk();
    let mut first = true;
    for child in root.children(&mut cursor) {
        if child.kind() == "comment" {
            continue;
        }
        // A leading string expression is the module docstring.
        if first && child.kind() == "expression_statement" {
            first = false;
            if child
                .named_child(0)
                .is_some_and(|n| n.kind() == "string" && child.named_child_count() == 1)
            {
                continue;
            }
            return child.start_byte();
        }
        return child.start_byte();
    }
    usize::MAX
}

/// The definition a block directive on `comment_line` attaches to: a
/// def/class whose header is on that line (trailing comment) or on the
/// next line (comment above). Decorated definitions count from their
/// first decorator.
fn block_range(root: Node, comment_line: usize, line_index: &LineIndex) -> Option<Range<usize>> {
    let mut found: Option<Range<usize>> = None;
    walk_tree(root, &mut |node| {
        if found.is_some() {
            return;
        }
        if !matches!(
            node.kind(),
            "function_definition" | "class_definition" | "decorated_definition"
        ) {
            return;
        }
        let header_line = line_index.line(node.start_byte());
        if header_line == comment_line || header_line == comment_line + 1 {
            let mut range = node.byte_range();
            if let Some(parent) = node.parent()
                && parent.kind() == "decorated_definition"
            {
                range = parent.byte_range();
            }
            found = Some(range);
        }
    });
    found
}

/// A single comment node can chain several markers
/// (`# type: ignore  # sweep: avoid-cycle`), so parse per `#` segment.
fn parse_comment(comment: &str) -> Vec<Parsed> {
    let mut parsed = Vec::new();
    for segment in comment.split('#') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some(directive) = parse_sweep_directive(segment) {
            parsed.push(directive);
        } else if is_blanket_marker(segment) {
            parsed.push(Parsed::Blanket);
        }
    }
    parsed
}

/// Bare `noqa` or bare `type: ignore` — code-carrying forms are not
/// for sweep.
fn is_blanket_marker(segment: &str) -> bool {
    if segment.eq_ignore_ascii_case("noqa") {
        return true;
    }
    let Some(rest) = segment.strip_prefix("type:") else {
        return false;
    };
    rest.trim() == "ignore"
}

fn parse_sweep_directive(segment: &str) -> Option<Parsed> {
    let rest = segment.strip_prefix("sweep:")?.trim();

    if rest == "avoid-cycle" || rest.starts_with("avoid-cycle ") {
        return Some(Parsed::Ignore(RuleFilter::Named(vec![
            "imports-ban-local".to_string(),
        ])));
    }
    for (keyword, build) in [
        ("expect", Parsed::Expect as fn(RuleFilter) -> Parsed),
        ("ignore-file", Parsed::IgnoreFile),
        ("ignore-block", Parsed::IgnoreBlock),
        ("ignore-start", Parsed::IgnoreStart),
        ("ignore-end", Parsed::IgnoreEnd),
        ("ignore", Parsed::Ignore),
    ] {
        if let Some(filter) = parse_keyword(rest, keyword) {
            return Some(build(filter));
        }
    }
    None
}

/// `keyword`, `keyword reason`, or `keyword[rule, rule] reason`.
fn parse_keyword(rest: &str, keyword: &str) -> Option<RuleFilter> {
    let after = rest.strip_prefix(keyword)?;
    if after.is_empty() || after.starts_with(' ') {
        return Some(RuleFilter::All);
    }
    let after = after.strip_prefix('[')?;
    let (list, _reason) = after.split_once(']')?;
    let rules: Vec<String> = list
        .split(',')
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    if rules.is_empty() {
        return Some(RuleFilter::All);
    }
    Some(RuleFilter::Named(rules))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sweep_directives() {
        assert_eq!(
            directives("# sweep: ignore"),
            vec![Parsed::Ignore(RuleFilter::All)]
        );
        assert_eq!(
            directives("# sweep: ignore[imports-ban-local] heavy dep"),
            vec![Parsed::Ignore(RuleFilter::Named(vec![
                "imports-ban-local".into()
            ]))]
        );
        assert_eq!(
            directives("# sweep: avoid-cycle models <-> tasks"),
            vec![Parsed::Ignore(RuleFilter::Named(vec![
                "imports-ban-local".into()
            ]))]
        );
        assert_eq!(
            directives("# sweep: ignore-file[docstring-style] legacy"),
            vec![Parsed::IgnoreFile(RuleFilter::Named(vec![
                "docstring-style".into()
            ]))]
        );
        assert_eq!(
            directives("# sweep: ignore-block"),
            vec![Parsed::IgnoreBlock(RuleFilter::All)]
        );
        assert_eq!(
            directives("# sweep: ignore-start[casing-enum-key] wire format"),
            vec![Parsed::IgnoreStart(RuleFilter::Named(vec![
                "casing-enum-key".into()
            ]))]
        );
        assert_eq!(
            directives("# sweep: ignore-end"),
            vec![Parsed::IgnoreEnd(RuleFilter::All)]
        );
        assert_eq!(
            directives("# sweep: expect[string-annotations] pending"),
            vec![Parsed::Expect(RuleFilter::Named(vec![
                "string-annotations".into()
            ]))]
        );
        assert!(directives("# regular comment").is_empty());
    }

    #[test]
    fn blanket_markers_suppress_only_bare_forms() {
        assert_eq!(directives("# noqa"), vec![Parsed::Blanket]);
        assert_eq!(directives("# NOQA"), vec![Parsed::Blanket]);
        assert_eq!(directives("# type: ignore"), vec![Parsed::Blanket]);
        assert!(directives("# noqa: F401").is_empty());
        assert!(directives("# type: ignore[union-attr]").is_empty());
        assert!(directives("# typed: ignore").is_empty());
    }

    #[test]
    fn chained_comment_segments() {
        assert_eq!(
            directives("# type: ignore  # sweep: avoid-cycle reason"),
            vec![
                Parsed::Blanket,
                Parsed::Ignore(RuleFilter::Named(vec!["imports-ban-local".into()]))
            ]
        );
    }

    fn directives(comment: &str) -> Vec<Parsed> {
        parse_comment(comment)
    }
}
