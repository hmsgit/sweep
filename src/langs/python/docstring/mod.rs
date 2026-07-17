//! Docstring style detection, parsing into a style-neutral IR, and
//! re-rendering in the configured style.

pub mod markup;
mod parse;
mod render;

pub use markup::markup_issues;

use tree_sitter::Node;

use crate::engine::config::DocStyle;
use crate::engine::context::walk_tree;
use crate::engine::fix::{Edit, Fix};
use crate::langs::python::{docstring_of_statement, line_start, module_docstring};

/// Style-neutral representation of a structured docstring. Lines are
/// stored dedented; renderers re-indent for the target style and the
/// rule re-applies the docstring's base indentation.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DocIr {
    /// Summary and free-form description lines before any section.
    pub preamble: Vec<String>,
    pub params: Vec<Field>,
    pub attributes: Vec<Field>,
    pub returns: Option<Value>,
    pub yields: Option<Value>,
    pub raises: Vec<Field>,
    /// Passthrough sections (Examples, Notes, …) kept verbatim.
    pub extras: Vec<(String, Vec<String>)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: Option<String>,
    pub desc: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Value {
    pub ty: Option<String>,
    pub desc: Vec<String>,
}

impl DocIr {
    pub fn has_sections(&self) -> bool {
        !self.params.is_empty()
            || !self.attributes.is_empty()
            || self.returns.is_some()
            || self.yields.is_some()
            || !self.raises.is_empty()
            || !self.extras.is_empty()
    }
}

const GOOGLE_SIGNATURE_HEADERS: &[&str] = &[
    "Args",
    "Arguments",
    "Keyword Args",
    "Keyword Arguments",
    "Returns",
    "Yields",
    "Raises",
    "Attributes",
    "Warns",
];

const NUMPY_SIGNATURE_HEADERS: &[&str] = &[
    "Parameters",
    "Other Parameters",
    "Returns",
    "Yields",
    "Raises",
    "Attributes",
    "Warns",
    "Receives",
];

const REST_FIELD_KEYWORDS: &[&str] = &[
    "param",
    "parameter",
    "arg",
    "argument",
    "key",
    "keyword",
    "type",
    "return",
    "returns",
    "rtype",
    "raise",
    "raises",
    "except",
    "exception",
    "yield",
    "yields",
    "ytype",
    "var",
    "ivar",
    "cvar",
    "vartype",
];

/// Detect which convention a docstring's *sections* follow. `None` means
/// the docstring has no recognizable section markers (plain prose is fine
/// in any convention).
pub fn detect(content: &str) -> Option<DocStyle> {
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // NumPy: a known header underlined with dashes on the next line.
        if NUMPY_SIGNATURE_HEADERS.contains(&trimmed)
            && let Some(next) = lines.get(i + 1)
        {
            let underline = next.trim();
            if underline.len() >= 3 && underline.chars().all(|c| c == '-') {
                return Some(DocStyle::Numpy);
            }
        }

        // Google: a known header ending with a colon on its own line.
        if let Some(header) = trimmed.strip_suffix(':')
            && GOOGLE_SIGNATURE_HEADERS.contains(&header)
        {
            return Some(DocStyle::Google);
        }

        // reST: a field list line like `:param name: …`.
        if let Some(rest) = trimmed.strip_prefix(':')
            && let Some((spec, _)) = rest.split_once(':')
        {
            let keyword = spec.split_whitespace().next().unwrap_or("");
            if REST_FIELD_KEYWORDS.contains(&keyword) {
                return Some(DocStyle::Rest);
            }
        }
    }
    None
}

/// Parse + re-render in one step. `None` when the docstring can't be
/// converted losslessly (the rule then warns without a fix). With
/// `width: Some(n)` prose is re-flowed to fit `n` columns.
pub fn convert(
    content: &str,
    from: DocStyle,
    to: DocStyle,
    width: Option<usize>,
    first_line_penalty: usize,
) -> Option<String> {
    let ir = parse::parse(content, from)?;
    if !ir.has_sections() {
        return None;
    }
    Some(render::render(&ir, to, width, first_line_penalty))
}

/// Re-render a docstring in its own style with prose re-flowed to fit
/// `width` columns. Unlike [`convert`], plain-prose docstrings without
/// sections are fine — there is still text to wrap.
pub fn rewrap(
    content: &str,
    style: DocStyle,
    width: usize,
    first_line_penalty: usize,
) -> Option<String> {
    let ir = parse::parse(content, style)?;
    Some(render::render(&ir, style, Some(width), first_line_penalty))
}

