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
pub struct ConstFinal;

impl Rule for ConstFinal {
    fn name(&self) -> &'static str {
        "const-final"
    }

    fn explain(&self) -> &'static str {
        "module constants (UPPER_CASE) should be annotated with typing.Final"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.const_final_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

        let root = ctx.root();
        let needs_import = !has_final_import(root, ctx.source);
        let import_edit = Edit::insert(
            top_insertion_offset(root, ctx.source),
            "from typing import Final\n".to_string(),
        );

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
            if !is_constant_name(name) {
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
