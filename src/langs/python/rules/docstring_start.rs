use crate::engine::context::FileContext;
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{base_indent, content_range, docstrings};

/// Multi-line docstrings start their content on the line after the
/// opening quotes, aligned with them (pydocstyle D213 style):
///
/// ```text
/// """
/// Summary line.
///
/// :param x: ...
/// """
/// ```
///
/// Single-line docstrings stay inline; the closing quotes are the
/// author's business and are never touched.
pub struct DocstringStart;

impl Rule for DocstringStart {
    fn name(&self) -> &'static str {
        "docstring-start"
    }

    fn explain(&self) -> &'static str {
        "multi-line docstrings start content on the line after the opening quotes"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.docstring_start_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        for string in docstrings(ctx.root()) {
            let Some((content_start, content_end)) = content_range(string, ctx.source) else {
                continue;
            };
            let content = &ctx.source[content_start..content_end];
            if !content.contains('\n') {
                continue;
            }
            let first_line = &content[..content.find('\n').unwrap_or(content.len())];
            if first_line.trim().is_empty() {
                continue;
            }

            let mut diagnostic = Diagnostic::new(
                self.name(),
                "multi-line docstring; start the content on the line after the opening quotes"
                    .to_string(),
                string.start_byte(),
                content_start + first_line.len(),
            )
            .with_severity(severity);

            if level.applies_fixes()
                && let Some(indent) = base_indent(string, ctx.source)
            {
                // Replace the first line (not a zero-width insert) so this
                // fix conflicts cleanly with whole-docstring rewrites from
                // other rules and the fixpoint loop orders them.
                diagnostic = diagnostic.with_fix(Fix::new(vec![Edit::replace(
                    content_start,
                    content_start + first_line.len(),
                    format!("\n{indent}{}", first_line.trim_end()),
                )]));
            }
            diagnostics.push(diagnostic);
        }
        diagnostics
    }
}