/// Parse a docstring into the style-neutral IR (for rules that inspect
/// or rebuild sections).
pub fn parse_ir(content: &str, style: DocStyle) -> Option<DocIr> {
    parse::parse(content, style)
}

/// Render an IR back to dedented docstring content, preserving the
/// author's line wrapping.
pub fn render_ir(ir: &DocIr, style: DocStyle) -> String {
    render::render(ir, style, None, 0)
}

/// Function docstrings with their owning function definition.
pub fn function_docstrings(root: Node) -> Vec<(Node, Node)> {
    let mut found = Vec::new();
    walk_tree(root, &mut |node| {
        if node.kind() != "function_definition" {
            return;
        }
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        let Some(first) = body.named_child(0) else {
            return;
        };
        if let Some(doc) = docstring_of_statement(first) {
            found.push((node, doc));
        }
    });
    found
}

/// Every docstring in a parsed file: module, class and function bodies.
pub fn docstrings(root: Node) -> Vec<Node> {
    let mut found = Vec::new();
    if let Some(doc) = module_docstring(root) {
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
/// byte prefixes and concatenations, where rewriting content could
/// change semantics.
pub fn content_range(string: Node, source: &str) -> Option<(usize, usize)> {
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

/// The whitespace before the docstring's opening quotes — its base
/// indentation. `None` if anything else shares the line.
pub fn base_indent<'a>(string: Node, source: &'a str) -> Option<&'a str> {
    let stmt_line_start = line_start(source, string.start_byte());
    let indent = &source[stmt_line_start..string.start_byte()];
    indent
        .chars()
        .all(|c| c == ' ' || c == '\t')
        .then_some(indent)
}

/// Build the fix that replaces a docstring's content with `rendered`
/// (dedented lines), re-applying the base indentation and keeping the
/// closing-quote shape. `None` when splicing is unsafe or a no-op.
pub fn splice_fix(
    string: Node,
    source: &str,
    content_start: usize,
    content: &str,
    rendered: &str,
) -> Option<Fix> {
    let indent = base_indent(string, source)?;

    let mut new_content = String::new();
    for line in rendered.lines() {
        new_content.push('\n');
        if !line.is_empty() {
            new_content.push_str(indent);
        }
        new_content.push_str(line);
    }
    let multi_line = rendered.contains('\n');
    if !multi_line {
        // Single-line content stays inline with the quotes.
        new_content = rendered.to_string();
    }

    let is_triple = source[string.start_byte()..content_start].contains("\"\"\"")
        || source[string.start_byte()..content_start].contains("'''");
    let closes_on_own_line = content.trim_end_matches([' ', '\t']).ends_with('\n');
    if multi_line {
        // Multi-line content needs triple quotes; it starts on the line
        // after the opening quotes, aligned with them. Closing quotes
        // keep the author's placement (own line only if it was so).
        if !is_triple {
            return None;
        }
        if closes_on_own_line {
            new_content.push('\n');
            new_content.push_str(indent);
        }
    } else if is_triple && closes_on_own_line {
        // Preserve an existing closing-quotes-on-own-line shape.
        new_content.push('\n');
        new_content.push_str(indent);
    }

    if new_content == content {
        return None;
    }
    Some(Fix::new(vec![Edit::replace(
        content_start,
        content_start + content.len(),
        new_content,
    )]))
}

/// Strip the common leading indentation from every line after the first.
pub(crate) fn dedent(lines: &[&str]) -> Vec<String> {
    let indent = lines
        .iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                l.trim_end().to_string()
            } else if l.trim().is_empty() {
                String::new()
            } else {
                l[indent.min(l.len() - l.trim_start().len())..]
                    .trim_end()
                    .to_string()
            }
        })
        .collect()
}

/// Leading-space count of a line.
pub(crate) fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Split `line` at the first `:` that sits outside (), [] and {}.
pub(crate) fn split_at_top_level_colon(line: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (i, c) in line.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ':' if depth == 0 => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_google() {
        let doc = "Do a thing.\n\n    Args:\n        x: the input\n";
        assert_eq!(detect(doc), Some(DocStyle::Google));
    }

    #[test]
    fn detects_numpy() {
        let doc = "Do a thing.\n\n    Parameters\n    ----------\n    x : int\n";
        assert_eq!(detect(doc), Some(DocStyle::Numpy));
    }

    #[test]
    fn detects_rest() {
        let doc = "Do a thing.\n\n    :param x: the input\n";
        assert_eq!(detect(doc), Some(DocStyle::Rest));
    }

    #[test]
    fn plain_prose_is_no_style() {
        assert_eq!(detect("Just a summary line."), None);
        assert_eq!(detect("Note: this is fine.\nExample: also fine."), None);
    }
}
