//! `sweep verify` — the runtime counterpart of imports-required-extras.
//!
//! Builds one throwaway venv per extras set (base install plus
//! each declared extra, via `uv run --isolated`), imports every shipped
//! module in each, and judges the outcome against the same mapping the
//! static rule enforces: a module whose required extras are satisfied
//! must import cleanly; an unsatisfied module may fail, but only with
//! ModuleNotFoundError. It catches what static analysis can't —
//! import-time side effects, broken installed deps, metadata that
//! disagrees with reality.

use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use rayon::prelude::*;

use crate::engine::config::{Config, ProjectDeps};
use crate::langs::python::rules::imports_required_extras::{
    RequireSource, required_extras_with_source,
};
use crate::output::count;

/// Imports each argv module, one TSV result line per module. BaseException
/// so SystemExit-raising module bodies are reported, not fatal. The
/// SIGALRM watchdog handles imports whose side effects hang the
/// interpreter (gevent monkey-patching being the classic): it reports
/// the module and exits, and the runner restarts with the remainder.
const IMPORT_SCRIPT: &str = "\
import importlib, os, signal, sys

def bail(signum, frame):
    print('HANG\\t' + current, flush=True)
    os._exit(37)

signal.signal(signal.SIGALRM, bail)
for current in sys.argv[1:]:
    signal.alarm(90)
    try:
        importlib.import_module(current)
        print('OK\\t' + current, flush=True)
    except BaseException as exc:
        message = ' '.join(str(exc).split())
        print('FAIL\\t' + current + '\\t' + type(exc).__name__ + '\\t' + message, flush=True)
signal.alarm(0)
";

/// Ceiling on interpreter restarts per venv after hangs or
/// crashes — each restart makes progress, so this only stops a
/// pathological project from running forever.
const MAX_RESTARTS: usize = 25;

