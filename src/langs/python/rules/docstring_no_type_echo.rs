use crate::engine::context::FileContext;
use crate::engine::diagnostic::Diagnostic;
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{
    content_range, detect, function_docstrings, parse_ir, render_ir, splice_fix,
};
use crate::langs::python::function_params;

/// Flags docstring type declarations (`:type x: int`, `x (int):`,
/// `:rtype: bool`) that repeat an identical signature annotation —
/// pure duplication once the code is annotated, and a second place
/// for the type to go stale. The fix drops only the echoed types;
/// differing ones (deliberate doc-level detail) are left alone.
pub struct DocstringNoTypeEcho;

impl Rule for DocstringNoTypeEcho {
    fn name(&self) -> &'static str {
        "docstring-no-type-echo"
    }

    fn explain(&self) -> &'static str {
        "docstring types that repeat signature annotations are duplication"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.docstring_no_type_echo_level;
        let Some(severity) = level.severity() else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        for (function, string) in function_docstrings(ctx.root()) {
            let Some((content_start, content_end)) = content_range(string, ctx.source) else {
                continue;
            };
            let content = &ctx.source[content_start..content_end];
            let style = detect(content).unwrap_or(ctx.config.docstring_style);
            let Some(_ir) = parse_ir(content, style) else {
                continue;
            };

            let annotations: Vec<(String, String)> = function_params(function, ctx.source)
                .into_iter()
                .filter_map(|(name, ty)| ty.map(|t| (name, t)))
                .collect();
            let return_annotation = function
                .child_by_field_name("return_type")
                .map(|t| ctx.source[t.byte_range()].to_string());

            let mut echoed: Vec<String> = Vec::new();
            let mut cleaned = parse_ir(content, style).expect("parsed above");
            for param in &mut cleaned.params {
                let Some(doc_ty) = &param.ty else { continue };
                let matches_signature = annotations
                    .iter()
                    .any(|(name, ty)| name == &param.name && same_type(ty, doc_ty));
                if matches_signature {
                    echoed.push(format!(
                        "docstring type for `{}` repeats the signature annotation",
                        param.name
                    ));
                    param.ty = None;
                }
            }
            if let Some(ret) = &mut cleaned.returns
                && let (Some(doc_ty), Some(sig_ty)) = (&ret.ty, &return_annotation)
                && same_type(sig_ty, doc_ty)
            {
                echoed.push("docstring rtype repeats the return annotation".to_string());
                ret.ty = None;
            }
            if echoed.is_empty() {
                continue;
            }

            let mut fix = None;
            if level.applies_fixes() {
                let rendered = render_ir(&cleaned, ctx.config.docstring_style);
                fix = splice_fix(
                    string,
                    ctx.source,
                    content_start,
                    content,
                    &rendered,
                    ctx.config.docstring_start.start,
                );
            }

            for message in echoed {
                let mut diagnostic =
                    Diagnostic::new(self.name(), message, string.start_byte(), string.end_byte())
                        .with_severity(severity);
                if let Some(f) = fix.take() {
                    diagnostic = diagnostic.with_fix(f);
                }
                diagnostics.push(diagnostic);
            }
        }
        diagnostics
    }
}

/// Type texts compared ignoring whitespace, so `dict[str,int]` echoes
/// `dict[str, int]`.
fn same_type(a: &str, b: &str) -> bool {
    let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    strip(a) == strip(b)
}
