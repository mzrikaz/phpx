//! Native side of the Reflection API. The classes themselves are PHP
//! (prelude_reflection.php); these `__phpx_refl_*` built-ins describe
//! classes, functions, parameters, properties and constants as arrays, and
//! do what PHP code can't (read defaults in their declaring scope, call
//! private methods, create objects without their constructor).
//!
//! Functions and attributes get small integer ids (a per-thread registry)
//! so defaults and attribute arguments are evaluated only when asked for.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use phpx_syntax::ast::{self, ClassKind, Member, Type};

use crate::builtins::{arg, has, int};
use crate::class::{Class, ConstSlot, MethodBody, PropDefault, Vis};
use crate::interp::{BuiltinFn, Callable, ClosureKind, Frame, Interp, NsCtx, PRELUDE_FILE, R};
use crate::value::{pstr, Internal, Key, PhpArray, Value};

/// Where a function's parameters (and their defaults) come from.
#[derive(Clone, Copy)]
enum Src {
    Func(&'static ast::Function),
    Closure(&'static ast::Closure),
    Arrow(&'static ast::ArrowFn),
}

impl Src {
    fn key(self) -> usize {
        match self {
            Src::Func(f) => f as *const _ as usize,
            Src::Closure(c) => c as *const _ as usize,
            Src::Arrow(a) => a as *const _ as usize,
        }
    }
    fn params(self) -> &'static [ast::Param] {
        match self {
            Src::Func(f) => &f.params,
            Src::Closure(c) => &c.params,
            Src::Arrow(a) => &a.params,
        }
    }
    fn ret(self) -> Option<&'static Type> {
        match self {
            Src::Func(f) => f.ret.as_ref(),
            Src::Closure(c) => c.ret.as_ref(),
            Src::Arrow(a) => a.ret.as_ref(),
        }
    }
    fn attrs(self) -> &'static [ast::Attribute] {
        match self {
            Src::Func(f) => &f.attrs,
            Src::Closure(c) => &c.attrs,
            Src::Arrow(a) => &a.attrs,
        }
    }
    fn span(self) -> phpx_syntax::Span {
        match self {
            Src::Func(f) => f.span,
            Src::Closure(c) => c.span,
            Src::Arrow(a) => a.span,
        }
    }
}

struct FnReg {
    src: Src,
    ctx: Rc<NsCtx>,
    /// Lowercase declaring class (scope for `self::` in defaults).
    class: Option<Rc<str>>,
}

struct AttrReg {
    attr: &'static ast::Attribute,
    ctx: Rc<NsCtx>,
    class: Option<Rc<str>>,
}

thread_local! {
    static FNS: RefCell<(Vec<FnReg>, HashMap<usize, usize>)> = RefCell::new((vec![], HashMap::new()));
    static ATTRS: RefCell<(Vec<AttrReg>, HashMap<usize, usize>)> = RefCell::new((vec![], HashMap::new()));
}

fn fn_id(src: Src, ctx: &Rc<NsCtx>, class: Option<Rc<str>>) -> i64 {
    FNS.with(|r| {
        let mut r = r.borrow_mut();
        if let Some(&i) = r.1.get(&src.key()) {
            return i as i64;
        }
        let i = r.0.len();
        r.0.push(FnReg { src, ctx: ctx.clone(), class });
        r.1.insert(src.key(), i);
        i as i64
    })
}

fn fn_reg<T>(id: i64, g: impl FnOnce(&FnReg) -> T) -> Option<T> {
    FNS.with(|r| r.borrow().0.get(usize::try_from(id).ok()?).map(g))
}

fn assoc(items: Vec<(&str, Value)>) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in items {
        a.insert(Key::Str(pstr(k)), v);
    }
    Value::array(a)
}

fn list(items: Vec<Value>) -> Value {
    Value::array(PhpArray::from_list(items))
}

fn opt_str(s: Option<&str>) -> Value {
    s.map(Value::str).unwrap_or(Value::Bool(false))
}

const BUILTIN_TYPES: &[&str] = &[
    "int", "float", "string", "bool", "array", "mixed", "void", "null", "callable", "iterable", "object", "never", "false", "true", "static",
];

/// PHP's order for the parts of a union (zend_type_to_string): classes
/// first, then the built-in types in a fixed order, null last.
fn type_rank(name: &str, builtin: bool) -> usize {
    if !builtin {
        return 0;
    }
    ["static", "callable", "object", "array", "string", "int", "float", "bool", "false", "true", "void", "never", "mixed", "null"]
        .iter()
        .position(|t| *t == name)
        .map_or(0, |i| i + 1)
}

