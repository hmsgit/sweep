use std::path::Path;

use tree_sitter::Node;

use crate::engine::config::{Config, ImportsRequiredExtrasConfig, ProjectDeps, normalize_flat};
use crate::engine::context::FileContext;
use crate::engine::diagnostic::Diagnostic;
use crate::engine::rule::Rule;
use crate::langs::python::stdlib::is_stdlib;

/// Flags module-level imports a partial install cannot satisfy: a
/// module importable with the base distribution (or a declared set of
/// extras) must not eagerly import a dependency that only ships in
/// extras it doesn't require. Checking every file's own eager imports —
/// first-party ones included — makes the whole import closure safe by
/// induction, so no import graph is needed.
///
/// Imports under `if TYPE_CHECKING:`, `try:` or any other guard are
/// not eager; imports inside functions are the sanctioned escape hatch
/// (`# sweep: deferred-import`).
pub struct ImportsRequiredExtras;

impl Rule for ImportsRequiredExtras {
    fn name(&self) -> &'static str {
        "imports-required-extras"
    }

    fn explain(&self) -> &'static str {
        "modules must not eagerly import dependencies from extras they don't require"
    }

    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic> {
        let rule_config = &ctx.config.imports_required_extras;
        let deps = &ctx.config.project_deps;
        let Some(severity) = rule_config.level.severity() else {
            return Vec::new();
        };
        if deps.extras.is_empty() || deps.package_roots.is_empty() {
            return Vec::new();
        }
        let Some(module) = module_path(ctx.path, ctx.config, deps) else {
            return Vec::new();
        };
        let is_package = ctx
            .path
            .file_name()
            .is_some_and(|name| name == "__init__.py");
        let allowed = required_extras(&module.dotted, rule_config, deps);

        let mut diagnostics = Vec::new();
        let mut cursor = ctx.root().walk();
        for statement in ctx.root().children(&mut cursor) {
            if !matches!(
                statement.kind(),
                "import_statement" | "import_from_statement"
            ) {
                continue;
            }
            let Some(violation) = check_import(
                statement,
                ctx.source,
                &module,
                is_package,
                &allowed,
                rule_config,
                deps,
            ) else {
                continue;
            };
            diagnostics.push(
                Diagnostic::new(
                    self.name(),
                    violation.message(&module.dotted, &allowed),
                    statement.start_byte(),
                    statement.end_byte(),
                )
                .with_severity(severity),
            );
        }
        diagnostics
    }
}

/// The checked file as a dotted module path, split into its package
/// root and the remainder. None for files outside any shipped package
/// (tests, scripts) or outside the config directory.
struct ModulePath {
    dotted: String,
}

fn module_path(path: &Path, config: &Config, deps: &ProjectDeps) -> Option<ModulePath> {
    let config_dir = config.config_dir.as_ref()?;
    let abs = path
        .canonicalize()
        .unwrap_or_else(|_| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()));
    let rel = abs.strip_prefix(config_dir).ok()?;

    let mut segments: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let last = segments.pop()?;
    let stem = last.strip_suffix(".py")?;
    if stem != "__init__" {
        segments.push(stem.to_string());
    }
    let root = segments.first()?;
    if !deps.package_roots.contains(root) {
        return None;
    }
    Some(ModulePath {
        dotted: segments.join("."),
    })
}

/// Extras a module (given as a dotted path rooted at a package root)
/// requires: the longest explicit `requires` prefix wins; unmapped
/// modules fall back to the name-match convention on their first-level
/// subpackage; everything else is base.
pub(crate) fn required_extras(
    dotted: &str,
    config: &ImportsRequiredExtrasConfig,
    deps: &ProjectDeps,
) -> Vec<String> {
    // requires is sorted longest-key-first at parse time.
    for (key, extras) in &config.requires {
        let matched = if let Some(relative) = key.strip_prefix('.') {
            dotted.split_once('.').is_some_and(|(root, rest)| {
                deps.package_roots.iter().any(|r| r == root) && prefix_matches(relative, rest)
            })
        } else {
            prefix_matches(key, dotted)
        };
        if matched {
            return extras.clone();
        }
    }

    if config.match_by_name
        && let Some(first_level) = dotted.split('.').nth(1)
    {
        let flat = normalize_flat(first_level);
        for (extra, _) in &deps.extras {
            if normalize_flat(extra) == flat {
                return vec![extra.clone()];
            }
        }
    }
    Vec::new()
}

