use crate::engine::context::{FileContext, walk_tree};
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::{Edit, Fix};
use crate::engine::rule::Rule;
use crate::langs::python::{line_end_inclusive, line_start};

/// Flags narration comments that restate the adjacent code — the
/// classic LLM artifact (`# create the payload` above
/// `payload = create_payload(...)`). A comment is an echo when every
/// content word either appears among the code line's identifier
/// tokens or is a generic narration verb, with at least one real
/// token match. Comments that add information survive.
pub struct CommentsNoEcho;

/// Words that never need matching: articles, glue, and the narration
/// verbs LLM comments open with.
const FREE_WORDS: &[&str] = &[
    // glue
    "the",
    "a",
    "an",
    "to",
    "of",
    "for",
    "and",
    "or",
    "in",
    "on",
    "with",
    "from",
    "is",
    "are",
    "be",
    "this",
    "that",
    "it",
    "its",
    "we",
    "our",
    "now",
    "then",
    "also",
    "just",
    "simply",
    "new",
    "each",
    "all",
    "into",
    "as",
    "at",
    "by",
    "up",
    // narration verbs
    "initialize",
    "initialise",
    "init",
    "create",
    "define",
    "declare",
    "import",
    "loop",
    "iterate",
    "call",
    "invoke",
    "return",
    "print",
    "set",
    "assign",
    "get",
    "fetch",
    "retrieve",
    "check",
    "validate",
    "update",
    "add",
    "append",
    "remove",
    "delete",
    "compute",
    "calculate",
    "build",
    "construct",
    "instantiate",
    "convert",
    "parse",
    "open",
    "close",
    "read",
    "write",
    "send",
    "handle",
    "process",
    "execute",
    "run",
    "perform",
    "make",
    "setup",
    "step",
    "using",
    "use",
];

impl Rule for CommentsNoEcho {
    fn name(&self) -> &'static str {
        "comments-no-echo"
    }

    fn explain(&self) -> &'static str {
        "narration comments that restate the adjacent code add nothing"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.comments_no_echo_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        walk_tree(ctx.root(), &mut |node| {
            if node.kind() != "comment" {
                return;
            }
            let text = &ctx.source[node.byte_range()];
            if is_exempt(text) {
                return;
            }

            let start_of_line = line_start(ctx.source, node.start_byte());
            let before = &ctx.source[start_of_line..node.start_byte()];
            let standalone = before.chars().all(char::is_whitespace);

            let code = if standalone {
                // The next line must be code (not blank, not a comment).
                let next_start = line_end_inclusive(ctx.source, node.end_byte());
                let next_end = line_end_inclusive(ctx.source, next_start.min(ctx.source.len()));
                let line = ctx.source.get(next_start..next_end).unwrap_or("");
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    return;
                }
                line.to_string()
            } else {
                before.to_string()
            };

            if !is_echo(text, &code) {
                return;
            }

            let mut diagnostic = Diagnostic::new(
                self.name(),
                "comment restates the adjacent code".to_string(),
                node.start_byte(),
                node.end_byte(),
            )
            .with_severity(severity);

            if level.applies_fixes() {
                let edit = if standalone {
                    Edit::delete(
                        start_of_line,
                        line_end_inclusive(ctx.source, node.end_byte()),
                    )
                } else {
                    let mut from = node.start_byte();
                    while from > 0 && ctx.source.as_bytes()[from - 1] == b' ' {
                        from -= 1;
                    }
                    Edit::delete(from, node.end_byte())
                };
                diagnostic = diagnostic.with_fix(Fix::new(vec![edit]));
            }
            diagnostics.push(diagnostic);
        });
        diagnostics
    }
}

/// Directives, shebangs, encoding cookies, URLs — never narration.
fn is_exempt(comment: &str) -> bool {
    let body = comment.trim_start_matches('#').trim();
    comment.starts_with("#!")
        || body.starts_with("-*-")
        || body.contains("://")
        || body.starts_with("sweep:")
        || body.starts_with("type:")
        || body.starts_with("noqa")
        || body.eq_ignore_ascii_case("noqa")
        || body.starts_with("pragma")
}

/// Every content word appears among the code tokens or is a free word,
/// and at least one word actually matched a code token.
fn is_echo(comment: &str, code: &str) -> bool {
    let code_tokens = tokens(code);
    let mut matched_any = false;
    let mut content_words = 0usize;

    for word in words(comment.trim_start_matches('#')) {
        if word.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        content_words += 1;
        if code_tokens.iter().any(|t| loose_eq(&word, t)) {
            matched_any = true;
        } else if !FREE_WORDS.contains(&word.as_str()) {
            return false;
        }
    }
    content_words > 0 && matched_any
}

/// Lowercased alphanumeric runs.
pub(super) fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Code tokens: alphanumeric runs plus their snake_case components.
pub(super) fn tokens(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for run in code.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if run.is_empty() {
            continue;
        }
        out.push(run.to_lowercase());
        for part in run.split('_').filter(|p| !p.is_empty()) {
            out.push(part.to_lowercase());
        }
    }
    out
}

/// Equal up to a trailing plural/3rd-person `s`.
pub(super) fn loose_eq(word: &str, token: &str) -> bool {
    word == token
        || word.strip_suffix('s').is_some_and(|w| w == token)
        || token.strip_suffix('s').is_some_and(|t| t == word)
}