pub fn verify_command(path: &Path, only: &[String], skip: &[String]) -> Result<ExitCode> {
    let (project_dir, config) = load_project(path)?;
    let deps = &config.project_deps;
    let Some(dist_name) = deps.dist_name.clone() else {
        anyhow::bail!(
            "pyproject.toml in {} has no [project].name; nothing to install",
            project_dir.display()
        );
    };
    if deps.package_roots.is_empty() {
        anyhow::bail!("no shipped packages found under {}", project_dir.display());
    }

    for requested in only.iter().chain(skip) {
        if !deps.extras.iter().any(|(name, _)| name == requested) {
            anyhow::bail!(
                "unknown extra `{requested}` (declared: {})",
                deps.extras
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    let modules = collect_modules(&project_dir, &config)?;
    if modules.is_empty() {
        anyhow::bail!(
            "no modules found under package roots {:?} in {}",
            deps.package_roots,
            project_dir.display()
        );
    }

    // base plus one venv per extra; extras adding no dependencies
    // resolve to the base venv and are skipped as duplicates.
    let environments: Vec<Option<String>> = std::iter::once(None)
        .chain(
            deps.extras
                .iter()
                .filter(|(name, paths)| {
                    (only.is_empty() || only.contains(name))
                        && !skip.contains(name)
                        && !paths.is_empty()
                })
                .map(|(name, _)| Some(name.clone())),
        )
        .collect();

    let results: Vec<(String, Result<BTreeMap<String, ImportOutcome>>)> = environments
        .par_iter()
        .map(|extra| {
            let env_name = extra.clone().unwrap_or_else(|| "base".to_string());
            let outcome = run_environment(&project_dir, &dist_name, extra.as_deref(), &modules);
            (env_name, outcome)
        })
        .collect();

    let styled = std::io::stdout().is_terminal();
    let mut errors = 0usize;
    // Modules that imported fine somewhere their requirements weren't
    // met, grouped by the mapping decision that set the requirement —
    // the unit a reader would edit. One note per mapping, not per
    // module per venv.
    let mut overmapped: BTreeMap<RequireSource, Overmapped> = BTreeMap::new();

    for (env_name, outcome) in &results {
        let available = available_paths(deps, env_name);
        let imports = match outcome {
            Ok(imports) => imports,
            Err(err) => {
                print_finding(styled, "error", env_name, &format!("{err:#}"));
                errors += 1;
                continue;
            }
        };
        // One root cause (a missing dependency deep in a shared import
        // chain) fails whole subpackage trees; grouping by the module's
        // scope (its rules key, name-match, or first-level package) plus
        // the verdict keeps the report one line per cause per scope.
        let mut grouped: BTreeMap<(String, String), Vec<&str>> = BTreeMap::new();
        for module in &modules {
            let (requires, source) =
                required_extras_with_source(module, &config.imports_required_extras, deps);
            let satisfied = requires.iter().all(|extra| {
                deps.extras
                    .iter()
                    .find(|(name, _)| name == extra)
                    .is_some_and(|(_, paths)| paths.iter().all(|p| available.contains(p.as_str())))
            });
            let verdict = match imports.get(module) {
                Some(ImportOutcome::Ok) => {
                    if !satisfied {
                        let entry = overmapped.entry(source).or_insert_with(|| Overmapped {
                            requires: requires.clone(),
                            modules: BTreeSet::new(),
                            venvs: BTreeSet::new(),
                        });
                        entry.modules.insert(module);
                        entry.venvs.insert(env_name);
                    }
                    continue;
                }
                Some(ImportOutcome::Fail { exc_type, message }) => {
                    if satisfied {
                        format!(
                            "expected to import here (this venv provides {}) \
                             yet failed: {exc_type}: {message}",
                            describe_requires(&requires),
                        )
                    } else if exc_type != "ModuleNotFoundError" {
                        format!(
                            "failed with more than a missing optional dependency: {exc_type}: {message}"
                        )
                    } else {
                        continue;
                    }
                }
                None => "produced no result — import crashed the interpreter".to_string(),
            };
            grouped
                .entry((scope_of(module, &source), verdict))
                .or_default()
                .push(module);
        }
        for ((scope, verdict), affected) in grouped {
            errors += affected.len();
            print_finding(
                styled,
                "error",
                env_name,
                &describe_group(&scope, &verdict, &affected),
            );
        }
    }

    let mut notes = 0usize;
    for (source, hits) in overmapped {
        // How many modules this mapping decision governs in total, so
        // the note can say "all 24" vs "17 of 24".
        let governed = modules
            .iter()
            .filter(|module| {
                required_extras_with_source(module, &config.imports_required_extras, deps).1
                    == source
            })
            .count();
        let scope = match &source {
            RequireSource::Explicit(key) => format!(
                "the requires entry \"{key}\" maps to {}",
                describe_requires(&hits.requires),
            ),
            RequireSource::NameMatch { prefix, extra } => {
                format!("{prefix} is name-matched to extra {extra}")
            }
            RequireSource::Base => continue,
        };
        let coverage = if governed == 1 {
            "its module".to_string()
        } else if hits.modules.len() == governed {
            format!("all {}", count(governed, "module"))
        } else {
            format!(
                "{} of its {}",
                hits.modules.len(),
                count(governed, "module")
            )
        };
        let module_samples: Vec<&str> = hits.modules.iter().copied().collect();
        let venv_samples: Vec<&str> = hits.venvs.iter().copied().collect();
        print_finding(
            styled,
            "info",
            "mapping",
            &format!(
                "{scope}, yet {coverage} imported fine in {} \
                 without {} ({}). Nothing is broken — but either the mapping \
                 claims more than the code needs, or a dependency is only \
                 arriving transitively today.{}",
                count(hits.venvs.len(), "venv"),
                if hits.requires.len() == 1 {
                    "it"
                } else {
                    "them"
                },
                sample(&venv_samples),
                if hits.modules.len() < governed {
                    format!(" (e.g. {})", sample(&module_samples))
                } else {
                    String::new()
                },
            ),
        );
        notes += 1;
    }

    println!(
        "sweep verify: {} × {}: {}, {}",
        count(results.len(), "venv"),
        count(modules.len(), "module"),
        count(errors, "error"),
        count(notes, "note"),
    );
    Ok(if errors > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

enum ImportOutcome {
    Ok,
    Fail { exc_type: String, message: String },
}

/// One mapping decision whose modules imported fine where the mapping
/// said they couldn't: the shared requirement, the modules it
/// happened to, and the venvs it happened in.
struct Overmapped<'m> {
    requires: Vec<String>,
    modules: BTreeSet<&'m str>,
    venvs: BTreeSet<&'m str>,
}

/// The pyproject governing `path`: the file itself, or the nearest one
/// in its parent directories.
fn load_project(path: &Path) -> Result<(PathBuf, Config)> {
    let abs = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?;
    let start = if abs.is_dir() {
        abs.as_path()
    } else {
        abs.parent().unwrap_or(abs.as_path())
    };
    for dir in start.ancestors() {
        let pyproject = dir.join("pyproject.toml");
        if pyproject.is_file() {
            let text = std::fs::read_to_string(&pyproject)
                .with_context(|| format!("reading {}", pyproject.display()))?;
            let config = Config::from_toml(&text, &pyproject)?;
            return Ok((dir.to_path_buf(), config));
        }
    }
    anyhow::bail!("no pyproject.toml found above {}", abs.display())
}

/// Every shipped module as a dotted path: walk the package roots for
/// .py files, honoring the config's excludes.
fn collect_modules(project_dir: &Path, config: &Config) -> Result<Vec<String>> {
    let mut modules = BTreeSet::new();
    for root in &config.project_deps.package_roots {
        let root_dir = project_dir.join(root);
        if !root_dir.is_dir() {
            continue;
        }
        for entry in ignore::WalkBuilder::new(&root_dir).build() {
            let entry = entry?;
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let file = entry.path();
            if file.extension().is_none_or(|e| e != "py") {
                continue;
            }
            let display = file.to_string_lossy();
            if config
                .exclude
                .iter()
                .any(|pat| display.contains(pat.as_str()))
            {
                continue;
            }
            let Ok(rel) = file.strip_prefix(project_dir) else {
                continue;
            };
            let mut segments: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let Some(last) = segments.pop() else {
                continue;
            };
            match last.strip_suffix(".py") {
                Some("__init__") => {}
                Some(stem) => segments.push(stem.to_string()),
                None => continue,
            }
            if !segments.is_empty() {
                modules.insert(segments.join("."));
            }
        }
    }
    Ok(modules.into_iter().collect())
}

/// Import paths available in a venv: base dependencies plus
/// the environment's extra.
fn available_paths<'d>(deps: &'d ProjectDeps, env_name: &str) -> BTreeSet<&'d str> {
    let mut available: BTreeSet<&str> = deps.base.iter().map(String::as_str).collect();
    if let Some((_, paths)) = deps.extras.iter().find(|(name, _)| name == env_name) {
        available.extend(paths.iter().map(String::as_str));
    }
    available
}

/// Install the project into an isolated venv via uv and import
/// every module there. A hanging or crashing import kills only that
/// interpreter: the offender is recorded and a fresh one continues
/// with the remaining modules.
fn run_environment(
    project_dir: &Path,
    dist_name: &str,
    extra: Option<&str>,
    modules: &[String],
) -> Result<BTreeMap<String, ImportOutcome>> {
    let requirement = match extra {
        Some(extra) => format!("{dist_name}[{extra}] @ file://{}", project_dir.display()),
        None => format!("{dist_name} @ file://{}", project_dir.display()),
    };

    let mut results: BTreeMap<String, ImportOutcome> = BTreeMap::new();
    let mut pending: Vec<&String> = modules.iter().collect();
    for _restart in 0..=MAX_RESTARTS {
        let output = std::process::Command::new("uv")
            .args([
                "run",
                "--isolated",
                "--no-project",
                "--quiet",
                "--with",
                &requirement,
                "python",
                "-c",
                IMPORT_SCRIPT,
            ])
            .args(&pending)
            .current_dir(project_dir)
            .output()
            .context("running uv (is uv installed and on PATH?)")?;

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let mut fields = line.splitn(4, '\t');
            let outcome = match (fields.next(), fields.next()) {
                (Some("OK"), Some(module)) => (module, ImportOutcome::Ok),
                (Some("FAIL"), Some(module)) => (
                    module,
                    ImportOutcome::Fail {
                        exc_type: fields.next().unwrap_or("unknown").to_string(),
                        message: fields.next().unwrap_or("").to_string(),
                    },
                ),
                (Some("HANG"), Some(module)) => (
                    module,
                    ImportOutcome::Fail {
                        exc_type: "ImportHang".to_string(),
                        message: "import did not finish within 90s; \
                                  interpreter killed and restarted"
                            .to_string(),
                    },
                ),
                _ => continue,
            };
            results.insert(outcome.0.to_string(), outcome.1);
        }

        // First round producing nothing: the venv never came up
        // (resolution failure, missing interpreter) — surface uv's stderr.
        if results.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "venv failed to build: {}",
                stderr.lines().last().unwrap_or("(no uv output)")
            );
        }

        pending.retain(|module| !results.contains_key(module.as_str()));
        if pending.is_empty() {
            break;
        }
        // Exit 37 is the watchdog: the hang was reported above, the
        // remainder just needs a fresh interpreter. Any other exit with
        // modules remaining means the interpreter died silently
        // (segfault, hard exit) — blame the first unreported module so
        // every round progresses.
        if output.status.code() != Some(37) {
            let culprit = pending.remove(0);
            results.insert(
                culprit.clone(),
                ImportOutcome::Fail {
                    exc_type: "InterpreterCrash".to_string(),
                    message: format!(
                        "interpreter exited ({}) while importing",
                        output
                            .status
                            .code()
                            .map_or("by signal".to_string(), |c| format!("code {c}")),
                    ),
                },
            );
            if pending.is_empty() {
                break;
            }
        }
    }
    Ok(results)
}