/// True when `prefix` equals `dotted` or is a dot-boundary prefix of it.
fn prefix_matches(prefix: &str, dotted: &str) -> bool {
    dotted == prefix
        || (dotted.len() > prefix.len()
            && dotted.starts_with(prefix)
            && dotted.as_bytes()[prefix.len()] == b'.')
}

enum Violation {
    /// A third-party dependency that only ships in these extras.
    ThirdParty {
        import: String,
        ships_in: Vec<String>,
    },
    /// A first-party module that requires these extras.
    FirstParty {
        import: String,
        requires: Vec<String>,
    },
}

impl Violation {
    fn message(&self, module: &str, allowed: &[String]) -> String {
        let guarantee = if allowed.is_empty() {
            format!("`{module}` must import with the base install")
        } else {
            format!("`{module}` may only assume {}", extras_phrase(allowed))
        };
        match self {
            Violation::ThirdParty { import, ships_in } => format!(
                "`{import}` only ships in {}, but {guarantee}; \
                 defer the import into the using function (# sweep: deferred-import) \
                 or extend rules.imports-required-extras.requires",
                extras_phrase(ships_in),
            ),
            Violation::FirstParty { import, requires } => format!(
                "`{import}` requires {}, but {guarantee}; \
                 defer the import into the using function (# sweep: deferred-import) \
                 or extend rules.imports-required-extras.requires",
                extras_phrase(requires),
            ),
        }
    }
}