/// A type as an array: ['named', name, nullable, builtin] /
/// ['union' | 'intersection', [types], nullable]. `self` and `parent`
/// resolve against `class` (lowercase name of the declaring class).
fn type_desc(it: &Interp, ctx: &NsCtx, class: Option<&str>, t: &Type, implicit_null: bool) -> Value {
    fn named(it: &Interp, ctx: &NsCtx, class: Option<&str>, n: &str, nullable: bool) -> (Value, usize) {
        let l = n.trim_start_matches('\\').to_ascii_lowercase();
        let (name, builtin) = if BUILTIN_TYPES.contains(&l.as_str()) {
            (l.clone(), l != "static")
        } else if l == "self" || l == "parent" {
            let cls = class.and_then(|c| class_by_lname(it, c));
            let target = if l == "self" { cls.map(|c| c.name.to_string()) } else { cls.and_then(|c| c.parent.as_ref().map(|p| p.name.to_string())) };
            (target.unwrap_or(l.clone()), false)
        } else {
            (it.resolve_class_name(ctx, n), false)
        };
        let nullable = nullable || l == "mixed" || l == "null";
        let rank = type_rank(&name, builtin || name == "static");
        (list(vec![Value::str("named"), Value::str(&name), Value::Bool(nullable), Value::Bool(builtin)]), rank)
    }
    fn parts(it: &Interp, ctx: &NsCtx, class: Option<&str>, ts: &[Type]) -> Value {
        let mut v: Vec<(usize, usize, Value)> = ts
            .iter()
            .enumerate()
            .map(|(i, t)| match t {
                Type::Named(n) => {
                    let (d, rank) = named(it, ctx, class, n, false);
                    (rank, i, d)
                }
                other => (0, i, type_desc(it, ctx, class, other, false)),
            })
            .collect();
        v.sort_by_key(|x| (x.0, x.1));
        list(v.into_iter().map(|x| x.2).collect())
    }
    match t {
        Type::Named(n) => named(it, ctx, class, n, implicit_null).0,
        Type::Nullable(inner) => match &**inner {
            Type::Named(n) => named(it, ctx, class, n, true).0,
            other => type_desc(it, ctx, class, other, true),
        },
        Type::Union(ts) => {
            let non_null: Vec<&Type> = ts.iter().filter(|t| !matches!(t, Type::Named(n) if n.eq_ignore_ascii_case("null"))).collect();
            let has_null = non_null.len() < ts.len() || implicit_null;
            if non_null.len() == 1 && has_null {
                if let Type::Named(n) = non_null[0] {
                    return named(it, ctx, class, n, true).0;
                }
            }
            let mut all = ts.clone();
            if implicit_null && non_null.len() == ts.len() {
                all.push(Type::Named("null".into()));
            }
            list(vec![Value::str("union"), parts(it, ctx, class, &all), Value::Bool(has_null)])
        }
        Type::Intersection(ts) => list(vec![Value::str("intersection"), parts(it, ctx, class, ts), Value::Bool(implicit_null)]),
    }
}

/// Type of a built-in's parameter or return, from its arginfo string.
fn type_from_str(ty: &str) -> Value {
    if ty.is_empty() {
        return Value::Null;
    }
    let (nullable, body) = match ty.strip_prefix('?') {
        Some(b) => (true, b),
        None => (false, ty),
    };
    let one = |n: &str, nullable: bool| {
        let l = n.trim_start_matches('\\').to_ascii_lowercase();
        let builtin = BUILTIN_TYPES.contains(&l.as_str()) && l != "static";
        let name = if builtin { l.clone() } else { n.trim_start_matches('\\').to_string() };
        list(vec![Value::str("named"), Value::str(&name), Value::Bool(nullable || l == "mixed"), Value::Bool(builtin)])
    };
    let parts: Vec<&str> = body.split('|').collect();
    let non_null: Vec<&str> = parts.iter().copied().filter(|p| !p.eq_ignore_ascii_case("null")).collect();
    let has_null = nullable || non_null.len() < parts.len();
    if non_null.len() == 1 {
        return one(non_null[0], has_null);
    }
    list(vec![Value::str("union"), list(parts.iter().map(|p| one(p, false)).collect()), Value::Bool(has_null)])
}

fn attrs_desc(it: &Interp, attrs: &'static [ast::Attribute], ctx: &Rc<NsCtx>, class: Option<Rc<str>>) -> Value {
    let mut out = vec![];
    for a in attrs {
        let id = ATTRS.with(|r| {
            let mut r = r.borrow_mut();
            let key = a as *const _ as usize;
            if let Some(&i) = r.1.get(&key) {
                return i;
            }
            let i = r.0.len();
            r.0.push(AttrReg { attr: a, ctx: ctx.clone(), class: class.clone() });
            r.1.insert(key, i);
            i
        });
        out.push(list(vec![Value::str(&it.resolve_class_name(ctx, &a.name)), Value::Int(id as i64)]));
    }
    list(out)
}

