//! Renderers from the neutral [`DocIr`] into each docstring convention.
//! Output lines are dedented; the rule re-applies the docstring's base
//! indentation when splicing the content back between the quotes.

use crate::engine::config::DocStyle;

use super::{DocIr, Field, Value};

pub fn render(ir: &DocIr, style: DocStyle) -> String {
    let mut lines: Vec<String> = ir.preamble.clone();
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
                fields.extend(rest_field("param", &p.name, &p.desc));
                if let Some(ty) = &p.ty {
                    fields.push(format!(":type {}: {}", p.name, ty));
                }
            }
            if let Some(r) = &ir.returns {
                fields.extend(rest_value("returns", "rtype", r));
            }
            if let Some(y) = &ir.yields {
                fields.extend(rest_value("yields", "ytype", y));
            }
            for r in &ir.raises {
                fields.extend(rest_field(&format!("raises {}", r.name), "", &r.desc));
            }
            for a in &ir.attributes {
                fields.extend(rest_field("ivar", &a.name, &a.desc));
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
            push_section(google_entries("Args", &ir.params));
            if let Some(r) = &ir.returns {
                push_section(google_value("Returns", r));
            }
            if let Some(y) = &ir.yields {
                push_section(google_value("Yields", y));
            }
            push_section(google_entries("Raises", &ir.raises));
            push_section(google_entries("Attributes", &ir.attributes));
            for (title, body) in &ir.extras {
                let mut section = vec![format!("{title}:")];
                section.extend(body.iter().map(|l| indent_line(l, 4)));
                push_section(section);
            }
        }
        DocStyle::Numpy => {
            push_section(numpy_entries("Parameters", &ir.params));
            if let Some(r) = &ir.returns {
                push_section(numpy_value("Returns", r));
            }
            if let Some(y) = &ir.yields {
                push_section(numpy_value("Yields", y));
            }
            push_section(numpy_entries("Raises", &ir.raises));
            push_section(numpy_entries("Attributes", &ir.attributes));
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

// ---- reST ------------------------------------------------------------------

/// `:param name: first line` with continuations indented four spaces.
fn rest_field(keyword: &str, name: &str, desc: &[String]) -> Vec<String> {
    let label = if name.is_empty() {
        format!(":{keyword}:")
    } else {
        format!(":{keyword} {name}:")
    };
    let mut lines = Vec::new();
    match desc.split_first() {
        Some((first, rest)) => {
            lines.push(format!("{label} {first}"));
            lines.extend(rest.iter().map(|l| indent_line(l, 4)));
        }
        None => lines.push(label),
    }
    lines
}

fn rest_value(keyword: &str, type_keyword: &str, value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if !value.desc.is_empty() {
        lines.extend(rest_field(keyword, "", &value.desc));
    }
    if let Some(ty) = &value.ty {
        lines.push(format!(":{type_keyword}: {ty}"));
    }
    lines
}

// ---- Google ----------------------------------------------------------------

fn google_entries(header: &str, fields: &[Field]) -> Vec<String> {
    if fields.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!("{header}:")];
    for f in fields {
        let head = match &f.ty {
            Some(ty) => format!("{} ({})", f.name, ty),
            None => f.name.clone(),
        };
        match f.desc.split_first() {
            Some((first, rest)) => {
                lines.push(format!("    {head}: {first}"));
                lines.extend(rest.iter().map(|l| indent_line(l, 8)));
            }
            None => lines.push(format!("    {head}:")),
        }
    }
    lines
}

fn google_value(header: &str, value: &Value) -> Vec<String> {
    let mut lines = vec![format!("{header}:")];
    let desc = value.desc.join(" ");
    match (&value.ty, desc.is_empty()) {
        (Some(ty), false) => {
            lines.push(format!("    {ty}: {}", value.desc[0]));
            lines.extend(value.desc[1..].iter().map(|l| indent_line(l, 8)));
        }
        (Some(ty), true) => lines.push(format!("    {ty}")),
        (None, false) => {
            lines.extend(value.desc.iter().map(|l| indent_line(l, 4)));
        }
        (None, true) => return Vec::new(),
    }
    lines
}

// ---- NumPy -----------------------------------------------------------------

fn numpy_header(title: &str) -> Vec<String> {
    vec![title.to_string(), "-".repeat(title.len())]
}

fn numpy_entries(header: &str, fields: &[Field]) -> Vec<String> {
    if fields.is_empty() {
        return Vec::new();
    }
    let mut lines = numpy_header(header);
    for f in fields {
        match &f.ty {
            Some(ty) => lines.push(format!("{} : {}", f.name, ty)),
            None => lines.push(f.name.clone()),
        }
        lines.extend(f.desc.iter().map(|l| indent_line(l, 4)));
    }
    lines
}

fn numpy_value(header: &str, value: &Value) -> Vec<String> {
    let mut lines = numpy_header(header);
    // NumPy expects a type line; without one, only the description is kept.
    if let Some(ty) = &value.ty {
        lines.push(ty.clone());
    }
    lines.extend(value.desc.iter().map(|l| indent_line(l, 4)));
    lines
}
