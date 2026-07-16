use tree_sitter::Node;

use crate::engine::context::{FileContext, walk_tree};
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::{has_future_annotations, top_insertion_offset};

/// Flags string-literal ("forward reference") type annotations. With
/// `from __future__ import annotations` every annotation is lazy, so the
/// quotes are noise. --fix unquotes and inserts the future import.
pub struct StringAnnotations;

impl Rule for StringAnnotations {
    fn name(&self) -> &'static str {
        "string-annotations"
    }

    fn explain(&self) -> &'static str {
        "quoted type annotations should be unquoted under `from __future__ import annotations`"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.string_annotations_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

        let root = ctx.root();
        let needs_future_import = !has_future_annotations(root, ctx.source);
        let future_edit = Edit::insert(
            top_insertion_offset(root, ctx.source),
            "from __future__ import annotations\n".to_string(),
        );

        let mut diagnostics = Vec::new();
        walk_tree(root, &mut |annotation| {
            // Only outermost annotation nodes: `type` nests inside
            // generic parameters (list["Foo"] holds an inner `type`).
            if annotation.kind() != "type" || has_type_ancestor(annotation) {
                return;
            }
            walk_tree(annotation, &mut |node| {
                if node.kind() != "string" {
                    return;
                }
                if !eligible(node, annotation, ctx.source) {
                    return;
                }
                let Some(content) = literal_content(node, ctx.source) else {
                    return;
                };

                let mut diagnostic = Diagnostic::new(
                    self.name(),
                    format!(
                        "string annotation {}; unquote it and rely on `from __future__ import annotations`",
                        ctx.text(node),
                    ),
                    node.start_byte(),
                    node.end_byte(),
                )
                .with_severity(severity);
                if level.applies_fixes() {
                    let mut edits =
                        vec![Edit::replace(node.start_byte(), node.end_byte(), content)];
                    if needs_future_import {
                        edits.insert(0, future_edit.clone());
                    }
                    diagnostic = diagnostic.with_fix(Fix::new(edits));
                }
                diagnostics.push(diagnostic);
            });
        });
        diagnostics
    }
}

fn has_type_ancestor(node: Node) -> bool {
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if ancestor.kind() == "type" {
            return true;
        }
        current = ancestor.parent();
    }
    false
}

/// A string inside an annotation is a forward reference unless it is a
/// value position: contents of Literal[...], metadata arguments of
/// Annotated[...], or arguments of any call expression.
fn eligible(string: Node, annotation: Node, source: &str) -> bool {
    let mut current = string.parent();
    while let Some(node) = current {
        match node.kind() {
            "call" | "argument_list" => return false,
            // `Literal["a"]` in type position parses as generic_type;
            // in expression position (rare) as subscript.
            "generic_type" | "subscript" => {
                let base_node = match node.kind() {
                    "generic_type" => node.named_child(0),
                    _ => node.child_by_field_name("value"),
                };
                let Some(base_node) = base_node else {
                    return false;
                };
                match value_base_name(base_node, source) {
                    "Literal" => return false,
                    "Annotated"
                        // Only the first element is a type; the rest is
                        // metadata that may legitimately be strings.
                        if element_position(node, string) != Some(0) => {
                            return false;
                        }
                    _ => {}
                }
            }
            // Inside another string (f-string interpolation) — skip.
            "string" | "interpolation" | "concatenated_string" => return false,
            _ => {}
        }
        if node == annotation {
            break;
        }
        current = node.parent();
    }

    // Skip f-strings and strings with no simple content.
    let mut cursor = string.walk();
    !string
        .children(&mut cursor)
        .any(|c| c.kind() == "interpolation")
}

/// The last attribute component of a subscript base: `typing.Literal`
/// and `Literal` both yield "Literal".
fn value_base_name<'a>(value: Node, source: &'a str) -> &'a str {
    let text = &source[value.byte_range()];
    text.rsplit('.').next().unwrap_or(text)
}

/// Which bracket element the descendant `child` sits in: for `A[x, y]`
/// returns 0 for x, 1 for y. Handles both generic_type (elements under
/// a type_parameter node) and subscript (elements as subscript fields
/// or a tuple).
fn element_position(node: Node, child: Node) -> Option<usize> {
    let elements: Vec<Node> = if node.kind() == "generic_type" {
        let mut cursor = node.walk();
        let type_parameter = node
            .named_children(&mut cursor)
            .find(|c| c.kind() == "type_parameter")?;
        let mut cursor = type_parameter.walk();
        type_parameter.named_children(&mut cursor).collect()
    } else {
        let index = node.child_by_field_name("subscript")?;
        if index.kind() == "tuple" {
            let mut cursor = index.walk();
            index.named_children(&mut cursor).collect()
        } else {
            vec![index]
        }
    };
    elements.iter().position(|e| contains(*e, child))
}

fn contains(haystack: Node, needle: Node) -> bool {
    haystack.start_byte() <= needle.start_byte() && needle.end_byte() <= haystack.end_byte()
}

/// The raw content between the quotes of a plain string literal.
fn literal_content(string: Node, source: &str) -> Option<String> {
    let mut content = String::new();
    let mut cursor = string.walk();
    for child in string.children(&mut cursor) {
        match child.kind() {
            "string_start" | "string_end" => {}
            "string_content" => content.push_str(&source[child.byte_range()]),
            "escape_sequence" => content.push_str(&source[child.byte_range()]),
            _ => return None,
        }
    }
    (!content.contains('\n')).then_some(content)
}