fn file_of(it: &Interp, ctx: &NsCtx) -> Value {
    if ctx.file == PRELUDE_FILE {
        return Value::Bool(false);
    }
    let path = it.files.borrow()[ctx.file].path.display().to_string();
    Value::str(&path)
}

fn line(it: &Interp, ctx: &NsCtx, off: u32) -> Value {
    if ctx.file == PRELUDE_FILE {
        return Value::Bool(false);
    }
    Value::Int(it.line_of(ctx.file, off) as i64)
}

fn vis_flag(v: Vis) -> i64 {
    match v {
        Vis::Public => 1,
        Vis::Protected => 2,
        Vis::Private => 4,
    }
}

fn class_arg(it: &Interp, v: &Value) -> R<Option<Rc<Class>>> {
    match v {
        Value::Object(o) => Ok(Some(o.class.clone())),
        other => {
            let n = it.to_str(other)?;
            let n = String::from_utf8_lossy(&n).into_owned();
            it.find_class(n.trim_start_matches('\\'))
        }
    }
}

fn class_by_lname(it: &Interp, l: &str) -> Option<Rc<Class>> {
    it.classes.borrow().get(&*it.lower.get(l)).cloned()
}

fn params_desc(it: &Interp, id: i64) -> Value {
    fn_reg(id, |r| {
        let params = r.src.params();
        let required = params.iter().rposition(|p| p.default.is_none() && !p.variadic).map_or(0, |i| i + 1);
        let out = params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let implicit_null = matches!(&p.default, Some(e) if matches!(e.kind, ast::ExprKind::Null) || matches!(&e.kind, ast::ExprKind::Name(n) if n.eq_ignore_ascii_case("null")));
                assoc(vec![
                    ("name", Value::str(&p.name.name)),
                    ("pos", Value::Int(i as i64)),
                    ("type", p.ty.as_ref().map(|t| type_desc(it, &r.ctx, r.class.as_deref(), t, implicit_null)).unwrap_or(Value::Null)),
                    ("optional", Value::Bool(i >= required || p.variadic)),
                    ("has_default", Value::Bool(p.default.is_some())),
                    ("variadic", Value::Bool(p.variadic)),
                    ("byref", Value::Bool(p.by_ref)),
                    ("promoted", Value::Bool(p.promote.is_some())),
                    ("attrs", attrs_desc(it, &p.attrs, &r.ctx, r.class.clone())),
                    ("default_const", default_const(it, &r.ctx, p.default.as_ref())),
                    ("fid", Value::Int(id)),
                ])
            })
            .collect();
        list(out)
    })
    .unwrap_or_else(|| list(vec![]))
}

/// getDefaultValueConstantName(): a default that is a bare constant.
fn default_const(it: &Interp, ctx: &NsCtx, e: Option<&ast::Expr>) -> Value {
    match e.map(|e| &e.kind) {
        Some(ast::ExprKind::Name(n)) if !matches!(&*n.to_ascii_lowercase(), "null" | "true" | "false") => match n.strip_prefix('\\') {
            Some(full) => Value::str(full),
            // Unqualified: PHP reports the namespaced name it tries first.
            None if !ctx.ns.is_empty() => Value::str(&format!("{}\\{n}", ctx.ns)),
            None => Value::str(n),
        },
        Some(ast::ExprKind::ClassConst { class: ast::ClassRef::Name(c), name }) => {
            let c = if matches!(&*c.to_ascii_lowercase(), "self" | "static" | "parent") { c.clone() } else { it.resolve_class_name(ctx, c) };
            Value::str(&format!("{c}::{}", name.name))
        }
        _ => Value::Null,
    }
}

/// Description of a user function / closure / method body.
fn src_desc(it: &Interp, id: i64, name: &str) -> Value {
    let Some((src, ctx)) = fn_reg(id, |r| (r.src, r.ctx.clone())) else { return Value::Null };
    let class = fn_reg(id, |r| r.class.clone()).flatten();
    let (doc, by_ref, generator, is_static, end) = match src {
        Src::Func(f) => (f.doc.as_deref(), f.by_ref, f.is_generator, false, f.span.end),
        Src::Closure(c) => (None, c.by_ref, c.is_generator, c.is_static, c.span.end),
        Src::Arrow(a) => (None, a.by_ref, a.is_generator, a.is_static, a.body.span.end),
    };
    assoc(vec![
        ("name", Value::str(name)),
        ("fid", Value::Int(id)),
        ("params", params_desc(it, id)),
        ("ret", src.ret().map(|t| type_desc(it, &ctx, class.as_deref(), t, false)).unwrap_or(Value::Null)),
        ("doc", opt_str(doc)),
        ("file", file_of(it, &ctx)),
        ("start", line(it, &ctx, src.span().start)),
        ("end", line(it, &ctx, end)),
        ("attrs", attrs_desc(it, src.attrs(), &ctx, class)),
        ("byref", Value::Bool(by_ref)),
        ("generator", Value::Bool(generator)),
        ("static", Value::Bool(is_static)),
        ("internal", Value::Bool(ctx.file == PRELUDE_FILE)),
        ("ns", Value::str(&ctx.ns)),
    ])
}

