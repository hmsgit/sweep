use tree_sitter::Node;

use crate::engine::config::Level;
use crate::engine::context::{FileContext, walk_tree};
use crate::engine::diagnostic::{Diagnostic, Severity};
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{convert, detect, markup_issues};
use crate::langs::python::{docstring_of_statement, line_start};

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

                    if let Some(fix) = conversion_fix(
                        string,
                        ctx.source,
                        content,
                        content_start,
                        style,
                        configured,
                    ) {
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

/// Every docstring in the file: module, class and function bodies.
fn docstrings(root: Node) -> Vec<Node> {
    let mut found = Vec::new();
    if let Some(doc) = super::super::module_docstring(root) {
        found.push(doc);
    }
    walk_tree(root, &mut |node| {
        if !matches!(node.kind(), "function_definition" | "class_definition") {
            return;
        }
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        let Some(first) = body.named_child(0) else {
            return;
        };
        if let Some(doc) = docstring_of_statement(first) {
            found.push(doc);
        }
    });
    found
}

/// Byte range of the text between the quotes. Bails on f-strings,
/// raw/byte prefixes with escapes we'd mangle, and concatenations.
fn content_range(string: Node, source: &str) -> Option<(usize, usize)> {
    let mut start = None;
    let mut end = None;
    let mut cursor = string.walk();
    for child in string.children(&mut cursor) {
        match child.kind() {
            "string_start" => start = Some(child.end_byte()),
            "string_end" => end = Some(child.start_byte()),
            "string_content" => {}
            _ => return None,
        }
    }
    let (start, end) = (start?, end?);
    // Only plain or raw triple/single quoted strings; reject prefixes
    // like f/b where rewriting content could change semantics.
    let opener = &source[string.start_byte()..start];
    if !opener
        .trim_end_matches(['"', '\''])
        .chars()
        .all(|c| matches!(c, 'r' | 'R' | 'u' | 'U'))
    {
        return None;
    }
    Some((start, end))
}

fn conversion_fix(
    string: Node,
    source: &str,
    content: &str,
    content_start: usize,
    from: crate::engine::config::DocStyle,
    to: crate::engine::config::DocStyle,
) -> Option<Fix> {
    let rendered = convert(content, from, to)?;

    // Re-apply the docstring's base indentation (the whitespace before
    // the opening quotes) to every line after the first.
    let stmt_line_start = line_start(source, string.start_byte());
    let indent = &source[stmt_line_start..string.start_byte()];
    if !indent.chars().all(|c| c == ' ' || c == '\t') {
        return None;
    }

    let mut new_content = String::new();
    for (i, line) in rendered.lines().enumerate() {
        if i > 0 {
            new_content.push('\n');
            if !line.is_empty() {
                new_content.push_str(indent);
            }
        }
        new_content.push_str(line);
    }

    // Multi-line content needs triple quotes and a closing-quote line.
    let is_triple = source[string.start_byte()..content_start].contains("\"\"\"")
        || source[string.start_byte()..content_start].contains("'''");
    if new_content.contains('\n') {
        if !is_triple {
            return None;
        }
        new_content.push('\n');
        new_content.push_str(indent);
    } else if is_triple && content.trim_end_matches([' ', '\t']).ends_with('\n') {
        // Preserve an existing closing-quotes-on-own-line shape.
        new_content.push('\n');
        new_content.push_str(indent);
    }

    Some(Fix::new(vec![Edit::replace(
        content_start,
        content_start + content.len(),
        new_content,
    )]))
}
