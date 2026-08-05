use tree_sitter::Node;

use crate::engine::config::Case;
use crate::engine::context::FileContext;
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::{is_typing_special_assignment, top_insertion_offset};

/// Module-level constants should carry a `Final` annotation. The fix
/// adds `: Final` (or wraps an existing annotation as `Final[T]`) and
/// inserts `from typing import Final` if missing.
/// Naming is casing-module-const's business; this pass only annotates.
///
/// A constant is any UPPER_CASE name; when casing-module-const is
/// configured `lower` (so caps no longer mark constant-ness), a plain
/// lowercase name also counts if its value is a simple literal —
/// string without interpolation, number, bool, None, or a tuple of
/// those. List/dict/set literals and computed values stay exempt:
/// they are indistinguishable from deliberately mutable module state.
///
/// Either way a name only counts when nothing contradicts it:
/// assigned exactly once at module level and never declared `global`
/// anywhere in the file — otherwise Final would be a lie.
pub struct AnnotateModuleConst;

impl Rule for AnnotateModuleConst {
    fn name(&self) -> &'static str {
        "annotate-module-const"
    }

    fn explain(&self) -> &'static str {
        "module constants should be annotated with typing.Final"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.annotate_module_const_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

        let casing = ctx.config.casing_module_const;
        let lower_constants = casing.level.severity().is_some() && casing.case == Case::Lower;

        let root = ctx.root();
        let needs_import = !has_final_import(root, ctx.source);
        let import_edit = Edit::insert(
            top_insertion_offset(root, ctx.source),
            "from typing import Final\n".to_string(),
        );
        let rebound = rebound_names(root, ctx.source);

        let mut diagnostics = Vec::new();
        let mut cursor = root.walk();
        for stmt in root.children(&mut cursor) {
            if stmt.kind() != "expression_statement" {
                continue;
            }
            let Some(assignment) = stmt.named_child(0) else {
                continue;
            };
            if assignment.kind() != "assignment" {
                continue;
            }
            let Some(left) = assignment.child_by_field_name("left") else {
                continue;
            };
            if left.kind() != "identifier" {
                continue;
            }
            let name = &ctx.source[left.byte_range()];
            let named_constant = is_constant_name(name);
            let literal_constant = lower_constants
                && is_lower_constant_name(name)
                && assignment
                    .child_by_field_name("right")
                    .is_some_and(is_simple_literal);
            if !(named_constant || literal_constant)
                || rebound.contains(&name.to_string())
                || is_typing_special_assignment(assignment, ctx.source)
            {
                continue;
            }
            let annotation = assignment.child_by_field_name("type");
            if annotation.is_some_and(|t| ctx.source[t.byte_range()].contains("Final")) {
                continue;
            }

            // An existing annotation is the author's choice — hint that
            // Final[T] exists, but never rewrite it.
            if let Some(t) = annotation {
                let ty = &ctx.source[t.byte_range()];
                diagnostics.push(
                    Diagnostic::new(
                        self.name(),
                        format!(
                            "module constant `{name}` is not Final; consider `{name}: Final[{ty}] = …`"
                        ),
                        left.start_byte(),
                        left.end_byte(),
                    )
                    .with_severity(severity),
                );
                continue;
            }

            let mut diagnostic = Diagnostic::new(
                self.name(),
                format!("module constant `{name}` lacks a Final annotation"),
                left.start_byte(),
                left.end_byte(),
            )
            .with_severity(severity);

            if level.applies_fixes() {
                let mut edits = Vec::new();
                if needs_import {
                    edits.push(import_edit.clone());
                }
                edits.push(Edit::insert(left.end_byte(), ": Final".to_string()));
                diagnostic = diagnostic.with_fix(Fix::new(edits));
            }
            diagnostics.push(diagnostic);
        }
        diagnostics
    }
}

/// UPPER_CASE with at least one letter, not a dunder like `__all__`.
fn is_constant_name(name: &str) -> bool {
    !name.starts_with('_')
        && name.chars().any(|c| c.is_alphabetic())
        && !name.chars().any(|c| c.is_lowercase())
}

/// lower_case with at least one letter, not underscore-prefixed —
/// mirrors casing-module-const's notion of a public module name.
fn is_lower_constant_name(name: &str) -> bool {
    !name.starts_with('_')
        && name.chars().any(|c| c.is_alphabetic())
        && !name.chars().any(|c| c.is_uppercase())
}

/// Immutable literal value: string (no f-string interpolation),
/// number, bool, None, or a tuple of simple literals. Lists, dicts,
/// sets, and computed values don't qualify — those shapes routinely
/// back deliberately mutable module state.
fn is_simple_literal(node: Node) -> bool {
    match node.kind() {
        "integer" | "float" | "true" | "false" | "none" => true,
        "string" | "concatenated_string" => !has_interpolation(node),
        "unary_operator" => node
            .child_by_field_name("argument")
            .is_some_and(|arg| matches!(arg.kind(), "integer" | "float")),
        "tuple" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor).all(is_simple_literal)
        }
        _ => false,
    }
}

fn has_interpolation(node: Node) -> bool {
    let mut found = false;
    crate::engine::context::walk_tree(node, &mut |n| {
        if n.kind() == "interpolation" {
            found = true;
        }
    });
    found
}

/// Names that are provably not constants: assigned more than once at
/// module level (including augmented assignment) or declared `global`
/// anywhere in the file.
fn rebound_names(root: Node, source: &str) -> Vec<String> {
    use std::collections::HashMap;

    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut cursor = root.walk();
    for stmt in root.children(&mut cursor) {
        if stmt.kind() != "expression_statement" {
            continue;
        }
        let Some(expr) = stmt.named_child(0) else {
            continue;
        };
        if !matches!(expr.kind(), "assignment" | "augmented_assignment") {
            continue;
        }
        let Some(left) = expr.child_by_field_name("left") else {
            continue;
        };
        if left.kind() == "identifier" {
            let weight = if expr.kind() == "augmented_assignment" {
                2
            } else {
                1
            };
            *counts.entry(&source[left.byte_range()]).or_default() += weight;
        }
    }

    let mut rebound: Vec<String> = counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(name, _)| name.to_string())
        .collect();

    crate::engine::context::walk_tree(root, &mut |node| {
        if node.kind() == "global_statement" {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "identifier" {
                    rebound.push(source[child.byte_range()].to_string());
                }
            }
        }
    });
    rebound
}

fn has_final_import(root: Node, source: &str) -> bool {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "import_from_statement" {
            let text = &source[child.byte_range()];
            if text.contains("typing")
                && text
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|w| w == "Final")
            {
                return true;
            }
        }
    }
    false
}
