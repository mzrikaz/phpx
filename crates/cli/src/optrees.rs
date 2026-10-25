//! Dev tool: `phpx optrees < file-list` prints every operator tree per file in
//! the same format as tools/oracle/optrees.php (nikic/php-parser), so a diff
//! shows precedence/associativity bugs. Files with errors are marked `!!`.

use std::collections::HashSet;
use std::io::BufRead;
use std::process::ExitCode;

use phpx_syntax::ast::{BinOp, Expr, ExprKind, UnOp};
use phpx_syntax::checks::walk_stmts;

fn binop(op: BinOp) -> &'static str {
    use BinOp::*;
    match op {
        Add => "+", Sub => "-", Mul => "*", Div => "/", Mod => "%", Pow => "**", Concat => ".",
        BitAnd => "&", BitOr => "|", BitXor => "^", Shl => "<<", Shr => ">>",
        And => "and", Or => "or", Xor => "xor",
        Eq => "==", NotEq => "!=", Identical => "===", NotIdentical => "!==",
        Lt => "<", LtEq => "<=", Gt => ">", GtEq => ">=", Spaceship => "<=>", Coalesce => "??",
    }
}

fn op(e: &Expr) -> Option<(String, Vec<&Expr>)> {
    use ExprKind::*;
    Some(match &e.kind {
        Binary { op, left, right } => (binop(*op).into(), vec![left, right]),
        AssignOp { op, target, value } => (format!("{}=", binop(*op)), vec![target, value]),
        Assign { target, value } | AssignRef { target, value } => ("=".into(), vec![target, value]),
        Unary { op, expr } => {
            let l = match op {
                UnOp::Not => "!",
                UnOp::Neg => "neg",
                UnOp::Plus => "plus",
                UnOp::BitNot => "~",
                UnOp::Silence => "@",
            };
            (l.into(), vec![expr])
        }
        Cast { expr, .. } => ("cast".into(), vec![expr]),
        IncDec { expr, .. } => ("incdec".into(), vec![expr]),
        Clone(x) => ("clone".into(), vec![x]),
        Print(x) => ("print".into(), vec![x]),
        Throw(x) => ("throw".into(), vec![x]),
        Instanceof { expr, .. } => ("instanceof".into(), vec![expr]),
        Ternary { cond, then: None, else_ } => ("?:".into(), vec![cond, else_]),
        Ternary { cond, then: Some(t), else_ } => ("?".into(), vec![cond, t, else_]),
        _ => return None,
    })
    .map(|(l, v)| (l, v.into_iter().map(|b| &**b).collect()))
}

fn tree(e: &Expr, inlined: &mut HashSet<*const Expr>) -> String {
    match op(e) {
        None => "_".into(),
        Some((label, operands)) => {
            let parts: Vec<String> = operands
                .into_iter()
                .map(|o| {
                    if op(o).is_some() {
                        inlined.insert(o as *const Expr);
                    }
                    tree(o, inlined)
                })
                .collect();
            format!("({label} {})", parts.join(" "))
        }
    }
}

pub fn run() -> ExitCode {
    for path in std::io::stdin().lock().lines().map_while(Result::ok) {
        println!("== {path}");
        let raw = std::fs::read(&path).unwrap_or_default();
        let src = String::from_utf8_lossy(&raw);
        let (file, diags) = phpx_syntax::check(&src);
        if diags.iter().any(|d| d.is_error()) {
            println!("!! has errors");
            continue;
        }
        let mut out = vec![];
        let mut inlined = HashSet::new();
        walk_stmts(
            &file.stmts,
            &mut |e| {
                if op(e).is_some() && !inlined.contains(&(e as *const Expr)) {
                    out.push(tree(e, &mut inlined));
                }
            },
            &mut |_| {},
        );
        out.sort();
        for l in out {
            println!("{l}");
        }
    }
    ExitCode::SUCCESS
}
