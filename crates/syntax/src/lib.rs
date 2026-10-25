//! phpx front end: tokens, syntax tree, and the subset rules.
//!
//! Every file phpx accepts is also valid PHP 8.x. Features phpx drops are
//! reported as `E1xxx` diagnostics with a suggested replacement.

pub mod ast;
pub mod checks;
pub mod diag;
pub mod lexer;
pub mod parser;

pub use diag::{Diagnostic, Severity, Span};

/// Parse and run the subset checks.
pub fn check(src: &str) -> (ast::File, Vec<Diagnostic>) {
    let (file, mut diags) = parser::parse(src);
    diags.extend(checks::check(&file));
    diags.sort_by_key(|d| d.span.start);
    (file, diags)
}
