use tree_sitter::Node;

use crate::engine::context::{FileContext, walk_tree};
use crate::engine::diagnostic::Diagnostic;
use crate::engine::rule::Rule;
use crate::langs::python::imports;

/// Flags imports written inside function bodies. Deliberate local imports
/// (cycle avoidance, lazy heavy deps) should carry `# sweep: avoid-cycle`
/// or `# sweep: ignore[imports-ban-local] <reason>`; everything else gets
/// hoisted to the module import block under --fix.
pub struct ImportsBanLocal;

impl Rule for ImportsBanLocal {
    fn name(&self) -> &'static str {
        "imports-ban-local"
    }

    fn explain(&self) -> &'static str {
        "imports inside functions must be justified (# sweep: avoid-cycle) or hoisted to module level"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.imports_ban_local_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        walk_tree(ctx.root(), &mut |node| {
            if !matches!(node.kind(), "import_statement" | "import_from_statement") {
                return;
            }
            let Some(placement) = placement_of(node) else {
                return;
            };
            if placement == Placement::TopLevel {
                return;
            }

            let mut diagnostic = Diagnostic::new(
                self.name(),
                format!(
                    "`{}` inside a function; hoist to module level or mark it `# sweep: avoid-cycle`",
                    first_line(ctx.text(node)),
                ),
                node.start_byte(),
                node.end_byte(),
            )
            .with_severity(severity);

            if level.applies_fixes()
                && placement == Placement::Hoistable
                && let Some(fix) = imports::hoist_fix(node, ctx.source, ctx.config)
            {
                diagnostic = diagnostic.with_fix(fix);
            }
            diagnostics.push(diagnostic);
        });
        diagnostics
    }
}

#[derive(PartialEq, Eq)]
enum Placement {
    TopLevel,
    /// Inside a function, nested only in plain function/class blocks —
    /// safe to hoist mechanically.
    Hoistable,
    /// Inside a function but under try/if/with/loop — conditional imports
    /// (e.g. try/except ImportError) are warned about, never auto-hoisted.
    Conditional,
}

fn placement_of(node: Node) -> Option<Placement> {
    let mut inside_function = false;
    let mut conditional = false;
    let mut current = node.parent();
    while let Some(ancestor) = current {
        match ancestor.kind() {
            "function_definition" => inside_function = true,
            "module" | "block" | "class_definition" | "decorated_definition" => {}
            _ => conditional = true,
        }
        current = ancestor.parent();
    }
    if !inside_function {
        return Some(Placement::TopLevel);
    }
    Some(if conditional {
        Placement::Conditional
    } else {
        Placement::Hoistable
    })
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}
