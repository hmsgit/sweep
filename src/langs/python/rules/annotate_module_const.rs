use tree_sitter::Node;

use crate::engine::context::FileContext;
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::top_insertion_offset;

/// Module-level constants (UPPER_CASE names) should carry a `Final`
/// annotation. The fix adds `: Final` (or wraps an existing annotation
/// as `Final[T]`) and inserts `from typing import Final` if missing.
/// Naming is casing-module-const's business; this pass only annotates.
///
/// A name only counts as a constant when nothing contradicts it:
/// assigned exactly once at module level and never declared `global`
/// anywhere in the file — otherwise Final would be a lie.
pub struct AnnotateModuleConst;

impl Rule for AnnotateModuleConst {
    fn name(&self) -> &'static str {
        "annotate-module-const"
    }

    fn explain(&self) -> &'static str {
        "module constants (UPPER_CASE) should be annotated with typing.Final"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.annotate_module_const_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

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
            if !is_constant_name(name) || rebound.contains(&name.to_string()) {
                continue;
            }
            let annotation = assignment.child_by_field_name("type");
            if annotation.is_some_and(|t| ctx.source[t.byte_range()].contains("Final")) {
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
                match annotation {
                    Some(t) => edits.push(Edit::replace(
                        t.start_byte(),
                        t.end_byte(),
                        format!("Final[{}]", &ctx.source[t.byte_range()]),
                    )),
                    None => edits.push(Edit::insert(left.end_byte(), ": Final".to_string())),
                }
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