/// A grouped finding: the verdict, prefixed with the affected module
/// (single) or the scope with a count plus samples (a whole scope
/// sharing one root cause).
fn describe_group(scope: &str, verdict: &str, affected: &[&str]) -> String {
    match affected {
        [module] => format!("{module} {verdict}"),
        _ => format!(
            "{scope}: {} {verdict} — e.g. {}",
            count(affected.len(), "module"),
            sample(affected),
        ),
    }
}

/// The reporting scope of a module: its explicit rules key, its
/// name-match prefix, or — for everything unmapped — its first-level
/// package (`pkg.sub` for `pkg.sub.a.b`, the module itself at the root).
fn scope_of(module: &str, source: &RequireSource) -> String {
    match source {
        RequireSource::Explicit(key) => format!("requires entry \"{key}\""),
        RequireSource::NameMatch { prefix, .. } => prefix.clone(),
        RequireSource::Base => match module.splitn(3, '.').collect::<Vec<_>>()[..] {
            [root, first, _] | [root, first] => format!("{root}.{first}"),
            _ => module.to_string(),
        },
    }
}

/// Up to three names, with an ellipsis when more exist.
fn sample(names: &[&str]) -> String {
    let mut out = names.iter().take(3).copied().collect::<Vec<_>>().join(", ");
    if names.len() > 3 {
        out.push_str(", …");
    }
    out
}

fn describe_requires(requires: &[String]) -> String {
    if requires.is_empty() {
        "the base install".to_string()
    } else {
        format!(
            "{} {}",
            if requires.len() == 1 {
                "extra"
            } else {
                "extras"
            },
            requires.join(", "),
        )
    }
}

fn print_finding(styled: bool, severity: &str, env: &str, message: &str) {
    if styled {
        let color = if severity == "error" { "31" } else { "36" };
        println!("\x1b[1;{color}m{severity}[verify:{env}]\x1b[0m {message}");
    } else {
        println!("{severity}[verify:{env}] {message}");
    }
}
