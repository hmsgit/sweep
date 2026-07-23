use crate::engine::context::FileContext;
use crate::engine::diagnostic::Diagnostic;
use crate::engine::rule::Rule;
use crate::langs::python::docstring::{
    Field, content_range, detect, function_docstrings, parse_ir, render_ir, splice_fix,
};
use crate::langs::python::function_params;

/// Keeps documented parameters in sync with the signature. Only fires
/// when the docstring documents parameters at all — whether to document
/// them is a style choice; documenting the wrong ones is drift.
/// The fix rebuilds the parameter section in signature order, keeping
/// existing descriptions and adding empty stubs for missing entries.
pub struct DocstringSync;

impl Rule for DocstringSync {
    fn name(&self) -> &'static str {
        "docstring-sync"
    }

    fn explain(&self) -> &'static str {
        "documented parameters must match the signature (drift = stale or missing entries)"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let level = ctx.config.docstring_sync_level;
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
            let Some(ir) = parse_ir(content, style) else {
                continue;
            };
            if ir.params.is_empty() {
                continue;
            }

            let signature: Vec<String> = function_params(function, ctx.source)
                .into_iter()
                .map(|(name, _)| name)
                .filter(|name| name != "self" && name != "cls")
                .collect();

            let stale: Vec<&Field> = ir
                .params
                .iter()
                .filter(|p| !signature.contains(&p.name))
                .collect();
            let missing: Vec<&String> = signature
                .iter()
                .filter(|name| !ir.params.iter().any(|p| &p.name == *name))
                .collect();
            if stale.is_empty() && missing.is_empty() {
                continue;
            }

            let mut messages: Vec<String> = stale
                .iter()
                .map(|p| format!("docstring documents unknown parameter `{}`", p.name))
                .collect();
            messages.extend(
                missing
                    .iter()
                    .map(|name| format!("parameter `{name}` is missing from the docstring")),
            );

            // One fix, carried by the first finding: rebuild the param
            // section in signature order.
            let mut fix = None;
            if level.applies_fixes() {
                let mut synced = parse_ir(content, style).expect("parsed above");
                synced.params = signature
                    .iter()
                    .map(|name| {
                        ir.params
                            .iter()
                            .find(|p| &p.name == name)
                            .map(|p| Field {
                                name: p.name.clone(),
                                ty: p.ty.clone(),
                                desc: p.desc.clone(),
                            })
                            .unwrap_or_else(|| Field {
                                name: name.clone(),
                                ty: None,
                                desc: Vec::new(),
                            })
                    })
                    .collect();
                let rendered = render_ir(&synced, ctx.config.docstring_style);
                fix = splice_fix(
                    string,
                    ctx.source,
                    content_start,
                    content,
                    &rendered,
                    ctx.config.docstring_start.start,
                );
            }

            for message in messages {
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
