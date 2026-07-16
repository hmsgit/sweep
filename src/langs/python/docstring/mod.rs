//! Docstring style detection, parsing into a style-neutral IR, and
//! re-rendering in the configured style.

pub mod markup;
mod parse;
mod render;

pub use markup::markup_issues;

use crate::engine::config::DocStyle;

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
/// converted losslessly (the rule then warns without a fix).
pub fn convert(content: &str, from: DocStyle, to: DocStyle) -> Option<String> {
    let ir = parse::parse(content, from)?;
    if !ir.has_sections() {
        return None;
    }
    Some(render::render(&ir, to))
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
