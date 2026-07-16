//! Naming-convention passes: enum member names, enum string values,
//! and module constant names. All warn-only — renaming identifiers
//! safely needs cross-file refactoring, and changing enum *values*
//! changes serialized data; both are human decisions.

use tree_sitter::Node;

use crate::engine::context::{FileContext, walk_tree};
use crate::engine::diagnostic::Diagnostic;
use crate::engine::rule::Rule;

const ENUM_BASES: &[&str] = &["Enum", "IntEnum", "StrEnum", "Flag", "IntFlag"];

pub struct CasingEnumKey;

impl Rule for CasingEnumKey {
    fn name(&self) -> &'static str {
        "casing-enum-key"
    }

    fn explain(&self) -> &'static str {
        "enum member names must follow the configured case (warn-only)"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let config = ctx.config.casing_enum_key;
        let Some(severity) = config.level.severity() else {
            return Vec::new();
        };
        let mut diagnostics = Vec::new();
        for_each_enum_member(ctx, &mut |name_node, name, _value| {
            if !config.case.matches(name) {
                diagnostics.push(
                    Diagnostic::new(
                        self.name(),
                        format!("enum member `{name}` should be {}", config.case.describe()),
                        name_node.start_byte(),
                        name_node.end_byte(),
                    )
                    .with_severity(severity),
                );
            }
        });
        diagnostics
    }
}

pub struct CasingEnumVal;

impl Rule for CasingEnumVal {
    fn name(&self) -> &'static str {
        "casing-enum-val"
    }

    fn explain(&self) -> &'static str {
        "enum string values must follow the configured case (warn-only)"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let config = ctx.config.casing_enum_val;
        let Some(severity) = config.level.severity() else {
            return Vec::new();
        };
        let mut diagnostics = Vec::new();
        for_each_enum_member(ctx, &mut |_name_node, name, value| {
            let Some(value) = value else { return };
            let Some(content) = plain_string_content(value, ctx.source) else {
                return;
            };
            if content.chars().any(|c| c.is_alphabetic()) && !config.case.matches(content) {
                diagnostics.push(
                    Diagnostic::new(
                        self.name(),
                        format!(
                            "value of enum member `{name}` should be {}",
                            config.case.describe()
                        ),
                        value.start_byte(),
                        value.end_byte(),
                    )
                    .with_severity(severity),
                );
            }
        });
        diagnostics
    }
}

pub struct CasingModuleConst;

impl Rule for CasingModuleConst {
    fn name(&self) -> &'static str {
        "casing-module-const"
    }

    fn explain(&self) -> &'static str {
        "module constant names must follow the configured case (warn-only)"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let config = ctx.config.casing_module_const;
        let Some(severity) = config.level.severity() else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        let root = ctx.root();
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
            if name.starts_with('_') || !name.chars().any(|c| c.is_alphabetic()) {
                continue;
            }
            // Typing special forms (T = TypeVar("T")) follow their own
            // naming convention.
            if crate::langs::python::is_typing_special_assignment(assignment, ctx.source) {
                continue;
            }
            // A "constant" is recognizable by SCREAMING_CASE or an
            // existing Final annotation; plain lowercase assignments
            // are indistinguishable from module state and are skipped.
            let screaming = !name.chars().any(|c| c.is_lowercase());
            let has_final = assignment
                .child_by_field_name("type")
                .is_some_and(|t| ctx.source[t.byte_range()].contains("Final"));
            if !(screaming || has_final) {
                continue;
            }
            if !config.case.matches(name) {
                diagnostics.push(
                    Diagnostic::new(
                        self.name(),
                        format!(
                            "module constant `{name}` should be {}",
                            config.case.describe()
                        ),
                        left.start_byte(),
                        left.end_byte(),
                    )
                    .with_severity(severity),
                );
            }
        }
        diagnostics
    }
}

/// Visit every enum member assignment: (name node, name, value node).
fn for_each_enum_member<'t>(
    ctx: &FileContext<'t>,
    f: &mut dyn FnMut(Node<'t>, &str, Option<Node<'t>>),
) {
    walk_tree(ctx.root(), &mut |node| {
        if node.kind() != "class_definition" || !is_enum_class(node, ctx.source) {
            return;
        }
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        let mut cursor = body.walk();
        for stmt in body.named_children(&mut cursor) {
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
            // _sunder_/dunder names are enum machinery, not members.
            if name.starts_with('_') {
                continue;
            }
            f(left, name, assignment.child_by_field_name("right"));
        }
    });
}

fn is_enum_class(class: Node, source: &str) -> bool {
    let Some(superclasses) = class.child_by_field_name("superclasses") else {
        return false;
    };
    let mut cursor = superclasses.walk();
    superclasses.named_children(&mut cursor).any(|base| {
        let text = &source[base.byte_range()];
        let last = text.rsplit('.').next().unwrap_or(text);
        ENUM_BASES.contains(&last)
    })
}

/// Content of a plain (non-f, non-concatenated) string literal.
fn plain_string_content<'a>(node: Node, source: &'a str) -> Option<&'a str> {
    if node.kind() != "string" {
        return None;
    }
    let mut content = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "string_start" | "string_end" => {}
            "string_content" if content.is_none() => {
                content = Some(&source[child.byte_range()]);
            }
            _ => return None,
        }
    }
    content
}
