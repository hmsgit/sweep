use crate::engine::context::{FileContext, walk_tree};
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{content_range, docstrings};

/// Flags typographic dashes — em dash, en dash, horizontal bar — that
/// belong to prose typography, not code. In comments and docstrings
/// --fix replaces them with an ASCII hyphen; inside other string
/// literals they are warn-only, since rewriting could change
/// user-facing or serialized text.
pub struct NoEmdash;

impl Rule for NoEmdash {
    fn name(&self) -> &'static str {
        "no-emdash"
    }

    fn explain(&self) -> &'static str {
        "typographic dashes (em/en dash) become ASCII hyphens"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.no_emdash_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

        // Ranges where rewriting is safe: comments and docstring content.
        let mut fixable: Vec<(usize, usize)> = Vec::new();
        walk_tree(ctx.root(), &mut |node| {
            if node.kind() == "comment" {
                fixable.push((node.start_byte(), node.end_byte()));
            }
        });
        for string in docstrings(ctx.root()) {
            if let Some((start, end)) = content_range(string, ctx.source) {
                fixable.push((start, end));
            }
        }

        let mut diagnostics = Vec::new();
        for (offset, c) in ctx.source.char_indices() {
            let Some(dash) = dash_name(c) else {
                continue;
            };
            let end = offset + c.len_utf8();
            let safe_to_fix = fixable.iter().any(|(s, e)| *s <= offset && end <= *e);

            let mut diagnostic = Diagnostic::new(
                self.name(),
                format!("{dash} `{c}` should be a hyphen"),
                offset,
                end,
            )
            .with_severity(severity);
            if level.applies_fixes() && safe_to_fix {
                diagnostic = diagnostic.with_fix(Fix::new(vec![Edit::replace(
                    offset,
                    end,
                    "-".to_string(),
                )]));
            }
            diagnostics.push(diagnostic);
        }
        diagnostics
    }
}

fn dash_name(c: char) -> Option<&'static str> {
    match c {
        '\u{2014}' => Some("em dash"),
        '\u{2013}' => Some("en dash"),
        '\u{2015}' => Some("horizontal bar"),
        _ => None,
    }
}
