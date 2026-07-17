pub mod docstring;
pub mod imports;
pub mod rules;
pub mod stdlib;

use tree_sitter::{Node, Parser};

pub fn parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .expect("tree-sitter-python language incompatible with tree-sitter runtime");
    parser
}

/// The module docstring node, if the first statement is a plain string.
pub fn module_docstring(root: Node) -> Option<Node> {
    let first = root.named_child(0)?;
    docstring_of_statement(first)
}

/// If `stmt` is an expression statement wrapping a single string, return
/// the string node (used for module/class/function docstrings).
pub fn docstring_of_statement(stmt: Node) -> Option<Node> {
    if stmt.kind() != "expression_statement" || stmt.named_child_count() != 1 {
        return None;
    }
    let child = stmt.named_child(0)?;
    (child.kind() == "string").then_some(child)
}

/// Byte offset where new top-of-module code (future imports, hoisted
/// imports when no import block exists) should be inserted: after the
/// module docstring and any comments preceding the first statement.
pub fn top_insertion_offset(root: Node, source: &str) -> usize {
    let docstring_stmt = module_docstring(root).and_then(|d| d.parent());
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "comment" || Some(child) == docstring_stmt {
            continue;
        }
        return line_start(source, child.start_byte());
    }
    source.len()
}

/// Offset of the first byte of the line containing `offset`.
pub fn line_start(source: &str, offset: usize) -> usize {
    source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

/// Offset just past the newline ending the line that contains `offset`
/// (or end of file).
pub fn line_end_inclusive(source: &str, offset: usize) -> usize {
    source[offset..]
        .find('\n')
        .map(|i| offset + i + 1)
        .unwrap_or(source.len())
}

/// True when the assignment's right side is a typing special-form
/// constructor — `T = TypeVar("T")`, `P = ParamSpec("P")`, … Their
/// conventional UPPER names are neither constants to annotate nor
/// casing subjects.
pub fn is_typing_special_assignment(assignment: Node, source: &str) -> bool {
    let Some(right) = assignment.child_by_field_name("right") else {
        return false;
    };
    if right.kind() != "call" {
        return false;
    }
    let Some(function) = right.child_by_field_name("function") else {
        return false;
    };
    let text = &source[function.byte_range()];
    let base = text.rsplit('.').next().unwrap_or(text);
    matches!(
        base,
        "TypeVar"
            | "ParamSpec"
            | "TypeVarTuple"
            | "NewType"
            | "TypeAliasType"
            | "NamedTuple"
            | "namedtuple"
    )
}

/// Parameter names and annotation texts of a function definition, in
/// signature order. Splat parameters (`*args`, `**kwargs`) appear by
/// their bare name; bare `*` and `/` separators are skipped.
pub fn function_params(func: Node, source: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let Some(params) = func.child_by_field_name("parameters") else {
        return out;
    };
    let mut cursor = params.walk();
    for child in params.named_children(&mut cursor) {
        match child.kind() {
            "identifier" => out.push((source[child.byte_range()].to_string(), None)),
            "typed_parameter" | "typed_default_parameter" | "default_parameter" => {
                let name = child
                    .child_by_field_name("name")
                    .or_else(|| child.named_child(0));
                let Some(name) = name.filter(|n| n.kind() == "identifier") else {
                    continue;
                };
                let ty = child
                    .child_by_field_name("type")
                    .map(|t| source[t.byte_range()].to_string());
                out.push((source[name.byte_range()].to_string(), ty));
            }
            "list_splat_pattern" | "dictionary_splat_pattern" => {
                if let Some(name) = child.named_child(0) {
                    out.push((source[name.byte_range()].to_string(), None));
                }
            }
            _ => {}
        }
    }
    out
}

/// True when the module already has `from __future__ import annotations`.
pub fn has_future_annotations(root: Node, source: &str) -> bool {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "future_import_statement" {
            let text = &source[child.byte_range()];
            if text
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|w| w == "annotations")
            {
                return true;
            }
        }
    }
    false
}
