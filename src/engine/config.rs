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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Case {
    Lower,
    Upper,
}

impl Case {
    pub fn matches(self, name: &str) -> bool {
        match self {
            Case::Lower => !name.chars().any(|c| c.is_uppercase()),
            Case::Upper => !name.chars().any(|c| c.is_lowercase()),
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Case::Lower => "lower_case",
            Case::Upper => "UPPER_CASE",
        }
    }
}

/// Config for the casing-* rules: a level plus the target case.
#[derive(Debug, Clone, Copy)]
pub struct CasingConfig {
    pub level: Level,
    pub case: Case,
}

impl Default for CasingConfig {
    fn default() -> Self {
        Self {
            level: Level::Off,
            case: Case::Lower,
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
    pub docstring_start_level: Level,
    pub string_annotations_level: Level,
    pub imports_ban_local_level: Level,
    pub known_first_party: Vec<String>,
    pub docstring_line_length_level: Level,
    pub dict_kwargs_level: Level,
    pub annotate_module_const_level: Level,
    pub casing_enum_key: CasingConfig,
    pub casing_enum_val: CasingConfig,
    pub casing_module_const: CasingConfig,
    pub no_emoji_level: Level,
    pub allowed_emojis: Vec<char>,
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
            imports_ban_local_level: Level::Error,
            known_first_party: Vec::new(),
            docstring_line_length_level: Level::Info,
            // House-style rules are opt-in.
            dict_kwargs_level: Level::Off,
            annotate_module_const_level: Level::Off,
            casing_enum_key: CasingConfig::default(),
            casing_enum_val: CasingConfig::default(),
            casing_module_const: CasingConfig::default(),
            no_emoji_level: Level::Off,
            allowed_emojis: Vec::new(),
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
    imports_ban_local: RawImportsBanLocal,
    docstring_style: RawRuleEntry,
    docstring_start: RawRuleEntry,
    string_annotations: RawRuleEntry,
    docstring_line_length: RawRuleEntry,
    dict_kwargs: RawRuleEntry,
    annotate_module_const: RawRuleEntry,
    casing_enum_key: RawCasing,
    casing_enum_val: RawCasing,
    casing_module_const: RawCasing,
    no_emoji: RawNoEmoji,
}

/// Casing rules accept `casing-enum-key = "lower"` (case shorthand,
/// which also enables the rule at warn), a bare level, or the table
/// form with `level` and `case`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawCasing {
    Token(String),
    Table(RawCasingTable),
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawCasingTable {
    level: Option<Level>,
    case: Option<Case>,
}

impl RawCasing {
    fn resolve(&self, path: &Path) -> Result<CasingConfig> {
        let defaults = CasingConfig::default();
        match self {
            RawCasing::Token(token) => match token.as_str() {
                "lower" => Ok(CasingConfig {
                    level: Level::Warn,
                    case: Case::Lower,
                }),
                "upper" => Ok(CasingConfig {
                    level: Level::Warn,
                    case: Case::Upper,
                }),
                "off" => Ok(CasingConfig {
                    level: Level::Off,
                    ..defaults
                }),
                "info" => Ok(CasingConfig {
                    level: Level::Info,
                    ..defaults
                }),
                "warn" => Ok(CasingConfig {
                    level: Level::Warn,
                    ..defaults
                }),
                "error" => Ok(CasingConfig {
                    level: Level::Error,
                    ..defaults
                }),
                other => anyhow::bail!(
                    "invalid casing value {other:?} in {} (expected lower|upper or a level)",
                    path.display()
                ),
            },
            RawCasing::Table(t) => Ok(CasingConfig {
                level: t.level.unwrap_or(Level::Warn),
                case: t.case.unwrap_or(defaults.case),
            }),
        }
    }
}

impl Default for RawCasing {
    fn default() -> Self {
        RawCasing::Table(RawCasingTable {
            level: Some(Level::Off),
            case: None,
        })
    }
}

/// no-emoji accepts a bare level (`no-emoji = "warn"`), a bare string
/// of allowed characters (`no-emoji = "→✓"`, which enables the rule at
/// warn), or the table form with `level` and `allowed`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawNoEmoji {
    Token(String),
    Table(RawNoEmojiTable),
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawNoEmojiTable {
    level: Option<Level>,
    allowed: Option<String>,
}

impl RawNoEmoji {
    fn resolve(&self) -> (Level, Vec<char>) {
        match self {
            RawNoEmoji::Token(token) => match token.as_str() {
                "off" => (Level::Off, Vec::new()),
                "info" => (Level::Info, Vec::new()),
                "warn" => (Level::Warn, Vec::new()),
                "error" => (Level::Error, Vec::new()),
                allowed => (Level::Warn, allowed.chars().collect()),
            },
            RawNoEmoji::Table(t) => (
                t.level.unwrap_or(Level::Warn),
                t.allowed.as_deref().unwrap_or("").chars().collect(),
            ),
        }
    }
}

impl Default for RawNoEmoji {
    fn default() -> Self {
        RawNoEmoji::Table(RawNoEmojiTable {
            level: Some(Level::Off),
            allowed: None,
        })
    }
}

/// A rule entry accepts either the bare-level shorthand
/// (`docstring-style = "warn"` under `[tool.sweep.rules]`) or the
/// table form (`[tool.sweep.rules.docstring-style]` / inline table).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawRuleEntry {
    Level(Level),
    Table(RawLevelOnly),
}

impl RawRuleEntry {
    fn level(&self) -> Option<Level> {
        match self {
            RawRuleEntry::Level(level) => Some(*level),
            RawRuleEntry::Table(t) => t.level,
        }
    }
}

impl Default for RawRuleEntry {
    fn default() -> Self {
        RawRuleEntry::Table(RawLevelOnly::default())
    }
}

/// imports-ban-local carries extra settings, so it accepts the bare level
/// or a table with `level` and `known-first-party`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawImportsBanLocal {
    Level(Level),
    Table(RawImportsBanLocalTable),
}

