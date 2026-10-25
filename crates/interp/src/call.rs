//! Calls, name resolution, classes and type coercion.

use std::collections::HashMap;
use std::rc::Rc;

use indexmap::IndexMap;
use phpx_syntax::ast::{self, ClassKind, Member, Stmt, Type};

use crate::class::{Class, ConstSlot, Method, MethodBody, PropDecl, PropDefault, StaticProp, Vis};
use crate::interp::{qualify, slot, Callable, ClosureKind, Ctrl, Flow, Frame, Interp, NsCtx, UserFn, R};
use crate::value::{Internal, Key, Obj, PhpArray, Value};

/// By-reference parameter positions not in the arginfo table (variadic
/// ones expanded, and phpx-only built-ins).
const BY_REF: &[(&str, usize)] = &[
    ("sort", 0), ("rsort", 0), ("usort", 0), ("uasort", 0), ("uksort", 0), ("ksort", 0), ("krsort", 0),
    ("asort", 0), ("arsort", 0), ("shuffle", 0), ("array_push", 0), ("array_pop", 0), ("array_shift", 0),
    ("array_unshift", 0), ("array_splice", 0), ("array_walk", 0), ("preg_match", 2), ("preg_match_all", 2),
    ("settype", 0), ("natsort", 0), ("natcasesort", 0), ("array_multisort", 0),
    ("preg_replace", 4), ("parse_str", 1), ("sscanf", 2), ("sscanf", 3), ("sscanf", 4), ("sscanf", 5), ("sscanf", 6), ("sscanf", 7), ("next", 0), ("prev", 0), ("reset", 0), ("end", 0), ("headers_sent", 0), ("headers_sent", 1), ("similar_text", 2), ("openssl_random_pseudo_bytes", 1), ("preg_replace_callback", 4), ("str_replace", 3), ("str_ireplace", 3),
];

/// Positions of built-in `name`'s by-reference parameters (from PHP's
/// arginfo, plus `BY_REF`); empty for none.
pub(crate) fn by_ref_params(name: &str) -> &'static [usize] {
    static MAP: std::sync::OnceLock<rustc_hash::FxHashMap<&'static str, Vec<usize>>> = std::sync::OnceLock::new();
    let map = MAP.get_or_init(|| {
        let mut m: rustc_hash::FxHashMap<&'static str, Vec<usize>> = Default::default();
        for (f, ps) in crate::arginfo_table::FUNCS {
            // Implemented on a copy of the array.
            if *f == "array_walk_recursive" {
                continue;
            }
            for (i, p) in ps.iter().enumerate() {
                if p.2 & 2 != 0 {
                    m.entry(*f).or_default().push(i);
                }
            }
        }
        for &(f, i) in BY_REF {
            let v = m.entry(f).or_default();
            if !v.contains(&i) {
                v.push(i);
            }
        }
        m
    });
    map.get(name).map(Vec::as_slice).unwrap_or(&[])
}

/// Below this much free task stack, a call throws instead of recursing.
const STACK_RESERVE: usize = 256 * 1024;

#[derive(Clone, Copy)]
enum Body {
    Block(&'static [Stmt]),
    Expr(&'static ast::Expr),
}

struct Invoke<'a> {
    params: &'static [ast::Param],
    body: Body,
    ret: Option<&'static Type>,
    /// For errors: `foo`, `Foo::bar`, `{closure}`.
    display: Rc<str>,
    func: Rc<str>,
    this: Option<Rc<Obj>>,
    class: Option<Rc<Class>>,
    static_class: Option<Rc<Class>>,
    ctx: Rc<NsCtx>,
    fn_id: usize,
    captured: &'a [(&'static str, Value, bool)],
    is_generator: bool,
}

/// Parameters of user code, so arguments for `&$x` params can be passed by
/// reference. None for built-ins (they use the BY_REF table).
pub fn callable_params(c: &Callable) -> Option<&'static [ast::Param]> {
    match c {
        Callable::User(u) => Some(&u.decl.params),
        Callable::Closure(cd) => match &cd.kind {
            ClosureKind::Closure(c) => Some(&c.params),
            ClosureKind::Arrow(a) => Some(&a.params),
            ClosureKind::Wrap(w) => callable_params(w),
        },
        Callable::Method { method, .. } => method_params(method),
        Callable::Builtin(..) => None,
    }
}

pub fn method_params(m: &Method) -> Option<&'static [ast::Param]> {
    match &m.body {
        MethodBody::User(f) => Some(&f.params),
        MethodBody::Native(_) => None,
    }
}

fn is_ref_target(e: &ast::Expr) -> bool {
    matches!(e.kind, ast::ExprKind::Var(_) | ast::ExprKind::Index { .. } | ast::ExprKind::Prop { .. } | ast::ExprKind::StaticProp { .. })
}

pub fn type_str(t: &Type) -> String {
    match t {
        Type::Named(n) => n.trim_start_matches('\\').to_string(),
        Type::Nullable(t) => format!("?{}", type_str(t)),
        Type::Union(ts) => ts.iter().map(type_str).collect::<Vec<_>>().join("|"),
        Type::Intersection(ts) => ts.iter().map(type_str).collect::<Vec<_>>().join("&"),
    }
}

impl Interp {
    // ---- name resolution ---------------------------------------------------

    pub fn resolve_class_name(&self, ctx: &NsCtx, name: &str) -> String {
        if let Some(rest) = name.strip_prefix('\\') {
            return rest.to_string();
        }
        let lower = name.to_ascii_lowercase();
        if matches!(lower.as_str(), "self" | "static" | "parent") {
            return name.to_string();
        }
        if lower.starts_with("namespace\\") {
            return qualify(&ctx.ns, &name[10..]);
        }
        let (first, rest) = match name.split_once('\\') {
            Some((a, b)) => (a, Some(b)),
            None => (name, None),
        };
        if let Some(full) = ctx.uses.get(&first.to_ascii_lowercase()) {
            return match rest {
                Some(r) => format!("{full}\\{r}"),
                None => full.clone(),
            };
        }
        qualify(&ctx.ns, name)
    }

    pub fn lookup_class(&self, full: &str) -> R<Rc<Class>> {
        let full = full.trim_start_matches('\\');
        if let Some(c) = self.find_class(full)? {
            return Ok(c);
        }
        Err(self.error("Error", &format!("Class \"{full}\" not found")))
    }

    /// Declared class, or one an autoloader declares on demand.
    pub fn find_class(&self, full: &str) -> R<Option<Rc<Class>>> {
        let l = self.lower.get(full.trim_start_matches('\\'));
        if let Some(c) = self.classes.borrow().get(&*l) {
            return Ok(Some(c.clone()));
        }
        if !self.autoloading.borrow_mut().insert(l.to_string()) {
            return Ok(None);
        }
        let loaders = self.autoloaders.borrow().clone();
        let mut result = Ok(None);
        for loader in loaders {
            if let Err(e) = self.call_callable(&loader, vec![Value::str(full)], vec![]) {
                result = Err(e);
                break;
            }
            if let Some(c) = self.classes.borrow().get(&*l) {
                result = Ok(Some(c.clone()));
                break;
            }
        }
        self.autoloading.borrow_mut().remove(&*l);
        result
    }

    pub fn class_ref(&self, f: &mut Frame, c: &'static ast::ClassRef) -> R<Rc<Class>> {
        match c {
            ast::ClassRef::Name(n) => {
                // A name in the source always resolves to the same class
                // (classes can't be redeclared): cache per AST node.
                let key = n.as_ptr() as usize;
                if let Some(c) = self.class_cache.borrow().get(&key) {
                    return Ok(c.clone());
                }
                let c = self.named_class(f, n)?;
                let special = ["self", "static", "parent"].iter().any(|s| n.eq_ignore_ascii_case(s));
                if !special {
                    self.class_cache.borrow_mut().insert(key, c.clone());
                }
                Ok(c)
            }
            ast::ClassRef::Dynamic(e) => match self.eval(f, e)? {
                Value::Object(o) => Ok(o.class.clone()),
                Value::Str(s) => self.lookup_class(&String::from_utf8_lossy(&s)),
                _ => Err(self.error("Error", "Cannot use value as class name")),
            },
            ast::ClassRef::Anonymous(c) => self.declare_anon_class(f, c),
        }
    }

    fn named_class(&self, f: &Frame, n: &str) -> R<Rc<Class>> {
        match &*n.to_ascii_lowercase() {
            "self" => f.class.clone().ok_or_else(|| self.error("Error", "Cannot use \"self\" when no class scope is active")),
            "static" => f
                .static_class
                .clone()
                .ok_or_else(|| self.error("Error", "Cannot use \"static\" when no class scope is active")),
            "parent" => match &f.class {
                Some(c) => c
                    .parent
                    .clone()
                    .ok_or_else(|| self.error("Error", "Cannot use \"parent\" when current class scope has no parent")),
                None => Err(self.error("Error", "Cannot use \"parent\" when no class scope is active")),
            },
            _ => self.lookup_class(&self.resolve_class_name(&f.ctx, n)),
        }
    }

    /// A class named by a run-time string: always fully qualified (the
    /// current namespace doesn't apply), except self/parent/static.
    fn string_class(&self, f: &Frame, n: &str) -> R<Rc<Class>> {
        match &*n.to_ascii_lowercase() {
            "self" | "static" | "parent" => self.named_class(f, n),
            _ => self.named_class(f, &format!("\\{}", n.trim_start_matches('\\'))),
        }
    }

    pub fn find_function(&self, ctx: &NsCtx, name: &str) -> Option<Callable> {
        let lookup = |full: &str| -> Option<Callable> {
            let l = self.lower.get(full);
            if let Some(u) = self.functions.borrow().get(&*l) {
                return Some(Callable::User(u.clone()));
            }
            if !l.contains('\\') {
                if let Some((k, b)) = self.builtins.get_key_value(&*l) {
                    return Some(Callable::Builtin(k, *b));
                }
            }
            None
        };
        if let Some(rest) = name.strip_prefix('\\') {
            return lookup(rest);
        }
        if name.contains('\\') {
            return lookup(&self.resolve_class_name(ctx, name));
        }
        if let Some(full) = ctx.use_fns.get(&name.to_ascii_lowercase()) {
            return lookup(full);
        }
        if !ctx.ns.is_empty() {
            if let Some(c) = lookup(&qualify(&ctx.ns, name)) {
                return Some(c);
            }
        }
        lookup(name)
    }

