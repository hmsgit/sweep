use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DocStyle {
    #[default]
    Rest,
    Google,
    Numpy,
}

impl fmt::Display for DocStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocStyle::Rest => write!(f, "reST"),
            DocStyle::Google => write!(f, "Google"),
            DocStyle::Numpy => write!(f, "NumPy"),
        }
    }
}

/// The single per-rule knob. Severity decides everything: whether the
/// finding is shown, whether --fix rewrites it, and whether it fails
/// the run.
///
/// | level | shown | --fix rewrites | fails the run |
/// |-------|-------|----------------|---------------|
/// | off   | no    | no             | no            |
/// | info  | yes   | no             | no            |
/// | warn  | yes   | yes            | only with --strict |
/// | error | yes   | yes            | yes           |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Off,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn severity(self) -> Option<crate::engine::diagnostic::Severity> {
        use crate::engine::diagnostic::Severity;
        match self {
            Level::Off => None,
            Level::Info => Some(Severity::Info),
            Level::Warn => Some(Severity::Warning),
            Level::Error => Some(Severity::Error),
        }
    }

    /// info-level findings are purely informational; only warn and
    /// error get rewritten under --fix.
    pub fn applies_fixes(self) -> bool {
        matches!(self, Level::Warn | Level::Error)
    }
}

pub const DEFAULT_LINE_LENGTH: usize = 79;

#[derive(Debug, Clone)]
pub struct Config {
    pub exclude: Vec<String>,
    pub line_length: usize,
    pub docstring_style: DocStyle,
    pub docstring_level: Level,
    pub docstring_start_level: Level,
    pub string_annotations_level: Level,
    pub local_imports_level: Level,
    pub known_first_party: Vec<String>,
    pub docstring_line_length_level: Level,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude: Vec::new(),
            line_length: DEFAULT_LINE_LENGTH,
            docstring_style: DocStyle::default(),
            docstring_level: Level::Error,
            docstring_start_level: Level::Error,
            string_annotations_level: Level::Error,
            local_imports_level: Level::Error,
            known_first_party: Vec::new(),
            docstring_line_length_level: Level::Info,
        }
    }
}

// ---- raw TOML shapes -------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawSweep {
    exclude: Vec<String>,
    line_length: Option<usize>,
    python: RawPython,
    rules: RawRules,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawPython {
    docstring_style: Option<DocStyle>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawRules {
    local_imports: RawLocalImports,
    docstring_style: RawLevelOnly,
    docstring_start: RawLevelOnly,
    string_annotations: RawLevelOnly,
    docstring_line_length: RawLevelOnly,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawLocalImports {
    level: Option<Level>,
    known_first_party: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawLevelOnly {
    level: Option<Level>,
}

/// Resolves the effective config per checked file: the nearest
/// `sweep.toml` or `pyproject.toml` in the file's parent directories
/// wins, so a monorepo with pre-commit at the root and one
/// `app/*/pyproject.toml` per app gets each file checked against its
/// own app's config. An explicit `--config` path overrides discovery
/// for every file. Lookups are memoized per directory.
pub struct ConfigResolver {
    explicit: Option<Arc<Config>>,
    cache: Mutex<HashMap<PathBuf, Arc<Config>>>,
    fallback: Arc<Config>,
}

impl ConfigResolver {
    pub fn new(explicit: Option<&Path>) -> Result<Self> {
        let explicit = match explicit {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("reading config {}", path.display()))?;
                Some(Arc::new(Config::from_toml(&text, path)?))
            }
            None => None,
        };
        Ok(Self {
            explicit,
            cache: Mutex::new(HashMap::new()),
            fallback: Arc::new(Config::default()),
        })
    }

    /// Effective config for a file or directory path.
    pub fn for_path(&self, path: &Path) -> Result<Arc<Config>> {
        if let Some(config) = &self.explicit {
            return Ok(config.clone());
        }

        let abs = path
            .canonicalize()
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default().join(path));
        let start = if abs.is_dir() {
            abs.as_path()
        } else {
            abs.parent().unwrap_or(abs.as_path())
        };

        // Walk upward until a config file or a cached directory is found.
        let mut visited: Vec<PathBuf> = Vec::new();
        let mut found: Option<Arc<Config>> = None;
        for dir in start.ancestors() {
            if let Some(hit) = self.cache.lock().unwrap().get(dir) {
                found = Some(hit.clone());
                break;
            }
            visited.push(dir.to_path_buf());

            let mut config_file = None;
            let sweep_toml = dir.join("sweep.toml");
            let pyproject = dir.join("pyproject.toml");
            if sweep_toml.is_file() {
                config_file = Some(sweep_toml);
            } else if pyproject.is_file() {
                config_file = Some(pyproject);
            }
            if let Some(file) = config_file {
                let text = std::fs::read_to_string(&file)
                    .with_context(|| format!("reading config {}", file.display()))?;
                found = Some(Arc::new(Config::from_toml(&text, &file)?));
                break;
            }
        }

        let config = found.unwrap_or_else(|| self.fallback.clone());
        let mut cache = self.cache.lock().unwrap();
        for dir in visited {
            cache.insert(dir, config.clone());
        }
        Ok(config)
    }
}

