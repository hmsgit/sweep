use crate::engine::context::{FileContext, walk_tree};
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{content_range, docstrings};

/// Flags characters from the configured banned set (typically emoji
/// and unicode decorations like ✓/✗) anywhere in the file. Occurrences
/// in comments and docstrings are deleted under --fix; inside other
/// string literals they are warn-only, since deleting could change
/// runtime behavior.
pub struct BannedEmoji;

impl Rule for BannedEmoji {
    fn name(&self) -> &'static str {
        "banned-emoji"
    }

    fn explain(&self) -> &'static str {
        "banned characters (configured via banned-emoji = \"…\") must not appear in code"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.banned_emoji_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };
        let banned = &ctx.config.banned_emoji_chars;
        if banned.is_empty() {
            return Vec::new();
        }

        // Ranges where deletion is safe: comments and docstring content.
        let mut deletable: Vec<(usize, usize)> = Vec::new();
        walk_tree(ctx.root(), &mut |node| {
            if node.kind() == "comment" {
                deletable.push((node.start_byte(), node.end_byte()));
            }
        });
        for string in docstrings(ctx.root()) {
            if let Some((start, end)) = content_range(string, ctx.source) {
                deletable.push((start, end));
            }
        }

        let mut diagnostics = Vec::new();
        for (offset, c) in ctx.source.char_indices() {
            if !banned.contains(&c) {
                continue;
            }
            let end = offset + c.len_utf8();
            let safe_to_delete = deletable.iter().any(|(s, e)| *s <= offset && end <= *e);

            let mut diagnostic =
                Diagnostic::new(self.name(), format!("banned character `{c}`"), offset, end)
                    .with_severity(severity);
            if level.applies_fixes() && safe_to_delete {
                // Also swallow one preceding space so "done ✓" -> "done".
                let start = if ctx.source[..offset].ends_with(' ') {
                    offset - 1
                } else {
                    offset
                };
                diagnostic = diagnostic.with_fix(Fix::new(vec![Edit::delete(start, end)]));
            }
            diagnostics.push(diagnostic);
        }
        diagnostics
    }
}
