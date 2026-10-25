//! phpx front end: tokens, syntax tree, and the subset rules.
//!
//! Every file phpx accepts is also valid PHP 8.x. Features phpx drops are
//! reported as `E1xxx` diagnostics with a suggested replacement.

pub mod ast;
pub mod checks;
pub mod diag;
pub mod lexer;
pub mod parser;
pub mod stack;

pub use diag::{Diagnostic, Severity, Span};

/// Parse and run the subset checks.
pub fn check(src: &str) -> (ast::File, Vec<Diagnostic>) {
    stack::with_default_budget(|| {
        let (mut file, mut diags) = parser::parse(src);
        diags.extend(checks::check(&file));
        if stack::take_exhausted() {
            diags.push(Diagnostic::error("E0003", Span { start: 0, end: 0 }, stack::TOO_DEEP));
            // Dropping a tree this deep would recurse past the stack too:
            // leak it (the file failed to compile anyway).
            std::mem::forget(std::mem::replace(&mut file, ast::File { stmts: vec![] }));
        }
        diags.sort_by_key(|d| d.span.start);
        (file, diags)
    })
}
