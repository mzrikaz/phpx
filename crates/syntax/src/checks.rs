//! Subset rules that need the tree rather than tokens: calls that create or
//! overwrite local variables by name, which a compiler can't resolve.
//! Reading the scope by name (`compact($names)`, `get_defined_vars()`) is
//! fine: the compiler knows every local of a function.

use crate::ast::*;
use crate::diag::Diagnostic;

pub fn check(file: &File) -> Vec<Diagnostic> {
    let (mut out, mut members) = (vec![], vec![]);
    walk_stmts(&file.stmts, &mut |e| check_expr(e, &mut out), &mut |m| check_member(m, &mut members));
    out.append(&mut members);
    out
}

fn check_expr(e: &Expr, out: &mut Vec<Diagnostic>) {
    match &e.kind {
        ExprKind::Call { callee, args } => {
            let ExprKind::Name(n) = &callee.kind else { return };
            let n = n.trim_start_matches('\\').to_ascii_lowercase();
            let (msg, help) = match n.as_str() {
                "create_function" => ("`create_function()` was removed in PHP 8", "use a closure: `fn($x) => ...`"),
                _ => return,
            };
            let _ = args;
            out.push(Diagnostic::error("E1007", callee.span, msg).with_help(help));
        }
        _ => {}
    }
}

fn check_member(m: &Member, out: &mut Vec<Diagnostic>) {
    if let Member::Property { has_hooks: true, span, .. } = m {
        out.push(
            Diagnostic::error("E1008", *span, "property hooks are not supported yet")
                .with_help("use a getter/setter method for now"),
        );
    }
}

// ---- generic walker ----------------------------------------------------


pub fn walk_stmts<'a>(stmts: &'a [Stmt], fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    for s in stmts {
        walk_stmt(s, fe, fm);
    }
}

fn walk_opt<'a>(e: &'a Option<Expr>, fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    if let Some(e) = e {
        walk_expr(e, fe, fm);
    }
}

fn walk_stmt<'a>(s: &'a Stmt, fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    if crate::stack::exhausted() {
        return;
    }
    use StmtKind::*;
    match &s.kind {
        Expr(e) => walk_expr(e, fe, fm),
        Echo(es) | Unset(es) | Global(es) => es.iter().for_each(|e| walk_expr(e, fe, fm)),
        If { cond, then, elseifs, else_ } => {
            walk_expr(cond, fe, fm);
            walk_stmts(then, fe, fm);
            for (c, b) in elseifs {
                walk_expr(c, fe, fm);
                walk_stmts(b, fe, fm);
            }
            if let Some(b) = else_ {
                walk_stmts(b, fe, fm);
            }
        }
        While { cond, body } | DoWhile { body, cond } => {
            walk_expr(cond, fe, fm);
            walk_stmts(body, fe, fm);
        }
        For { init, cond, step, body } => {
            init.iter().chain(cond).chain(step).for_each(|e| walk_expr(e, fe, fm));
            walk_stmts(body, fe, fm);
        }
        Foreach { subject, key, value, body, .. } => {
            walk_expr(subject, fe, fm);
            walk_opt(key, fe, fm);
            walk_expr(value, fe, fm);
            walk_stmts(body, fe, fm);
        }
        Switch { subject, cases } => {
            walk_expr(subject, fe, fm);
            for c in cases {
                walk_opt(&c.test, fe, fm);
                walk_stmts(&c.body, fe, fm);
            }
        }
        Break(e) | Continue(e) | Return(e) => walk_opt(e, fe, fm),
        Function(f) => walk_function(f, fe, fm),
        Class(c) => walk_class(c, fe, fm),
        Try { body, catches, finally } => {
            walk_stmts(body, fe, fm);
            catches.iter().for_each(|c| walk_stmts(&c.body, fe, fm));
            if let Some(f) = finally {
                walk_stmts(f, fe, fm);
            }
        }
        Namespace { body: Some(b), .. } | Block(b) => walk_stmts(b, fe, fm),
        Declare { items, body } => {
            items.iter().for_each(|(_, e)| walk_expr(e, fe, fm));
            if let Some(b) = body {
                walk_stmts(b, fe, fm);
            }
        }
        Const(items) => items.iter().for_each(|(_, e)| walk_expr(e, fe, fm)),
        StaticVar(vars) => vars.iter().for_each(|(_, e)| walk_opt(e, fe, fm)),
        Namespace { body: None, .. } | Use { .. } | InlineHtml(_) | Label(_) | Goto(_) | Nop => {}
    }
}

fn walk_params<'a>(ps: &'a [Param], fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    for p in ps {
        walk_opt(&p.default, fe, fm);
        walk_attrs(&p.attrs, fe, fm);
    }
}

fn walk_attrs<'a>(attrs: &'a [Attribute], fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    attrs.iter().flat_map(|a| &a.args).for_each(|a| walk_expr(&a.value, fe, fm));
}

fn walk_function<'a>(f: &'a Function, fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    walk_params(&f.params, fe, fm);
    walk_attrs(&f.attrs, fe, fm);
    if let Some(b) = &f.body {
        walk_stmts(b, fe, fm);
    }
}

