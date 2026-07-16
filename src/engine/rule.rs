use crate::engine::context::FileContext;
use crate::engine::diagnostic::Diagnostic;

/// One deterministic check/cleanup pass. Each implementation does exactly
/// one thing and runs independently of every other rule.
pub trait Rule: Send + Sync {
    /// Kebab-case identifier used in output, --select/--ignore, config
    /// tables and suppression directives (e.g. "local-imports").
    fn name(&self) -> &'static str;

    /// One-line description shown by `sweep rules`.
    fn explain(&self) -> &'static str;

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic>;
}
