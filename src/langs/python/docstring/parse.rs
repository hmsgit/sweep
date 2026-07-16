//! Parsers from each docstring convention into the neutral [`DocIr`].
//! Every parser is conservative: anything it can't represent faithfully
//! makes it return `None`, and the rule falls back to warn-only.

use crate::engine::config::DocStyle;

use super::{DocIr, Field, Value, dedent, indent_of, split_at_top_level_colon};

pub fn parse(content: &str, style: DocStyle) -> Option<DocIr> {
    let raw: Vec<&str> = content.lines().collect();
    let lines = dedent(&raw);
    match style {
        DocStyle::Google => parse_google(&lines),
        DocStyle::Numpy => parse_numpy(&lines),
        DocStyle::Rest => parse_rest(&lines),
    }
}

// ---- Google ----------------------------------------------------------------

const GOOGLE_EXTRA_HEADERS: &[&str] = &[
    "Example",
    "Examples",
    "Note",
    "Notes",
    "Todo",
    "Warning",
    "Warnings",
    "See Also",
    "References",
];

fn google_header(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if indent_of(line) != 0 {
        return None;
    }
    let header = trimmed.strip_suffix(':')?;
    (super::GOOGLE_SIGNATURE_HEADERS.contains(&header) || GOOGLE_EXTRA_HEADERS.contains(&header))
        .then_some(header)
}

fn parse_google(lines: &[String]) -> Option<DocIr> {
    let mut ir = DocIr::default();
    let mut i = 0;

    while i < lines.len() && google_header(&lines[i]).is_none() {
        ir.preamble.push(lines[i].clone());
        i += 1;
    }
    trim_blank_edges(&mut ir.preamble);

    while i < lines.len() {
        let header = google_header(&lines[i])?.to_string();
        i += 1;
        let mut section: Vec<&str> = Vec::new();
        while i < lines.len() && google_header(&lines[i]).is_none() {
            // A flush-left non-blank line outside any section is ambiguous.
            if !lines[i].is_empty() && indent_of(&lines[i]) == 0 {
                return None;
            }
            section.push(&lines[i]);
            i += 1;
        }
        while section.last().is_some_and(|l| l.trim().is_empty()) {
            section.pop();
        }
        let section: Vec<String> = dedent_section(&section);

        match header.as_str() {
            "Args" | "Arguments" | "Keyword Args" | "Keyword Arguments" => {
                ir.params.extend(parse_google_entries(&section)?);
            }
            "Attributes" => ir.attributes.extend(parse_google_entries(&section)?),
            "Raises" | "Warns" => ir.raises.extend(parse_google_entries(&section)?),
            "Returns" => ir.returns = Some(parse_google_value(&section)?),
            "Yields" => ir.yields = Some(parse_google_value(&section)?),
            _ => ir.extras.push((header, section)),
        }
    }
    Some(ir)
}

/// `name (ty): desc` / `name: desc` entries with indented continuations.
fn parse_google_entries(lines: &[String]) -> Option<Vec<Field>> {
    let mut fields: Vec<Field> = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        if indent_of(line) > 0 {
            fields.last_mut()?.desc.push(line.trim().to_string());
            continue;
        }
        let (head, desc) = split_at_top_level_colon(line)?;
        let head = head.trim();
        let (name, ty) = match head.split_once('(') {
            Some((name, ty)) => (
                name.trim().to_string(),
                Some(ty.trim_end_matches(')').trim().to_string()),
            ),
            None => (head.to_string(), None),
        };
        if name.is_empty() || name.contains(' ') {
            return None;
        }
        let desc = desc.trim();
        fields.push(Field {
            name,
            ty,
            desc: if desc.is_empty() {
                Vec::new()
            } else {
                vec![desc.to_string()]
            },
        });
    }
    Some(fields)
}

/// Returns/Yields body: optionally `ty: desc` on the first line.
fn parse_google_value(lines: &[String]) -> Option<Value> {
    let mut value = Value::default();
    let mut rest_start = 0;
    if let Some(first) = lines.first()
        && let Some((head, desc)) = split_at_top_level_colon(first)
        && looks_like_type(head.trim())
    {
        value.ty = Some(head.trim().to_string());
        let desc = desc.trim();
        if !desc.is_empty() {
            value.desc.push(desc.to_string());
        }
        rest_start = 1;
    }
    for line in &lines[rest_start..] {
        value.desc.push(line.trim().to_string());
    }
    trim_blank_edges(&mut value.desc);
    Some(value)
}

