use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::engine::config::Config;
use crate::engine::context::FileContext;
use crate::engine::diagnostic::Diagnostic;
use crate::engine::fix::apply_fixes;
use crate::engine::rule::Rule;
use crate::engine::source::LineIndex;
use crate::engine::suppress::Suppressions;

/// Bound on fix→re-check iterations, in case a fix keeps producing new
/// fixable diagnostics.
const MAX_FIX_ITERATIONS: usize = 10;

pub struct FileReport {
    pub path: PathBuf,
    /// Diagnostics that remain after fixing (all diagnostics in check mode).
    pub diagnostics: Vec<RenderedDiagnostic>,
    pub fixes_applied: usize,
    /// New file content when --fix changed anything.
    pub fixed_source: Option<String>,
}

/// A diagnostic with its position resolved to line/column, detached from
/// the source lifetime so reports can cross thread boundaries.
pub struct RenderedDiagnostic {
    pub rule: &'static str,
    pub message: String,
    pub line: usize,
    pub col: usize,
    pub severity: crate::engine::diagnostic::Severity,
    pub fixable: bool,
}

pub fn check_file(
    path: &Path,
    source: &str,
    config: &Config,
    rules: &[&dyn Rule],
    fix: bool,
) -> Result<FileReport> {
    let mut parser = crate::langs::python::parser();
    let mut current = source.to_string();
    let mut fixes_applied = 0usize;

    for _ in 0..MAX_FIX_ITERATIONS {
        let tree = parser
            .parse(&current, None)
            .ok_or_else(|| anyhow!("{}: parser failure", path.display()))?;
        let line_index = LineIndex::new(&current);
        let suppressions = Suppressions::from_tree(tree.root_node(), &current, &line_index);
        let ctx = FileContext {
            path,
            source: &current,
            tree: &tree,
            config,
            line_index: &line_index,
        };

        let mut diagnostics: Vec<Diagnostic> = rules
            .iter()
            .flat_map(|rule| rule.check(&ctx))
            .filter(|d| !suppressions.is_suppressed(d.rule, line_index.line(d.start)))
            .collect();
        diagnostics.sort_by_key(|d| (d.start, d.end));

        let has_fixes = diagnostics.iter().any(|d| d.fix.is_some());
        if !fix || !has_fixes {
            return Ok(FileReport {
                path: path.to_path_buf(),
                diagnostics: diagnostics.iter().map(|d| render(d, &line_index)).collect(),
                fixes_applied,
                fixed_source: (fixes_applied > 0).then_some(current),
            });
        }

        let fixes: Vec<_> = diagnostics.iter().filter_map(|d| d.fix.as_ref()).collect();
        let (next, applied) = apply_fixes(&current, &fixes);
        if applied == 0 {
            return Ok(FileReport {
                path: path.to_path_buf(),
                diagnostics: diagnostics.iter().map(|d| render(d, &line_index)).collect(),
                fixes_applied,
                fixed_source: (fixes_applied > 0).then_some(current),
            });
        }
        fixes_applied += applied;
        current = next;
    }

    Err(anyhow!(
        "{}: fixes did not converge after {MAX_FIX_ITERATIONS} iterations",
        path.display()
    ))
}

fn render(d: &Diagnostic, line_index: &LineIndex) -> RenderedDiagnostic {
    let (line, col) = line_index.line_col(d.start);
    RenderedDiagnostic {
        rule: d.rule,
        message: d.message.clone(),
        line,
        col,
        severity: d.severity,
        fixable: d.fix.is_some(),
    }
}
