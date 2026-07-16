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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    #[default]
    Warn,
    Error,
    Off,
}

#[derive(Debug, Clone)]
pub struct LocalImportsConfig {
    pub level: Level,
    /// Whether --fix hoists local imports to the module import block.
    pub hoist: bool,
    pub known_first_party: Vec<String>,
}

impl Default for LocalImportsConfig {
    fn default() -> Self {
        Self {
            level: Level::Warn,
            hoist: true,
            known_first_party: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DocstringLineLengthConfig {
    pub level: Level,
    /// Whether --fix re-wraps docstring prose to fit the line length.
    pub rewrap: bool,
}

impl Default for DocstringLineLengthConfig {
    fn default() -> Self {
        Self {
            level: Level::Warn,
            rewrap: false,
        }
    }
}

pub const DEFAULT_LINE_LENGTH: usize = 79;

#[derive(Debug, Clone)]
pub struct Config {
    pub exclude: Vec<String>,
    pub line_length: usize,
    pub docstring_style: DocStyle,
    pub docstring_level: Level,
    pub string_annotations_level: Level,
    pub local_imports: LocalImportsConfig,
    pub docstring_line_length: DocstringLineLengthConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude: Vec::new(),
            line_length: DEFAULT_LINE_LENGTH,
            docstring_style: DocStyle::default(),
            docstring_level: Level::default(),
            string_annotations_level: Level::default(),
            local_imports: LocalImportsConfig::default(),
            docstring_line_length: DocstringLineLengthConfig::default(),
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
    string_annotations: RawLevelOnly,
    docstring_line_length: RawDocstringLineLength,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawDocstringLineLength {
    level: Option<Level>,
    fix: Option<toml::Value>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawLocalImports {
    level: Option<Level>,
    fix: Option<toml::Value>,
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

        let mut config = Config {
            exclude: raw.exclude,
            line_length: raw
                .line_length
                .or(ruff_line_length)
                .unwrap_or(DEFAULT_LINE_LENGTH),
            docstring_style: raw.python.docstring_style.unwrap_or_default(),
            docstring_level: raw.rules.docstring_style.level.unwrap_or_default(),
            string_annotations_level: raw.rules.string_annotations.level.unwrap_or_default(),
            local_imports: LocalImportsConfig {
                level: raw.rules.local_imports.level.unwrap_or_default(),
                hoist: match &raw.rules.local_imports.fix {
                    None => true,
                    Some(toml::Value::Boolean(b)) => *b,
                    Some(toml::Value::String(s)) => s == "hoist",
                    Some(_) => true,
                },
                known_first_party: raw.rules.local_imports.known_first_party,
            },
            docstring_line_length: DocstringLineLengthConfig {
                level: raw.rules.docstring_line_length.level.unwrap_or_default(),
                rewrap: match &raw.rules.docstring_line_length.fix {
                    None => false,
                    Some(toml::Value::Boolean(b)) => *b,
                    Some(toml::Value::String(s)) => s == "rewrap",
                    Some(_) => false,
                },
            },
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
            if !self.local_imports.known_first_party.contains(&normalized) {
                self.local_imports.known_first_party.push(normalized);
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
        assert!(c.local_imports.hoist);
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
        assert!(!c.docstring_line_length.rewrap);
        assert_eq!(c.docstring_line_length.level, Level::Warn);
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
level = "error"
fix = "off"
"#;
        let c = Config::from_toml(text, Path::new("pyproject.toml")).unwrap();
        assert_eq!(c.docstring_style, DocStyle::Google);
        assert_eq!(c.local_imports.level, Level::Error);
        assert!(!c.local_imports.hoist);
        assert!(
            c.local_imports
                .known_first_party
                .contains(&"my_pkg".to_string())
        );
        assert!(
            c.local_imports
                .known_first_party
                .contains(&"internal_lib".to_string())
        );
    }
}
