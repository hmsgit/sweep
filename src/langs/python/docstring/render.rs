//! Renderers from the neutral [`DocIr`] into each docstring convention.
//! Output lines are dedented; the rule re-applies the docstring's base
//! indentation when splicing the content back between the quotes.
//!
//! With `width: Some(n)` prose descriptions are re-flowed to fit `n`
//! columns (in dedented space — callers subtract the docstring's base
//! indent). Anything that doesn't look like plain prose (bullets,
//! doctests, directives) keeps its original line breaks; `None`
//! preserves the author's wrapping everywhere.

use crate::engine::config::DocStyle;

use super::{DocIr, Field, Value};

/// `first_line_penalty` is how many extra columns the docstring's very
/// first line already spends on the opening quotes (usually 3), so the
/// summary wraps to fit alongside them.
pub fn render(
    ir: &DocIr,
    style: DocStyle,
    width: Option<usize>,
    first_line_penalty: usize,
) -> String {
    let mut lines: Vec<String> = flow_paragraphs(&ir.preamble, 0, width, first_line_penalty);
    let mut push_section = |section: Vec<String>| {
        if section.is_empty() {
            return;
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(section);
    };

    match style {
        DocStyle::Rest => {
            let mut fields: Vec<String> = Vec::new();
            for p in &ir.params {
                emit_desc(rest_label("param", &p.name), 4, &p.desc, width, &mut fields);
                if let Some(ty) = &p.ty {
                    fields.push(format!(":type {}: {}", p.name, ty));
                }
            }
            if let Some(r) = &ir.returns {
                rest_value("returns", "rtype", r, width, &mut fields);
            }
            if let Some(y) = &ir.yields {
                rest_value("yields", "ytype", y, width, &mut fields);
            }
            for r in &ir.raises {
                emit_desc(
                    rest_label(&format!("raises {}", r.name), ""),
                    4,
                    &r.desc,
                    width,
                    &mut fields,
                );
            }
            for a in &ir.attributes {
                emit_desc(rest_label("ivar", &a.name), 4, &a.desc, width, &mut fields);
                if let Some(ty) = &a.ty {
                    fields.push(format!(":vartype {}: {}", a.name, ty));
                }
            }
            push_section(fields);
            for (title, body) in &ir.extras {
                let mut section = vec![format!("{title}:")];
                section.extend(body.iter().map(|l| indent_line(l, 4)));
                push_section(section);
            }
        }
        DocStyle::Google => {
            push_section(google_entries("Args", &ir.params, width));
            if let Some(r) = &ir.returns {
                push_section(google_value("Returns", r, width));
            }
            if let Some(y) = &ir.yields {
                push_section(google_value("Yields", y, width));
            }
            push_section(google_entries("Raises", &ir.raises, width));
            push_section(google_entries("Attributes", &ir.attributes, width));
            for (title, body) in &ir.extras {
                let mut section = vec![format!("{title}:")];
                section.extend(body.iter().map(|l| indent_line(l, 4)));
                push_section(section);
            }
        }
        DocStyle::Numpy => {
            push_section(numpy_entries("Parameters", &ir.params, width));
            if let Some(r) = &ir.returns {
                push_section(numpy_value("Returns", r, width));
            }
            if let Some(y) = &ir.yields {
                push_section(numpy_value("Yields", y, width));
            }
            push_section(numpy_entries("Raises", &ir.raises, width));
            push_section(numpy_entries("Attributes", &ir.attributes, width));
            for (title, body) in &ir.extras {
                let mut section = numpy_header(title);
                section.extend(body.iter().cloned());
                push_section(section);
            }
        }
    }

    lines.join("\n")
}

fn indent_line(line: &str, spaces: usize) -> String {
    if line.trim().is_empty() {
        String::new()
    } else {
        format!("{}{}", " ".repeat(spaces), line)
    }
}

/// Only re-flow text that is clearly plain prose. Bullets, numbered
/// lists, doctests, reST directives and literal-block introducers keep
/// their line structure.
fn is_prose(lines: &[String]) -> bool {
    lines.iter().all(|line| {
        let t = line.trim_start();
        if t.is_empty() {
            return true;
        }
        let bullet =
            matches!(t.as_bytes()[0], b'-' | b'*' | b'+') && t.len() > 1 && t.as_bytes()[1] == b' ';
        let numbered = t.split_once(['.', ')']).is_some_and(|(n, rest)| {
            !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && rest.starts_with(' ')
        });
        !(bullet || numbered || t.starts_with(">>>") || t.starts_with(".. ") || t.ends_with("::"))
    })
}

/// Emit a description after `first_prefix` (e.g. `":param x: "`), with
/// continuation lines indented by `cont_indent`. With a width and plain
/// prose, paragraphs are re-flowed; otherwise original lines are kept.
fn emit_desc(
    first_prefix: String,
    cont_indent: usize,
    desc: &[String],
    width: Option<usize>,
    out: &mut Vec<String>,
) {
    if desc.is_empty() {
        out.push(first_prefix.trim_end().to_string());
        return;
    }

    match width {
        Some(w) if is_prose(desc) => {
            let mut first = Some(first_prefix);
            for paragraph in desc.split(|l| l.trim().is_empty()) {
                if paragraph.is_empty() {
                    continue;
                }
                let lead = match first.take() {
                    Some(prefix) => prefix,
                    None => {
                        out.push(String::new());
                        " ".repeat(cont_indent)
                    }
                };
                wrap_words(
                    paragraph.iter().flat_map(|l| l.split_whitespace()),
                    lead,
                    cont_indent,
                    w,
                    w,
                    out,
                );
            }
        }
        _ => {
            let (head, tail) = desc.split_first().expect("desc is non-empty");
            out.push(format!("{first_prefix}{head}"));
            out.extend(tail.iter().map(|l| indent_line(l, cont_indent)));
        }
    }
}

/// Flow standalone paragraphs (the preamble) at a fixed indent. The
/// very first line's budget is reduced by `first_line_penalty` (the
/// opening quotes it shares the line with).
fn flow_paragraphs(
    lines: &[String],
    indent: usize,
    width: Option<usize>,
    first_line_penalty: usize,
) -> Vec<String> {
    match width {
        Some(w) if is_prose(lines) => {
            let mut out = Vec::new();
            for paragraph in lines.split(|l| l.trim().is_empty()) {
                if paragraph.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    out.push(String::new());
                }
                let first_width = if out.is_empty() {
                    w.saturating_sub(first_line_penalty).max(16)
                } else {
                    w
                };
                wrap_words(
                    paragraph.iter().flat_map(|l| l.split_whitespace()),
                    " ".repeat(indent),
                    indent,
                    first_width,
                    w,
                    &mut out,
                );
            }
            out
        }
        _ => lines.to_vec(),
    }
}

