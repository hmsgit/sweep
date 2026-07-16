mod engine;
mod langs;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use rayon::prelude::*;

use engine::config::ConfigResolver;
use engine::diagnostic::Severity;
use engine::rule::Rule;
use engine::runner::{FileReport, check_file};

/// Fast, multi-language cleanup passes for pre-commit.
#[derive(Parser)]
#[command(name = "sweep", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check files (and optionally fix them in place).
    Check {
        /// Files or directories to check; directories are walked
        /// recursively for supported files, honoring .gitignore.
        #[arg(default_value = ".")]
        paths: Vec<PathBuf>,
        /// Apply available fixes in place.
        #[arg(long)]
        fix: bool,
        /// Treat warnings as errors for the exit code.
        #[arg(long)]
        strict: bool,
        /// Comma-separated rule names to run (default: all).
        #[arg(long, value_delimiter = ',')]
        select: Vec<String>,
        /// Comma-separated rule names to skip.
        #[arg(long, value_delimiter = ',')]
        ignore: Vec<String>,
        /// Explicit config file (default: nearest sweep.toml or
        /// pyproject.toml with [tool.sweep]).
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// List available rules.
    Rules,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("sweep: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    match Cli::parse().command {
        Command::Rules => {
            for rule in langs::python::rules::all_rules() {
                println!("{:<20} {}", rule.name(), rule.explain());
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Check {
            paths,
            fix,
            strict,
            select,
            ignore,
            config,
        } => check_command(&paths, fix, strict, &select, &ignore, config.as_deref()),
    }
}

fn check_command(
    paths: &[PathBuf],
    fix: bool,
    strict: bool,
    select: &[String],
    ignore: &[String],
    config_path: Option<&Path>,
) -> Result<ExitCode> {
    // Config is resolved per file: each file uses the nearest sweep.toml
    // or pyproject.toml above it, so monorepos with per-app configs work
    // when pre-commit runs from the repo root. --config overrides all.
    let resolver = ConfigResolver::new(config_path)?;

    let all_rules = langs::python::rules::all_rules();
    let known: BTreeSet<&str> = all_rules.iter().map(|r| r.name()).collect();
    for requested in select.iter().chain(ignore) {
        if !known.contains(requested.as_str()) {
            anyhow::bail!(
                "unknown rule `{requested}` (known: {})",
                known.iter().copied().collect::<Vec<_>>().join(", ")
            );
        }
    }
    let rules: Vec<&dyn Rule> = all_rules
        .iter()
        .map(|r| r.as_ref())
        .filter(|r| select.is_empty() || select.iter().any(|s| s == r.name()))
        .filter(|r| !ignore.iter().any(|s| s == r.name()))
        .collect();

    let files = collect_files(paths, &resolver)?;
    let mut reports: Vec<FileReport> = files
        .par_iter()
        .map(|path| -> Result<FileReport> {
            let config = resolver.for_path(path)?;
            let source = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            let report = check_file(path, &source, &config, &rules, fix)?;
            if let Some(fixed) = &report.fixed_source {
                std::fs::write(path, fixed)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            Ok(report)
        })
        .collect::<Result<Vec<_>>>()?;
    reports.sort_by(|a, b| a.path.cmp(&b.path));

    let mut counts: [usize; 3] = [0, 0, 0]; // info, warning, error
    let mut fixed = 0usize;
    let mut fixable = 0usize;
    for report in &reports {
        fixed += report.fixes_applied;
        for d in &report.diagnostics {
            counts[match d.severity {
                Severity::Info => 0,
                Severity::Warning => 1,
                Severity::Error => 2,
            }] += 1;
            if d.fixable {
                fixable += 1;
            }
            println!(
                "{}:{}:{}: {} [{}] {}{}",
                report.path.display(),
                d.line,
                d.col,
                d.severity,
                d.rule,
                d.message,
                if d.fixable { " [fixable]" } else { "" },
            );
        }
    }

    let [infos, warnings, errors] = counts;
    let remaining = infos + warnings + errors;
    match (remaining, fixed) {
        (0, 0) => println!("All clean ({} files).", reports.len()),
        (0, _) => println!(
            "Fixed {fixed} issue(s); all clean ({} files).",
            reports.len()
        ),
        _ => {
            let breakdown: Vec<String> = [
                (errors, "error(s)"),
                (warnings, "warning(s)"),
                (infos, "info"),
            ]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, label)| format!("{n} {label}"))
            .collect();
            let mut summary = format!("{remaining} issue(s) found ({})", breakdown.join(", "));
            if fixed > 0 {
                summary.push_str(&format!(", {fixed} fixed"));
            }
            if !fix && fixable > 0 {
                summary.push_str(&format!("; {fixable} fixable with --fix"));
            }
            println!("{summary}.");
        }
    }

    // Only errors gate the run; --strict promotes warnings. info never
    // fails: it exists to notify, not to block.
    let failing = errors + if strict { warnings } else { 0 };
    Ok(if failing > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn collect_files(paths: &[PathBuf], resolver: &ConfigResolver) -> Result<Vec<PathBuf>> {
    let mut files = BTreeSet::new();
    for path in paths {
        if path.is_file() {
            // Explicitly passed files (e.g. by pre-commit) are always
            // taken, regardless of excludes.
            if is_supported(path) {
                files.insert(path.clone());
            }
            continue;
        }
        for entry in ignore::WalkBuilder::new(path).build() {
            let entry = entry?;
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let p = entry.path();
            if !is_supported(p) {
                continue;
            }
            // Excludes come from the config governing each file.
            let display = p.to_string_lossy();
            if resolver
                .for_path(p)?
                .exclude
                .iter()
                .any(|pat| display.contains(pat.as_str()))
            {
                continue;
            }
            files.insert(p.to_path_buf());
        }
    }
    Ok(files.into_iter().collect())
}

fn is_supported(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "py")
}