impl Config {
    fn from_toml(text: &str, path: &Path) -> Result<Self> {
        let doc = toml::Value::Table(
            text.parse::<toml::Table>()
                .with_context(|| format!("parsing {}", path.display()))?,
        );

        let is_pyproject = path.file_name().is_some_and(|n| n == "pyproject.toml");
        let sweep_table = if is_pyproject {
            doc.get("tool").and_then(|t| t.get("sweep"))
        } else {
            Some(&doc)
        };

        let raw: RawSweep = match sweep_table {
            Some(v) => v
                .clone()
                .try_into()
                .with_context(|| format!("invalid [tool.sweep] config in {}", path.display()))?,
            None => RawSweep::default(),
        };

        // Line length: sweep's own setting wins, then ruff's, then 79.
        let ruff_line_length = doc
            .get("tool")
            .and_then(|t| t.get("ruff"))
            .and_then(|r| r.get("line-length"))
            .and_then(|v| v.as_integer())
            .and_then(|n| usize::try_from(n).ok());

        let defaults = Config::default();
        let mut config = Config {
            exclude: raw.exclude,
            line_length: raw
                .line_length
                .or(ruff_line_length)
                .unwrap_or(DEFAULT_LINE_LENGTH),
            docstring_style: raw.python.docstring_style.unwrap_or_default(),
            docstring_level: raw
                .rules
                .docstring_style
                .level
                .unwrap_or(defaults.docstring_level),
            docstring_start_level: raw
                .rules
                .docstring_start
                .level
                .unwrap_or(defaults.docstring_start_level),
            string_annotations_level: raw
                .rules
                .string_annotations
                .level
                .unwrap_or(defaults.string_annotations_level),
            local_imports_level: raw
                .rules
                .local_imports
                .level
                .unwrap_or(defaults.local_imports_level),
            known_first_party: raw.rules.local_imports.known_first_party,
            docstring_line_length_level: raw
                .rules
                .docstring_line_length
                .level
                .unwrap_or(defaults.docstring_line_length_level),
        };

        if is_pyproject {
            config.absorb_first_party_hints(&doc);
        }
        Ok(config)
    }

    /// Pull first-party package names from [project], [tool.poetry],
    /// [tool.ruff.lint.isort] and [tool.isort] so hoisting can place
    /// imports in the right section without sweep-specific config.
    fn absorb_first_party_hints(&mut self, doc: &toml::Value) {
        let mut add = |name: &str| {
            let normalized = name.replace('-', "_");
            if !self.known_first_party.contains(&normalized) {
                self.known_first_party.push(normalized);
            }
        };

        for path in [&["project", "name"][..], &["tool", "poetry", "name"][..]] {
            let mut v = doc;
            let mut found = true;
            for key in path {
                match v.get(key) {
                    Some(next) => v = next,
                    None => {
                        found = false;
                        break;
                    }
                }
            }
            if found && let Some(name) = v.as_str() {
                add(name);
            }
        }

        for path in [
            &["tool", "ruff", "lint", "isort", "known-first-party"][..],
            &["tool", "isort", "known_first_party"][..],
        ] {
            let mut v = doc;
            let mut found = true;
            for key in path {
                match v.get(key) {
                    Some(next) => v = next,
                    None => {
                        found = false;
                        break;
                    }
                }
            }
            if found && let Some(items) = v.as_array() {
                for item in items {
                    if let Some(name) = item.as_str() {
                        add(name);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.docstring_style, DocStyle::Rest);
        assert_eq!(c.local_imports_level, Level::Error);
        assert_eq!(c.docstring_level, Level::Error);
        assert_eq!(c.string_annotations_level, Level::Error);
        assert_eq!(c.docstring_line_length_level, Level::Info);
        assert!(!Level::Info.applies_fixes());
        assert!(Level::Warn.applies_fixes());
        assert!(Level::Error.applies_fixes());
    }

    #[test]
    fn line_length_falls_back_to_ruff_then_default() {
        let with_ruff = "[tool.ruff]\nline-length = 100\n";
        let c = Config::from_toml(with_ruff, Path::new("pyproject.toml")).unwrap();
        assert_eq!(c.line_length, 100);

        let own_wins = "[tool.ruff]\nline-length = 100\n\n[tool.sweep]\nline-length = 120\n";
        let c = Config::from_toml(own_wins, Path::new("pyproject.toml")).unwrap();
        assert_eq!(c.line_length, 120);

        let c = Config::from_toml("", Path::new("pyproject.toml")).unwrap();
        assert_eq!(c.line_length, DEFAULT_LINE_LENGTH);
        assert_eq!(c.docstring_line_length_level, Level::Info);
    }

    #[test]
    fn parses_pyproject() {
        let text = r#"
[project]
name = "my-pkg"

[tool.ruff.lint.isort]
known-first-party = ["internal_lib"]

[tool.sweep.python]
docstring-style = "google"

[tool.sweep.rules.local-imports]
level = "info"

[tool.sweep.rules.docstring-line-length]
level = "warn"
"#;
        let c = Config::from_toml(text, Path::new("pyproject.toml")).unwrap();
        assert_eq!(c.docstring_style, DocStyle::Google);
        assert_eq!(c.local_imports_level, Level::Info);
        assert_eq!(c.docstring_line_length_level, Level::Warn);
        assert!(c.known_first_party.contains(&"my_pkg".to_string()));
        assert!(c.known_first_party.contains(&"internal_lib".to_string()));
    }
}