/// "extra `llm`" or "extras `llm`, `pgsql`".
fn extras_phrase(names: &[String]) -> String {
    format!(
        "{} {}",
        if names.len() == 1 { "extra" } else { "extras" },
        names
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// The first unsatisfiable import target in a module-level import
/// statement, if any.
fn check_import(
    statement: Node,
    source: &str,
    module: &ModulePath,
    is_package: bool,
    allowed: &[String],
    config: &ImportsRequiredExtrasConfig,
    deps: &ProjectDeps,
) -> Option<Violation> {
    for target in import_targets(statement, source, &module.dotted, is_package) {
        let root = target.split('.').next().unwrap_or(&target);
        if is_stdlib(root) {
            continue;
        }
        if deps.package_roots.iter().any(|r| r == root) {
            let requires = required_extras(&target, config, deps);
            if !requires.iter().all(|extra| allowed.contains(extra)) {
                return Some(Violation::FirstParty {
                    import: target,
                    requires,
                });
            }
            continue;
        }
        // Third-party: fine if a base dependency provides it; otherwise
        // some allowed extra must. Imports matching no declared
        // dependency (transitive deps, stubs, bare namespace roots like
        // `import google` under a `google.genai` dep) are not this
        // rule's call.
        if deps.base.iter().any(|dep| prefix_matches(dep, &target)) {
            continue;
        }
        let ships_in: Vec<String> = deps
            .extras
            .iter()
            .filter(|(_, paths)| paths.iter().any(|dep| prefix_matches(dep, &target)))
            .map(|(extra, _)| extra.clone())
            .collect();
        if ships_in.is_empty() || ships_in.iter().any(|extra| allowed.contains(extra)) {
            continue;
        }
        return Some(Violation::ThirdParty {
            import: target,
            ships_in,
        });
    }
    None
}

/// Dotted paths an import statement loads eagerly. From-imports extend
/// the module path with each imported name so `from pkg import mcp`
/// resolves like `pkg.mcp`; relative imports resolve against the
/// current module.
fn import_targets(statement: Node, source: &str, module: &str, is_package: bool) -> Vec<String> {
    let mut targets = Vec::new();
    match statement.kind() {
        "import_statement" => {
            let mut cursor = statement.walk();
            for child in statement.named_children(&mut cursor) {
                let name = match child.kind() {
                    "dotted_name" => child,
                    "aliased_import" => match child.child_by_field_name("name") {
                        Some(name) => name,
                        None => continue,
                    },
                    _ => continue,
                };
                targets.push(source[name.byte_range()].to_string());
            }
        }
        "import_from_statement" => {
            let Some(module_node) = statement.child_by_field_name("module_name") else {
                return targets;
            };
            let module_text = &source[module_node.byte_range()];
            let base = if module_node.kind() == "relative_import" {
                match resolve_relative(module_text, module, is_package) {
                    Some(base) => base,
                    None => return targets,
                }
            } else {
                module_text.to_string()
            };

            let mut names = Vec::new();
            let mut cursor = statement.walk();
            for child in statement.named_children(&mut cursor) {
                if child.id() == module_node.id() {
                    continue;
                }
                let name = match child.kind() {
                    "dotted_name" => child,
                    "aliased_import" => match child.child_by_field_name("name") {
                        Some(name) => name,
                        None => continue,
                    },
                    "wildcard_import" => continue,
                    _ => continue,
                };
                names.push(source[name.byte_range()].to_string());
            }
            if names.is_empty() {
                targets.push(base);
            } else {
                for name in names {
                    targets.push(format!("{base}.{name}"));
                }
            }
        }
        _ => {}
    }
    targets
}

/// Resolve `from .x`/`from ..` module text against the current module.
/// None when the dots climb above the package root.
fn resolve_relative(text: &str, module: &str, is_package: bool) -> Option<String> {
    let dots = text.chars().take_while(|c| *c == '.').count();
    let rest = &text[dots..];

    let mut segments: Vec<&str> = module.split('.').collect();
    // One dot means the containing package: the module itself for an
    // __init__.py, its parent otherwise.
    let climb = if is_package { dots - 1 } else { dots };
    if climb >= segments.len() {
        return None;
    }
    segments.truncate(segments.len() - climb);
    if !rest.is_empty() {
        segments.extend(rest.split('.'));
    }
    Some(segments.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::config::Level;
    use crate::engine::source::LineIndex;
    use crate::langs::python::parser;

    fn test_config() -> Config {
        // Longest requires key first, matching the parse-time sort.
        let mut config = Config {
            config_dir: Some(std::path::PathBuf::from("/repo")),
            imports_required_extras: ImportsRequiredExtrasConfig {
                level: Level::Error,
                match_by_name: true,
                requires: vec![
                    (
                        ".io.airtable".to_string(),
                        vec!["airtable".to_string(), "datascience".to_string()],
                    ),
                    (".api".to_string(), vec!["fastapi".to_string()]),
                    (".llm2".to_string(), vec!["llm".to_string()]),
                    (".mcp".to_string(), vec!["llm".to_string()]),
                ],
                import_names: vec![("google-genai".to_string(), vec!["google.genai".to_string()])],
            },
            project_deps: ProjectDeps {
                dist_name: Some("cobrainer".to_string()),
                base: vec!["boto3".to_string(), "pydantic".to_string()],
                extras: vec![
                    ("fastapi".to_string(), vec!["fastapi".to_string()]),
                    (
                        "llm".to_string(),
                        vec![
                            "fastmcp".to_string(),
                            "litellm".to_string(),
                            "redis".to_string(),
                        ],
                    ),
                    (
                        "datascience".to_string(),
                        vec!["pandas".to_string(), "bidict".to_string()],
                    ),
                    ("airtable".to_string(), vec!["pyairtable".to_string()]),
                    ("polyglot".to_string(), vec!["deepl".to_string()]),
                    ("loadtest".to_string(), vec!["locust".to_string()]),
                ],
                package_roots: vec!["cobrainer".to_string()],
            },
            ..Config::default()
        };
        config
            .imports_required_extras
            .requires
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        config
    }

    fn check_at(path: &str, source: &str, config: &Config) -> Vec<String> {
        let tree = parser().parse(source, None).unwrap();
        let line_index = LineIndex::new(source);
        let ctx = FileContext {
            path: Path::new(path),
            source,
            tree: &tree,
            config,
            line_index: &line_index,
        };
        ImportsRequiredExtras
            .check(&ctx)
            .into_iter()
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn base_module_cannot_eagerly_import_extra_deps() {
        let config = test_config();
        let findings = check_at(
            "/repo/cobrainer/types/locale.py",
            "import pycountry\nfrom fastmcp.tools.base import Tool\n",
            &config,
        );
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].contains("`fastmcp.tools.base.Tool`"),
            "{findings:?}"
        );
        assert!(findings[0].contains("extra `llm`"), "{findings:?}");
        assert!(findings[0].contains("base install"), "{findings:?}");
    }

    #[test]
    fn base_deps_stdlib_and_unknown_imports_pass() {
        let config = test_config();
        let findings = check_at(
            "/repo/cobrainer/aws/sns.py",
            "import os\nimport boto3\nfrom pydantic import BaseModel\n\
             from mypy_boto3_sns.client import SNSClient\n",
            &config,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn mapped_subpackage_may_import_its_extras() {
        let config = test_config();
        let findings = check_at(
            "/repo/cobrainer/mcp/server.py",
            "from fastmcp import FastMCP\nimport redis\n",
            &config,
        );
        assert!(findings.is_empty(), "{findings:?}");

        // But not deps from unrelated extras.
        let findings = check_at("/repo/cobrainer/mcp/server.py", "import pandas\n", &config);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("`datascience`"), "{findings:?}");
        assert!(
            findings[0].contains("may only assume extra `llm`"),
            "{findings:?}"
        );
    }

    #[test]
    fn first_party_boundaries_are_enforced() {
        let config = test_config();
        // Base module importing an extra-mapped subpackage.
        let findings = check_at(
            "/repo/cobrainer/types/locale.py",
            "from cobrainer.mcp.server import Server\n",
            &config,
        );
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].contains("`cobrainer.mcp.server.Server`"),
            "{findings:?}"
        );

        // from-import of a subpackage off the root resolves like the
        // subpackage itself.
        let findings = check_at(
            "/repo/cobrainer/types/locale.py",
            "from cobrainer import mcp\n",
            &config,
        );
        assert_eq!(findings.len(), 1);

        // Importing base first-party from anywhere is fine.
        let findings = check_at(
            "/repo/cobrainer/mcp/server.py",
            "from cobrainer.types import Uuid\nfrom cobrainer import logging\n",
            &config,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn name_match_convention_applies_to_unmapped_subpackages() {
        let config = test_config();
        // .polyglot is unmapped; extra "polyglot" matches by name.
        let findings = check_at(
            "/repo/cobrainer/polyglot/translate.py",
            "import deepl\n",
            &config,
        );
        assert!(findings.is_empty(), "{findings:?}");

        // Separators are ignored: .load_test matches extra "loadtest".
        let findings = check_at(
            "/repo/cobrainer/load_test/run.py",
            "import locust\n",
            &config,
        );
        assert!(findings.is_empty(), "{findings:?}");

        // Base module importing the name-matched subpackage is flagged.
        let findings = check_at(
            "/repo/cobrainer/utils.py",
            "from cobrainer.polyglot import translate\n",
            &config,
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("`polyglot`"), "{findings:?}");

        let mut no_convention = test_config();
        no_convention.imports_required_extras.match_by_name = false;
        let findings = check_at(
            "/repo/cobrainer/polyglot/translate.py",
            "import deepl\n",
            &no_convention,
        );
        assert_eq!(findings.len(), 1, "{findings:?}");
    }

    #[test]
    fn multi_extra_mappings_require_all() {
        let config = test_config();
        let findings = check_at(
            "/repo/cobrainer/io/airtable.py",
            "import pandas\nimport pyairtable\n",
            &config,
        );
        assert!(findings.is_empty(), "{findings:?}");

        // A base module importing it must satisfy both extras.
        let findings = check_at(
            "/repo/cobrainer/utils.py",
            "from cobrainer.io.airtable import read_table\n",
            &config,
        );
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].contains("`airtable`, `datascience`"),
            "{findings:?}"
        );
    }

    #[test]
    fn guarded_and_function_local_imports_are_not_eager() {
        let config = test_config();
        let findings = check_at(
            "/repo/cobrainer/types/locale.py",
            "from typing import TYPE_CHECKING\n\
             if TYPE_CHECKING:\n    from fastmcp import FastMCP\n\
             try:\n    import litellm\nexcept ImportError:\n    litellm = None\n\
             def use():\n    from fastmcp.tools.base import Tool\n    return Tool\n",
            &config,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn relative_imports_resolve_against_the_module() {
        let config = test_config();
        // cobrainer/__init__.py pulling the llm-only subpackage eagerly.
        let findings = check_at(
            "/repo/cobrainer/__init__.py",
            "from . import mcp\n",
            &config,
        );
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("`cobrainer.mcp`"), "{findings:?}");

        // Sibling import within the same subpackage is fine.
        let findings = check_at(
            "/repo/cobrainer/mcp/server.py",
            "from .middleware import Middleware\nfrom . import utils\n",
            &config,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn files_outside_shipped_packages_are_skipped() {
        let config = test_config();
        let findings = check_at(
            "/repo/tests/test_locale.py",
            "import fastmcp\nimport pandas\n",
            &config,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn namespace_package_deps_match_dotted_import_paths() {
        let mut config = test_config();
        config
            .project_deps
            .extras
            .push(("vertex".to_string(), vec!["google.genai".to_string()]));
        let findings = check_at(
            "/repo/cobrainer/utils.py",
            "from google.genai.types import JobState\n",
            &config,
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("`vertex`"), "{findings:?}");

        // A bare namespace root matches no declared dep and is skipped.
        let findings = check_at("/repo/cobrainer/utils.py", "import google\n", &config);
        assert!(findings.is_empty(), "{findings:?}");
    }
}