// ---- NumPy -----------------------------------------------------------------

fn numpy_header_at(lines: &[String], i: usize) -> Option<&str> {
    let header = lines[i].trim();
    if header.is_empty() || indent_of(&lines[i]) != 0 {
        return None;
    }
    let underline = lines.get(i + 1)?.trim();
    (underline.len() >= 3 && underline.chars().all(|c| c == '-')).then_some(header)
}

fn parse_numpy(lines: &[String]) -> Option<DocIr> {
    let mut ir = DocIr::default();
    let mut i = 0;

    while i < lines.len() && numpy_header_at(lines, i).is_none() {
        ir.preamble.push(lines[i].clone());
        i += 1;
    }
    trim_blank_edges(&mut ir.preamble);

    while i < lines.len() {
        let header = numpy_header_at(lines, i)?.to_string();
        i += 2;
        let mut section: Vec<&str> = Vec::new();
        while i < lines.len() && numpy_header_at(lines, i).is_none() {
            section.push(&lines[i]);
            i += 1;
        }
        while section.last().is_some_and(|l| l.trim().is_empty()) {
            section.pop();
        }
        let section: Vec<String> = section.iter().map(|l| l.trim_end().to_string()).collect();

        match header.as_str() {
            "Parameters" | "Other Parameters" => ir.params.extend(parse_numpy_entries(&section)?),
            "Attributes" => ir.attributes.extend(parse_numpy_entries(&section)?),
            "Raises" | "Warns" => ir.raises.extend(parse_numpy_entries(&section)?),
            "Returns" => ir.returns = Some(parse_numpy_value(&section)?),
            "Yields" => ir.yields = Some(parse_numpy_value(&section)?),
            _ => ir.extras.push((header, section)),
        }
    }
    Some(ir)
}

/// `name : ty` / `name` entries, description on indented lines below.
fn parse_numpy_entries(lines: &[String]) -> Option<Vec<Field>> {
    let mut fields: Vec<Field> = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        if indent_of(line) > 0 {
            fields.last_mut()?.desc.push(line.trim().to_string());
            continue;
        }
        let (name, ty) = match line.split_once(':') {
            Some((name, ty)) => (name.trim().to_string(), Some(ty.trim().to_string())),
            None => (line.trim().to_string(), None),
        };
        if name.is_empty() || name.contains(' ') {
            return None;
        }
        fields.push(Field {
            name,
            ty,
            desc: Vec::new(),
        });
    }
    Some(fields)
}

/// NumPy Returns/Yields: a type on a flush line, description indented.
/// Multiple return entries can't be represented — bail.
fn parse_numpy_value(lines: &[String]) -> Option<Value> {
    let mut value = Value::default();
    let mut seen_entry = false;
    for line in lines {
        if line.trim().is_empty() {
            if !value.desc.is_empty() {
                value.desc.push(String::new());
            }
            continue;
        }
        if indent_of(line) == 0 {
            if seen_entry {
                return None;
            }
            seen_entry = true;
            // `name : ty` or bare `ty`.
            value.ty = Some(match line.split_once(':') {
                Some((_, ty)) => ty.trim().to_string(),
                None => line.trim().to_string(),
            });
        } else {
            value.desc.push(line.trim().to_string());
        }
    }
    trim_blank_edges(&mut value.desc);
    Some(value)
}

// ---- reST ------------------------------------------------------------------

fn rest_field(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    if indent_of(line) != 0 {
        return None;
    }
    let rest = trimmed.strip_prefix(':')?;
    let (spec, value) = rest.split_once(':')?;
    let keyword = spec.split_whitespace().next()?;
    super::REST_FIELD_KEYWORDS
        .contains(&keyword)
        .then(|| (spec.trim().to_string(), value.trim().to_string()))
}