/// Description of a built-in function from its arginfo.
fn builtin_desc(name: &str) -> Value {
    let spec = crate::arginfo_table::FUNCS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name));
    let params = spec
        .map(|(_, ps)| {
            ps.iter()
                .enumerate()
                .map(|(i, &(pname, ty, flags, default))| {
                    assoc(vec![
                        ("name", Value::str(pname)),
                        ("pos", Value::Int(i as i64)),
                        ("type", type_from_str(ty)),
                        ("optional", Value::Bool(flags & 5 != 0)),
                        ("has_default", Value::Bool(flags & 1 != 0 && !default.is_empty())),
                        ("variadic", Value::Bool(flags & 4 != 0)),
                        ("byref", Value::Bool(flags & 2 != 0)),
                        ("promoted", Value::Bool(false)),
                        ("attrs", list(vec![])),
                        ("default_const", Value::Null),
                        ("default_src", Value::str(default)),
                        ("fid", Value::Int(-1)),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    assoc(vec![
        ("name", Value::str(name)),
        ("fid", Value::Int(-1)),
        ("params", list(params)),
        ("ret", Value::Null),
        ("doc", Value::Bool(false)),
        ("file", Value::Bool(false)),
        ("start", Value::Bool(false)),
        ("end", Value::Bool(false)),
        ("attrs", list(vec![])),
        ("byref", Value::Bool(false)),
        ("generator", Value::Bool(false)),
        ("static", Value::Bool(false)),
        ("internal", Value::Bool(true)),
        ("ns", Value::str("")),
    ])
}

fn method_desc(it: &Interp, cls: &Rc<Class>, m: &crate::class::Method) -> Value {
    let declaring = class_by_lname(it, &m.declaring).map(|c| c.name.to_string()).unwrap_or_else(|| cls.name.to_string());
    let fid = match &m.body {
        MethodBody::User(f) => fn_id(Src::Func(f), &m.ctx, Some(m.declaring.clone())),
        MethodBody::Native(_) => -1,
    };
    assoc(vec![
        ("name", Value::str(&m.name)),
        ("class", Value::str(&declaring)),
        ("static", Value::Bool(m.is_static)),
        ("abstract", Value::Bool(m.is_abstract)),
        ("final", Value::Bool(m.is_final)),
        ("vis", Value::Int(vis_flag(m.vis))),
        ("fid", Value::Int(fid)),
        ("internal", Value::Bool(fid < 0 || m.ctx.file == PRELUDE_FILE)),
    ])
}

/// The AST of property `name` as declared in `decl` (or one of its traits):
/// (modifiers, type, doc, attrs, promoted).
fn find_prop_ast(it: &Interp, decl: &'static ast::Class, ctx: &Rc<NsCtx>, name: &str) -> Option<(&'static ast::Modifiers, Option<&'static Type>, Option<&'static str>, &'static [ast::Attribute], bool, Rc<NsCtx>)> {
    for m in &decl.members {
        match m {
            Member::Property { modifiers, ty, items, attrs, doc, .. } if items.iter().any(|(n, _)| n.name == name) => {
                return Some((modifiers, ty.as_ref(), doc.as_deref(), attrs, false, ctx.clone()));
            }
            Member::Method { func, .. } if func.name.name.eq_ignore_ascii_case("__construct") => {
                if let Some(p) = func.params.iter().find(|p| p.promote.is_some() && p.name.name == name) {
                    return Some((p.promote.as_ref().unwrap(), p.ty.as_ref(), None, &p.attrs, true, ctx.clone()));
                }
            }
            Member::TraitUse { traits, .. } => {
                for t in traits {
                    if let Some(tc) = class_by_lname(it, &it.resolve_class_name(ctx, t)) {
                        if let Some(r) = find_prop_ast(it, tc.decl, &tc.decl_ctx, name) {
                            return Some(r);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn props_desc(it: &Interp, cls: &Rc<Class>) -> Value {
    // PHP's order: declared here (in source order), from traits, inherited.
    let rank = |name: &str, declaring: &Rc<Class>| -> (usize, usize) {
        if !Rc::ptr_eq(declaring, cls) {
            return (2, 0);
        }
        let pos = cls.decl.members.iter().position(|m| match m {
            Member::Property { items, .. } => items.iter().any(|(n, _)| n.name == name),
            Member::Method { func, .. } => func.name.name.eq_ignore_ascii_case("__construct") && func.params.iter().any(|p| p.promote.is_some() && p.name.name == name),
            _ => false,
        });
        pos.map_or((1, 0), |p| (0, p))
    };
    let mut out: Vec<((usize, usize), Value)> = vec![];
    let mut push = |name: &str, declaring: Rc<Class>, vis: Vis, is_static: bool, has_default: bool| {
        let ast = find_prop_ast(it, declaring.decl, &declaring.decl_ctx, name);
        let (modifiers, ty, doc, attrs, promoted, ctx) = match &ast {
            Some((m, t, d, a, p, c)) => (Some(*m), *t, *d, *a, *p, c.clone()),
            None => (None, None, None, &[][..], false, declaring.decl_ctx.clone()),
        };
        let readonly = modifiers.is_some_and(|m| m.readonly) || declaring.decl.modifiers.readonly;
        let r = rank(name, &declaring);
        out.push((r, assoc(vec![
            ("name", Value::str(name)),
            ("class", Value::str(&declaring.name)),
            ("vis", Value::Int(vis_flag(vis))),
            ("static", Value::Bool(is_static)),
            ("readonly", Value::Bool(readonly && !is_static)),
            ("type", ty.map(|t| type_desc(it, &ctx, Some(&declaring.lname), t, false)).unwrap_or(Value::Null)),
            ("has_default", Value::Bool(has_default && !promoted)),
            ("doc", opt_str(doc)),
            ("attrs", attrs_desc(it, attrs, &ctx, Some(declaring.lname.clone()))),
            ("promoted", Value::Bool(promoted)),
        ])));
    };
    for p in &cls.props {
        let declaring = class_by_lname(it, &p.declaring).unwrap_or_else(|| cls.clone());
        push(&p.name, declaring, p.vis, false, !matches!(p.default, PropDefault::Uninit));
    }
    for (name, sp) in &cls.static_props {
        let declaring = class_by_lname(it, &sp.declaring).unwrap_or_else(|| cls.clone());
        let typed = find_prop_ast(it, declaring.decl, &declaring.decl_ctx, name).is_some_and(|a| a.1.is_some());
        push(name, declaring, sp.vis, true, sp.init.is_some() || !typed);
    }
    out.sort_by_key(|x| x.0);
    list(out.into_iter().map(|x| x.1).collect())
}

/// The class (cls or an ancestor) whose declaration has constant `name`,
/// with that declaration's modifiers, doc and attributes.
fn find_const_ast(it: &Interp, cls: &Rc<Class>, name: &str) -> Option<(Rc<Class>, Option<&'static ast::Modifiers>, Option<&'static str>, &'static [ast::Attribute], bool)> {
    for a in &cls.ancestors {
        let Some(c) = class_by_lname(it, a) else { continue };
        for m in &c.decl.members {
            match m {
                Member::Const { modifiers, items, attrs, doc, .. } if items.iter().any(|(n, _)| n.name == name) => {
                    return Some((c.clone(), Some(modifiers), doc.as_deref(), attrs, false));
                }
                Member::EnumCase { name: n, attrs, doc, .. } if n.name == name => {
                    return Some((c.clone(), None, doc.as_deref(), attrs, true));
                }
                _ => {}
            }
        }
    }
    None
}

/// Whether class constant `name` of `cls` is visible from `scope`.
pub fn const_visible(it: &Interp, cls: &Rc<Class>, name: &str, scope: Option<&Class>) -> bool {
    match find_const_ast(it, cls, name) {
        Some((declaring, Some(m), ..)) => it.member_visible(scope, Vis::of(m), &declaring.lname),
        _ => true,
    }
}

fn consts_desc(it: &Interp, cls: &Rc<Class>) -> Value {
    let names: Vec<Rc<str>> = cls.consts.borrow().keys().cloned().collect();
    // Own constants first, then each ancestor's (PHP's order).
    let mut found: Vec<_> = names
        .into_iter()
        .map(|n| {
            let f = find_const_ast(it, cls, &n).unwrap_or((cls.clone(), None, None, &[], false));
            let rank = cls.ancestors.iter().position(|a| *a == f.0.lname).unwrap_or(usize::MAX);
            (rank, n, f)
        })
        .collect();
    found.sort_by_key(|x| x.0);
    let mut out = vec![];
    for (_, n, (declaring, modifiers, doc, attrs, case)) in found {
        let case = case || matches!(cls.consts.borrow().get(&n), Some(ConstSlot::EnumCase { .. }));
        out.push(assoc(vec![
            ("name", Value::str(&n)),
            ("class", Value::str(&declaring.name)),
            ("vis", Value::Int(modifiers.map_or(1, |m| vis_flag(Vis::of(m))))),
            ("final", Value::Bool(modifiers.is_some_and(|m| m.final_))),
            ("doc", opt_str(doc)),
            ("attrs", attrs_desc(it, attrs, &declaring.decl_ctx, Some(declaring.lname.clone()))),
            ("case", Value::Bool(case)),
        ]));
    }
    list(out)
}

fn class_desc(it: &Interp, cls: &Rc<Class>) -> Value {
    let d = cls.decl;
    let ctx = &cls.decl_ctx;
    let kind = match cls.kind {
        ClassKind::Class => 0,
        ClassKind::Interface => 1,
        ClassKind::Trait => 2,
        ClassKind::Enum => 3,
    };
    let interfaces: Vec<Value> = cls
        .ancestors
        .iter()
        .filter_map(|a| class_by_lname(it, a))
        .filter(|c| c.kind == ClassKind::Interface && !Rc::ptr_eq(c, cls))
        .map(|c| Value::str(&c.name))
        .collect();
    let backing = d.enum_type.as_ref().map(|t| Value::str(&crate::call::type_str(t).to_ascii_lowercase())).unwrap_or(Value::Null);
    assoc(vec![
        ("name", Value::str(&cls.name)),
        ("kind", Value::Int(kind)),
        ("abstract", Value::Bool(cls.is_abstract || (cls.kind == ClassKind::Class && cls.methods.values().any(|m| m.is_abstract)))),
        ("explicit_abstract", Value::Bool(d.modifiers.abstract_)),
        ("final", Value::Bool(d.modifiers.final_ || cls.kind == ClassKind::Enum)),
        ("readonly", Value::Bool(d.modifiers.readonly)),
        ("anon", Value::Bool(d.name.is_none())),
        ("internal", Value::Bool(ctx.file == PRELUDE_FILE)),
        ("parent", cls.parent.as_ref().map(|p| Value::str(&p.name)).unwrap_or(Value::Bool(false))),
        ("interfaces", list(interfaces)),
        ("traits", list(cls.traits.iter().map(|t| Value::str(t)).collect())),
        ("file", file_of(it, ctx)),
        ("start", line(it, ctx, d.span.start)),
        ("end", line(it, ctx, d.span.end)),
        ("doc", opt_str(d.doc.as_deref())),
        ("attrs", attrs_desc(it, &d.attrs, ctx, Some(cls.lname.clone()))),
        ("backing", backing),
        ("has_ctor", Value::Bool(cls.ctor.is_some())),
    ])
}

/// Evaluate `e` in a function's or class's declaring scope.
fn eval_in(it: &Interp, ctx: &Rc<NsCtx>, class: Option<&str>, e: &'static ast::Expr) -> R<Value> {
    let mut fr = Frame::new(ctx.clone(), 0);
    let cls = class.and_then(|c| class_by_lname(it, c));
    fr.class = cls.clone();
    fr.static_class = cls;
    it.eval(&mut fr, e)
}

fn str_arg(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(String::from_utf8_lossy(&it.to_str(&arg(a, i))?).into_owned())
}

/// A frame whose scope is class `c` (visibility as seen from inside it).
fn scope_frame(it: &Interp, c: &Rc<Class>) -> Frame {
    let mut fr = Frame::new(c.decl_ctx.clone(), 0);
    fr.class = Some(c.clone());
    fr.static_class = Some(c.clone());
    let _ = it;
    fr
}

fn closure_desc(it: &Interp, c: &Callable, closure_name: Option<&str>) -> Value {
    match c {
        Callable::User(u) => src_desc(it, fn_id(Src::Func(u.decl), &u.ctx, None), &u.name),
        Callable::Builtin(n, _) => builtin_desc(n),
        Callable::Method { class, method, .. } => {
            let fid = match &method.body {
                MethodBody::User(f) => fn_id(Src::Func(f), &method.ctx, Some(method.declaring.clone())),
                MethodBody::Native(_) => return builtin_desc(&method.name),
            };
            let mut d = src_desc(it, fid, &method.name);
            if let Value::Array(a) = &mut d {
                std::rc::Rc::make_mut(a).insert(Key::Str(pstr("method_class")), Value::str(&class.name));
            }
            d
        }
        Callable::Closure(data) => {
            let class = data.scope.as_ref().map(|c| c.lname.clone());
            let name = closure_name.unwrap_or(&data.name);
            let mut d = match &data.kind {
                ClosureKind::Closure(c) => src_desc(it, fn_id(Src::Closure(c), &data.ctx, class), name),
                ClosureKind::Arrow(a) => src_desc(it, fn_id(Src::Arrow(a), &data.ctx, class), name),
                ClosureKind::Wrap(inner) => return closure_desc(it, inner, None),
            };
            if let Value::Array(a) = &mut d {
                let a = std::rc::Rc::make_mut(a);
                a.insert(Key::Str(pstr("this")), data.this.clone().map(Value::Object).unwrap_or(Value::Null));
                a.insert(Key::Str(pstr("scope")), data.scope.as_ref().map(|c| Value::str(&c.name)).unwrap_or(Value::Null));
                let mut used = PhpArray::new();
                for (n, v, _) in &data.captured {
                    used.insert(Key::Str(pstr(n)), v.deref());
                }
                a.insert(Key::Str(pstr("used")), Value::array(used));
                a.insert(Key::Str(pstr("closure")), Value::Bool(true));
            }
            d
        }
    }
}

/// getMethods() order: own methods as declared, then those from traits,
/// then inherited ones.
fn method_order(c: &Rc<Class>) -> Vec<Rc<crate::class::Method>> {
    let pos = |m: &crate::class::Method| {
        c.decl.members.iter().position(|x| matches!(x, Member::Method { func, .. } if matches!(&m.body, MethodBody::User(f) if std::ptr::eq(*f, func))))
    };
    let mut own: Vec<(usize, Rc<crate::class::Method>)> = vec![];
    let mut rest = vec![];
    for m in c.method_order() {
        if m.declaring == c.lname {
            own.push((pos(&m).unwrap_or(usize::MAX), m));
        } else {
            rest.push(m);
        }
    }
    own.sort_by_key(|x| x.0);
    own.into_iter().map(|x| x.1).chain(rest).collect()
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    // E_DEPRECATED from prelude code, reported at the caller's line.
    t.insert("__phpx_deprecated", |it, _, a| {
        it.deprecated(&str_arg(it, a, 0)?);
        Ok(Value::Null)
    });
    // Class description, or null if there's no such class.
    t.insert("__phpx_refl_class", |it, _, a| Ok(match class_arg(it, &arg(a, 0))? {
        Some(c) => class_desc(it, &c),
        None => Value::Null,
    }));
    t.insert("__phpx_refl_methods", |it, _, a| {
        let Some(c) = class_arg(it, &arg(a, 0))? else { return Ok(list(vec![])) };
        Ok(list(method_order(&c).iter().map(|m| method_desc(it, &c, m)).collect()))
    });
    t.insert("__phpx_refl_method", |it, _, a| {
        let Some(c) = class_arg(it, &arg(a, 0))? else { return Ok(Value::Null) };
        let m = str_arg(it, a, 1)?.to_ascii_lowercase();
        Ok(c.find_method(&m).map(|m| method_desc(it, &c, m)).unwrap_or(Value::Null))
    });
    t.insert("__phpx_refl_props", |it, _, a| Ok(match class_arg(it, &arg(a, 0))? {
        Some(c) => props_desc(it, &c),
        None => list(vec![]),
    }));
    t.insert("__phpx_refl_consts", |it, _, a| Ok(match class_arg(it, &arg(a, 0))? {
        Some(c) => consts_desc(it, &c),
        None => list(vec![]),
    }));
    // Function by name, closure object, or registered id.
    t.insert("__phpx_refl_function", |it, f, a| {
        let v = arg(a, 0);
        Ok(match &v {
            Value::Int(id) => {
                let name = has(a, 1).then(|| arg(a, 1)).map(|n| it.to_str(&n)).transpose()?;
                let name = name.map(|n| String::from_utf8_lossy(&n).into_owned()).unwrap_or_default();
                src_desc(it, *id, &name)
            }
            Value::Object(o) => match &*o.internal() {
                Internal::Closure(c) => closure_desc(it, c, None),
                _ => Value::Null,
            },
            other => {
                let n = String::from_utf8_lossy(&it.to_str(other)?).into_owned();
                match it.find_function(&f.ctx, &format!("\\{}", n.trim_start_matches('\\'))) {
                    Some(c) => closure_desc(it, &c, None),
                    None => Value::Null,
                }
            }
        })
    });
    t.insert("__phpx_refl_params", |it, _, a| Ok(params_desc(it, int(it, a, 0)?)));
    t.insert("__phpx_refl_param_default", |it, _, a| {
        let id = int(it, a, 0)?;
        let pos = int(it, a, 1)? as usize;
        let Some((e, ctx, class)) = fn_reg(id, |r| (r.src.params().get(pos).and_then(|p| p.default.as_ref()), r.ctx.clone(), r.class.clone())) else {
            return Ok(Value::Null);
        };
        match e {
            Some(e) => eval_in(it, &ctx, class.as_deref(), e),
            None => Ok(Value::Null),
        }
    });
    t.insert("__phpx_refl_attr_args", |it, _, a| {
        let id = int(it, a, 0)? as usize;
        let Some((attr, ctx, class)) = ATTRS.with(|r| r.borrow().0.get(id).map(|r| (r.attr, r.ctx.clone(), r.class.clone()))) else {
            return Ok(list(vec![]));
        };
        let mut out = PhpArray::new();
        for x in &attr.args {
            let v = eval_in(it, &ctx, class.as_deref(), &x.value)?;
            match &x.name {
                Some(n) => out.insert(Key::Str(pstr(&n.name)), v),
                None => {
                    out.push(v);
                }
            }
        }
        Ok(Value::array(out))
    });
    t.insert("__phpx_refl_prop_default", |it, _, a| {
        let Some(c) = class_arg(it, &arg(a, 0))? else { return Ok(Value::Null) };
        let name = str_arg(it, a, 1)?;
        if let Some(p) = c.find_prop(&name) {
            return Ok(it.prop_default(&c, p)?.unwrap_or(Value::Null));
        }
        if let Some(sp) = c.static_props.get(name.as_str()) {
            if let Some(e) = sp.init {
                return eval_in(it, &sp.ctx, Some(&sp.declaring), e);
            }
        }
        Ok(Value::Null)
    });
    // Instance property read / write as seen from the declaring class.
    t.insert("__phpx_refl_prop_get", |it, _, a| {
        let Value::Object(o) = arg(a, 0) else { return Ok(Value::Null) };
        let Some(c) = class_arg(it, &arg(a, 1))? else { return Ok(Value::Null) };
        let name = str_arg(it, a, 2)?;
        it.get_prop(&scope_frame(it, &c), &o, &name)
    });
    t.insert("__phpx_refl_prop_set", |it, _, a| {
        let Value::Object(o) = arg(a, 0) else { return Ok(Value::Null) };
        let Some(c) = class_arg(it, &arg(a, 1))? else { return Ok(Value::Null) };
        let name = str_arg(it, a, 2)?;
        it.set_prop(&scope_frame(it, &c), &o, &name, arg(a, 3))?;
        Ok(Value::Null)
    });
    t.insert("__phpx_refl_prop_isset", |it, _, a| {
        let Value::Object(o) = arg(a, 0) else { return Ok(Value::Bool(false)) };
        let name = str_arg(it, a, 1)?;
        let set = o.props.borrow().contains_key(name.as_str());
        Ok(Value::Bool(set))
    });
    t.insert("__phpx_refl_static_get", |it, _, a| {
        let Some(c) = class_arg(it, &arg(a, 0))? else { return Ok(Value::Null) };
        let name = str_arg(it, a, 1)?;
        match c.static_props.get(name.as_str()) {
            Some(sp) => it.static_prop_slot(sp),
            None => Err(it.error("ReflectionException", &format!("Property {}::${name} does not exist", c.name))),
        }
    });
    t.insert("__phpx_refl_static_set", |it, _, a| {
        let Some(c) = class_arg(it, &arg(a, 0))? else { return Ok(Value::Null) };
        let name = str_arg(it, a, 1)?;
        match c.static_props.get(name.as_str()) {
            Some(sp) => {
                it.static_prop_slot(sp)?;
                let v = arg(a, 2);
                let mut slot = sp.value.borrow_mut();
                match slot.as_ref() {
                    Some(Value::Ref(r)) => *r.borrow_mut() = v,
                    _ => *slot = Some(v),
                }
                Ok(Value::Null)
            }
            None => Err(it.error("ReflectionException", &format!("Class {} does not have a property named {name}", c.name))),
        }
    });
    // Call the reflected method itself (not an override), any visibility.
    t.insert("__phpx_refl_invoke", |it, _, a| {
        let Some(c) = class_arg(it, &arg(a, 0))? else { return Ok(Value::Null) };
        let m = str_arg(it, a, 1)?.to_ascii_lowercase();
        let Some(m) = c.find_method(&m).cloned() else { return Ok(Value::Null) };
        let this = match arg(a, 2) {
            Value::Object(o) if !m.is_static => Some(o),
            _ => None,
        };
        let static_class = this.as_ref().map(|o| o.class.clone()).unwrap_or_else(|| c.clone());
        let mut pos = vec![];
        let mut named = vec![];
        if let Value::Array(args) = arg(a, 3) {
            for (k, v) in args.iter() {
                match k {
                    Key::Int(_) => pos.push(v.clone()),
                    Key::Str(s) => named.push((Rc::from(&*String::from_utf8_lossy(&s)), v.clone())),
                }
            }
        }
        it.invoke_method(this, &m, static_class, pos, named)
    });
    t.insert("__phpx_refl_new_without_ctor", |it, _, a| {
        let Some(c) = class_arg(it, &arg(a, 0))? else { return Ok(Value::Null) };
        Ok(Value::Object(it.new_instance(&c)?))
    });
    // A Closure for a method, bound to $obj (or static).
    t.insert("__phpx_refl_method_closure", |it, _, a| {
        let Some(c) = class_arg(it, &arg(a, 0))? else { return Ok(Value::Null) };
        let m = str_arg(it, a, 1)?.to_ascii_lowercase();
        let Some(m) = c.find_method(&m).cloned() else { return Ok(Value::Null) };
        let this = match arg(a, 2) {
            Value::Object(o) if !m.is_static => Some(o),
            _ => None,
        };
        let static_class = this.as_ref().map(|o| o.class.clone()).unwrap_or_else(|| c.clone());
        Ok(it.closure_object(Callable::Method { this, class: c.clone(), method: m, static_class }))
    });
}