impl RawImportsBanLocal {
    fn level(&self) -> Option<Level> {
        match self {
            RawImportsBanLocal::Level(level) => Some(*level),
            RawImportsBanLocal::Table(t) => t.level,
        }
    }

    fn known_first_party(self) -> Vec<String> {
        match self {
            RawImportsBanLocal::Level(_) => Vec::new(),
            RawImportsBanLocal::Table(t) => t.known_first_party,
        }
    }
}

impl Default for RawImportsBanLocal {
    fn default() -> Self {
        RawImportsBanLocal::Table(RawImportsBanLocalTable::default())
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct RawImportsBanLocalTable {
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
                .level()
                .unwrap_or(defaults.docstring_level),
            docstring_start_level: raw
                .rules
                .docstring_start
                .level()
                .unwrap_or(defaults.docstring_start_level),
            string_annotations_level: raw
                .rules
                .string_annotations
                .level()
                .unwrap_or(defaults.string_annotations_level),
            imports_ban_local_level: raw
                .rules
                .imports_ban_local
                .level()
                .unwrap_or(defaults.imports_ban_local_level),
            known_first_party: raw.rules.imports_ban_local.known_first_party(),
            docstring_line_length_level: raw
                .rules
                .docstring_line_length
                .level()
                .unwrap_or(defaults.docstring_line_length_level),
            dict_kwargs_level: raw
                .rules
                .dict_kwargs
                .level()
                .unwrap_or(defaults.dict_kwargs_level),
            annotate_module_const_level: raw
                .rules
                .annotate_module_const
                .level()
                .unwrap_or(defaults.annotate_module_const_level),
            casing_enum_key: raw.rules.casing_enum_key.resolve(path)?,
            casing_enum_val: raw.rules.casing_enum_val.resolve(path)?,
            casing_module_const: raw.rules.casing_module_const.resolve(path)?,
            no_emoji_level: raw.rules.no_emoji.resolve().0,
            allowed_emojis: raw.rules.no_emoji.resolve().1,
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
        assert_eq!(c.imports_ban_local_level, Level::Error);
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
    fn rule_levels_accept_bare_shorthand() {
        let text = r#"
[tool.sweep.rules]
docstring-style = "warn"
imports-ban-local = "info"
string-annotations = { level = "off" }

[tool.sweep.rules.docstring-line-length]
level = "warn"
"#;
        let c = Config::from_toml(text, Path::new("pyproject.toml")).unwrap();
        assert_eq!(c.docstring_level, Level::Warn);
        assert_eq!(c.imports_ban_local_level, Level::Info);
        assert_eq!(c.string_annotations_level, Level::Off);
        assert_eq!(c.docstring_line_length_level, Level::Warn);
        assert_eq!(c.docstring_start_level, Level::Error); // untouched default
    }

    #[test]
    fn house_style_rules_default_off_and_accept_shorthand() {
        let c = Config::from_toml("", Path::new("pyproject.toml")).unwrap();
        assert_eq!(c.dict_kwargs_level, Level::Off);
        assert_eq!(c.annotate_module_const_level, Level::Off);
        assert_eq!(c.casing_enum_key.level, Level::Off);
        assert_eq!(c.no_emoji_level, Level::Off);

        let text = r#"
[tool.sweep.rules]
dict-kwargs = "warn"
casing-enum-key = "lower"
casing-module-const = { level = "error", case = "upper" }
no-emoji = "→✓"
"#;
        let c = Config::from_toml(text, Path::new("pyproject.toml")).unwrap();
        assert_eq!(c.dict_kwargs_level, Level::Warn);
        assert_eq!(c.casing_enum_key.level, Level::Warn);
        assert_eq!(c.casing_enum_key.case, Case::Lower);
        assert_eq!(c.casing_module_const.level, Level::Error);
        assert_eq!(c.casing_module_const.case, Case::Upper);
        assert_eq!(c.casing_enum_val.level, Level::Off);
        assert_eq!(c.no_emoji_level, Level::Warn);
        assert_eq!(c.allowed_emojis, vec!['→', '✓']);

        // A bare level enables the rule with no exceptions.
        let c = Config::from_toml(
            "[tool.sweep.rules]\nno-emoji = \"error\"\n",
            Path::new("pyproject.toml"),
        )
        .unwrap();
        assert_eq!(c.no_emoji_level, Level::Error);
        assert!(c.allowed_emojis.is_empty());
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

[tool.sweep.rules.imports-ban-local]
level = "info"

[tool.sweep.rules.docstring-line-length]
level = "warn"
"#;
        let c = Config::from_toml(text, Path::new("pyproject.toml")).unwrap();
        assert_eq!(c.docstring_style, DocStyle::Google);
        assert_eq!(c.imports_ban_local_level, Level::Info);
        assert_eq!(c.docstring_line_length_level, Level::Warn);
        assert!(c.known_first_party.contains(&"my_pkg".to_string()));
        assert!(c.known_first_party.contains(&"internal_lib".to_string()));
    }
}