fn parse_rest(lines: &[String]) -> Option<DocIr> {
    let mut ir = DocIr::default();
    let mut i = 0;

    while i < lines.len() && rest_field(&lines[i]).is_none() {
        // Any reST directive or unknown field means we can't round-trip.
        if lines[i].trim_start().starts_with(":") || lines[i].trim_start().starts_with(".. ") {
            return None;
        }
        ir.preamble.push(lines[i].clone());
        i += 1;
    }
    trim_blank_edges(&mut ir.preamble);

    // Where continuation text of the most recent field should land.
    enum Slot {
        Param(usize),
        Attr(usize),
        Raise(usize),
        Return,
        Yield,
        Type,
    }
    let mut last: Option<Slot> = None;

    while i < lines.len() {
        let line = &lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        if let Some((spec, value)) = rest_field(line) {
            let parts: Vec<&str> = spec.split_whitespace().collect();
            let keyword = parts[0];
            match keyword {
                "param" | "parameter" | "arg" | "argument" | "key" | "keyword" => {
                    if parts.len() < 2 || parts.len() > 3 {
                        return None;
                    }
                    let name = parts.last().unwrap().to_string();
                    let ty = (parts.len() == 3).then(|| parts[1].to_string());
                    ir.params.push(Field {
                        name,
                        ty,
                        desc: if value.is_empty() {
                            Vec::new()
                        } else {
                            vec![value]
                        },
                    });
                    last = Some(Slot::Param(ir.params.len() - 1));
                }
                "type" => {
                    if parts.len() != 2 {
                        return None;
                    }
                    let param = ir.params.iter_mut().find(|p| p.name == parts[1])?;
                    param.ty = Some(value);
                    last = Some(Slot::Type);
                }
                "return" | "returns" => {
                    let entry = ir.returns.get_or_insert_with(Value::default);
                    if !value.is_empty() {
                        entry.desc.push(value);
                    }
                    last = Some(Slot::Return);
                }
                "rtype" => {
                    ir.returns.get_or_insert_with(Value::default).ty = Some(value);
                    last = Some(Slot::Type);
                }
                "yield" | "yields" => {
                    let entry = ir.yields.get_or_insert_with(Value::default);
                    if !value.is_empty() {
                        entry.desc.push(value);
                    }
                    last = Some(Slot::Yield);
                }
                "ytype" => {
                    ir.yields.get_or_insert_with(Value::default).ty = Some(value);
                    last = Some(Slot::Type);
                }
                "raise" | "raises" | "except" | "exception" => {
                    if parts.len() < 2 {
                        return None;
                    }
                    ir.raises.push(Field {
                        name: parts[1..].join(" "),
                        ty: None,
                        desc: if value.is_empty() {
                            Vec::new()
                        } else {
                            vec![value]
                        },
                    });
                    last = Some(Slot::Raise(ir.raises.len() - 1));
                }
                "var" | "ivar" | "cvar" => {
                    if parts.len() < 2 || parts.len() > 3 {
                        return None;
                    }
                    let name = parts.last().unwrap().to_string();
                    let ty = (parts.len() == 3).then(|| parts[1].to_string());
                    ir.attributes.push(Field {
                        name,
                        ty,
                        desc: if value.is_empty() {
                            Vec::new()
                        } else {
                            vec![value]
                        },
                    });
                    last = Some(Slot::Attr(ir.attributes.len() - 1));
                }
                "vartype" => {
                    if parts.len() != 2 {
                        return None;
                    }
                    let attr = ir.attributes.iter_mut().find(|a| a.name == parts[1])?;
                    attr.ty = Some(value);
                    last = Some(Slot::Type);
                }
                _ => return None,
            }
            i += 1;
            continue;
        }

        // Indented continuation of the previous field.
        if indent_of(line) > 0 {
            let text = line.trim().to_string();
            match &last {
                Some(Slot::Param(idx)) => ir.params[*idx].desc.push(text),
                Some(Slot::Attr(idx)) => ir.attributes[*idx].desc.push(text),
                Some(Slot::Raise(idx)) => ir.raises[*idx].desc.push(text),
                Some(Slot::Return) => ir.returns.as_mut()?.desc.push(text),
                Some(Slot::Yield) => ir.yields.as_mut()?.desc.push(text),
                Some(Slot::Type) | None => return None,
            }
            i += 1;
            continue;
        }

        // Flush-left prose after fields started — can't round-trip.
        return None;
    }
    Some(ir)
}

// ---- shared ----------------------------------------------------------------

fn dedent_section(lines: &[&str]) -> Vec<String> {
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent_of(l))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                l[indent..].trim_end().to_string()
            }
        })
        .collect()
}

fn trim_blank_edges(lines: &mut Vec<String>) {
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
}

/// Heuristic for `ty: desc` first lines in Google Returns sections: a
/// type expression has no top-level spaces (spaces inside brackets are
/// fine, `dict[str, int]`).
fn looks_like_type(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => return false,
            _ => {}
        }
    }
    true
}
