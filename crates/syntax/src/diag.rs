//! Diagnostics with source spans, rendered rustc-style.

use std::fmt::Write;

/// Byte range in the source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start: start as u32, end: end as u32 }
    }

    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable code. `E0xxx` = syntax error, `E1xxx` = PHP feature phpx drops.
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Error, code, message: message.into(), span, help: None }
    }

    pub fn warning(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Warning, code, message: message.into(), span, help: None }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// True for "valid PHP, but phpx doesn't support it" (as opposed to a syntax error).
    pub fn is_unsupported(&self) -> bool {
        self.code.starts_with("E1")
    }

    /// Render with file location and a caret line under the offending code.
    pub fn render(&self, path: &str, src: &str) -> String {
        let (line, col) = line_col(src, self.span.start as usize);
        let line_text = src.lines().nth(line - 1).unwrap_or("");
        let kind = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let gutter = line.to_string().len();
        let width = {
            let end = (self.span.end as usize).min(src.len());
            let start = self.span.start as usize;
            let same_line = !src[start.min(end)..end].contains('\n');
            if same_line { src[start.min(end)..end].chars().count().max(1) } else { 1 }
        };
        let mut out = String::new();
        let _ = writeln!(out, "{kind}[{}]: {}", self.code, self.message);
        let _ = writeln!(out, "{:gutter$}--> {path}:{line}:{col}", "");
        let _ = writeln!(out, "{:gutter$} |", "");
        let _ = writeln!(out, "{line} | {line_text}");
        let pad: String = line_text.chars().take(col - 1).map(|c| if c == '\t' { '\t' } else { ' ' }).collect();
        let _ = writeln!(out, "{:gutter$} | {pad}{}", "", "^".repeat(width));
        if let Some(h) = &self.help {
            let _ = writeln!(out, "{:gutter$} = help: {h}", "");
        }
        out
    }
}

/// 1-based line and column (in chars) of a byte offset.
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(src.len());
    let before = &src[..floor_char_boundary(src, offset)];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, col)
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