fn walk_class<'a>(c: &'a Class, fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    walk_attrs(&c.attrs, fe, fm);
    for m in &c.members {
        fm(m);
        match m {
            Member::Property { items, attrs, .. } => {
                walk_attrs(attrs, fe, fm);
                items.iter().for_each(|(_, e)| walk_opt(e, fe, fm));
            }
            Member::Method { func, .. } => walk_function(func, fe, fm),
            Member::Const { items, attrs, .. } => {
                walk_attrs(attrs, fe, fm);
                items.iter().for_each(|(_, e)| walk_expr(e, fe, fm));
            }
            Member::EnumCase { value, attrs, .. } => {
                walk_attrs(attrs, fe, fm);
                walk_opt(value, fe, fm);
            }
            Member::TraitUse { .. } => {}
        }
    }
}

fn walk_args<'a>(args: &'a [Arg], fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    args.iter().for_each(|a| walk_expr(&a.value, fe, fm));
}

fn walk_class_ref<'a>(c: &'a ClassRef, fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    match c {
        ClassRef::Name(_) => {}
        ClassRef::Dynamic(e) => walk_expr(e, fe, fm),
        ClassRef::Anonymous(c) => walk_class(c, fe, fm),
    }
}

fn walk_member_name<'a>(n: &'a MemberName, fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    if let MemberName::Dynamic(e) = n {
        walk_expr(e, fe, fm);
    }
}

pub fn walk_expr<'a>(e: &'a Expr, fe: &mut dyn FnMut(&'a Expr), fm: &mut dyn FnMut(&'a Member)) {
    if crate::stack::exhausted() {
        return;
    }
    fe(e);
    use ExprKind::*;
    match &e.kind {
        Int(_) | Float(_) | Str(_) | Bytes(_) | Bool(_) | Null | Var(_) | Name(_) | MagicConst(_) | Error => {}
        VarVar(x) | GlobalVar(x) => walk_expr(x, fe, fm),
        Interp(parts) => parts.iter().for_each(|p| {
            if let InterpPart::Expr(e) = p {
                walk_expr(e, fe, fm)
            }
        }),
        Array(items) => items.iter().flatten().for_each(|i| {
            if let Some(k) = &i.key {
                walk_expr(k, fe, fm);
            }
            walk_expr(&i.value, fe, fm);
        }),
        Assign { target, value } | AssignRef { target, value } | AssignOp { target, value, .. } => {
            walk_expr(target, fe, fm);
            walk_expr(value, fe, fm);
        }
        Binary { left, right, .. } => {
            walk_expr(left, fe, fm);
            walk_expr(right, fe, fm);
        }
        Unary { expr, .. } | IncDec { expr, .. } | Cast { expr, .. } => walk_expr(expr, fe, fm),
        Clone(e) | Empty(e) | Throw(e) | Print(e) | YieldFrom(e) | CallableRef(e) => walk_expr(e, fe, fm),
        Include { path, .. } => walk_expr(path, fe, fm),
        Exit(e) => {
            if let Some(e) = e {
                walk_expr(e, fe, fm);
            }
        }
        Ternary { cond, then, else_ } => {
            walk_expr(cond, fe, fm);
            if let Some(t) = then {
                walk_expr(t, fe, fm);
            }
            walk_expr(else_, fe, fm);
        }
        Call { callee, args } => {
            walk_expr(callee, fe, fm);
            walk_args(args, fe, fm);
        }
        Prop { object, name, .. } => {
            walk_expr(object, fe, fm);
            walk_member_name(name, fe, fm);
        }
        MethodCall { object, name, args, .. } => {
            walk_expr(object, fe, fm);
            walk_member_name(name, fe, fm);
            walk_args(args, fe, fm);
        }
        StaticProp { class, name } => {
            walk_class_ref(class, fe, fm);
            walk_member_name(name, fe, fm);
        }
        ClassConst { class, .. } => walk_class_ref(class, fe, fm),
        StaticCall { class, name, args } => {
            walk_class_ref(class, fe, fm);
            walk_member_name(name, fe, fm);
            walk_args(args, fe, fm);
        }
        Index { base, index } => {
            walk_expr(base, fe, fm);
            if let Some(i) = index {
                walk_expr(i, fe, fm);
            }
        }
        New { class, args } => {
            walk_class_ref(class, fe, fm);
            walk_args(args, fe, fm);
        }
        Closure(c) => {
            walk_params(&c.params, fe, fm);
            walk_stmts(&c.body, fe, fm);
        }
        ArrowFn(f) => {
            walk_params(&f.params, fe, fm);
            walk_expr(&f.body, fe, fm);
        }
        Match { subject, arms } => {
            walk_expr(subject, fe, fm);
            for a in arms {
                a.conds.iter().flatten().for_each(|c| walk_expr(c, fe, fm));
                walk_expr(&a.body, fe, fm);
            }
        }
        Instanceof { expr, class } => {
            walk_expr(expr, fe, fm);
            walk_class_ref(class, fe, fm);
        }
        Isset(es) => es.iter().for_each(|e| walk_expr(e, fe, fm)),
        Yield { key, value } => {
            if let Some(k) = key {
                walk_expr(k, fe, fm);
            }
            if let Some(v) = value {
                walk_expr(v, fe, fm);
            }
        }
    }
}