    pub fn constant(&self, f: &Frame, name: &str) -> R<Value> {
        let consts = self.constants.borrow();
        let found = if let Some(rest) = name.strip_prefix('\\') {
            consts.get(rest).cloned()
        } else if name.contains('\\') {
            consts.get(&self.resolve_class_name(&f.ctx, name)).cloned()
        } else if let Some(full) = f.ctx.use_consts.get(name) {
            consts.get(full).cloned()
        } else {
            consts
                .get(&qualify(&f.ctx.ns, name))
                .or_else(|| consts.get(name))
                .or_else(|| {
                    // A few built-in constants are case-insensitive in practice.
                    let up = name.to_ascii_uppercase();
                    if matches!(up.as_str(), "PHP_EOL" | "E_ALL") { consts.get(&up) } else { None }
                })
                .cloned()
        };
        drop(consts);
        found.ok_or_else(|| self.error("Error", &format!("Undefined constant \"{}\"", name.trim_start_matches('\\'))))
    }

    // ---- arguments ---------------------------------------------------------

    pub fn eval_args(&self, f: &mut Frame, args: &'static [ast::Arg]) -> R<(Vec<Value>, Vec<(Rc<str>, Value)>)> {
        let mut pos = vec![];
        let mut named = vec![];
        for a in args {
            let v = self.eval(f, &a.value)?;
            if a.spread {
                let pairs: Vec<(Value, Value)> = match v {
                    Value::Array(arr) => arr.iter().map(|(k, v)| (k.to_value(), v.clone())).collect(),
                    Value::Object(o) if o.class.is_a("traversable") => self.iterate(&o)?,
                    _ => return Err(self.error("Error", "Only arrays and Traversables can be unpacked")),
                };
                for (k, v) in pairs {
                    match k {
                        Value::Str(s) => named.push((Rc::from(String::from_utf8_lossy(&s).as_ref()), v)),
                        _ => pos.push(v),
                    }
                }
            } else if let Some(n) = &a.name {
                named.push((Rc::from(&*n.name), v));
            } else {
                if !named.is_empty() {
                    return Err(self.error("Error", "Cannot use positional argument after named argument"));
                }
                pos.push(v);
            }
        }
        Ok((pos, named))
    }

    /// Like eval_args, but arguments for `&$x` parameters are passed as the
    /// caller's cell (`Value::Ref`).
    pub fn eval_args_for(
        &self,
        f: &mut Frame,
        args: &'static [ast::Arg],
        params: Option<&'static [ast::Param]>,
    ) -> R<(Vec<Value>, Vec<(Rc<str>, Value)>)> {
        let Some(params) = params.filter(|ps| ps.iter().any(|p| p.by_ref)) else { return self.eval_args(f, args) };
        let by_ref_at = |i: usize| match params.get(i) {
            Some(p) => p.by_ref,
            None => params.last().is_some_and(|p| p.variadic && p.by_ref),
        };
        if args.iter().any(|a| a.spread) {
            return self.eval_args(f, args);
        }
        let mut pos = vec![];
        let mut named = vec![];
        for a in args {
            let by_ref = match &a.name {
                Some(n) => params.iter().any(|p| p.by_ref && p.name.name == n.name),
                None => by_ref_at(pos.len()),
            };
            let v = if by_ref && is_ref_target(&a.value) { Value::Ref(self.make_ref(f, &a.value)?) } else { self.eval(f, &a.value)? };
            match &a.name {
                Some(n) => named.push((Rc::from(&*n.name), v)),
                None => pos.push(v),
            }
        }
        Ok((pos, named))
    }

    /// `foo(...)` where `foo` is a name: user function or built-in.
    pub fn call_named(&self, f: &mut Frame, name: &'static str, args: &'static [ast::Arg]) -> R<Value> {
        let callable = self.resolve_call(f, name)?;
        if let Callable::Builtin(bname, fun) = &callable {
            return self.call_builtin_named(f, *bname, *fun, args);
        }
        let (pos, named) = self.eval_args_for(f, args, callable_params(&callable))?;
        self.call_callable(&callable, pos, named)
    }

    /// Resolve a called name. Each call site's name lives at a unique
    /// (leaked) address; user functions can't be redeclared, so resolving
    /// once is enough.
    #[inline(never)]
    pub(crate) fn resolve_call(&self, f: &Frame, name: &'static str) -> R<Callable> {
        let site = name.as_ptr() as usize;
        if let Some(c) = self.call_cache.borrow().get(&site) {
            return Ok(c.clone());
        }
        let c = self.find_function(&f.ctx, name);
        match c {
            Some(c) => {
                if let Callable::User(_) = &c {
                    self.call_cache.borrow_mut().insert(site, c.clone());
                }
                Ok(c)
            }
            None => Err(self.error("Error", &format!("Call to undefined function {}()", name.trim_start_matches('\\')))),
        }
    }

    #[inline(never)]
    fn call_builtin_named(&self, f: &mut Frame, bname: &'static str, fun: crate::interp::BuiltinFn, args: &'static [ast::Arg]) -> R<Value> {
        // Built-ins that read the caller's scope.
        match bname {
            "compact" => return self.compact(f, args),
            // Direct two-argument min()/max(): PHP 8.4+ compiles these to
            // "frameless" calls with their own tie rules.
            "min" | "max" if args.len() == 2 && args.iter().all(|a| !a.spread && a.name.is_none()) => {
                let (a, b) = (self.eval(f, &args[0].value)?.into_deref(), self.eval(f, &args[1].value)?.into_deref());
                if !matches!(a, Value::Object(_)) && !matches!(b, Value::Object(_)) {
                    return Ok(crate::compiler::fastbi::frameless_pick(&a, &b, bname == "max"));
                }
                let mut vals = vec![a, b];
                return crate::builtins::invoke(self, bname, crate::arginfo::spec(bname), fun, f, &mut vals);
            }
            "extract" => return self.extract(f, args),
            "eval" => return self.eval_code(f, args),
            "func_get_args" => return Ok(Value::array(PhpArray::from_list(f.args.clone()))),
            "get_defined_vars" => {
                let mut out = PhpArray::new();
                for (name, s) in f.vars.iter() {
                    out.insert(Key::Str(crate::value::pstr(name)), s.borrow().clone());
                }
                return Ok(Value::array(out));
            }
            "func_num_args" => return Ok(Value::Int(f.args.len() as i64)),
            "get_called_class" => {
                return Ok(f.static_class.as_ref().map(|c| Value::str(&c.name)).unwrap_or(Value::Bool(false)))
            }
            "get_class" | "get_parent_class" if args.is_empty() => {
                let c = if bname == "get_class" { f.class.clone() } else { f.class.as_ref().and_then(|c| c.parent.clone()) };
                return Ok(c.map(|c| Value::str(&c.name)).unwrap_or(Value::Bool(false)));
            }
            _ => {}
        }
        let idxs = by_ref_params(bname);
        if !idxs.is_empty() {
            return self.call_by_ref_builtin(f, bname, fun, args, idxs);
        }
        let (pos, named) = self.eval_args(f, args)?;
        if named.is_empty() {
            // Run with the caller's frame: scope-dependent built-ins
            // (get_object_vars, get_class_methods, ...) see its class.
            let mut vals: Vec<Value> = pos.into_iter().map(Value::into_deref).collect();
            return crate::builtins::invoke(self, bname, crate::arginfo::spec(bname), fun, f, &mut vals);
        }
        let callable = Callable::Builtin(bname, fun);
        self.call_callable(&callable, pos, named)
    }

    /// `extract($array, $flags, $prefix)`: array keys become local variables.
    fn extract(&self, f: &mut Frame, args: &'static [ast::Arg]) -> R<Value> {
        let vals = args.iter().map(|a| self.eval(f, &a.value)).collect::<R<Vec<_>>>()?;
        let Some(Value::Array(arr)) = vals.first() else {
            return Err(self.error("TypeError", "extract(): Argument #1 ($array) must be of type array"));
        };
        let flags = match vals.get(1) {
            Some(v) => self.to_int(v)?,
            None => 0,
        };
        let prefix = match vals.get(2) {
            Some(v) => String::from_utf8_lossy(&self.to_str(v)?).into_owned(),
            None => String::new(),
        };
        let valid = |n: &str| {
            let b = n.as_bytes();
            !b.is_empty() && (b[0] == b'_' || b[0].is_ascii_alphabetic() || b[0] >= 0x80) && b.iter().all(|&c| c == b'_' || c.is_ascii_alphanumeric() || c >= 0x80)
        };
        let mut count = 0;
        for (k, v) in arr.iter() {
            let key = match &k {
                crate::value::Key::Str(s) => String::from_utf8_lossy(s).into_owned(),
                crate::value::Key::Int(i) if flags & 0xff == 3 => i.to_string(),
                _ => continue,
            };
            let exists = f.vars.contains_key(&*key);
            let name = match flags & 0xff {
                1 if exists => continue,
                2 if exists => format!("{prefix}_{key}"),
                3 => format!("{prefix}_{key}"),
                _ => key,
            };
            if !valid(&name) || name == "this" {
                continue;
            }
            let name = self.intern(&name);
            *f.var_slot(name).borrow_mut() = v.clone();
            count += 1;
        }
        Ok(Value::Int(count))
    }

    /// Evaluate a constant expression from source (a built-in parameter's
    /// default, e.g. `ENT_QUOTES | ENT_SUBSTITUTE`).
    pub fn eval_const_source(&self, src: &str) -> Option<Value> {
        let code: &'static str = Box::leak(format!("<?php return {src};").into_boxed_str());
        let (file, diags) = phpx_syntax::check(code);
        if diags.iter().any(|d| d.is_error()) {
            return None;
        }
        let file: &'static ast::File = Box::leak(Box::new(file));
        let ast::StmtKind::Return(Some(e)) = &file.stmts.first()?.kind else { return None };
        let mut f = Frame::new(self.file_ctx(0), 0);
        self.eval(&mut f, e).ok()
    }

