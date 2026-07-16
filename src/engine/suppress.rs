use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::engine::context::walk_tree;
use crate::engine::source::LineIndex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    /// `# sweep: ignore` — silence every rule on this line.
    IgnoreAll,
    /// `# sweep: ignore[rule-a, rule-b] optional reason`
    Ignore(Vec<String>),
    /// `# sweep: avoid-cycle` — shorthand for ignoring imports-ban-local
    /// with cycle-avoidance as the stated reason.
    AvoidCycle,
}

/// Suppression directives indexed by the 1-based line they appear on.
///
/// `# sweep:` directives suppress on their own line or the line
/// directly above. Blanket markers from other ecosystems — a bare
/// `# noqa` (no codes) or a bare `# type: ignore` (no brackets) — say
/// "tooling: leave this line alone" and are honored too, but only on
/// their own line, matching flake8/mypy semantics. Code-carrying forms
/// (`# noqa: F401`, `# type: ignore[union-attr]`) name *their* tool's
/// rules and don't affect sweep.
#[derive(Debug, Default)]
pub struct Suppressions {
    by_line: HashMap<usize, Vec<Directive>>,
    blanket_lines: HashSet<usize>,
}

impl Suppressions {
    pub fn from_tree(root: Node, source: &str, line_index: &LineIndex) -> Self {
        let mut by_line: HashMap<usize, Vec<Directive>> = HashMap::new();
        let mut blanket_lines = HashSet::new();
        walk_tree(root, &mut |node| {
            if node.kind() != "comment" {
                return;
            }
            let text = &source[node.byte_range()];
            let line = line_index.line(node.start_byte());
            let parsed = parse_comment(text);
            if !parsed.directives.is_empty() {
                by_line.entry(line).or_default().extend(parsed.directives);
            }
            if parsed.blanket {
                blanket_lines.insert(line);
            }
        });
        Self {
            by_line,
            blanket_lines,
        }
    }

    pub fn is_suppressed(&self, rule: &str, line: usize) -> bool {
        if self.blanket_lines.contains(&line) {
            return true;
        }
        [line, line.saturating_sub(1)]
            .iter()
            .filter_map(|l| self.by_line.get(l))
            .flatten()
            .any(|d| match d {
                Directive::IgnoreAll => true,
                Directive::Ignore(rules) => rules.iter().any(|r| r == rule),
                Directive::AvoidCycle => rule == "imports-ban-local",
            })
    }
}

#[derive(Debug, Default, PartialEq)]
struct ParsedComment {
    directives: Vec<Directive>,
    blanket: bool,
}

/// A single comment node can chain several markers
/// (`# type: ignore  # sweep: avoid-cycle`), so parse per `#` segment.
fn parse_comment(comment: &str) -> ParsedComment {
    let mut parsed = ParsedComment::default();
    for segment in comment.split('#') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some(directive) = parse_sweep_directive(segment) {
            parsed.directives.push(directive);
        } else if is_blanket_marker(segment) {
            parsed.blanket = true;
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

fn parse_sweep_directive(segment: &str) -> Option<Directive> {
    let rest = segment.strip_prefix("sweep:")?.trim();

    if rest == "avoid-cycle" || rest.starts_with("avoid-cycle ") {
        return Some(Directive::AvoidCycle);
    }
    if rest == "ignore" {
        return Some(Directive::IgnoreAll);
    }
    if let Some(after) = rest.strip_prefix("ignore[") {
        let (list, _reason) = after.split_once(']')?;
        let rules: Vec<String> = list
            .split(',')
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .collect();
        if rules.is_empty() {
            return Some(Directive::IgnoreAll);
        }
        return Some(Directive::Ignore(rules));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directives(comment: &str) -> Vec<Directive> {
        parse_comment(comment).directives
    }

    #[test]
    fn parses_sweep_directives() {
        assert_eq!(directives("# sweep: ignore"), vec![Directive::IgnoreAll]);
        assert_eq!(
            directives("# sweep: ignore[imports-ban-local] heavy dep"),
            vec![Directive::Ignore(vec!["imports-ban-local".into()])]
        );
        assert_eq!(
            directives("# sweep: avoid-cycle models <-> tasks"),
            vec![Directive::AvoidCycle]
        );
        assert!(directives("# regular comment").is_empty());
    }

    #[test]
    fn blanket_markers_suppress_only_bare_forms() {
        assert!(parse_comment("# noqa").blanket);
        assert!(parse_comment("# NOQA").blanket);
        assert!(parse_comment("# type: ignore").blanket);
        assert!(!parse_comment("# noqa: F401").blanket);
        assert!(!parse_comment("# type: ignore[union-attr]").blanket);
        assert!(!parse_comment("# typed: ignore").blanket);
        assert!(!parse_comment("# regular comment").blanket);
    }

    #[test]
    fn chained_comment_segments() {
        let parsed = parse_comment("# type: ignore  # sweep: avoid-cycle reason");
        assert!(parsed.blanket);
        assert_eq!(parsed.directives, vec![Directive::AvoidCycle]);
    }
}
