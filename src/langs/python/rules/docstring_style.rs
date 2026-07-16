use crate::engine::config::Level;
use crate::engine::context::FileContext;
use crate::engine::diagnostic::{Diagnostic, Severity};
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{
    base_indent, content_range, convert, detect, docstrings, markup_issues, splice_fix,
};

/// Enforces the configured docstring convention (reST/Google/NumPy).
/// Docstrings whose sections follow another convention are converted
/// under --fix; inline markup that doesn't match the convention (e.g.
/// Markdown `spans` in reST docstrings) is rewritten too.
pub struct DocstringStyle;

impl Rule for DocstringStyle {
    fn name(&self) -> &'static str {
        "docstring-style"
    }

    fn explain(&self) -> &'static str {
        "docstrings must follow the configured convention (docstring-style = rest|google|numpy)"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        if ctx.config.docstring_level == Level::Off {
            return Vec::new();
        }
        let severity = match ctx.config.docstring_level {
            Level::Error => Severity::Error,
            _ => Severity::Warning,
        };
        let configured = ctx.config.docstring_style;

        let mut diagnostics = Vec::new();
        for string in docstrings(ctx.root()) {
            let Some((content_start, content_end)) = content_range(string, ctx.source) else {
                continue;
            };
            let content = &ctx.source[content_start..content_end];

            match detect(content) {
                Some(style) if style != configured => {
                    let mut diagnostic = Diagnostic::new(
                        self.name(),
                        format!("docstring is {style}-style; project convention is {configured}"),
                        string.start_byte(),
                        string.end_byte(),
                    )
                    .with_severity(severity);

                    // When rewrap is on, converted output is wrapped to the
                    // line length right away; otherwise original wrapping
                    // is preserved.
                    let width = if ctx.config.docstring_line_length.rewrap {
                        let indent_len = base_indent(string, ctx.source).map_or(0, str::len);
                        Some(ctx.config.line_length.saturating_sub(indent_len).max(24))
                    } else {
                        None
                    };
                    let quote_len = content_start - string.start_byte();
                    if let Some(rendered) = convert(content, style, configured, width, quote_len)
                        && let Some(fix) =
                            splice_fix(string, ctx.source, content_start, content, &rendered)
                    {
                        diagnostic = diagnostic.with_fix(fix);
                    }
                    diagnostics.push(diagnostic);
                }
                _ => {
                    // Convention matches (or no sections): still check
                    // that inline markup follows the convention.
                    for issue in markup_issues(content, configured) {
                        diagnostics.push(
                            Diagnostic::new(
                                self.name(),
                                issue.message,
                                content_start + issue.start,
                                content_start + issue.end,
                            )
                            .with_severity(severity)
                            .with_fix(Fix::new(vec![Edit::replace(
                                content_start + issue.start,
                                content_start + issue.end,
                                issue.replacement,
                            )])),
                        );
                    }
                }
            }
        }
        diagnostics
    }
}
