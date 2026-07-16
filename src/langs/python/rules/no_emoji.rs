use crate::engine::context::{FileContext, walk_tree};
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{content_range, docstrings};

/// Flags emoji and unicode icons (pictographs, dingbats like ✓/✗,
/// arrows, geometric shapes) anywhere in the file, except characters
/// in the configured allowed set. Occurrences in comments and
/// docstrings are deleted under --fix; inside other string literals
/// they are warn-only, since deleting could change runtime behavior.
pub struct NoEmoji;

impl Rule for NoEmoji {
    fn name(&self) -> &'static str {
        "no-emoji"
    }

    fn explain(&self) -> &'static str {
        "no emoji or unicode icons in code (exceptions via allowed-emojis)"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.no_emoji_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };
        let allowed = &ctx.config.allowed_emojis;

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
            if !is_icon(c) || allowed.contains(&c) {
                continue;
            }
            let end = offset + c.len_utf8();
            let safe_to_delete = deletable.iter().any(|(s, e)| *s <= offset && end <= *e);

            let mut diagnostic = Diagnostic::new(
                self.name(),
                format!("emoji/icon character `{c}`"),
                offset,
                end,
            )
            .with_severity(severity);
            if level.applies_fixes() && safe_to_delete {
                // Also swallow one preceding space so "done ✓" -> "done",
                // and any trailing variation selector / joiner.
                let start = if ctx.source[..offset].ends_with(' ') {
                    offset - 1
                } else {
                    offset
                };
                let mut delete_end = end;
                for follower in ctx.source[end..].chars() {
                    if matches!(follower, '\u{FE0F}' | '\u{200D}') {
                        delete_end += follower.len_utf8();
                    } else {
                        break;
                    }
                }
                diagnostic = diagnostic.with_fix(Fix::new(vec![Edit::delete(start, delete_end)]));
            }
            diagnostics.push(diagnostic);
        }
        diagnostics
    }
}

/// Emoji and icon-like characters worth flagging. Invisible emoji
/// plumbing (variation selectors, zero-width joiner) is never flagged
/// on its own.
fn is_icon(c: char) -> bool {
    matches!(u32::from(c),
        0x1F000..=0x1FAFF   // emoji, pictographs, transport, supplemental
        | 0x2600..=0x27BF   // misc symbols + dingbats (✓ ✗ ✅ ❌ ☀ …)
        | 0x2B00..=0x2BFF   // arrows, stars (⭐)
        | 0x2190..=0x21FF   // arrows (→ ⇒)
        | 0x2300..=0x23FF   // misc technical (⌘ ⏰)
        | 0x25A0..=0x25FF   // geometric shapes (● ▶)
    )
}
