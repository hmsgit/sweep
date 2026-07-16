use tree_sitter::Node;

use crate::engine::context::{FileContext, walk_tree};
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;

/// Prefers `dict(key=val)` over `{"key": val}` when every key is a
/// string literal that is a valid Python identifier (and not a hard
/// keyword). Dicts with computed, non-identifier or duplicate keys are
/// left alone — they can't be expressed as keyword arguments.
pub struct DictCall;

const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

impl Rule for DictCall {
    fn name(&self) -> &'static str {
        "dict-call"
    }

    fn explain(&self) -> &'static str {
        "prefer dict(key=val) over {\"key\": val} when all keys are identifier strings"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.dict_call_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        walk_tree(ctx.root(), &mut |node| {
            if node.kind() != "dictionary" {
                return;
            }
            let Some(items) = convertible_items(node, ctx.source) else {
                return;
            };

            let mut diagnostic = Diagnostic::new(
                self.name(),
                "prefer dict(key=val) over {\"key\": val}".to_string(),
                node.start_byte(),
                node.end_byte(),
            )
            .with_severity(severity);

            if level.applies_fixes() {
                let call = format!("dict({})", items.join(", "));
                diagnostic = diagnostic.with_fix(Fix::new(vec![Edit::replace(
                    node.start_byte(),
                    node.end_byte(),
                    call,
                )]));
            }
            diagnostics.push(diagnostic);
        });
        diagnostics
    }
}

/// The dict rendered as keyword-argument items, or None when the
/// literal can't be converted faithfully.
fn convertible_items(dict: Node, source: &str) -> Option<Vec<String>> {
    let mut items = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    let mut cursor = dict.walk();
    for child in dict.children(&mut cursor) {
        match child.kind() {
            "{" | "}" | "," => {}
            // Comments inside the literal would be dropped by a rewrite.
            "comment" => return None,
            "dictionary_splat" => items.push(source[child.byte_range()].to_string()),
            "pair" => {
                let key = child.child_by_field_name("key")?;
                let value = child.child_by_field_name("value")?;
                let name = identifier_string_content(key, source)?;
                if KEYWORDS.contains(&name.as_str()) || keys.contains(&name) {
                    return None;
                }
                keys.push(name.clone());
                items.push(format!("{name}={}", &source[value.byte_range()]));
            }
            _ => return None,
        }
    }
    // Only pairs make the rewrite worthwhile; `{}` and `{**a}` stay.
    (!keys.is_empty()).then_some(items)
}

/// The content of a plain string literal, if it's a valid identifier.
fn identifier_string_content(key: Node, source: &str) -> Option<String> {
    if key.kind() != "string" {
        return None;
    }
    let mut content = None;
    let mut cursor = key.walk();
    for child in key.children(&mut cursor) {
        match child.kind() {
            "string_start" | "string_end" => {}
            "string_content" if content.is_none() => {
                content = Some(&source[child.byte_range()]);
            }
            _ => return None,
        }
    }
    let content = content?;
    let mut chars = content.chars();
    let first = chars.next()?;
    if !(first.is_alphabetic() || first == '_') {
        return None;
    }
    chars
        .all(|c| c.is_alphanumeric() || c == '_')
        .then(|| content.to_string())
}
