use std::fmt;

use crate::engine::fix::Fix;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Warning => write!(f, "warning"),
            Severity::Error => write!(f, "error"),
        }
    }
}

/// A single finding from one rule, anchored to a byte range in the source.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub rule: &'static str,
    pub message: String,
    /// Byte offset where the finding starts.
    pub start: usize,
    /// Byte offset where the finding ends (exclusive).
    pub end: usize,
    pub severity: Severity,
    pub fix: Option<Fix>,
}

impl Diagnostic {
    pub fn new(rule: &'static str, message: impl Into<String>, start: usize, end: usize) -> Self {
        Self {
            rule,
            message: message.into(),
            start,
            end,
            severity: Severity::Warning,
            fix: None,
        }
    }

    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fix = Some(fix);
        self
    }

    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }
}