/// Greedy word wrap: fill the first-line prefix (which may end with a
/// space, e.g. `":param x: "`) up to `width` columns, continuing on
/// `indent`-space lines. The first word of a line is always taken, so a
/// word longer than the width overflows rather than looping forever.
fn wrap_words<'a>(
    words: impl Iterator<Item = &'a str>,
    first_line_prefix: String,
    indent: usize,
    first_width: usize,
    width: usize,
    out: &mut Vec<String>,
) {
    let indent_str = " ".repeat(indent);
    let mut current = first_line_prefix;
    let mut current_width = first_width;
    let mut has_word = false;
    for word in words {
        if has_word && current.len() + 1 + word.len() > current_width {
            out.push(current.clone());
            current = indent_str.clone();
            current_width = width;
            has_word = false;
        }
        if has_word {
            current.push(' ');
        }
        current.push_str(word);
        has_word = true;
    }
    if has_word {
        out.push(current);
    }
}

// ---- reST ------------------------------------------------------------------

fn rest_label(keyword: &str, name: &str) -> String {
    if name.is_empty() {
        format!(":{keyword}: ")
    } else {
        format!(":{keyword} {name}: ")
    }
}

fn rest_value(
    keyword: &str,
    type_keyword: &str,
    value: &Value,
    width: Option<usize>,
    out: &mut Vec<String>,
) {
    if !value.desc.is_empty() {
        emit_desc(rest_label(keyword, ""), 4, &value.desc, width, out);
    }
    if let Some(ty) = &value.ty {
        out.push(format!(":{type_keyword}: {ty}"));
    }
}

// ---- Google ----------------------------------------------------------------

fn google_entries(header: &str, fields: &[Field], width: Option<usize>) -> Vec<String> {
    if fields.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!("{header}:")];
    for f in fields {
        let head = match &f.ty {
            Some(ty) => format!("{} ({})", f.name, ty),
            None => f.name.clone(),
        };
        emit_desc(format!("    {head}: "), 8, &f.desc, width, &mut lines);
    }
    lines
}

fn google_value(header: &str, value: &Value, width: Option<usize>) -> Vec<String> {
    let mut lines = vec![format!("{header}:")];
    match (&value.ty, value.desc.is_empty()) {
        (Some(ty), false) => emit_desc(format!("    {ty}: "), 8, &value.desc, width, &mut lines),
        (Some(ty), true) => lines.push(format!("    {ty}")),
        (None, false) => emit_desc("    ".to_string(), 4, &value.desc, width, &mut lines),
        (None, true) => return Vec::new(),
    }
    lines
}

// ---- NumPy -----------------------------------------------------------------

fn numpy_header(title: &str) -> Vec<String> {
    vec![title.to_string(), "-".repeat(title.len())]
}

fn numpy_entries(header: &str, fields: &[Field], width: Option<usize>) -> Vec<String> {
    if fields.is_empty() {
        return Vec::new();
    }
    let mut lines = numpy_header(header);
    for f in fields {
        match &f.ty {
            Some(ty) => lines.push(format!("{} : {}", f.name, ty)),
            None => lines.push(f.name.clone()),
        }
        if !f.desc.is_empty() {
            emit_desc("    ".to_string(), 4, &f.desc, width, &mut lines);
        }
    }
    lines
}

fn numpy_value(header: &str, value: &Value, width: Option<usize>) -> Vec<String> {
    let mut lines = numpy_header(header);
    // NumPy expects a type line; without one, only the description is kept.
    if let Some(ty) = &value.ty {
        lines.push(ty.clone());
    }
    if !value.desc.is_empty() {
        emit_desc("    ".to_string(), 4, &value.desc, width, &mut lines);
    }
    lines
}
