use crate::engine::config::Level;
use crate::engine::context::FileContext;
use crate::engine::diagnostic::{Diagnostic, Severity};
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{
    base_indent, content_range, detect, docstrings, rewrap, splice_fix,
};
use crate::langs::python::line_start;

/// Reports docstring lines that exceed the configured line length
/// (default 79, or ruff's line-length when set). By default this only
/// informs; with `fix = "rewrap"` --fix re-flows the docstring's prose
/// to fit. Non-prose content (bullets, doctests, directives) is never
/// re-flowed.
pub struct DocstringLineLength;

impl Rule for DocstringLineLength {
    fn name(&self) -> &'static str {
        "docstring-line-length"
    }

    fn explain(&self) -> &'static str {
        "docstring lines must fit the configured line-length (fix = \"rewrap\" enables re-flow)"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let config = &ctx.config.docstring_line_length;
        if config.level == Level::Off {
            return Vec::new();
        }
        let severity = match config.level {
            Level::Error => Severity::Error,
            _ => Severity::Warning,
        };
        let limit = ctx.config.line_length;

        let mut diagnostics = Vec::new();
        for string in docstrings(ctx.root()) {
            let mut fix = None;
            if config.rewrap {
                fix = rewrap_fix(ctx, string, limit);
            }

            // Measure every source line the docstring spans, quotes and
            // indentation included.
            let span_start = line_start(ctx.source, string.start_byte());
            let mut offset = span_start;
            for line in ctx.source[span_start..string.end_byte()].split_inclusive('\n') {
                let text = line.trim_end_matches('\n');
                let length = text.chars().count();
                if length > limit {
                    let mut diagnostic = Diagnostic::new(
                        self.name(),
                        format!("docstring line is {length} characters ({limit} allowed)"),
                        offset,
                        offset + text.len(),
                    )
                    .with_severity(severity);
                    // One rewrap fix per docstring, carried by the first
                    // overlong line; fixing it resolves the others too.
                    if let Some(f) = fix.take() {
                        diagnostic = diagnostic.with_fix(f);
                    }
                    diagnostics.push(diagnostic);
                }
                offset += line.len();
            }
        }
        diagnostics
    }
}

fn rewrap_fix(
    ctx: &FileContext,
    string: tree_sitter::Node,
    limit: usize,
) -> Option<crate::engine::fix::Fix> {
    let (content_start, content_end) = content_range(string, ctx.source)?;
    let content = &ctx.source[content_start..content_end];

    // A docstring in the wrong convention is docstring-style's job first;
    // its converted output is already wrapped when rewrap is on.
    let configured = ctx.config.docstring_style;
    if detect(content).is_some_and(|style| style != configured) {
        return None;
    }

    let indent_len = base_indent(string, ctx.source)?.len();
    let width = limit.saturating_sub(indent_len).max(24);
    let quote_len = content_start - string.start_byte();
    let rendered = rewrap(content, configured, width, quote_len)?;
    splice_fix(string, ctx.source, content_start, content, &rendered)
}
