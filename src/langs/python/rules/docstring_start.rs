use crate::engine::config::DocStart;
use crate::engine::context::FileContext;
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{base_indent, content_range, docstrings};

/// Where a multi-line docstring's content starts. The default,
/// `next-line` (pydocstyle D213), puts it on the line after the opening
/// quotes, aligned with them:
///
/// ```text
/// """
/// Summary line.
///
/// :param x: ...
/// """
/// ```
///
/// `same-line` (D212) keeps the summary on the opening-quote line:
///
/// ```text
/// """Summary line.
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
        "multi-line docstrings start content on the configured side of the opening quotes"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.docstring_start.level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };
        let start = ctx.config.docstring_start.start;

        let mut diagnostics = Vec::new();
        for string in docstrings(ctx.root()) {
            let Some((content_start, content_end)) = content_range(string, ctx.source) else {
                continue;
            };
            let content = &ctx.source[content_start..content_end];
            if !content.contains('\n') || content.trim().is_empty() {
                continue;
            }
            let first_line = &content[..content.find('\n').unwrap_or(content.len())];
            let first_line_blank = first_line.trim().is_empty();

            let (message, fix) = match start {
                DocStart::NextLine => {
                    if first_line_blank {
                        continue;
                    }
                    let fix = base_indent(string, ctx.source).map(|indent| {
                        // Replace the first line (not a zero-width insert) so
                        // this fix conflicts cleanly with whole-docstring
                        // rewrites from other rules and the fixpoint loop
                        // orders them.
                        Fix::new(vec![Edit::replace(
                            content_start,
                            content_start + first_line.len(),
                            format!("\n{indent}{}", first_line.trim_end()),
                        )])
                    });
                    (
                        "multi-line docstring; start the content on the line after the \
                         opening quotes",
                        fix,
                    )
                }
                DocStart::SameLine => {
                    if !first_line_blank {
                        continue;
                    }
                    // Pull the first content line (past any blank lines and
                    // its indentation) up next to the opening quotes.
                    let lead = content.len() - content.trim_start().len();
                    let fix = Some(Fix::new(vec![Edit::replace(
                        content_start,
                        content_start + lead,
                        String::new(),
                    )]));
                    (
                        "multi-line docstring; start the content on the opening-quote line",
                        fix,
                    )
                }
            };

            let mut diagnostic = Diagnostic::new(
                self.name(),
                message.to_string(),
                string.start_byte(),
                content_start + first_line.len(),
            )
            .with_severity(severity);
            if level.applies_fixes()
                && let Some(fix) = fix
            {
                diagnostic = diagnostic.with_fix(fix);
            }
            diagnostics.push(diagnostic);
        }
        diagnostics
    }
}
