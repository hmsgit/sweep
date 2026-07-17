use crate::engine::context::FileContext;
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{content_range, detect, function_docstrings, parse_ir};
use crate::langs::python::rules::comments_no_echo::{loose_eq, words};
use crate::langs::python::{function_params, line_end_inclusive, line_start};

/// Flags docstrings that only restate the function's name (and
/// parameters): `def send_email(): """Send email."""` documents
/// nothing. A docstring survives if it has sections or says any word
/// not already in the signature.
pub struct DocstringNoEcho;

const GLUE: &[&str] = &[
    "the", "a", "an", "this", "that", "of", "to", "for", "and", "or", "in", "on", "with", "from",
    "is", "are", "be", "it", "its", "given", "new",
];

impl Rule for DocstringNoEcho {
    fn name(&self) -> &'static str {
        "docstring-no-echo"
    }

    fn explain(&self) -> &'static str {
        "docstrings that only restate the function name document nothing"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.docstring_no_echo_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        for (function, string) in function_docstrings(ctx.root()) {
            let Some((content_start, content_end)) = content_range(string, ctx.source) else {
                continue;
            };
            let content = &ctx.source[content_start..content_end];
            let style = detect(content).unwrap_or(ctx.config.docstring_style);
            let Some(ir) = parse_ir(content, style) else {
                continue;
            };
            if ir.has_sections() || ir.preamble.is_empty() {
                continue;
            }

            // Signature vocabulary: name parts and parameter name parts.
            let mut vocabulary: Vec<String> = Vec::new();
            if let Some(name) = function.child_by_field_name("name") {
                vocabulary.extend(words(&ctx.source[name.byte_range()]));
            }
            for (param, _) in function_params(function, ctx.source) {
                vocabulary.extend(words(&param));
            }

            let doc_words: Vec<String> = ir
                .preamble
                .iter()
                .flat_map(|line| words(line))
                .filter(|w| !GLUE.contains(&w.as_str()))
                .collect();
            if doc_words.is_empty()
                || !doc_words
                    .iter()
                    .all(|w| vocabulary.iter().any(|v| loose_eq(w, v)))
            {
                continue;
            }

            let mut diagnostic = Diagnostic::new(
                self.name(),
                "docstring only restates the function name; say more or drop it".to_string(),
                string.start_byte(),
                string.end_byte(),
            )
            .with_severity(severity);

            if level.applies_fixes()
                && let Some(stmt) = string.parent()
            {
                let block = stmt.parent();
                let edit = if block.is_some_and(|b| b.named_child_count() == 1) {
                    Edit::replace(stmt.start_byte(), stmt.end_byte(), "pass".to_string())
                } else {
                    Edit::delete(
                        line_start(ctx.source, stmt.start_byte()),
                        line_end_inclusive(ctx.source, stmt.end_byte()),
                    )
                };
                diagnostic = diagnostic.with_fix(Fix::new(vec![edit]));
            }
            diagnostics.push(diagnostic);
        }
        diagnostics
    }
}
