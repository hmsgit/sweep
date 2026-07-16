//! Import classification and the hoist fix used by the imports-ban-local rule.

use tree_sitter::Node;

use crate::engine::config::Config;
use crate::engine::fix::{Edit, Fix};
use crate::langs::python::stdlib::is_stdlib;
use crate::langs::python::{line_end_inclusive, line_start, top_insertion_offset};

/// Import sections in sort order, following the common isort layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Future,
    Stdlib,
    ThirdParty,
    FirstParty,
    Relative,
}

pub fn classify(node_kind: &str, module: &str, config: &Config) -> Section {
    if node_kind == "future_import_statement" {
        return Section::Future;
    }
    if module.starts_with('.') {
        return Section::Relative;
    }
    let root = module.split('.').next().unwrap_or(module);
    if root == "__future__" {
        return Section::Future;
    }
    if config.known_first_party.iter().any(|p| p == root) {
        return Section::FirstParty;
    }
    if is_stdlib(root) {
        return Section::Stdlib;
    }
    Section::ThirdParty
}

/// The dotted module path an import statement sorts by:
/// `import a.b, c` → "a.b"; `from a.b import c` → "a.b"; `from . import x` → ".".
pub fn sort_module(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "import_statement" => {
            let first = node.named_child(0)?;
            let name = match first.kind() {
                "aliased_import" => first.child_by_field_name("name")?,
                _ => first,
            };
            Some(source[name.byte_range()].to_string())
        }
        "import_from_statement" | "future_import_statement" => {
            let module = node.child_by_field_name("module_name")?;
            Some(source[module.byte_range()].to_string())
        }
        _ => None,
    }
}

pub fn is_import_kind(kind: &str) -> bool {
    matches!(
        kind,
        "import_statement" | "import_from_statement" | "future_import_statement"
    )
}

fn sort_key(section: Section, module: &str) -> (Section, String) {
    (section, module.trim_start_matches('.').to_lowercase())
}

/// Build the fix that removes a function-local import and re-inserts it
/// in the module's top import region, in the right section, alphabetically.
/// Returns None when the surrounding code makes a safe hoist impossible.
pub fn hoist_fix(node: Node, source: &str, config: &Config) -> Option<Fix> {
    let import_text = &source[node.byte_range()];
    let module = sort_module(node, source)?;
    let section = classify(node.kind(), &module, config);
    if section == Section::Relative {
        // A relative import inside a function almost always dodges a cycle;
        // hoisting it is exactly what the author avoided.
        return None;
    }

    // Refuse to touch lines that hold anything besides the import itself
    // (e.g. `x = 1; import os` or a trailing comment).
    let start_of_line = line_start(source, node.start_byte());
    if source[start_of_line..node.start_byte()]
        .chars()
        .any(|c| !c.is_whitespace())
    {
        return None;
    }
    let end_of_line = line_end_inclusive(source, node.end_byte());
    if source[node.end_byte()..end_of_line]
        .chars()
        .any(|c| !c.is_whitespace())
    {
        return None;
    }

    // If removing the import would leave its block empty, substitute `pass`.
    let parent_block = node.parent()?;
    let removal = if parent_block.kind() == "block" && parent_block.named_child_count() == 1 {
        Edit::replace(node.start_byte(), node.end_byte(), "pass")
    } else {
        Edit::delete(start_of_line, end_of_line)
    };

    let root = root_of(node);
    let top_imports = collect_top_imports(root, source, config);

    // An identical import already at top level: just remove the local one.
    if top_imports
        .iter()
        .any(|entry| source[entry.node.byte_range()].trim() == import_text.trim())
    {
        return Some(Fix::new(vec![removal]));
    }

    let insert = insertion_edit(root, source, &top_imports, section, &module, import_text);
    Some(Fix::new(vec![removal, insert]))
}

fn root_of(node: Node) -> Node {
    let mut current = node;
    while let Some(parent) = current.parent() {
        current = parent;
    }
    current
}

struct TopImport<'t> {
    node: Node<'t>,
    section: Section,
    module: String,
}

/// Top-level import statements in the module's leading region: docstring
/// and comments are skipped, and the region ends at the first other
/// statement.
fn collect_top_imports<'t>(root: Node<'t>, source: &str, config: &Config) -> Vec<TopImport<'t>> {
    let docstring_stmt = super::module_docstring(root).and_then(|d| d.parent());
    let mut imports = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "comment" || Some(child) == docstring_stmt {
            continue;
        }
        if !is_import_kind(child.kind()) {
            break;
        }
        let module = sort_module(child, source).unwrap_or_default();
        let section = classify(child.kind(), &module, config);
        imports.push(TopImport {
            node: child,
            section,
            module,
        });
    }
    imports
}

fn insertion_edit(
    root: Node,
    source: &str,
    top_imports: &[TopImport],
    section: Section,
    module: &str,
    import_text: &str,
) -> Edit {
    if top_imports.is_empty() {
        let at = top_insertion_offset(root, source);
        let mut text = format!("{import_text}\n");
        // Keep a blank line between the new import and whatever follows.
        if !source[at..].starts_with('\n') && !source[at..].is_empty() {
            text.push('\n');
        }
        return Edit::insert(at, text);
    }

    let new_key = sort_key(section, module);
    let insert_after = top_imports
        .iter()
        .rfind(|entry| sort_key(entry.section, &entry.module) <= new_key);

    match insert_after {
        Some(prev) => {
            let at = line_end_inclusive(source, prev.node.end_byte());
            Edit::insert(at, format!("{import_text}\n"))
        }
        None => {
            let at = line_start(source, top_imports[0].node.start_byte());
            Edit::insert(at, format!("{import_text}\n"))
        }
    }
}