    /// `eval($code)`: run code in the caller's scope.
    fn eval_code(&self, f: &mut Frame, args: &'static [ast::Arg]) -> R<Value> {
        let Some(a) = args.first() else { return Ok(Value::Null) };
        let v = self.eval(f, &a.value)?;
        let code = String::from_utf8_lossy(&self.to_str(&v)?).into_owned();
        let (file, line) = self.here();
        let path = format!("{file}({line}) : eval()'d code");
        // Parsed code is kept (the AST is 'static): reuse it when the same
        // site evaluates the same code again, so eval in a loop doesn't grow.
        let key = format!("{path}\0{code}");
        let cached = self.eval_cache.borrow().get(&key).copied();
        let parsed = match cached {
            Some(x) => Ok(x),
            None => {
                let r = self.parse_file(std::path::Path::new(&path), format!("<?php {code}"));
                if let Ok(x) = &r {
                    self.eval_cache.borrow_mut().insert(key, *x);
                }
                r
            }
        };
        match parsed {
            Ok((id, parsed)) => {
                let saved = f.ctx.clone();
                let r = self.run_stmts_top(f, &parsed.stmts, id);
                f.ctx = saved;
                match r? {
                    crate::interp::Flow::Return(v) => Ok(v),
                    _ => Ok(Value::Null),
                }
            }
            Err(msg) => {
                let first = msg.lines().next().unwrap_or("syntax error").trim_start_matches("error").trim_start_matches(|c: char| c == '[' || c.is_ascii_alphanumeric() || c == ']').trim_start_matches(':').trim();
                Err(self.error("ParseError", first))
            }
        }
    }

    fn compact(&self, f: &mut Frame, args: &'static [ast::Arg]) -> R<Value> {
        let mut out = PhpArray::new();
        fn add(it: &Interp, f: &Frame, v: &Value, out: &mut PhpArray) {
            match v {
                Value::Str(s) => {
                    let n = String::from_utf8_lossy(s);
                    if let Some(slot) = f.vars.get(&*n) {
                        out.insert(Key::Str(s.clone()), slot.borrow().clone());
                    } else if n == "this" {
                        if let Some(o) = &f.this {
                            out.insert(Key::Str(s.clone()), Value::Object(o.clone()));
                        }
                    } else {
                        it.warn(&format!("compact(): Undefined variable ${n}"));
                    }
                }
                Value::Array(a) => a.values().for_each(|v| add(it, f, &v, out)),
                _ => {}
            }
        }
        for a in args {
            let v = self.eval(f, &a.value)?;
            add(self, f, &v, &mut out);
        }
        Ok(Value::array(out))
    }

    fn call_by_ref_builtin(
        &self,
        f: &mut Frame,
        name: &str,
        fun: crate::interp::BuiltinFn,
        args: &'static [ast::Arg],
        idxs: &[usize],
    ) -> R<Value> {
        let mut vals = vec![];
        let mut places = vec![];
        for (i, a) in args.iter().enumerate() {
            if idxs.contains(&i) {
                if !a.value.is_lvalue() {
                    return Err(self.error("Error", &format!("{name}(): Argument #{} could not be passed by reference", i + 1)));
                }
                let p = self.place(f, &a.value)?;
                vals.push(self.place_get(f, &p)?.unwrap_or(Value::Null));
                places.push((i, p));
            } else {
                vals.push(self.eval(f, &a.value)?);
            }
        }
        // Drop the variables' own references so in-place edits don't copy.
        for (_, p) in &places {
            self.place_set(f, p, Value::Null)?;
        }
        let r = crate::builtins::invoke(self, name, crate::arginfo::spec(name), fun, f, &mut vals);
        for (i, p) in places {
            let v = std::mem::take(&mut vals[i]);
            self.place_set(f, &p, v)?;
        }
        r
    }

    // ---- calling -----------------------------------------------------------

    pub fn to_callable(&self, f: &Frame, v: &Value) -> R<Callable> {
        match v {
            Value::Object(o) => {
                if let Internal::Closure(c) = &*o.internal() {
                    return Ok(c.clone());
                }
                match o.class.find_method("__invoke") {
                    Some(m) => Ok(Callable::Method { this: Some(o.clone()), class: o.class.clone(), method: m.clone(), static_class: o.class.clone() }),
                    None => Err(self.error("Error", "Object of type {} is not callable")),
                }
            }
            Value::Str(s) => {
                let s = String::from_utf8_lossy(s).into_owned();
                if let Some((cls, m)) = s.split_once("::") {
                    let class = self.string_class(f, cls)?;
                    return self.method_callable(f, None, &class, m);
                }
                self.find_function(&f.ctx, &format!("\\{}", s.trim_start_matches('\\')))
                    .ok_or_else(|| self.error("Error", &format!("Call to undefined function {s}()")))
            }
            Value::Array(a) if a.len() == 2 => {
                let target = a.get(&Key::Int(0)).unwrap_or_default();
                let m = a.get(&Key::Int(1)).unwrap_or_default();
                let m = self.to_string_lossy(&m)?;
                match target {
                    Value::Object(o) => {
                        let class = o.class.clone();
                        self.method_callable(f, Some(o), &class, &m)
                    }
                    Value::Str(c) => {
                        let class = self.string_class(f, &String::from_utf8_lossy(&c))?;
                        self.method_callable(f, None, &class, &m)
                    }
                    _ => Err(self.error("Error", "Array callback must have exactly two elements")),
                }
            }
            other => Err(self.error("Error", &format!("Value of type {} is not callable", other.type_name()))),
        }
    }

    fn method_callable(&self, f: &Frame, this: Option<Rc<Obj>>, class: &Rc<Class>, m: &str) -> R<Callable> {
        let m = m.strip_prefix("parent::").map(|x| x.to_string()).unwrap_or_else(|| m.to_string());
        match class.find_method(&m.to_ascii_lowercase()) {
            Some(method) => {
                let this = if method.is_static { None } else { this.or_else(|| f.this.clone()) };
                let static_class = this.as_ref().map(|o| o.class.clone()).unwrap_or_else(|| class.clone());
                Ok(Callable::Method { this, class: class.clone(), method: method.clone(), static_class })
            }
            None => Err(self.error("Error", &format!("Call to undefined method {}::{m}()", class.name))),
        }
    }

    /// `strlen(...)`, `$obj->m(...)`, `Foo::m(...)`: wrap as a Closure.
    pub fn callable_ref(&self, f: &mut Frame, inner: &'static ast::Expr) -> R<Value> {
        let c = match &inner.kind {
            ast::ExprKind::Name(n) => self
                .find_function(&f.ctx, n)
                .ok_or_else(|| self.error("Error", &format!("Call to undefined function {n}()")))?,
            ast::ExprKind::Prop { object, name, .. } => {
                let Value::Object(o) = self.eval(f, object)? else {
                    return Err(self.error("Error", "Call to a member function on non-object"));
                };
                let m = self.member_name(f, name)?;
                let class = o.class.clone();
                self.method_callable(f, Some(o), &class, &m)?
            }
            ast::ExprKind::ClassConst { class, name } => {
                let cls = self.class_ref(f, class)?;
                self.method_callable(f, None, &cls, &name.name)?
            }
            _ => {
                let v = self.eval(f, inner)?;
                self.to_callable(f, &v)?
            }
        };
        Ok(self.closure_object(c))
    }

    pub fn call_value(&self, f: &Frame, v: &Value, args: Vec<Value>) -> R<Value> {
        let c = self.to_callable(f, v)?;
        self.call_callable(&c, args, vec![])
    }

