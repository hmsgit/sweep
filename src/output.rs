//! Ruff-style terminal output: a `path:line:col: severity[rule] message`
//! header, the offending source line with a caret underline, colors on
//! TTYs, and OSC 8 hyperlinks on the location for terminals that render
//! them (iTerm2, WezTerm, kitty, VS Code, …).

use std::io::IsTerminal;
use std::path::Path;

use clap::ValueEnum;

use crate::engine::diagnostic::Severity;
use crate::engine::runner::RenderedDiagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// One block per finding: header plus the source line with carets.
    Full,
    /// One line per finding.
    Concise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TermMode {
    /// Colors when stdout is a terminal, hyperlinks when it supports them.
    Auto,
    /// No escape sequences at all.
    Plain,
    /// Force colors, no hyperlinks.
    Color,
    /// Force colors and hyperlinks.
    Hyper,
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const CYAN: &str = "\x1b[36m";

pub struct Formatter {
    color: bool,
    links: bool,
    format: OutputFormat,
}

impl Formatter {
    pub fn new(mode: TermMode, format: OutputFormat) -> Self {
        let (color, links) = match mode {
            TermMode::Plain => (false, false),
            TermMode::Color => (true, false),
            TermMode::Hyper => (true, true),
            TermMode::Auto => {
                let tty = std::io::stdout().is_terminal();
                let dumb = std::env::var("TERM").is_ok_and(|t| t == "dumb");
                let color = tty && !dumb && std::env::var_os("NO_COLOR").is_none();
                (color, color && supports_hyperlinks())
            }
        };
        Self {
            color,
            links,
            format,
        }
    }

    fn paint(&self, style: &str, text: &str) -> String {
        if self.color {
            format!("{style}{text}{RESET}")
        } else {
            text.to_string()
        }
    }

    fn severity_color(severity: Severity) -> &'static str {
        match severity {
            Severity::Info => CYAN,
            Severity::Warning => YELLOW,
            Severity::Error => RED,
        }
    }

    /// Wrap `text` in an OSC 8 hyperlink to the file.
    fn link(&self, path: &Path, text: &str) -> String {
        if !self.links {
            return text.to_string();
        }
        let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        format!(
            "\x1b]8;;file://{}\x1b\\{text}\x1b]8;;\x1b\\",
            target.display()
        )
    }

    pub fn print_diagnostic(&self, path: &Path, d: &RenderedDiagnostic) {
        let location = format!("{}:{}:{}", path.display(), d.line, d.col);
        let location = self.link(path, &self.paint(BOLD, &location));
        let sev_color = Self::severity_color(d.severity);
        let label = self.paint(
            &format!("{BOLD}{sev_color}"),
            &format!("{}[{}]", d.severity, d.rule),
        );
        let fix_marker = if d.fixable {
            format!(" {}", self.paint(DIM, "[*]"))
        } else {
            String::new()
        };
        println!("{location}: {label} {}{fix_marker}", d.message);

        if self.format == OutputFormat::Concise {
            return;
        }

        let gutter_width = d.line.to_string().len();
        let bar = self.paint(BLUE, "|");
        let line_no = self.paint(BLUE, &format!("{:>gutter_width$}", d.line));
        let pad = " ".repeat(gutter_width);
        let carets = self.paint(sev_color, &"^".repeat(d.snippet.underline_len));
        println!("{pad} {bar}");
        println!("{line_no} {bar} {}", d.snippet.text);
        println!(
            "{pad} {bar} {}{carets}",
            " ".repeat(d.snippet.underline_start)
        );
        println!("{pad} {bar}");
        println!();
    }

    pub fn print_summary(
        &self,
        files: usize,
        counts: [usize; 3], // info, warning, error
        fixed: usize,
        fixable: usize,
        fix_mode: bool,
    ) {
        let [infos, warnings, errors] = counts;
        let remaining = infos + warnings + errors;
        match (remaining, fixed) {
            (0, 0) => println!("All clean ({files} files)."),
            (0, _) => println!("Fixed {fixed} issue(s); all clean ({files} files)."),
            _ => {
                let breakdown: Vec<String> = [
                    (errors, "error(s)", RED),
                    (warnings, "warning(s)", YELLOW),
                    (infos, "info", CYAN),
                ]
                .iter()
                .filter(|(n, _, _)| *n > 0)
                .map(|(n, label, color)| self.paint(color, &format!("{n} {label}")))
                .collect();
                let mut summary = format!("Found {remaining} issue(s) ({})", breakdown.join(", "));
                if fixed > 0 {
                    summary.push_str(&format!(", {fixed} fixed"));
                }
                summary.push('.');
                println!("{summary}");
                if !fix_mode && fixable > 0 {
                    println!(
                        "{} {fixable} fixable with the `--fix` option.",
                        self.paint(DIM, "[*]")
                    );
                }
            }
        }
    }
}

/// Terminals known to render OSC 8 hyperlinks.
fn supports_hyperlinks() -> bool {
    if let Ok(program) = std::env::var("TERM_PROGRAM")
        && matches!(
            program.as_str(),
            "iTerm.app" | "WezTerm" | "vscode" | "Hyper" | "ghostty" | "Tabby"
        )
    {
        return true;
    }
    if std::env::var("TERM").is_ok_and(|t| t.contains("kitty") || t.contains("foot")) {
        return true;
    }
    std::env::var_os("VTE_VERSION").is_some() || std::env::var_os("KONSOLE_VERSION").is_some()
}
