use tree_sitter::Node;

use crate::engine::config::DictForm;
use crate::engine::context::{FileContext, walk_tree};
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;

/// Enforces one dict-construction style in either direction:
/// `dict-style = "function"` rewrites `{"key": val}` literals into
/// `dict(key=val)` calls; `dict-style = "literal"` rewrites
/// keyword-only `dict(...)` calls into literals. Only faithful
/// conversions are made — computed keys, positional `dict(...)`
/// arguments, keywords, duplicates and comments inside the expression
/// all make sweep leave it alone.
///
/// Splats never become `**` in a call (`dict(**d, a=5)` raises
/// TypeError where `{**d, "a": 5}` overrides). Instead they fold in as
/// positional mappings, chained with `|` when the shape demands it:
/// `{**d, "a": 5}` -> `dict(d, a=5)` and
/// `{"a": 1, **d}` -> `dict(a=1) | dict(d)` — both merge exactly like
/// the literal. The call-to-literal direction converts `**` freely:
/// any `dict(**d, ...)` that runs without raising builds the same dict
/// as the literal.
pub struct DictStyle;

const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

impl Rule for DictStyle {
    fn name(&self) -> &'static str {
        "dict-style"
    }

    fn explain(&self) -> &'static str {
        "dicts follow the configured construction style (dict-style = literal|function)"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let config = ctx.config.dict_style;
        let Some(severity) = config.level.severity() else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        walk_tree(ctx.root(), &mut |node| {
            let rewrite = match config.form {
                DictForm::Function if node.kind() == "dictionary" => {
                    literal_to_call(node, ctx.source).map(|replacement| {
                        (replacement, "prefer dict(key=val) over {\"key\": val}")
                    })
                }
                DictForm::Literal if node.kind() == "call" => call_to_literal(node, ctx.source)
                    .map(|items| {
                        (
                            format!("{{{}}}", items.join(", ")),
                            "prefer {\"key\": val} over dict(key=val)",
                        )
                    }),
                _ => None,
            };
            let Some((replacement, message)) = rewrite else {
                return;
            };

            let mut diagnostic = Diagnostic::new(
                self.name(),
                message.to_string(),
                node.start_byte(),
                node.end_byte(),
            )
            .with_severity(severity);
            if config.level.applies_fixes() {
                diagnostic = diagnostic.with_fix(Fix::new(vec![Edit::replace(
                    node.start_byte(),
                    node.end_byte(),
                    replacement,
                )]));
            }
            diagnostics.push(diagnostic);
        });
        diagnostics
    }
}

/// A `{...}` literal as an equivalent call expression, or None when no
/// faithful form exists. Plain literals become `dict(k=v, ...)`. A
/// splat starts a new chunk with itself as the positional mapping —
/// `dict(d, a=5)` lets the keywords override d exactly like the
/// literal — and chunks merge with `|`, whose later-wins semantics
/// also match the literal:
///
/// `{**d, "a": 5}`        -> `dict(d, a=5)`
/// `{"a": 1, **d, "b": 2}` -> `dict(a=1) | dict(d, b=2)`
fn literal_to_call(dict: Node, source: &str) -> Option<String> {
    // Each chunk: an optional splatted mapping plus following pairs.
    let mut chunks: Vec<(Option<String>, Vec<String>)> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    let mut cursor = dict.walk();
    for child in dict.children(&mut cursor) {
        match child.kind() {
            "{" | "}" | "," => {}
            // Comments inside the literal would be dropped by a rewrite.
            "comment" => return None,
            "dictionary_splat" => {
                let expr = child.named_child(0)?;
                chunks.push((Some(source[expr.byte_range()].to_string()), Vec::new()));
            }
            "pair" => {
                let key = child.child_by_field_name("key")?;
                let value = child.child_by_field_name("value")?;
                let name = identifier_string_content(key, source)?;
                if KEYWORDS.contains(&name.as_str()) || keys.contains(&name) {
                    return None;
                }
                keys.push(name.clone());
                if chunks.is_empty() {
                    chunks.push((None, Vec::new()));
                }
                let pairs = &mut chunks.last_mut()?.1;
                pairs.push(format!("{name}={}", &source[value.byte_range()]));
            }
            _ => return None,
        }
    }
    // Only pairs make the rewrite worthwhile; `{}` and `{**a}` stay.
    if keys.is_empty() {
        return None;
    }

    let rendered: Vec<String> = chunks
        .iter()
        .map(|(splat, pairs)| {
            let mut args: Vec<String> = Vec::new();
            args.extend(splat.iter().cloned());
            args.extend(pairs.iter().cloned());
            format!("dict({})", args.join(", "))
        })
        .collect();
    if rendered.len() == 1 {
        return rendered.into_iter().next();
    }
    let joined = rendered.join(" | ");
    Some(if union_needs_parens(dict) {
        format!("({joined})")
    } else {
        joined
    })
}

/// A multi-chunk union replaces the literal with a `|` expression,
/// which binds looser than the literal did; parenthesize unless the
/// surrounding node already delimits it.
fn union_needs_parens(dict: Node) -> bool {
    let Some(parent) = dict.parent() else {
        return false;
    };
    !matches!(
        parent.kind(),
        "expression_statement"
            | "assignment"
            | "augmented_assignment"
            | "return_statement"
            | "argument_list"
            | "keyword_argument"
            | "pair"
            | "list"
            | "set"
            | "tuple"
            | "parenthesized_expression"
            | "yield"
    )
}

/// `dict(key=val, **rest)` items as literal pairs, or None when the
/// call can't be expressed as a literal (positional args, nothing to
/// convert).
fn call_to_literal(call: Node, source: &str) -> Option<Vec<String>> {
    let function = call.child_by_field_name("function")?;
    if function.kind() != "identifier" || &source[function.byte_range()] != "dict" {
        return None;
    }
    let arguments = call.child_by_field_name("arguments")?;

    let mut items = Vec::new();
    let mut keywords = 0usize;
    let mut cursor = arguments.walk();
    for child in arguments.children(&mut cursor) {
        match child.kind() {
            "(" | ")" | "," => {}
            "comment" => return None,
            "dictionary_splat" => items.push(source[child.byte_range()].to_string()),
            "keyword_argument" => {
                let name = child.child_by_field_name("name")?;
                let value = child.child_by_field_name("value")?;
                items.push(format!(
                    "\"{}\": {}",
                    &source[name.byte_range()],
                    &source[value.byte_range()]
                ));
                keywords += 1;
            }
            // Positional arguments (dict(mapping), dict(iterable)) have
            // no literal equivalent.
            _ => return None,
        }
    }
    (keywords > 0).then_some(items)
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