    pub fn call_callable(&self, c: &Callable, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Value> {
        match c {
            Callable::User(u) => {
                if named.is_empty() {
                    if let Some(v) = self.try_jit(u, &pos) {
                        return Ok(v);
                    }
                }
                self.invoke(
                Invoke {
                    params: &u.decl.params,
                    body: Body::Block(u.decl.body.as_deref().unwrap_or(&[])),
                    ret: u.decl.ret.as_ref(),
                    display: u.name.clone(),
                    func: u.name.clone(),
                    this: None,
                    class: None,
                    static_class: None,
                    ctx: u.ctx.clone(),
                    fn_id: u.decl as *const _ as usize,
                    captured: &[],
                    is_generator: u.decl.is_generator,
                },
                pos,
                named,
            )
            }
            Callable::Builtin(name, fun) => self.call_builtin(name, *fun, pos, named),
            Callable::Closure(cd) => self.call_closure(cd, pos, named),
            Callable::Method { this, method, static_class, .. } => {
                self.invoke_method(this.clone(), method, static_class.clone(), pos, named)
            }
        }
    }

    #[inline(never)]
    fn call_builtin(&self, name: &str, fun: crate::interp::BuiltinFn, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Value> {
        let spec = crate::arginfo::spec(name);
        let pos = match (named.is_empty(), spec) {
            (true, _) => pos,
            (false, Some(s)) => s.bind_named(self, pos, named)?,
            (false, None) => return Err(self.error("Error", &format!("phpx: named arguments to built-in {name}() are not supported yet"))),
        };
        let ctx = self.file_ctx(self.pos.get().file as usize);
        let mut frame = Frame::new(ctx, 0);
        let mut args: Vec<Value> = pos.into_iter().map(Value::into_deref).collect();
        crate::builtins::invoke(self, name, spec, fun, &mut frame, &mut args)
    }

    #[inline(never)]
    fn call_closure(&self, cd: &Rc<crate::interp::ClosureData>, mut pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Value> {
        let (params, body, ret, fn_id, is_generator): (&'static [ast::Param], Body, _, usize, bool) = match &cd.kind {
            ClosureKind::Closure(c) => (&c.params, Body::Block(&c.body), c.ret.as_ref(), *c as *const _ as usize, c.is_generator),
            ClosureKind::Arrow(a) => (&a.params, Body::Expr(&a.body), a.ret.as_ref(), *a as *const _ as usize, a.is_generator),
            ClosureKind::Wrap(inner) => return self.call_callable(inner, pos, named),
        };
        // Compiled closure (callbacks from built-ins, ...): enter it with
        // the arguments directly, as native call sites do.
        if named.is_empty() && !is_generator {
            let entry = self.native_entry(fn_id, || crate::compiler::Target {
                fn_id,
                display: cd.name.clone(),
                params,
                captured: cd.captured.iter().map(|(n, _, _)| *n).collect(),
                body: match body {
                    Body::Block(s) => crate::compiler::Body::Block(s),
                    Body::Expr(e) => crate::compiler::Body::Expr(e),
                },
                ctx: cd.ctx.clone(),
                class: cd.scope.clone(),
                ret,
            });
            if let Some(entry) = entry {
                let mut frame = Frame::for_call(cd.ctx.clone(), cd.name.clone(), fn_id);
                let mut out = Value::Null;
                // SAFETY: `pos` outlives the call; consumed slots are left null.
                let st = unsafe { crate::compiler::rt::enter_closure(self, cd, entry, &mut frame, pos.as_mut_ptr(), pos.len() as u64, &mut out) };
                match st {
                    Some(0) => return Ok(out),
                    Some(_) => return Err(self.pending.borrow_mut().take().expect("native error without pending exception")),
                    None => {}
                }
            }
        }
        self.invoke(
            Invoke {
                params,
                body,
                ret,
                display: cd.name.clone(),
                func: cd.name.clone(),
                this: cd.this.clone(),
                class: cd.scope.clone(),
                static_class: cd.static_class.clone(),
                ctx: cd.ctx.clone(),
                fn_id,
                captured: &cd.captured,
                is_generator,
            },
            pos,
            named,
        )
    }

    /// Run a hot numeric function natively. None = use the interpreter.
    #[inline(never)]
    pub(crate) fn try_jit(&self, u: &UserFn, pos: &[Value]) -> Option<Value> {
        use phpx_jit::{JitVal, Outcome};
        let mut guard = self.jit.try_borrow_mut().ok()?;
        let jit = guard.as_mut()?;
        if pos.len() != u.decl.params.len() || jit.rejected(u.decl) {
            return None;
        }
        if pos.len() > 28 {
            return None;
        }
        let mut buf = [JitVal::Int(0); 28];
        for (i, (v, p)) in pos.iter().zip(&u.decl.params).enumerate() {
            let jv = match v.deref() {
                Value::Int(n) => JitVal::Int(n),
                Value::Float(f) => JitVal::Float(f),
                Value::Bool(b) => JitVal::Bool(b),
                _ => return None,
            };
            // An int passed to a float parameter is widened (PHP does this in both modes).
            let jv = match (&p.ty, jv) {
                (Some(Type::Named(n)), JitVal::Int(i)) if n.eq_ignore_ascii_case("float") => JitVal::Float(i as f64),
                _ => jv,
            };
            buf[i] = jv;
        }
        let resolver = JitResolver(self);
        match jit.call(u.decl, &buf[..pos.len()], &resolver) {
            Outcome::Done(v) => Some(match v {
                JitVal::Int(n) => Value::Int(n),
                JitVal::Float(f) => Value::Float(f),
                JitVal::Bool(b) => Value::Bool(b),
            }),
            Outcome::Deopt | Outcome::Skip => None,
        }
    }

    pub fn invoke_method(
        &self,
        this: Option<Rc<Obj>>,
        m: &Rc<Method>,
        static_class: Rc<Class>,
        pos: Vec<Value>,
        named: Vec<(Rc<str>, Value)>,
    ) -> R<Value> {
        match &m.body {
            MethodBody::Native(fun) => {
                if !named.is_empty() {
                    return Err(self.error("Error", &format!("phpx: named arguments to {}() are not supported yet", m.name)));
                }
                fun(self, this.as_ref(), &static_class, pos.into_iter().map(Value::into_deref).collect())
            }
            MethodBody::User(func) => {
                let class = self.classes.borrow().get(&*m.declaring).cloned();
                let Some(body) = &func.body else {
                    let cname = class.as_ref().map(|c| c.name.to_string()).unwrap_or_default();
                    return Err(self.error("Error", &format!("Cannot call abstract method {cname}::{}()", m.name)));
                };
                let display = format!("{}::{}", class.as_ref().map(|c| &*c.name).unwrap_or(""), m.name);
                self.invoke(
                    Invoke {
                        params: &func.params,
                        body: Body::Block(body),
                        ret: func.ret.as_ref(),
                        display: Rc::from(display),
                        func: m.name.clone(),
                        this: if m.is_static { None } else { this },
                        class,
                        static_class: Some(static_class),
                        ctx: m.ctx.clone(),
                        fn_id: *func as *const _ as usize,
                        captured: &[],
                        is_generator: func.is_generator,
                    },
                    pos,
                    named,
                )
            }
        }
    }

    /// Run a user function. This frame stays on the native stack for the
    /// whole call (and every PHP call nested in it), so everything not
    /// needed while the body runs lives in separate non-inlined helpers.
    fn invoke(&self, inv: Invoke, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Value> {
        if phpx_runtime::stack_remaining().is_some_and(|r| r < STACK_RESERVE) {
            return Err(self.stack_overflow());
        }
        let saved_pos = self.pos.get();
        let mut f = self.bind_args(&inv, pos, named, saved_pos)?;
        if inv.is_generator {
            return Ok(self.start_generator(f, inv.body, saved_pos));
        }
        if let Some(r) = self.native_call(&inv, &mut f) {
            // (Compiled code checked the declared return type.)
            self.pos.set(saved_pos);
            return r;
        }
        let _call = crate::trace::enter(self, &f, inv.params, saved_pos);
        let result = match inv.body {
            Body::Block(stmts) => match self.exec_block(&mut f, stmts)? {
                Flow::Return(v) => v,
                _ => Value::Null,
            },
            Body::Expr(e) => {
                if inv.ctx.file != crate::interp::PRELUDE_FILE {
                    self.pos.set(crate::interp::Pos { file: inv.ctx.file as u32, off: e.span.start });
                }
                self.eval(&mut f, e)?
            }
        };
        // Checked before leaving: a TypeError points at the `return`.
        let result = if inv.ret.is_none() { result } else { self.check_return(&inv, result)? };
        self.pos.set(saved_pos);
        Ok(result)
    }

    #[inline(never)]
    fn native_call(&self, inv: &Invoke, f: &mut Frame) -> Option<R<Value>> {
        if self.native.try_borrow().map(|n| n.is_none()).unwrap_or(true) {
            return None;
        }
        let mk = || crate::compiler::Target {
            fn_id: inv.fn_id,
            display: inv.display.clone(),
            params: inv.params,
            captured: inv.captured.iter().map(|(n, _, _)| *n).collect(),
            body: match inv.body {
                Body::Block(s) => crate::compiler::Body::Block(s),
                Body::Expr(e) => crate::compiler::Body::Expr(e),
            },
            ctx: inv.ctx.clone(),
            class: inv.class.clone(),
            ret: inv.ret,
        };
        self.try_native(inv.fn_id, mk, f)
    }

    /// Recursive built-ins (serialize, json, ...) check this before going
    /// deeper: true when less than 256 KiB of task stack is left.
    pub fn stack_low(&self) -> bool {
        phpx_runtime::stack_remaining().is_some_and(|r| r < 256 * 1024)
    }

    #[inline(never)]
    pub fn stack_overflow(&self) -> Ctrl {
        self.error(
            "Error",
            &format!("Maximum call stack size of {} bytes reached. Infinite recursion?", phpx_runtime::stack_size()),
        )
    }

    /// Build the callee's frame: captured variables, then parameters
    /// (positional, named, defaults, variadics, by-reference, type checks).
    #[inline(never)]
    fn bind_args(&self, inv: &Invoke, pos: Vec<Value>, mut named: Vec<(Rc<str>, Value)>, saved_pos: crate::interp::Pos) -> R<Frame> {
        let caller_strict = self.file_strict(self.pos.get().file as usize);
        let mut f = Frame::new(inv.ctx.clone(), 0);
        f.vars = crate::interp::Vars::with_capacity(inv.params.len() + inv.captured.len() + 4);
        f.this = inv.this.clone();
        f.class = inv.class.clone();
        f.static_class = inv.static_class.clone();
        f.func = inv.func.clone();
        f.fn_id = inv.fn_id;
        for (name, v, by_ref) in inv.captured {
            let s = match v {
                Value::Ref(s) if *by_ref => s.clone(),
                v => slot(v.clone()),
            };
            f.vars.insert(name, s);
        }

        let required = inv.params.iter().filter(|p| p.default.is_none() && !p.variadic).count();
        let mut pos_iter = pos.iter().cloned().enumerate().peekable();
        for (i, p) in inv.params.iter().enumerate() {
            let pname: &'static str = &p.name.name;
            if p.variadic {
                let mut rest = PhpArray::new();
                let keep = |v: Value| if p.by_ref && matches!(v, Value::Ref(_)) { v } else { v.into_deref() };
                for (_, v) in pos_iter.by_ref() {
                    rest.push(keep(v));
                }
                for (k, v) in named.drain(..) {
                    rest.insert(Key::Str(crate::value::pbytes(k.as_bytes())), keep(v));
                }
                let rest = match &p.ty {
                    Some(ty) => {
                        let mut checked = PhpArray::new();
                        for (k, v) in rest.iter() {
                            checked.insert(k.clone(), self.check_param(inv, i, p, ty, v.clone(), caller_strict)?);
                        }
                        checked
                    }
                    None => rest,
                };
                f.vars.insert(pname, slot(Value::array(rest)));
                break;
            }
            let v = if let Some((_, v)) = pos_iter.next_if(|(j, _)| *j == i) {
                if named.iter().any(|(n, _)| &**n == pname) {
                    return Err(self.error("Error", &format!("Named parameter ${pname} overwrites previous argument")));
                }
                v
            } else if let Some(at) = named.iter().position(|(n, _)| &**n == pname) {
                named.remove(at).1
            } else if let Some(d) = &p.default {
                self.eval(&mut f, d)?
            } else {
                self.pos.set(saved_pos);
                let (file, line) = self.here();
                let quant = if required == inv.params.len() { "exactly" } else { "at least" };
                return Err(self.error(
                    "ArgumentCountError",
                    &format!(
                        "Too few arguments to function {}(), {} passed in {file} on line {line} and {quant} {required} expected",
                        inv.display,
                        pos.len()
                    ),
                ));
            };
            if p.by_ref {
                // Bind the caller's cell; type coercion writes back into it, like PHP.
                let cell = match v {
                    Value::Ref(s) => s,
                    other => slot(other),
                };
                if let Some(ty) = &p.ty {
                    let cur = cell.borrow().clone();
                    if !(cur.is_null() && is_null_default(p)) {
                        let nv = self.check_param(inv, i, p, ty, cur, caller_strict)?;
                        *cell.borrow_mut() = nv;
                    }
                }
                f.vars.insert(pname, cell);
                continue;
            }
            let v = v.into_deref();
            let v = match &p.ty {
                Some(ty) if !(v.is_null() && is_null_default(p)) => self.check_param(inv, i, p, ty, v, caller_strict)?,
                _ => v,
            };
            if p.promote.is_some() {
                if let Some(o) = &inv.this {
                    o.set_prop(pname, v.clone());
                }
            }
            f.vars.insert(pname, slot(v));
        }
        if let Some((n, _)) = named.first() {
            return Err(self.error("Error", &format!("Unknown named parameter ${n}")));
        }
        f.args = pos.into_iter().map(Value::into_deref).collect();
        Ok(f)
    }

    /// The body runs lazily in its own task; the caller gets a Generator.
    #[inline(never)]
    fn start_generator(&self, f: Frame, body: Body, saved_pos: crate::interp::Pos) -> Value {
        self.pos.set(saved_pos);
        self.spawn_generator(move |it, g| {
            let mut f = f;
            f.gen = Some(g);
            match body {
                Body::Block(stmts) => match it.exec_block(&mut f, stmts)? {
                    Flow::Return(v) => Ok(v),
                    _ => Ok(Value::Null),
                },
                Body::Expr(e) => {
                    it.eval(&mut f, e)?;
                    Ok(Value::Null)
                }
            }
        })
    }

    #[inline(never)]
    fn check_return(&self, inv: &Invoke, result: Value) -> R<Value> {
        match inv.ret {
            None => Ok(result),
            Some(ty) => self.check_return_type(ty, result, &inv.ctx, inv.class.as_ref(), inv.static_class.as_ref(), &inv.display),
        }
    }

    pub(crate) fn check_return_type(
        &self,
        ty: &Type,
        result: Value,
        ctx: &NsCtx,
        class: Option<&Rc<Class>>,
        static_class: Option<&Rc<Class>>,
        display: &str,
    ) -> R<Value> {
        match ty {
            Type::Named(n) if n.eq_ignore_ascii_case("void") => Ok(Value::Null),
            Type::Named(n) if n.eq_ignore_ascii_case("never") => {
                Err(self.error("TypeError", &format!("{display}(): never-returning function must not implicitly return")))
            }
            ty => match self.coerce(result.clone(), ty, ctx, class, static_class, ctx.strict)? {
                Some(v) => Ok(v),
                None => Err(self.error(
                    "TypeError",
                    &format!("{display}(): Return value must be of type {}, {} returned", type_str(ty), result.debug_type()),
                )),
            },
        }
    }

    fn check_param(&self, inv: &Invoke, i: usize, p: &ast::Param, ty: &Type, v: Value, strict: bool) -> R<Value> {
        match self.coerce(v.clone(), ty, &inv.ctx, inv.class.as_ref(), inv.static_class.as_ref(), strict)? {
            Some(v) => Ok(v),
            None => Err(self.param_type_error(&inv.display, i, p, ty, &v, inv.ctx.file)),
        }
    }

    /// TypeError for a user function's argument, like PHP: thrown at the
    /// parameter's declaration, naming the call site when user code made
    /// the call (not a built-in calling back). `pos` must still be the
    /// call site.
    pub fn param_type_error(&self, display: &str, i: usize, p: &ast::Param, ty: &Type, v: &Value, file: usize) -> Ctrl {
        let mut msg = format!("{display}(): Argument #{} (${}) must be of type {}, {} given", i + 1, p.name.name, type_str(ty), v.debug_type());
        if file == crate::interp::PRELUDE_FILE {
            // A built-in class's method: reported at the call, like PHP.
            return self.error("TypeError", &msg);
        }
        let here = self.pos.get();
        if here != self.builtin_pos.get() {
            let (path, line) = self.here();
            msg.push_str(&format!(", called in {path} on line {line}"));
        }
        self.pos.set(crate::interp::Pos { file: file as u32, off: p.name.span.start as u32 });
        let e = self.error("TypeError", &msg);
        self.pos.set(here);
        e
    }

    pub fn file_strict(&self, file: usize) -> bool {
        self.file_strict.borrow().get(file).copied().unwrap_or(false)
    }

    // ---- methods -----------------------------------------------------------

    fn check_method_visible(&self, f: &Frame, cls: &Class, m: &Method) -> R<()> {
        if self.member_visible(f.class.as_deref(), m.vis, &m.declaring) {
            return Ok(());
        }
        let vis = if m.vis == Vis::Private { "private" } else { "protected" };
        let scope = f.class.as_ref().map(|c| format!("scope {}", c.name)).unwrap_or_else(|| "global scope".into());
        Err(self.error("Error", &format!("Call to {vis} method {}::{}() from {scope}", cls.name, m.name)))
    }

    pub fn method_call(&self, f: &Frame, o: &Rc<Obj>, name: &str, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Value> {
        let lname = name.to_ascii_lowercase();
        match o.class.find_method_from(&lname, f.class.as_deref()).cloned() {
            Some(m) => {
                if self.check_method_visible(f, &o.class, &m).is_err() && o.class.find_method("__call").is_some() {
                    return self.magic_call(o, name, pos);
                }
                self.check_method_visible(f, &o.class, &m)?;
                self.invoke_method(Some(o.clone()), &m, o.class.clone(), pos, named)
            }
            None if o.class.find_method("__call").is_some() => self.magic_call(o, name, pos),
            None => {
                // `$obj->prop(...)` where prop holds a closure is NOT a method call in PHP.
                Err(self.error("Error", &format!("Call to undefined method {}::{name}()", o.class.name)))
            }
        }
    }

    fn magic_call(&self, o: &Rc<Obj>, name: &str, pos: Vec<Value>) -> R<Value> {
        let m = o.class.find_method("__call").cloned().unwrap();
        let args = vec![Value::str(name), Value::array(PhpArray::from_list(pos))];
        self.invoke_method(Some(o.clone()), &m, o.class.clone(), args, vec![])
    }

    /// Internal method call (iterators, ArrayAccess, __toString...): no visibility check.
    pub fn call_method(&self, o: &Rc<Obj>, name: &str, args: Vec<Value>) -> R<Value> {
        match o.class.find_method(&name.to_ascii_lowercase()).cloned() {
            Some(m) => self.invoke_method(Some(o.clone()), &m, o.class.clone(), args, vec![]),
            None if o.class.find_method("__call").is_some() => self.magic_call(o, name, args),
            None => Err(self.error("Error", &format!("Call to undefined method {}::{name}()", o.class.name))),
        }
    }

    pub fn static_call(
        &self,
        f: &mut Frame,
        cref: &'static ast::ClassRef,
        cls: Rc<Class>,
        name: &str,
        pos: Vec<Value>,
        named: Vec<(Rc<str>, Value)>,
    ) -> R<Value> {
        let forwarding = matches!(cref, ast::ClassRef::Name(n) if matches!(&*n.to_ascii_lowercase(), "self" | "parent" | "static"));
        let lname = name.to_ascii_lowercase();
        let this_fits = f.this.as_ref().filter(|o| o.class.is_subclass_of(&cls)).cloned();
        match cls.find_method(&lname).cloned() {
            Some(m) => {
                self.check_method_visible(f, &cls, &m)?;
                if m.is_static {
                    let sc = if forwarding { f.static_class.clone().unwrap_or(cls.clone()) } else { cls.clone() };
                    self.invoke_method(None, &m, sc, pos, named)
                } else if let Some(this) = this_fits {
                    let sc = this.class.clone();
                    self.invoke_method(Some(this), &m, sc, pos, named)
                } else {
                    Err(self.error("Error", &format!("Non-static method {}::{}() cannot be called statically", cls.name, m.name)))
                }
            }
            None => {
                if let (Some(this), Some(_)) = (&this_fits, cls.find_method("__call")) {
                    return self.magic_call(this, name, pos);
                }
                if let Some(m) = cls.find_method("__callstatic").cloned() {
                    let args = vec![Value::str(name), Value::array(PhpArray::from_list(pos))];
                    let sc = if forwarding { f.static_class.clone().unwrap_or(cls.clone()) } else { cls.clone() };
                    return self.invoke_method(None, &m, sc, args, vec![]);
                }
                Err(self.error("Error", &format!("Call to undefined method {}::{name}()", cls.name)))
            }
        }
    }

    pub fn find_static_prop(&self, f: &mut Frame, cref: &'static ast::ClassRef, name: &str) -> R<Rc<StaticProp>> {
        let cls = self.class_ref(f, cref)?;
        match cls.static_props.get(name) {
            Some(sp) => {
                if !self.member_visible(f.class.as_deref(), sp.vis, &sp.declaring) {
                    return Err(self.error("Error", &format!("Cannot access non-public property {}::${name}", cls.name)));
                }
                Ok(sp.clone())
            }
            None => Err(self.error("Error", &format!("Access to undeclared static property {}::${name}", cls.name))),
        }
    }

    pub fn class_const(&self, f: &mut Frame, cref: &'static ast::ClassRef, name: &str) -> R<Value> {
        if name.eq_ignore_ascii_case("class") {
            if let ast::ClassRef::Name(n) = cref {
                if !matches!(&*n.to_ascii_lowercase(), "self" | "static" | "parent") {
                    return Ok(Value::str(&self.resolve_class_name(&f.ctx, n)));
                }
            }
            if let ast::ClassRef::Dynamic(e) = cref {
                return match self.eval(f, e)? {
                    Value::Object(o) => Ok(Value::str(&o.class.name)),
                    _ => Err(self.error("TypeError", "Cannot use \"::class\" on non-object")),
                };
            }
            let cls = self.class_ref(f, cref)?;
            return Ok(Value::str(&cls.name));
        }
        let cls = self.class_ref(f, cref)?;
        self.get_class_const(&cls, name)
    }

    pub fn get_class_const(&self, cls: &Rc<Class>, name: &str) -> R<Value> {
        let slot = cls.consts.borrow().get(name).cloned();
        match slot {
            Some(ConstSlot::Ready(v)) => Ok(v),
            Some(ConstSlot::Evaluating) => Err(self.error("Error", "Cannot declare self-referencing constant")),
            Some(ConstSlot::Pending { expr, declaring, ctx }) => {
                cls.consts.borrow_mut().insert(Rc::from(name), ConstSlot::Evaluating);
                let decl = self.classes.borrow().get(&*declaring).cloned();
                let mut fr = Frame::new(ctx, 0);
                fr.class = decl.clone();
                fr.static_class = decl;
                let r = self.eval(&mut fr, expr);
                let v = match r {
                    Ok(v) => v,
                    Err(e) => {
                        cls.consts.borrow_mut().insert(Rc::from(name), ConstSlot::Pending { expr, declaring, ctx: fr.ctx.clone() });
                        return Err(e);
                    }
                };
                cls.consts.borrow_mut().insert(Rc::from(name), ConstSlot::Ready(v.clone()));
                Ok(v)
            }
            Some(ConstSlot::EnumCase { name: cn, value, ctx }) => {
                // Enum cases are singletons stored back into the slot.
                let o = Obj::new(cls.clone(), Internal::None);
                o.props.borrow_mut().insert(Rc::from("name"), Value::str(&cn));
                if let Some(e) = value {
                    let mut fr = Frame::new(ctx, 0);
                    fr.class = Some(cls.clone());
                    fr.static_class = Some(cls.clone());
                    let v = self.eval(&mut fr, e)?;
                    o.props.borrow_mut().insert(Rc::from("value"), v);
                }
                let v = Value::Object(o);
                cls.consts.borrow_mut().insert(Rc::from(name), ConstSlot::Ready(v.clone()));
                Ok(v)
            }
            None => Err(self.error("Error", &format!("Undefined constant {}::{name}", cls.name))),
        }
    }

    // ---- objects -----------------------------------------------------------

    pub fn instantiate(&self, f: &Frame, cls: &Rc<Class>, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Rc<Obj>> {
        let o = self.instantiate_begin(f, cls)?;
        self.instantiate_finish(cls, o, pos, named)
    }

    /// `new`, first half: the checks and the object (PHP creates it before
    /// evaluating the constructor's arguments).
    pub fn instantiate_begin(&self, f: &Frame, cls: &Rc<Class>) -> R<Rc<Obj>> {
        let what = match cls.kind {
            ClassKind::Interface => Some("interface"),
            ClassKind::Trait => Some("trait"),
            ClassKind::Enum => Some("enum"),
            ClassKind::Class if cls.is_abstract => Some("abstract class"),
            ClassKind::Class => None,
        };
        if let Some(w) = what {
            return Err(self.error("Error", &format!("Cannot instantiate {w} {}", cls.name)));
        }
        // Like PHP: the object exists (and takes a handle) before the
        // constructor's visibility is checked.
        let o = self.new_instance(cls)?;
        if !cls.plain_new && cls.is_a("throwable") {
            self.set_throw_site(&o);
        }
        if let Some(ctor) = &cls.ctor {
            if !self.member_visible(f.class.as_deref(), ctor.vis, &ctor.declaring) {
                let vis = if ctor.vis == Vis::Private { "private" } else { "protected" };
                let scope = f.class.as_ref().map(|c| format!("scope {}", c.name)).unwrap_or_else(|| "global scope".into());
                return Err(self.error("Error", &format!("Call to {vis} {}::__construct() from {scope}", cls.name)));
            }
        }
        Ok(o)
    }

    /// `new`, second half: run the constructor.
    pub fn instantiate_finish(&self, cls: &Rc<Class>, o: Rc<Obj>, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Rc<Obj>> {
        if let Some(ctor) = cls.ctor.clone() {
            self.invoke_method(Some(o.clone()), &ctor, cls.clone(), pos, named)?;
        }
        Ok(o)
    }

    /// A declared property's default (None for typed props without one).
    pub fn prop_default(&self, cls: &Rc<Class>, p: &crate::class::PropDecl) -> R<Option<Value>> {
        Ok(match &p.default {
            PropDefault::Uninit => None,
            PropDefault::Value(v) => Some(v.clone()),
            PropDefault::Expr(e) => {
                let decl = self.classes.borrow().get(&*self.lower.get(&p.declaring)).cloned();
                let mut fr = Frame::new(p.ctx.clone(), 0);
                fr.class = decl.or_else(|| Some(cls.clone()));
                fr.static_class = Some(cls.clone());
                Some(self.eval(&mut fr, e)?)
            }
        })
    }

    /// Names of traits a class uses (its own, not its parents').
    pub fn class_traits(&self, cls: &Class) -> Vec<Rc<str>> {
        cls.traits.clone()
    }

    pub fn init_props(&self, o: &Rc<Obj>, cls: &Rc<Class>) -> R<()> {
        if let Some(t) = &*cls.prop_template.borrow() {
            *o.props.borrow_mut() = crate::props::Props::with_shape(cls.shape.clone(), t.clone());
            return Ok(());
        }
        self.init_props_slow(o, cls)
    }

    /// A new instance of `cls` with default properties (no constructor).
    pub fn new_instance(&self, cls: &Rc<Class>) -> R<Rc<Obj>> {
        if let Some(t) = &*cls.prop_template.borrow() {
            return Ok(Obj::from_template(cls, t));
        }
        let o = Obj::new(cls.clone(), Internal::None);
        self.init_props_slow(&o, cls)?;
        Ok(o)
    }

    fn init_props_slow(&self, o: &Rc<Obj>, cls: &Rc<Class>) -> R<()> {
        // Defaults are constant expressions: evaluate once per class, then
        // copy the whole map.
        let mut vals = Vec::with_capacity(cls.props.len());
        for p in &cls.props {
            let v = match &p.default {
                PropDefault::Uninit => continue,
                PropDefault::Value(v) => v.clone(),
                PropDefault::Expr(e) => {
                    let decl = self.classes.borrow().get(&*self.lower.get(&p.declaring)).cloned();
                    let mut fr = Frame::new(p.ctx.clone(), 0);
                    fr.class = decl.clone().or_else(|| Some(cls.clone()));
                    fr.static_class = Some(cls.clone());
                    self.eval(&mut fr, e)?
                }
            };
            vals.push(Some(v));
        }
        // Typed properties without a default stay uninitialized (empty slot).
        let vals: Vec<Option<Value>> = {
            let mut it = vals.into_iter();
            cls.props
                .iter()
                .map(|p| if matches!(p.default, PropDefault::Uninit) { None } else { it.next().flatten() })
                .collect()
        };
        *o.props.borrow_mut() = crate::props::Props::with_shape(cls.shape.clone(), vals.clone());
        *cls.prop_template.borrow_mut() = Some(vals);
        Ok(())
    }

    pub fn clone_object(&self, o: &Rc<Obj>) -> R<Rc<Obj>> {
        if o.class.is_enum() {
            return Err(self.error("Error", &format!("Trying to clone an uncloneable object of class {}", o.class.name)));
        }
        let internal = match &*o.internal() {
            Internal::None => Internal::None,
            Internal::Closure(c) => Internal::Closure(c.clone()),
            Internal::Promise(p) => Internal::Promise(p.clone()),
            Internal::Generator(_) => {
                return Err(self.error("Error", "Trying to clone an uncloneable object of class Generator"));
            }
            Internal::Data(d) => match d.clone_data() {
                Some(d) => Internal::Data(d),
                None => return Err(self.error("Error", &format!("Trying to clone an uncloneable object of class {}", o.class.name))),
            },
        };
        let n = Obj::new(o.class.clone(), internal);
        *n.props.borrow_mut() = o.props.borrow().clone();
        if o.class.find_method("__clone").is_some() {
            self.call_method(&n, "__clone", vec![])?;
        }
        Ok(n)
    }

    // ---- class declaration ---------------------------------------------------

    pub fn deps_ready(&self, c: &ast::Class, ctx: &NsCtx) -> bool {
        // Early binding only uses already-declared classes (no autoload), like PHP.
        let ready = |n: &String| self.classes.borrow().contains_key(&*self.lower.get(&self.resolve_class_name(ctx, n)));
        let traits = c.members.iter().filter_map(|m| if let Member::TraitUse { traits, .. } = m { Some(traits) } else { None }).flatten();
        c.extends.iter().all(ready) && c.implements.iter().all(ready) && traits.into_iter().all(ready)
    }

    pub fn declare_anon_class(&self, f: &Frame, c: &'static ast::Class) -> R<Rc<Class>> {
        let key = c as *const _ as usize;
        if let Some(cls) = self.anon_classes.borrow().get(&key) {
            return Ok(cls.clone());
        }
        let ctx = f.ctx.clone();
        let cls = self.build_class(c, &ctx, Some(format!("class@anonymous#{}", self.anon_classes.borrow().len())))?;
        self.anon_classes.borrow_mut().insert(key, cls.clone());
        Ok(cls)
    }

    pub fn declare_class(&self, c: &'static ast::Class, ctx: &Rc<NsCtx>) -> R<Rc<Class>> {
        let cls = self.build_class(c, ctx, None)?;
        self.declared.borrow_mut().insert(c as *const _ as usize);
        Ok(cls)
    }

    fn build_class(&self, c: &'static ast::Class, ctx: &Rc<NsCtx>, anon_name: Option<String>) -> R<Rc<Class>> {
        let name = match &anon_name {
            Some(n) => n.clone(),
            None => qualify(&ctx.ns, &c.name.as_ref().unwrap().name),
        };
        let lname = self.lower.get(&name);
        if anon_name.is_none() && self.classes.borrow().contains_key(&*lname) {
            return Err(self.error("Error", &format!("Cannot declare class {name}, because the name is already in use")));
        }
        let display: Rc<str> = Rc::from(if anon_name.is_some() { "class@anonymous" } else { name.as_str() });

        let (parent, ifaces): (Option<Rc<Class>>, Vec<&String>) = match c.kind {
            ClassKind::Interface => (None, c.extends.iter().collect()),
            _ => (
                match c.extends.first() {
                    Some(p) => Some(self.lookup_class(&self.resolve_class_name(ctx, p))?),
                    None => None,
                },
                c.implements.iter().collect(),
            ),
        };
        if let Some(p) = &parent {
            if p.kind != ClassKind::Class {
                return Err(self.error("Error", &format!("Class {name} cannot extend {} {}", kind_word(p.kind), p.name)));
            }
        }
        let mut ancestors: Vec<Rc<str>> = vec![lname.clone()];
        let mut consts: IndexMap<Rc<str>, ConstSlot> = IndexMap::new();
        let mut methods: crate::value::FxIndexMap<Rc<str>, Rc<Method>> = Default::default();
        let mut used_traits: Vec<Rc<str>> = vec![];
        let mut shadowed: Vec<(Rc<str>, Rc<str>, Rc<str>)> = parent.as_ref().map(|p| p.shadowed.clone()).unwrap_or_default();
        let mut trait_keys: Vec<Rc<str>> = vec![];
        let mut props: Vec<PropDecl> = vec![];
        let mut static_props: IndexMap<Rc<str>, Rc<StaticProp>> = IndexMap::new();
        if let Some(p) = &parent {
            ancestors.extend(p.ancestors.iter().cloned());
            consts.extend(p.consts.borrow().iter().map(|(k, v)| (k.clone(), v.clone())));
            methods.extend(p.methods.iter().map(|(k, v)| (k.clone(), v.clone())));
            props.extend(p.props.iter().cloned());
            static_props.extend(p.static_props.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        for i in &ifaces {
            let iface = self.lookup_class(&self.resolve_class_name(ctx, i))?;
            if iface.kind != ClassKind::Interface {
                return Err(self.error("Error", &format!("{name} cannot implement {} - it is not an interface", iface.name)));
            }
            ancestors.extend(iface.ancestors.iter().cloned());
            for (k, v) in iface.consts.borrow().iter() {
                consts.entry(k.clone()).or_insert_with(|| v.clone());
            }
            for (k, v) in &iface.methods {
                methods.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        if c.kind == ClassKind::Enum {
            ancestors.push(Rc::from("unitenum"));
            if c.enum_type.is_some() {
                ancestors.push(Rc::from("backedenum"));
            }
        }

        let mut cases = vec![];
        let mut case_values: Vec<(Rc<str>, Option<&'static ast::Expr>)> = vec![];
        for m in &c.members {
            match m {
                Member::TraitUse { traits, adaptations, .. } => {
                    for t in traits {
                        let tc = self.lookup_class(&self.resolve_class_name(ctx, t))?;
                        if tc.kind != ClassKind::Trait {
                            return Err(self.error("Error", &format!("{name} cannot use {} - it is not a trait", tc.name)));
                        }
                        used_traits.push(tc.name.clone());
                        let excluded = |mname: &str| {
                            adaptations.iter().any(|a| match a {
                                ast::TraitAdaptation::Insteadof { trait_, method, instead_of } => {
                                    method.name.eq_ignore_ascii_case(mname)
                                        && instead_of.iter().any(|x| self.resolve_class_name(ctx, x).eq_ignore_ascii_case(&tc.name))
                                        && !self.resolve_class_name(ctx, trait_).eq_ignore_ascii_case(&tc.name)
                                }
                                _ => false,
                            })
                        };
                        for (k, tm) in &tc.methods {
                            if excluded(k) {
                                continue;
                            }
                            let mut nm = (**tm).clone();
                            nm.declaring = lname.clone();
                            if !(nm.is_abstract && methods.contains_key(k)) {
                                methods.insert(k.clone(), Rc::new(nm));
                                trait_keys.push(k.clone());
                            }
                        }
                        for a in adaptations {
                            if let ast::TraitAdaptation::Alias { trait_, method, visibility, alias } = a {
                                if let Some(t) = trait_ {
                                    if !self.resolve_class_name(ctx, t).eq_ignore_ascii_case(&tc.name) {
                                        continue;
                                    }
                                }
                                let Some(tm) = tc.methods.get(&*method.name.to_ascii_lowercase()) else { continue };
                                let mut nm = (**tm).clone();
                                nm.declaring = lname.clone();
                                if let Some(v) = visibility {
                                    nm.vis = match v.as_str() {
                                        "private" => Vis::Private,
                                        "protected" => Vis::Protected,
                                        _ => Vis::Public,
                                    };
                                }
                                let key: Rc<str> = match alias {
                                    Some(al) => {
                                        nm.name = Rc::from(&*al.name);
                                        self.lower.get(&al.name)
                                    }
                                    None => self.lower.get(&method.name),
                                };
                                methods.insert(key, Rc::new(nm));
                            }
                        }
                        for p in &tc.props {
                            let mut np = p.clone();
                            np.declaring = display.clone();
                            upsert_prop(&mut props, np, &mut shadowed);
                        }
                        for (k, sp) in &tc.static_props {
                            static_props.insert(
                                k.clone(),
                                Rc::new(StaticProp {
                                    value: std::cell::RefCell::new(None),
                                    init: sp.init,
                                    vis: sp.vis,
                                    declaring: lname.clone(),
                                    ctx: sp.ctx.clone(),
                                }),
                            );
                        }
                        for (k, v) in tc.consts.borrow().iter() {
                            consts.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                }
                Member::Const { items, .. } => {
                    for (n, e) in items {
                        consts.insert(Rc::from(&*n.name), ConstSlot::Pending { expr: e, declaring: lname.clone(), ctx: ctx.clone() });
                    }
                }
                Member::Property { modifiers, ty, items, .. } => {
                    for (n, d) in items {
                        let pname: Rc<str> = Rc::from(&*n.name);
                        if modifiers.static_ {
                            static_props.insert(
                                pname,
                                Rc::new(StaticProp {
                                    value: std::cell::RefCell::new(None),
                                    init: d.as_ref(),
                                    vis: Vis::of(modifiers),
                                    declaring: lname.clone(),
                                    ctx: ctx.clone(),
                                }),
                            );
                            continue;
                        }
                        let default = match (d, ty) {
                            (Some(e), _) => const_default(e),
                            (None, Some(_)) => PropDefault::Uninit,
                            (None, None) => PropDefault::Value(Value::Null),
                        };
                        upsert_prop(
                            &mut props,
                            PropDecl {
                                name: pname,
                                default,
                                ty: ty.as_ref(),
                                vis: Vis::of(modifiers),
                                readonly: modifiers.readonly || c.modifiers.readonly,
                                declaring: display.clone(),
                                ctx: ctx.clone(),
                            },
                            &mut shadowed,
                        );
                    }
                }
                Member::Method { modifiers, func } => {
                    if func.name.name.eq_ignore_ascii_case("__construct") {
                        for p in func.params.iter().filter(|p| p.promote.is_some()) {
                            let m = p.promote.as_ref().unwrap();
                            upsert_prop(
                                &mut props,
                                PropDecl {
                                    name: Rc::from(&*p.name.name),
                                    default: if p.ty.is_some() { PropDefault::Uninit } else { PropDefault::Value(Value::Null) },
                                    ty: p.ty.as_ref(),
                                    vis: Vis::of(m),
                                    readonly: m.readonly || c.modifiers.readonly,
                                    declaring: display.clone(),
                                    ctx: ctx.clone(),
                                },
                                &mut shadowed,
                            );
                        }
                    }
                    let key = self.lower.get(&func.name.name);
                    methods.insert(
                        key,
                        Rc::new(Method {
                            name: Rc::from(&*func.name.name),
                            body: MethodBody::User(func),
                            is_static: modifiers.static_,
                            is_abstract: modifiers.abstract_ || func.body.is_none(),
                            is_final: modifiers.final_,
                            vis: Vis::of(modifiers),
                            declaring: lname.clone(),
                            ctx: ctx.clone(),
                        }),
                    );
                }
                Member::EnumCase { name: cn, value, .. } => {
                    cases.push(Rc::from(&*cn.name));
                    case_values.push((Rc::from(&*cn.name), value.as_ref()));
                }
            }
        }
        // Trait methods list after the class's own (get_class_methods order).
        for k in trait_keys {
            if let Some(m) = methods.shift_remove(&k) {
                methods.insert(k, m);
            }
        }
        // Native methods: per class, plus the shared enum helpers.
        for ((cl, m), fun) in &self.natives {
            let applies = **cl == *lname || (*cl == "*enum" && c.kind == ClassKind::Enum);
            if applies {
                let (is_static, mname) = match m.strip_prefix("static ") {
                    Some(n) => (true, n),
                    None => (false, *m),
                };
                if *cl == "*enum" && (mname == "from" || mname == "tryfrom") && c.enum_type.is_none() {
                    continue;
                }
                methods.insert(
                    Rc::from(mname),
                    Rc::new(Method {
                        name: Rc::from(native_display_name(mname)),
                        body: MethodBody::Native(*fun),
                        is_static,
                        is_abstract: false,
                        is_final: false,
                        vis: Vis::Public,
                        declaring: lname.clone(),
                        ctx: ctx.clone(),
                    }),
                );
            }
        }
        if methods.contains_key("__tostring") && !ancestors.iter().any(|a| &**a == "stringable") {
            ancestors.push(Rc::from("stringable"));
        }
        let is_abstract = c.modifiers.abstract_;
        if c.kind == ClassKind::Class && !is_abstract {
            let mut missing: Vec<String> = methods
                .values()
                .filter(|m| m.is_abstract)
                .map(|m| {
                    let decl = self.classes.borrow().get(&*m.declaring).map(|c| c.name.to_string()).unwrap_or_default();
                    format!("{decl}::{}", m.name)
                })
                .collect();
            if !missing.is_empty() {
                missing.sort();
                let n = missing.len();
                return Err(self.error(
                    "Error",
                    &format!(
                        "Class {name} contains {n} abstract method{} and must therefore be declared abstract or implement the remaining methods ({})",
                        if n == 1 { "" } else { "s" },
                        missing.join(", ")
                    ),
                ));
            }
        }
        let shape = crate::props::Shape::declared(props.iter().map(|p| p.name.clone()).collect());
        let ctor = methods.get("__construct").cloned();
        let plain_new = c.kind == ClassKind::Class && !is_abstract && !ancestors.iter().any(|a| &**a == "throwable");
        let cls = Rc::new(Class {
            name: display.clone(),
            lname: lname.clone(),
            kind: c.kind,
            parent,
            ancestors,
            is_abstract,
            consts: std::cell::RefCell::new(consts),
            static_props,
            props,
            has_destruct: methods.contains_key("__destruct"),
            traits: used_traits,
            prop_template: Default::default(),
            ctor,
            plain_new,
            shadowed,
            shape,
            methods,
            cases: cases.clone(),
            enum_backed: c.enum_type.is_some(),
            decl: c,
            decl_ctx: ctx.clone(),
        });
        if anon_name.is_none() {
            self.classes.borrow_mut().insert(lname.clone(), cls.clone());
        }
        for (cn, ve) in case_values {
            cls.consts.borrow_mut().insert(cn.clone(), ConstSlot::EnumCase { name: cn, value: ve, ctx: ctx.clone() });
        }
        Ok(cls)
    }

    // ---- types ---------------------------------------------------------------

    /// Check `v` against `ty`, converting scalars like PHP does in coercive
    /// mode. `None` = TypeError.
    pub fn coerce(
        &self,
        v: Value,
        ty: &Type,
        ctx: &NsCtx,
        self_class: Option<&Rc<Class>>,
        static_class: Option<&Rc<Class>>,
        strict: bool,
    ) -> R<Option<Value>> {
        match ty {
            Type::Nullable(t) => {
                if v.is_null() {
                    Ok(Some(v))
                } else {
                    self.coerce(v, t, ctx, self_class, static_class, strict)
                }
            }
            Type::Union(ts) => {
                for t in ts {
                    if self.type_matches(&v, t, ctx, self_class, static_class)? {
                        return Ok(Some(v));
                    }
                }
                let names: Vec<String> = ts.iter().map(|t| type_str(t).to_ascii_lowercase()).collect();
                if matches!(v, Value::Int(_)) && names.iter().any(|n| n == "float") {
                    return Ok(scalar_coerce(&v, "float", strict));
                }
                if strict || !matches!(v, Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_)) {
                    return Ok(None);
                }
                for pref in ["int", "float", "string", "bool"] {
                    if names.iter().any(|n| n == pref) {
                        if let Some(x) = scalar_coerce(&v, pref, false) {
                            return Ok(Some(x));
                        }
                    }
                }
                Ok(None)
            }
            Type::Intersection(ts) => {
                for t in ts {
                    if !self.type_matches(&v, t, ctx, self_class, static_class)? {
                        return Ok(None);
                    }
                }
                Ok(Some(v))
            }
            Type::Named(n) => {
                if self.type_matches(&v, ty, ctx, self_class, static_class)? {
                    return Ok(Some(v));
                }
                let l = n.to_ascii_lowercase();
                if matches!(l.as_str(), "int" | "float" | "string" | "bool") {
                    if l == "string" && !strict {
                        if let Value::Object(o) = &v {
                            if o.class.find_method("__tostring").is_some() {
                                return Ok(Some(Value::Str(self.to_str(&v)?)));
                            }
                        }
                    }
                    return Ok(scalar_coerce(&v, &l, strict));
                }
                Ok(None)
            }
        }
    }

    fn type_matches(&self, v: &Value, t: &Type, ctx: &NsCtx, self_class: Option<&Rc<Class>>, static_class: Option<&Rc<Class>>) -> R<bool> {
        let Type::Named(n) = t else {
            return Ok(self.coerce(v.clone(), t, ctx, self_class, static_class, true)?.is_some());
        };
        // Lowercase into a stack buffer: type names are checked on every
        // call, and the built-in ones are short.
        let mut buf = [0u8; 16];
        let l: &str = if n.len() <= buf.len() {
            for (d, c) in buf.iter_mut().zip(n.bytes()) {
                *d = c.to_ascii_lowercase();
            }
            std::str::from_utf8(&buf[..n.len()]).unwrap_or("")
        } else {
            ""
        };
        Ok(match l {
            "mixed" => true,
            "int" => matches!(v, Value::Int(_)),
            "float" => matches!(v, Value::Float(_)),
            "string" => matches!(v, Value::Str(_)),
            "bool" => matches!(v, Value::Bool(_)),
            "false" => matches!(v, Value::Bool(false)),
            "true" => matches!(v, Value::Bool(true)),
            "array" => matches!(v, Value::Array(_)),
            "null" | "void" => v.is_null(),
            "object" => matches!(v, Value::Object(_)),
            "iterable" => matches!(v, Value::Array(_)) || matches!(v, Value::Object(o) if o.class.is_a("traversable")),
            "callable" => {
                let f = Frame::new(Rc::new(NsCtx {
                    ns: ctx.ns.clone(),
                    uses: HashMap::new(),
                    use_fns: HashMap::new(),
                    use_consts: HashMap::new(),
                    file: ctx.file,
                    strict: ctx.strict,
                }), 0);
                matches!(v, Value::Str(_) | Value::Array(_) | Value::Object(_)) && self.to_callable(&f, v).is_ok()
            }
            "self" => matches!((v, self_class), (Value::Object(o), Some(c)) if o.class.is_a(&c.lname)),
            "static" => matches!((v, static_class), (Value::Object(o), Some(c)) if o.class.is_a(&c.lname)),
            "parent" => matches!((v, self_class.and_then(|c| c.parent.as_ref())), (Value::Object(o), Some(c)) if o.class.is_a(&c.lname)),
            "never" => false,
            _ => match v {
                Value::Object(o) => {
                    let key = n.as_ptr() as usize;
                    let cached = self.type_names.borrow().get(&key).cloned();
                    let lname = match cached {
                        Some(l) => l,
                        None => {
                            let full = self.resolve_class_name(ctx, n);
                            let l = self.lower.get(&full);
                            self.type_names.borrow_mut().insert(key, l.clone());
                            l
                        }
                    };
                    o.class.is_a(&lname)
                }
                _ => false,
            },
        })
    }

    pub fn include(&self, f: &mut Frame, kind: ast::IncludeKind, path: &str) -> R<Value> {
        use ast::IncludeKind::*;
        let p = std::path::Path::new(path);
        let candidates: Vec<std::path::PathBuf> = if p.is_absolute() {
            vec![p.to_path_buf()]
        } else {
            let cur = self.file_path(f.ctx.file);
            let dir = std::path::Path::new(&cur).parent().map(|d| d.to_path_buf()).unwrap_or_default();
            vec![dir.join(p), p.to_path_buf()]
        };
        let found = candidates.into_iter().find(|c| c.is_file());
        let Some(found) = found else {
            let func = match kind {
                Include => "include",
                IncludeOnce => "include_once",
                Require => "require",
                RequireOnce => "require_once",
            };
            if matches!(kind, Require | RequireOnce) {
                return Err(self.error("Error", &format!("Failed opening required '{path}'")));
            }
            self.warn(&format!("{func}({path}): Failed to open stream: No such file or directory"));
            return Ok(Value::Bool(false));
        };
        let canon = found.canonicalize().unwrap_or(found.clone());
        if matches!(kind, IncludeOnce | RequireOnce) && self.included.borrow().contains(&canon) {
            return Ok(Value::Bool(true));
        }
        self.included.borrow_mut().insert(canon.clone());
        if canon.ends_with("vendor/autoload.php") && canon.parent().is_some_and(|d| d.join("composer/autoload_psr4.php").is_file()) {
            return self.composer_boot(f, &canon);
        }
        let src = std::fs::read(&found).map_err(|e| self.error("Error", &e.to_string()))?;
        let src = String::from_utf8_lossy(&src).into_owned();
        match self.parse_file(&canon, src) {
            Ok((id, file)) => {
                self.register_strict(id, file);
                self.run_file_in(f, file, id)
            }
            Err(msg) => {
                eprint!("{msg}");
                Err(Ctrl::Exit(255))
            }
        }
    }

    pub fn register_strict(&self, id: usize, file: &ast::File) {
        let strict = file.stmts.iter().any(|s| match &s.kind {
            ast::StmtKind::Declare { items, .. } => items
                .iter()
                .any(|(k, v)| k.name.eq_ignore_ascii_case("strict_types") && matches!(v.kind, ast::ExprKind::Int(1))),
            _ => false,
        });
        let mut fs = self.file_strict.borrow_mut();
        if fs.len() <= id {
            fs.resize(id + 1, false);
        }
        fs[id] = strict;
    }
}

struct JitResolver<'a>(&'a Interp);

impl phpx_jit::Resolver for JitResolver<'_> {
    fn resolve(&self, from: &'static ast::Function, name: &str) -> Option<&'static ast::Function> {
        let ctx = self.0.decl_ctx.borrow().get(&(from as *const ast::Function as usize)).cloned()?;
        match self.0.find_function(&ctx, name)? {
            Callable::User(u) => Some(u.decl),
            _ => None,
        }
    }
}

fn is_null_default(p: &ast::Param) -> bool {
    matches!(p.default.as_ref().map(|d| &d.kind), Some(ast::ExprKind::Null))
}

/// Add or override a declared property. Redeclaring an ancestor's private
/// property creates a second, separate property (PHP semantics): the
/// ancestor's moves to a mangled key `\0Class\0name`.
fn upsert_prop(props: &mut Vec<PropDecl>, p: PropDecl, shadowed: &mut Vec<(Rc<str>, Rc<str>, Rc<str>)>) {
    match props.iter().position(|x| x.name == p.name) {
        Some(i) if props[i].vis == Vis::Private && !props[i].declaring.eq_ignore_ascii_case(&p.declaring) => {
            let old = &mut props[i];
            let mangled: Rc<str> = Rc::from(format!("\0{}\0{}", old.declaring, old.name).as_str());
            shadowed.push((Rc::from(old.declaring.to_ascii_lowercase().as_str()), old.name.clone(), mangled.clone()));
            old.name = mangled;
            props.push(p);
        }
        Some(i) => props[i] = p,
        None => props.push(p),
    }
}

/// A property key as shown to PHP code (`\0Class\0name` -> `name`).
pub fn plain_prop(name: &str) -> &str {
    match name.strip_prefix('\0') {
        Some(rest) => rest.split_once('\0').map_or(name, |(_, n)| n),
        None => name,
    }
}

/// Literal defaults are stored as values; anything else is evaluated per object.
fn const_default(e: &'static ast::Expr) -> PropDefault {
    use ast::ExprKind::*;
    match &e.kind {
        Int(n) => PropDefault::Value(Value::Int(*n)),
        Float(f) => PropDefault::Value(Value::Float(*f)),
        Str(s) => PropDefault::Value(Value::str(s)),
        Bool(b) => PropDefault::Value(Value::Bool(*b)),
        Null => PropDefault::Value(Value::Null),
        Array(items) if items.is_empty() => PropDefault::Value(Value::array(PhpArray::new())),
        _ => PropDefault::Expr(e),
    }
}

fn kind_word(k: ClassKind) -> &'static str {
    match k {
        ClassKind::Class => "class",
        ClassKind::Interface => "interface",
        ClassKind::Trait => "trait",
        ClassKind::Enum => "enum",
    }
}

fn native_display_name(lname: &str) -> &str {
    match lname {
        "tryfrom" => "tryFrom",
        "bindto" => "bindTo",
        "fromcallable" => "fromCallable",
        "allsettled" => "allSettled",
        "issettled" => "isSettled",
        "getreturn" => "getReturn",
        "__invoke" => "__invoke",
        other => other,
    }
}

/// Coercive-mode scalar conversion (strict mode only allows int -> float).
fn scalar_coerce(v: &Value, target: &str, strict: bool) -> Option<Value> {
    use crate::value::{parse_numeric, Num};
    match (target, v) {
        ("float", Value::Int(n)) => return Some(Value::Float(*n as f64)),
        _ if strict => return None,
        _ => {}
    }
    match target {
        "int" => match v {
            Value::Float(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.2e18 => Some(Value::Int(*f as i64)),
            Value::Float(f) if f.is_finite() && f.abs() < 9.2e18 => Some(Value::Int(*f as i64)),
            Value::Str(s) => match parse_numeric(s)? {
                Num::Int(n) => Some(Value::Int(n)),
                Num::Float(f) if f.fract() == 0.0 && f.abs() < 9.2e18 => Some(Value::Int(f as i64)),
                Num::Float(_) => None,
            },
            Value::Bool(b) => Some(Value::Int(*b as i64)),
            _ => None,
        },
        "float" => match v {
            Value::Str(s) => parse_numeric(s).map(|n| Value::Float(n.as_f64())),
            Value::Bool(b) => Some(Value::Float(*b as i64 as f64)),
            _ => None,
        },
        "string" => match v {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) => v.scalar_bytes().map(Value::Str),
            _ => None,
        },
        "bool" => match v {
            Value::Int(_) | Value::Float(_) | Value::Str(_) => Some(Value::Bool(v.truthy())),
            _ => None,
        },
        _ => None,
    }
}
