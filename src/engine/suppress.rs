use std::collections::HashMap;

use tree_sitter::Node;

use crate::engine::context::walk_tree;
use crate::engine::source::LineIndex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    /// `# sweep: ignore` — silence every rule on this line.
    IgnoreAll,
    /// `# sweep: ignore[rule-a, rule-b] optional reason`
    Ignore(Vec<String>),
    /// `# sweep: avoid-cycle` — shorthand for ignoring local-imports
    /// with cycle-avoidance as the stated reason.
    AvoidCycle,
}

/// Suppression directives indexed by the 1-based line they appear on.
/// A diagnostic is suppressed by a directive on its own line or the
/// line directly above.
#[derive(Debug, Default)]
pub struct Suppressions {
    by_line: HashMap<usize, Vec<Directive>>,
}

impl Suppressions {
    pub fn from_tree(root: Node, source: &str, line_index: &LineIndex) -> Self {
        let mut by_line: HashMap<usize, Vec<Directive>> = HashMap::new();
        walk_tree(root, &mut |node| {
            if node.kind() != "comment" {
                return;
            }
            let text = &source[node.byte_range()];
            if let Some(directive) = parse_directive(text) {
                let line = line_index.line(node.start_byte());
                by_line.entry(line).or_default().push(directive);
            }
        });
        Self { by_line }
    }

    pub fn is_suppressed(&self, rule: &str, line: usize) -> bool {
        [line, line.saturating_sub(1)]
            .iter()
            .filter_map(|l| self.by_line.get(l))
            .flatten()
            .any(|d| match d {
                Directive::IgnoreAll => true,
                Directive::Ignore(rules) => rules.iter().any(|r| r == rule),
                Directive::AvoidCycle => rule == "local-imports",
            })
    }
}

fn parse_directive(comment: &str) -> Option<Directive> {
    let text = comment.trim_start_matches('#').trim();
    let rest = text.strip_prefix("sweep:")?.trim();

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

    #[test]
    fn parses_directives() {
        assert_eq!(
            parse_directive("# sweep: ignore"),
            Some(Directive::IgnoreAll)
        );
        assert_eq!(
            parse_directive("# sweep: ignore[local-imports] heavy dep"),
            Some(Directive::Ignore(vec!["local-imports".into()]))
        );
        assert_eq!(
            parse_directive("# sweep: avoid-cycle models <-> tasks"),
            Some(Directive::AvoidCycle)
        );
        assert_eq!(parse_directive("# regular comment"), None);
        assert_eq!(parse_directive("# noqa: F401"), None);
    }
}
