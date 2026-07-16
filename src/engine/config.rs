use std::fmt;
use std::path::Path;

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

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub exclude: Vec<String>,
    pub docstring_style: DocStyle,
    pub docstring_level: Level,
    pub string_annotations_level: Level,
    pub local_imports: LocalImportsConfig,
}

// ---- raw TOML shapes -------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawSweep {
    exclude: Vec<String>,
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

impl Config {
    /// Load configuration. Explicit path wins; otherwise search upward from
    /// `start_dir` for `sweep.toml` or a `pyproject.toml` with `[tool.sweep]`.
    /// A `pyproject.toml` without `[tool.sweep]` still contributes
    /// first-party package hints (project name, isort/ruff config).
    pub fn load(explicit: Option<&Path>, start_dir: &Path) -> Result<Self> {
        if let Some(path) = explicit {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading config {}", path.display()))?;
            return Self::from_toml(&text, path);
        }

        let mut dir = Some(start_dir.to_path_buf());
        while let Some(d) = dir {
            let sweep_toml = d.join("sweep.toml");
            if sweep_toml.is_file() {
                let text = std::fs::read_to_string(&sweep_toml)?;
                return Self::from_toml(&text, &sweep_toml);
            }
            let pyproject = d.join("pyproject.toml");
            if pyproject.is_file() {
                let text = std::fs::read_to_string(&pyproject)?;
                return Self::from_toml(&text, &pyproject);
            }
            dir = d.parent().map(Path::to_path_buf);
        }
        Ok(Self::default())
    }

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

        let mut config = Config {
            exclude: raw.exclude,
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
