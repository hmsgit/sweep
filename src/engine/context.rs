use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::engine::config::Config;
use crate::engine::source::LineIndex;

/// Everything a rule needs to check one parsed file. Rules never mutate;
/// they return diagnostics with optional fixes.
pub struct FileContext<'a> {
    #[allow(dead_code)] // not used by the current rules, part of the rule API
    pub path: &'a Path,
    pub source: &'a str,
    pub tree: &'a Tree,
    pub config: &'a Config,
    #[allow(dead_code)]
    pub line_index: &'a LineIndex,
}

impl<'a> FileContext<'a> {
    pub fn text(&self, node: Node) -> &'a str {
        &self.source[node.byte_range()]
    }

    pub fn root(&self) -> Node<'a> {
        self.tree.root_node()
    }
}

/// Depth-first walk over every node in the tree.
pub fn walk_tree<'t>(node: Node<'t>, f: &mut dyn FnMut(Node<'t>)) {
    f(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_tree(child, f);
    }
}
