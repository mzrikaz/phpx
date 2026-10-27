//! Calls, name resolution, classes and type coercion.

use rustc_hash::FxHashMap as HashMap;
use std::rc::Rc;

use phpx_syntax::ast::{self, ClassKind, Member, Stmt, Type};

use crate::class::{Class, ConstSlot, Method, MethodBody, PropDecl, PropDefault, StaticProp, Vis};
use crate::interp::{qualify, slot, Callable, ClosureKind, Ctrl, Flow, Frame, Interp, NsCtx, UserFn, R};
use crate::value::{Internal, Key, Obj, PhpArray, Value};

thread_local! {
    /// Classes this thread's earlier requests built, by declaration (like
    /// opcache's immutable classes): declaring one again reuses it when
    /// nothing it was built from changed.
    static BUILT: std::cell::RefCell<rustc_hash::FxHashMap<usize, Rc<Class>>> = Default::default();
    /// Classes the current request declared.
    static IN_USE: std::cell::RefCell<Vec<Rc<Class>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// How long a server worker trusts a resolved include (PHP's
/// opcache.revalidate_freq default).
const REVALIDATE: std::time::Duration = std::time::Duration::from_secs(2);

thread_local! {
    /// Server workers: (including file id, or MAX for an absolute path;
    /// the path as written) -> the file it led to, and when that was
    /// checked on disk; and its file id and syntax tree as of parse
    /// generation `gen` (see `crate::interp::parse_gen`).
    static RESOLVED: std::cell::RefCell<HashMap<(usize, String), Resolved>> = Default::default();
}

struct Resolved {
    canon: std::path::PathBuf,
    checked: std::time::Instant,
    file: (usize, &'static ast::File, u64),
}

impl Resolved {
    fn new(canon: std::path::PathBuf, found: (usize, &'static ast::File)) -> Self {
        Resolved { canon, checked: std::time::Instant::now(), file: (found.0, found.1, crate::interp::parse_gen()) }
    }
}

/// Lets RESOLVED be searched with a borrowed path.
trait ResolvedKey {
    fn key(&self) -> (usize, &str);
}

impl ResolvedKey for (usize, String) {
    fn key(&self) -> (usize, &str) {
        (self.0, &self.1)
    }
}

impl ResolvedKey for (usize, &str) {
    fn key(&self) -> (usize, &str) {
        *self
    }
}

impl<'a> std::borrow::Borrow<dyn ResolvedKey + 'a> for (usize, String) {
    fn borrow(&self) -> &(dyn ResolvedKey + 'a) {
        self
    }
}

impl PartialEq for dyn ResolvedKey + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for dyn ResolvedKey + '_ {}

impl std::hash::Hash for dyn ResolvedKey + '_ {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state)
    }
}

/// The prelude's classes are in use by every request (see Interp::load_prelude).
pub(crate) fn use_classes(classes: impl Iterator<Item = Rc<Class>>) {
    IN_USE.with(|u| u.borrow_mut().extend(classes));
}

/// What a class declaration's parents, interfaces and traits resolve to:
/// (full name, lowercase name).
pub(crate) struct ClassDeps {
    extends: Vec<(String, Rc<str>)>,
    implements: Vec<(String, Rc<str>)>,
    traits: Vec<(String, Rc<str>)>,
}

thread_local! {
    /// ClassDeps by declaration, with the context they were resolved in
    /// (held, so its address cannot be reused by another).
    static CLASS_DEPS: std::cell::RefCell<rustc_hash::FxHashMap<usize, (Rc<NsCtx>, Rc<ClassDeps>)>> = Default::default();
}

/// A new request: the classes the last one declared go back to their
/// state as built (constants unevaluated, static properties unset).
/// Destructors queued by what they held are dropped, as PHP runs none
/// after shutdown.
pub(crate) fn reset_classes() {
    let used = IN_USE.with(|u| std::mem::take(&mut *u.borrow_mut()));
    let mut held: Vec<Box<dyn std::any::Any>> = vec![];
    for cls in &used {
        // (Only a class whose constants were evaluated needs them back.)
        let evaluated = {
            let c = cls.consts.borrow();
            c.len() != cls.consts_init.len() || c.values().any(|s| matches!(s, ConstSlot::Ready(_) | ConstSlot::Evaluating))
        };
        if evaluated {
            held.push(Box::new(cls.consts.replace(cls.consts_init.clone())));
        }
        for sp in cls.static_props.values() {
            if *sp.declaring == *cls.lname {
                held.push(Box::new(sp.value.borrow_mut().take()));
            }
        }
        held.push(Box::new(cls.prop_template.borrow_mut().take()));
    }
    drop(held);
    drop(crate::value::DESTRUCT_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut())));
}

/// By-reference parameter positions not in the arginfo table (variadic
/// ones expanded, and phpx-only built-ins).
const BY_REF: &[(&str, usize)] = &[
    ("sort", 0), ("rsort", 0), ("usort", 0), ("uasort", 0), ("uksort", 0), ("ksort", 0), ("krsort", 0),
    ("asort", 0), ("arsort", 0), ("shuffle", 0), ("array_push", 0), ("array_pop", 0), ("array_shift", 0),
    ("array_unshift", 0), ("array_splice", 0), ("array_walk", 0), ("preg_match", 2), ("preg_match_all", 2),
    ("settype", 0), ("natsort", 0), ("natcasesort", 0), ("array_multisort", 0), ("array_multisort", 1), ("array_multisort", 2), ("array_multisort", 3), ("array_multisort", 4), ("array_multisort", 5), ("array_multisort", 6), ("array_multisort", 7), ("array_multisort", 8), ("array_multisort", 9), ("array_multisort", 10), ("array_multisort", 11), ("fscanf", 2), ("fscanf", 3), ("fscanf", 4), ("fscanf", 5), ("fscanf", 6), ("fscanf", 7),
    ("preg_replace", 4), ("parse_str", 1), ("sscanf", 2), ("sscanf", 3), ("sscanf", 4), ("sscanf", 5), ("sscanf", 6), ("sscanf", 7), ("next", 0), ("prev", 0), ("reset", 0), ("end", 0), ("headers_sent", 0), ("headers_sent", 1), ("similar_text", 2), ("openssl_random_pseudo_bytes", 1), ("grapheme_extract", 4), ("preg_replace_callback", 4), ("str_replace", 3), ("str_ireplace", 3),
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
    /// `function &f()`: may hand back a reference (see `Interp::want_ref`).
    by_ref: bool,
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
        Callable::Builtin(..) | Callable::Magic { .. } => None,
    }
}

pub fn method_params(m: &Method) -> Option<&'static [ast::Param]> {
    match &m.body {
        MethodBody::User(f) => Some(&f.params),
        MethodBody::Native(_) => None,
    }
}

/// `self::`, `parent::` and `static::` forward the called class.
fn is_forwarding(cref: &ast::ClassRef) -> bool {
    matches!(cref, ast::ClassRef::Name(n) if ["self", "parent", "static"].iter().any(|w| n.eq_ignore_ascii_case(w)))
}

/// __call/__callStatic's `$arguments`: the positional ones, then named
/// ones under their names.
fn magic_args(pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> Value {
    let mut a = PhpArray::from_list(pos);
    for (k, v) in named {
        a.insert(crate::value::Key::Str(crate::value::pstr(&k)), v);
    }
    Value::array(a)
}

fn is_ref_target(e: &ast::Expr) -> bool {
    matches!(e.kind, ast::ExprKind::Var(_) | ast::ExprKind::Index { .. } | ast::ExprKind::Prop { .. } | ast::ExprKind::StaticProp { .. })
}

/// A type as PHP prints it: built-in names in lower case, and a union's
/// members in PHP's order (classes first as written, then static,
/// callable, object, array, string, int, float, bool, ..., null).
pub fn type_str(t: &Type) -> String {
    type_str_by(t, &|n: &str| n.to_string())
}

/// A declared type as PHP prints it, class names resolved by `class`.
pub fn type_str_by(t: &Type, class: &dyn Fn(&str) -> String) -> String {
    const ORDER: &[&str] = &["static", "callable", "object", "array", "string", "int", "float", "bool", "false", "true", "void", "never", "mixed", "null"];
    let named = |n: &str| {
        let n = n.trim_start_matches('\\');
        let l = n.to_ascii_lowercase();
        match ORDER.iter().position(|t| *t == l) {
            Some(r) => (r + 1, l),
            None if l == "iterable" => (0, l),
            None => (0, class(n)),
        }
    };
    match t {
        Type::Named(n) => named(n).1,
        Type::Nullable(t) => format!("?{}", type_str_by(t, class)),
        Type::Union(ts) => {
            let mut parts: Vec<(usize, usize, String)> = ts
                .iter()
                .enumerate()
                .map(|(i, t)| match t {
                    Type::Named(n) => {
                        let (r, s) = named(n);
                        (r, i, s)
                    }
                    Type::Intersection(_) => (0, i, format!("({})", type_str_by(t, class))),
                    t => (0, i, type_str_by(t, class)),
                })
                .collect();
            parts.sort();
            parts.into_iter().map(|p| p.2).collect::<Vec<_>>().join("|")
        }
        Type::Intersection(ts) => ts.iter().map(|t| type_str_by(t, class)).collect::<Vec<_>>().join("&"),
    }
}

impl Interp {
    // ---- name resolution ---------------------------------------------------

    /// A declared type as PHP prints it in errors: class names resolved
    /// in the declaration's context.
    /// `self` and `parent` name `class` and its parent, as PHP shows them.
    pub fn type_str_in(&self, t: &Type, ctx: &NsCtx, class: Option<&str>) -> String {
        type_str_by(t, &|n| {
            let cls = || class.and_then(|c| self.classes.borrow().get(&*c.to_ascii_lowercase()).cloned());
            match &*n.to_ascii_lowercase() {
                "self" => class.map_or_else(|| n.to_string(), str::to_string),
                "parent" => cls().and_then(|c| c.parent.as_ref().map(|p| p.name.to_string())).unwrap_or_else(|| n.to_string()),
                _ => self.resolve_class_name(ctx, n),
            }
        })
    }

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
        // Autoloaders see the name without a leading backslash.
        let full = full.trim_start_matches('\\');
        let l = self.lower.get(full);
        if let Some(c) = self.classes.borrow().get(&*l) {
            return Ok(Some(c.clone()));
        }
        if !self.autoloading.borrow_mut().insert(l.to_string()) {
            return Ok(None);
        }
        let loaders = self.autoloaders.borrow().clone();
        // Loading runs other files; errors and traces stay at the use site.
        let at = self.pos.get();
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
        self.pos.set(at);
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
            ast::ClassRef::Dynamic(e) => {
                let v = self.eval(f, e)?;
                self.class_of_value(&v)
            }
            ast::ClassRef::Anonymous(c) => self.declare_anon_class(f, c),
        }
    }

    /// A class reference that resolves to `cls` (through the per-node
    /// class cache). Its name starts with \x01, so it can't collide with a
    /// name from source and call sites keep it out of their per-site caches.
    pub fn dyn_class_ref(&self, cls: &Rc<Class>) -> &'static ast::ClassRef {
        let key = Rc::as_ptr(cls) as usize;
        if let Some(r) = self.dyn_class_refs.borrow().get(&key) {
            return r;
        }
        let r: &'static ast::ClassRef = Box::leak(Box::new(ast::ClassRef::Name(format!("\x01{}", cls.name))));
        let ast::ClassRef::Name(n) = r else { unreachable!() };
        self.class_cache.borrow_mut().insert(n.as_ptr() as usize, cls.clone());
        self.dyn_class_refs.borrow_mut().insert(key, r);
        r
    }

    /// The class a runtime value names (`$x::...`, `new $x`).
    pub fn class_of_value(&self, v: &Value) -> R<Rc<Class>> {
        match v {
            Value::Object(o) => Ok(o.class.clone()),
            Value::Str(s) => self.lookup_class(&crate::pstr::lossy(s)),
            _ => Err(self.error("Error", "Class name must be a valid object or a string")),
        }
    }

    /// `$x::class`.
    pub fn class_name_of_value(&self, v: &Value) -> R<Value> {
        match v {
            Value::Object(o) => Ok(o.class.name_value.clone()),
            v => Err(self.error("TypeError", &format!("Cannot use \"::class\" on {}", v.type_name()))),
        }
    }

    fn named_class(&self, f: &Frame, n: &str) -> R<Rc<Class>> {
        let special = |w: &str| n.eq_ignore_ascii_case(w);
        let key = if special("self") {
            "self"
        } else if special("static") {
            "static"
        } else if special("parent") {
            "parent"
        } else {
            ""
        };
        match key {
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
        let v = found.ok_or_else(|| self.error("Error", &format!("Undefined constant \"{}\"", name.trim_start_matches('\\'))))?;
        if let Some(msg) = crate::consts::deprecation(name.rsplit('\\').next().unwrap_or(name)) {
            self.deprecated(msg);
        }
        Ok(v)
    }

    // ---- arguments ---------------------------------------------------------

    pub fn eval_args(&self, f: &mut Frame, args: &'static [ast::Arg]) -> R<(Vec<Value>, Vec<(Rc<str>, Value)>)> {
        // The call keeps its own position: calls nested in the arguments
        // move it while they run.
        let at = self.pos.get();
        let r = self.eval_args_inner(f, args);
        self.pos.set(at);
        r
    }

    fn eval_args_inner(&self, f: &mut Frame, args: &'static [ast::Arg]) -> R<(Vec<Value>, Vec<(Rc<str>, Value)>)> {
        let mut pos = Vec::with_capacity(args.len());
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
                        Value::Str(s) => named.push((Rc::from(crate::pstr::lossy(&s).as_ref()), v)),
                        _ => pos.push(v),
                    }
                }
            } else if let Some(n) = &a.name {
                named.push((self.arg_name(&n.name), v));
            } else {
                if !named.is_empty() {
                    return Err(self.error("Error", "Cannot use positional argument after named argument"));
                }
                pos.push(v);
            }
        }
        Ok((pos, named))
    }

    /// Diagnostics from here on point at `off` (PHP attributes each
    /// operation in a multi-line statement to its own line).
    #[inline]
    pub(crate) fn mark(&self, f: &Frame, off: u32) {
        if f.ctx.file != crate::interp::PRELUDE_FILE {
            self.pos.set(crate::interp::Pos { file: f.ctx.file as u32, off });
        }
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
        let at = self.pos.get();
        let mut pos = Vec::with_capacity(args.len());
        let mut named = vec![];
        for a in args {
            let by_ref = match &a.name {
                Some(n) => params.iter().any(|p| p.by_ref && p.name.name == n.name),
                None => by_ref_at(pos.len()),
            };
            let v = if by_ref && is_ref_target(&a.value) { Value::Ref(self.make_ref(f, &a.value)?) } else { self.eval(f, &a.value)? };
            match &a.name {
                Some(n) => named.push((self.arg_name(&n.name), v)),
                None => pos.push(v),
            }
        }
        self.pos.set(at);
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

    #[inline]
    fn inline_builtin(&self, f: &mut Frame, bname: &'static str, fun: crate::interp::BuiltinFn, arg: &'static ast::Expr) -> Option<R<Value>> {
        let test: fn(&Value) -> Option<Value> = match bname {
            "is_null" => |v| Some(Value::Bool(matches!(v, Value::Null))),
            "is_array" => |v| Some(Value::Bool(matches!(v, Value::Array(_)))),
            "is_string" => |v| Some(Value::Bool(matches!(v, Value::Str(_)))),
            "is_int" | "is_integer" | "is_long" => |v| Some(Value::Bool(matches!(v, Value::Int(_)))),
            "is_bool" => |v| Some(Value::Bool(matches!(v, Value::Bool(_)))),
            "is_float" | "is_double" => |v| Some(Value::Bool(matches!(v, Value::Float(_)))),
            "is_object" => |v| Some(Value::Bool(matches!(v, Value::Object(_)))),
            "count" | "sizeof" => |v| match v {
                Value::Array(a) => Some(Value::Int(a.len() as i64)),
                _ => None,
            },
            "strlen" => |v| match v {
                Value::Str(s) => Some(Value::Int(s.len() as i64)),
                _ => None,
            },
            _ => return None,
        };
        let v = match self.eval(f, arg) {
            Ok(v) => v.into_deref(),
            Err(e) => return Some(Err(e)),
        };
        Some(match test(&v) {
            Some(r) => Ok(r),
            None => crate::builtins::invoke(self, bname, crate::arginfo::spec(bname), fun, f, &mut [v]),
        })
    }

    /// Whether the function a call site's (leaked) name names exists.
    pub(crate) fn function_exists_at(&self, f: &Frame, name: &'static str) -> bool {
        self.call_cache.borrow().contains_key(&(name.as_ptr() as usize)) || self.find_function(&f.ctx, name).is_some()
    }

    /// Resolve a called name. Each call site's name lives at a unique
    /// (leaked) address; functions can't be redeclared, so resolving once
    /// is enough (PHP's run-time cache keeps a namespaced call's fallback
    /// to the global function the same way).
    #[inline(never)]
    pub(crate) fn resolve_call(&self, f: &Frame, name: &'static str) -> R<Callable> {
        let site = name.as_ptr() as usize;
        if let Some(c) = self.call_cache.borrow().get(&site) {
            return Ok(c.clone());
        }
        let c = self.find_function(&f.ctx, name);
        match c {
            Some(c) => {
                self.call_cache.borrow_mut().insert(site, c.clone());
                Ok(c)
            }
            None => Err(self.error("Error", &format!("Call to undefined function {}()", name.trim_start_matches('\\')))),
        }
    }

    #[inline(never)]
    fn call_builtin_named(&self, f: &mut Frame, bname: &'static str, fun: crate::interp::BuiltinFn, args: &'static [ast::Arg]) -> R<Value> {
        // Type tests, count() and strlen() answered inline, as compiled code
        // does (the general path for anything they don't cover).
        if let [a] = args {
            if !a.spread && a.name.is_none() {
                if let Some(r) = self.inline_builtin(f, bname, fun, &a.value) {
                    return r;
                }
            }
        }
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
            "func_get_args" => {
                // Parameters as they are now, extra arguments as passed.
                let rec = unsafe { crate::trace::top().as_ref() };
                let params: &[ast::Param] = match rec {
                    Some(r) if !r.params.is_null() && std::ptr::eq(r.frame, f) => unsafe { std::slice::from_raw_parts(r.params, r.nparams) },
                    _ => &[],
                };
                let fixed = params.iter().position(|p| p.variadic).unwrap_or(params.len());
                let vals: Vec<Value> = f
                    .args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| match params.get(i).filter(|_| i < fixed).and_then(|p| f.vars.get(&p.name.name)) {
                        Some(s) => s.borrow().deref(),
                        None => a.clone(),
                    })
                    .collect();
                return Ok(Value::array(PhpArray::from_list(vals)));
            }
            "get_defined_vars" => {
                let mut out = PhpArray::new();
                for (name, s) in f.vars.iter() {
                    out.insert(Key::Str(crate::value::pstr(name)), s.borrow().clone());
                }
                return Ok(Value::array(out));
            }
            "func_num_args" => return Ok(Value::Int(f.args.len() as i64)),
            "get_called_class" => {
                return match &f.static_class {
                    Some(c) => Ok(Value::str(&c.name)),
                    None => Err(self.error("Error", "get_called_class() must be called from within a class")),
                }
            }
            "get_class" | "get_parent_class" if args.is_empty() => {
                let c = if bname == "get_class" { f.class.clone() } else { f.class.as_ref().and_then(|c| c.parent.clone()) };
                return Ok(c.map(|c| Value::str(c.php_name())).unwrap_or(Value::Bool(false)));
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
            Some(v) => crate::pstr::lossy(&self.to_str(v)?).into_owned(),
            None => String::new(),
        };
        let valid = |n: &str| {
            let b = n.as_bytes();
            !b.is_empty() && (b[0] == b'_' || b[0].is_ascii_alphabetic() || b[0] >= 0x80) && b.iter().all(|&c| c == b'_' || c.is_ascii_alphanumeric() || c >= 0x80)
        };
        let mut count = 0;
        for (k, v) in arr.iter() {
            let key = match &k {
                crate::value::Key::Str(s) => crate::pstr::lossy(s).into_owned(),
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
        let code = crate::pstr::lossy(&self.to_str(&v)?).into_owned();
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
                    let n = crate::pstr::lossy(s);
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

    pub(crate) fn call_by_ref_builtin(
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
                // array_multisort() takes its flags by value ("prefer-ref").
                if !a.value.is_lvalue() && name == "array_multisort" {
                    vals.push(self.eval(f, &a.value)?);
                    continue;
                }
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
        // Drop plain variables' own references so in-place edits don't copy
        // (not properties: a typed one can't hold null). Arrays left in a
        // property are remembered: if the built-in hands the same one back,
        // there's nothing to write (and writing would undo changes the call
        // made to that property meanwhile, as array_walk() callbacks do).
        let mut kept: Vec<Option<usize>> = vec![];
        let mut live = vec![];
        for (i, p) in &places {
            if name == "array_walk" && !(p.path.is_empty() && matches!(p.root, crate::expr::Root::Var(_))) {
                // array_walk() over a property: make it a reference, so the
                // walk sees what the callback does to it.
                let cell = crate::interp::slot(std::mem::take(&mut vals[*i]));
                self.place_set(f, p, Value::Ref(cell.clone()))?;
                vals[*i] = Value::Ref(cell);
                live.push(*i);
                kept.push(None);
                continue;
            }
            if p.path.is_empty() && matches!(p.root, crate::expr::Root::Var(_)) {
                self.place_set(f, p, Value::Null)?;
                kept.push(None);
            } else {
                kept.push(match &vals[*i] {
                    Value::Array(a) => Some(Rc::as_ptr(a) as usize),
                    _ => None,
                });
            }
        }
        let r = crate::builtins::invoke(self, name, crate::arginfo::spec(name), fun, f, &mut vals);
        for ((i, p), orig) in places.into_iter().zip(kept) {
            if live.contains(&i) {
                continue;
            }
            let v = std::mem::take(&mut vals[i]);
            if let (Some(o), Value::Array(a)) = (orig, &v) {
                if Rc::as_ptr(a) as usize == o {
                    continue;
                }
            }
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
            Value::Str(bytes) => {
                if let Some(c) = self.str_callables.borrow().get(&**bytes) {
                    return Ok(c.clone());
                }
                let s = crate::pstr::lossy(bytes).into_owned();
                if let Some((cls, m)) = s.split_once("::") {
                    let class = self.string_class(f, cls)?;
                    return self.method_callable(f, None, &class, m);
                }
                let c = self
                    .find_function(&f.ctx, &format!("\\{}", s.trim_start_matches('\\')))
                    .ok_or_else(|| self.error("Error", &format!("Call to undefined function {s}()")))?;
                // A function, once declared, stays.
                self.str_callables.borrow_mut().insert(bytes.to_vec().into_boxed_slice(), c.clone());
                Ok(c)
            }
            Value::Array(a) if a.len() == 2 => {
                let target = a.get(&Key::Int(0)).unwrap_or_default();
                let m = a.get(&Key::Int(1)).unwrap_or_default();
                let m: std::borrow::Cow<str> = match &m {
                    Value::Str(s) => match std::str::from_utf8(s) {
                        Ok(s) => std::borrow::Cow::Borrowed(s),
                        Err(_) => std::borrow::Cow::Owned(crate::pstr::lossy(s).into_owned()),
                    },
                    other => std::borrow::Cow::Owned(self.to_string_lossy(other)?),
                };
                match target {
                    Value::Object(o) => {
                        let class = o.class.clone();
                        self.method_callable(f, Some(o), &class, &m)
                    }
                    Value::Str(c) => {
                        let class = self.string_class(f, &crate::pstr::lossy(&c))?;
                        self.method_callable(f, None, &class, &m)
                    }
                    _ => Err(self.error("Error", "Array callback must have exactly two elements")),
                }
            }
            other => Err(self.error("Error", &format!("Value of type {} is not callable", other.type_name()))),
        }
    }

    /// is_callable(): whether `v` names something callable from `f`.
    pub fn is_callable_value(&self, f: &Frame, v: &Value) -> R<bool> {
        Ok(self.callable_error(f, v)?.is_none())
    }

    /// Why `v` can't be called from frame `f`, in PHP's words for an
    /// invalid callback argument; None when it can.
    pub fn callable_error(&self, f: &Frame, v: &Value) -> R<Option<String>> {
        let scope = f.class.as_deref();
        // `m` of `class`, on an object (`instance`) or statically.
        let method = |class: &Rc<Class>, m: &str, instance: bool| -> Option<String> {
            let m0 = m.strip_prefix("parent::").unwrap_or(m);
            let lname = if m0.bytes().any(|b| b.is_ascii_uppercase()) { std::borrow::Cow::Owned(m0.to_ascii_lowercase()) } else { std::borrow::Cow::Borrowed(m0) };
            let this_fits = instance || f.this.as_ref().is_some_and(|t| t.class.is_subclass_of(class));
            let magic = || class.find_method(if this_fits { "__call" } else { "__callstatic" }).is_some();
            match class.find_method_from(&lname, scope) {
                Some(meth) if !self.member_visible(scope, meth.vis, &meth.declaring) => {
                    let vis = if meth.vis == Vis::Private { "private" } else { "protected" };
                    (!magic()).then(|| format!("cannot access {vis} method {}::{}()", class.name, meth.name))
                }
                Some(meth) if !meth.is_static && !this_fits => Some(format!("non-static method {}::{}() cannot be called statically", class.name, meth.name)),
                Some(_) => None,
                None => (!magic()).then(|| format!("class {} does not have a method \"{m}\"", class.name)),
            }
        };
        // A class named in a callback ("self", "parent" and "static" too).
        let class = |n: &str| -> R<Result<Rc<Class>, String>> {
            let found = if n.eq_ignore_ascii_case("self") {
                f.class.clone()
            } else if n.eq_ignore_ascii_case("static") {
                f.static_class.clone()
            } else if n.eq_ignore_ascii_case("parent") {
                f.class.as_ref().and_then(|c| c.parent.clone())
            } else {
                self.find_class(n)?
            };
            Ok(found.ok_or_else(|| format!("class \"{n}\" not found")))
        };
        Ok(match v {
            Value::Ref(r) => return self.callable_error(f, &r.borrow()),
            Value::Object(o) => {
                if matches!(&*o.internal(), Internal::Closure(_)) || o.class.find_method("__invoke").is_some() {
                    None
                } else {
                    Some("no array or string given".into())
                }
            }
            Value::Str(s) => {
                if self.str_callables.borrow().contains_key(&**s) {
                    return Ok(None);
                }
                let name = crate::pstr::lossy(s).into_owned();
                match name.split_once("::") {
                    Some((cls, m)) => match class(cls)? {
                        Ok(c) => method(&c, m, false),
                        Err(why) => Some(why),
                    },
                    None if self.find_function(&f.ctx, &format!("\\{}", name.trim_start_matches('\\'))).is_some() => None,
                    None => Some(format!("function \"{name}\" not found or invalid function name")),
                }
            }
            Value::Array(a) if a.len() == 2 => {
                let Some(Value::Str(m)) = a.get(&Key::Int(1)) else {
                    return Ok(Some("second array member is not a valid method".into()));
                };
                let m = crate::pstr::lossy(&m);
                match a.get(&Key::Int(0)) {
                    Some(Value::Object(o)) => method(&o.class, &m, true),
                    Some(Value::Str(c)) => match class(&crate::pstr::lossy(&c))? {
                        Ok(c) => method(&c, &m, false),
                        Err(why) => Some(why),
                    },
                    _ => Some("first array member is not a valid class name or object".into()),
                }
            }
            Value::Array(_) => Some("array callback must have exactly two members".into()),
            _ => Some("no array or string given".into()),
        })
    }

    fn method_callable(&self, f: &Frame, this: Option<Rc<Obj>>, class: &Rc<Class>, m: &str) -> R<Callable> {
        let m = m.strip_prefix("parent::").unwrap_or(m);
        let lname = if m.bytes().any(|b| b.is_ascii_uppercase()) { std::borrow::Cow::Owned(m.to_ascii_lowercase()) } else { std::borrow::Cow::Borrowed(m) };
        let fitting_this = || f.this.clone().filter(|t| t.class.is_subclass_of(class));
        match class.find_method_from(&lname, f.class.as_deref()) {
            Some(method) if !self.member_visible(f.class.as_deref(), method.vis, &method.declaring) => {
                let this = this.or_else(fitting_this);
                let magic = if this.is_some() { "__call" } else { "__callstatic" };
                if class.find_method(magic).is_some() {
                    return Ok(Callable::Magic { this, class: class.clone(), name: Rc::from(m) });
                }
                let vis = if method.vis == Vis::Private { "private" } else { "protected" };
                let scope = f.class.as_ref().map(|c| format!("scope {}", c.name)).unwrap_or_else(|| "global scope".into());
                Err(self.error("Error", &format!("Call to {vis} method {}::{}() from {scope}", class.name, method.name)))
            }
            Some(method) => {
                let this = if method.is_static { None } else { this.or_else(fitting_this) };
                if this.is_none() && !method.is_static {
                    return Err(self.error("Error", &format!("Non-static method {}::{}() cannot be called statically", class.name, method.name)));
                }
                let static_class = this.as_ref().map(|o| o.class.clone()).unwrap_or_else(|| class.clone());
                Ok(Callable::Method { this, class: class.clone(), method: method.clone(), static_class })
            }
            // Answered by __call (on an object, or a static call from a
            // fitting $this) or __callStatic.
            None => {
                let this = this.or_else(|| f.this.clone().filter(|t| t.class.is_subclass_of(class)));
                let magic = if this.is_some() { "__call" } else { "__callstatic" };
                if class.find_method(magic).is_some() {
                    return Ok(Callable::Magic { this, class: class.clone(), name: Rc::from(m) });
                }
                Err(self.error("Error", &format!("Call to undefined method {}::{m}()", class.name)))
            }
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
                let m = self.method_name(f, name)?;
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

    /// call_callable() on behalf of frame `f`: a built-in callee checks
    /// its callback arguments from `f`'s scope.
    pub fn call_callable_from(&self, f: &Frame, c: &Callable, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Value> {
        match c {
            Callable::Builtin(name, fun) => self.call_builtin(name, *fun, pos, named, Some(f)),
            c => self.call_callable(c, pos, named),
        }
    }

    pub fn call_callable(&self, c: &Callable, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Value> {
        match c {
            Callable::User(u) => {
                if named.is_empty() && self.want_ref.get().is_none() {
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
                    by_ref: u.decl.by_ref,
                },
                pos,
                named,
            )
            }
            Callable::Builtin(name, fun) => self.call_builtin(name, *fun, pos, named, None),
            Callable::Closure(cd) => self.call_closure(cd, pos, named),
            Callable::Method { this, method, static_class, .. } => {
                self.invoke_method(this.clone(), method, static_class.clone(), pos, named)
            }
            Callable::Magic { this: Some(o), name, .. } => self.magic_call(o, name, pos, named),
            Callable::Magic { this: None, class, name } => {
                let m = class.find_method("__callstatic").cloned().ok_or_else(|| self.error("Error", &format!("Call to undefined method {}::{name}()", class.name)))?;
                self.invoke_method(None, &m, class.clone(), vec![Value::str(name), magic_args(pos, named)], vec![])
            }
        }
    }

    #[inline(never)]
    fn call_builtin(&self, name: &str, fun: crate::interp::BuiltinFn, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>, caller: Option<&Frame>) -> R<Value> {
        let spec = crate::arginfo::spec(name);
        let pos = match (named.is_empty(), spec) {
            (true, _) => pos,
            (false, Some(s)) => s.bind_named(self, pos, named)?,
            (false, None) => return Err(self.error("Error", &format!("phpx: named arguments to built-in {name}() are not supported yet"))),
        };
        let ctx = self.file_ctx(self.pos.get().file as usize);
        let mut frame = Frame::new(ctx, 0);
        if let Some(c) = caller {
            frame.class = c.class.clone();
            frame.static_class = c.static_class.clone();
            frame.this = c.this.clone();
        }
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
                by_ref: false,
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
                crate::errors::check(fun(self, this.as_ref(), &static_class, pos.into_iter().map(Value::into_deref).collect()))
            }
            MethodBody::User(func) => {
                let (class, display) = m.declared(&self.classes.borrow());
                let Some(body) = &func.body else {
                    let cname = class.as_ref().map(|c| c.name.to_string()).unwrap_or_default();
                    return Err(self.error("Error", &format!("Cannot call abstract method {cname}::{}()", m.name)));
                };
                self.invoke(
                    Invoke {
                        params: &func.params,
                        body: Body::Block(body),
                        ret: func.ret.as_ref(),
                        display,
                        func: m.name.clone(),
                        this: if m.is_static { None } else { this },
                        class,
                        static_class: Some(static_class),
                        ctx: m.ctx.clone(),
                        fn_id: *func as *const _ as usize,
                        captured: &[],
                        is_generator: func.is_generator,
                        by_ref: func.by_ref,
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
        crate::phpprof::count(0);
        if phpx_runtime::stack_remaining().is_some_and(|r| r < STACK_RESERVE) {
            return Err(self.stack_overflow());
        }
        let saved_pos = self.pos.get();
        let ret_ref = inv.by_ref
            && match self.want_ref.get() {
                Some(crate::interp::WANT_REF_ANY) => true,
                Some(off) => off == saved_pos.off,
                None => false,
            };
        if ret_ref {
            self.want_ref.set(None);
        }
        let entry = if inv.is_generator || ret_ref { None } else { self.native_entry_of(&inv) };
        let mut pos = pos;
        let mut named = named;
        if let Some(entry) = entry.filter(|_| inv.captured.is_empty()) {
            // Compiled: bind straight from the arguments, like calls
            // between compiled functions (declines leave them untouched).
            if named.is_empty() || self.named_in_order(&inv, &mut pos, &mut named)? {
                if let Some(r) = self.native_call_args(&inv, entry, &mut pos) {
                    return r;
                }
            }
        }
        let mut f = self.bind_args(&inv, pos, named, saved_pos)?;
        if inv.is_generator {
            return Ok(self.start_generator(f, inv.body, saved_pos));
        }
        if ret_ref {
            return self.invoke_ref(&inv, f, saved_pos);
        }
        if let Some(r) = entry.and_then(|e| self.run_native_entry(e, &mut f)) {
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

    /// A `function &f()` call whose caller binds the result by reference:
    /// interpreted, with `return $place;` handing back the place's cell.
    #[inline(never)]
    fn invoke_ref(&self, inv: &Invoke, mut f: Frame, saved_pos: crate::interp::Pos) -> R<Value> {
        let _call = crate::trace::enter(self, &f, inv.params, saved_pos);
        let prev = self.ref_return_depth.replace(f.depth);
        let result = match inv.body {
            Body::Block(stmts) => self.exec_block(&mut f, stmts).map(|flow| match flow {
                Flow::Return(v) => v,
                _ => Value::Null,
            }),
            Body::Expr(e) => self.eval(&mut f, e),
        };
        self.ref_return_depth.set(prev);
        let result = match result? {
            Value::Ref(cell) => {
                if inv.ret.is_some() {
                    let v = cell.borrow().clone();
                    let checked = self.check_return(inv, v)?;
                    *cell.borrow_mut() = checked;
                }
                Value::Ref(cell)
            }
            v if inv.ret.is_some() => self.check_return(inv, v)?,
            v => v,
        };
        self.pos.set(saved_pos);
        Ok(result)
    }

    /// Named arguments that only fill parameters after the positional ones
    /// (none variadic): moved into position, skipped parameters taking
    /// their defaults. False (nothing moved) for anything else, including
    /// every error case, which the full binder reports.
    #[inline(never)]
    fn named_in_order(&self, inv: &Invoke, pos: &mut Vec<Value>, named: &mut Vec<(Rc<str>, Value)>) -> R<bool> {
        let mut last = 0;
        for (n, _) in named.iter() {
            match inv.params.iter().position(|p| *p.name.name == **n) {
                Some(j) if j >= pos.len() && !inv.params[j].variadic && !inv.params[j].by_ref => last = last.max(j),
                _ => return Ok(false),
            }
        }
        if (pos.len()..last).any(|j| inv.params[j].default.is_none() && !named.iter().any(|(n, _)| *inv.params[j].name.name == **n)) {
            return Ok(false);
        }
        let mut f = Frame::for_call(inv.ctx.clone(), inv.func.clone(), inv.fn_id);
        f.this = inv.this.clone();
        f.class = inv.class.clone();
        f.static_class = inv.static_class.clone();
        for j in pos.len()..=last {
            let p = &inv.params[j];
            let v = match named.iter().position(|(n, _)| *p.name.name == **n) {
                Some(k) => named.swap_remove(k).1,
                None => self.eval(&mut f, p.default.as_ref().unwrap())?,
            };
            pos.push(v);
        }
        Ok(true)
    }

    /// Call a compiled entry with the arguments as they are. None = it
    /// declined to bind them (they are untouched).
    #[inline(never)]
    fn native_call_args(&self, inv: &Invoke, entry: crate::compiler::Entry, pos: &mut Vec<Value>) -> Option<R<Value>> {
        let mut f = Frame::for_call(inv.ctx.clone(), inv.func.clone(), inv.fn_id);
        f.this = inv.this.clone();
        f.class = inv.class.clone();
        f.static_class = inv.static_class.clone();
        let mut out = Value::Null;
        let st = unsafe { crate::compiler::rt::enter(self, entry, &mut f, pos.as_mut_ptr(), pos.len() as u64, &mut out) }?;
        Some(match st {
            0 => Ok(out),
            _ => Err(self.pending.borrow_mut().take().expect("native error without pending exception")),
        })
    }

    #[inline(never)]
    fn native_entry_of(&self, inv: &Invoke) -> Option<crate::compiler::Entry> {
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
        self.native_entry(inv.fn_id, mk)
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

        let mut last_named: Option<usize> = None;
        let had_named = !named.is_empty();
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
                        for (n, (k, v)) in rest.iter().enumerate() {
                            checked.insert(k.clone(), self.check_param(inv, i + n, p, ty, v.clone(), caller_strict)?);
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
                last_named = Some(i);
                named.remove(at).1
            } else if let Some(d) = &p.default {
                self.eval(&mut f, d)?
            } else if had_named {
                self.pos.set(saved_pos);
                return Err(self.error("ArgumentCountError", &format!("{}(): Argument #{} (${pname}) not passed", inv.display, i + 1)));
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
                if p.promote.is_some() {
                    if let Some(o) = &inv.this {
                        // `private $x` promoted from `&$x` shares the caller's
                        // reference; a readonly one can't (PHP's error).
                        // (Writes through a reference to a typed property
                        // aren't type-checked yet, here or for `&$o->prop`.)
                        let key = self.prop_key(&f, o, pname);
                        if o.class.find_prop(&key).is_some_and(|d| d.readonly) {
                            return Err(self.error("Error", &format!("Cannot indirectly modify readonly property {}::${key}", o.class.name)));
                        }
                        o.props.borrow_mut().insert(Rc::from(&*key), Value::Ref(cell.clone()));
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
                    // (Mangled when a subclass redeclared it privately.)
                    let key = self.prop_key(&f, o, pname);
                    self.promote_into(o, &key, pname, v.clone())?;
                }
            }
            f.vars.insert(pname, slot(v));
        }
        if let Some((n, _)) = named.first() {
            return Err(self.error("Error", &format!("Unknown named parameter ${n}")));
        }
        f.args = pos.into_iter().map(Value::into_deref).collect();
        // Named arguments count up to the last one passed; parameters
        // skipped before it count with their defaults (func_get_args()).
        if let Some(last) = last_named.filter(|&l| l >= f.args.len()) {
            for p in &inv.params[f.args.len()..=last] {
                let v = f.vars.get(&p.name.name).map(|s| s.borrow().deref()).unwrap_or_default();
                f.args.push(v);
            }
        }
        Ok(f)
    }

    /// A promoted constructor parameter's property write: a readonly one
    /// already set (the constructor called again) can't be changed.
    pub(crate) fn promote_into(&self, o: &Rc<Obj>, key: &str, name: &str, v: Value) -> R<()> {
        if o.class.find_prop(key).is_some_and(|d| d.readonly) && o.props.borrow().get(key).is_some() {
            return Err(self.error("Error", &format!("Cannot modify readonly property {}::${name}", o.class.name)));
        }
        o.set_prop(key, v);
        Ok(())
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
                    &format!("{display}(): Return value must be of type {}, {} returned", self.type_str_in(ty, ctx, class.map(|c| &*c.name)), result.debug_type()),
                )),
            },
        }
    }

    fn check_param(&self, inv: &Invoke, i: usize, p: &ast::Param, ty: &Type, v: Value, strict: bool) -> R<Value> {
        match self.coerce(v.clone(), ty, &inv.ctx, inv.class.as_ref(), inv.static_class.as_ref(), strict)? {
            Some(v) => Ok(v),
            None => Err(self.param_type_error(&inv.display, i, p, ty, &v, &inv.ctx)),
        }
    }

    /// TypeError for a user function's argument, like PHP: thrown at the
    /// parameter's declaration, naming the call site when user code made
    /// the call (not a built-in calling back). `pos` must still be the
    /// call site.
    pub fn param_type_error(&self, display: &str, i: usize, p: &ast::Param, ty: &Type, v: &Value, ctx: &NsCtx) -> Ctrl {
        let file = ctx.file;
        // A variadic parameter's arguments are numbered, not named.
        let which = if p.variadic { format!("#{}", i + 1) } else { format!("#{} (${})", i + 1, p.name.name) };
        // (A method's display name starts with its declaring class.)
        let class = display.split_once("::").map(|c| c.0).filter(|c| !c.starts_with('{'));
        let mut msg = format!("{display}(): Argument {which} must be of type {}, {} given", self.type_str_in(ty, ctx, class), v.debug_type());
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
        let mut buf = [0; 64];
        let lname = crate::class::lower_name(name, &mut buf);
        match o.class.find_method_from(&lname, f.class.as_deref()).cloned() {
            Some(m) => self.method_call_found(f, o, &m, name, pos, named),
            None if o.class.find_method("__call").is_some() => self.magic_call(o, name, pos, named),
            None => {
                // `$obj->prop(...)` where prop holds a closure is NOT a method call in PHP.
                Err(self.error("Error", &format!("Call to undefined method {}::{name}()", o.class.name)))
            }
        }
    }

    /// method_call() once the method is found: invisible ones go to `__call`
    /// when there is one.
    pub fn method_call_found(&self, f: &Frame, o: &Rc<Obj>, m: &Rc<Method>, name: &str, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Value> {
        if !self.member_visible(f.class.as_deref(), m.vis, &m.declaring) {
            if o.class.find_method("__call").is_some() {
                return self.magic_call(o, name, pos, named);
            }
            self.check_method_visible(f, &o.class, m)?;
        }
        self.invoke_method(Some(o.clone()), m, o.class.clone(), pos, named)
    }

    fn magic_call(&self, o: &Rc<Obj>, name: &str, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Value> {
        let m = o.class.find_method("__call").cloned().unwrap();
        let args = vec![Value::str(name), magic_args(pos, named)];
        self.invoke_method(Some(o.clone()), &m, o.class.clone(), args, vec![])
    }

    /// Internal method call (iterators, ArrayAccess, __toString...): no
    /// visibility check. The engine calls these by fixed names, so each
    /// class remembers what a name resolved to (keyed by the name's
    /// address: only `&'static str` names come here).
    pub fn call_method(&self, o: &Rc<Obj>, name: &'static str, args: Vec<Value>) -> R<Value> {
        let key = name.as_ptr() as usize;
        let cached = o.class.internal_calls.borrow().iter().find(|e| e.0 == key && e.1 == name.len()).map(|e| e.2.clone());
        let m = match cached {
            Some(m) => m,
            None => {
                let lname = name.to_ascii_lowercase();
                let m = o.class.find_method(&lname).cloned();
                let mut c = o.class.internal_calls.borrow_mut();
                if c.len() < 32 {
                    c.push((key, name.len(), m.clone()));
                }
                m
            }
        };
        match m {
            Some(m) => self.invoke_method(Some(o.clone()), &m, o.class.clone(), args, vec![]),
            None if o.class.find_method("__call").is_some() => self.magic_call(o, name, args, vec![]),
            None => Err(self.error("Error", &format!("Call to undefined method {}::{name}()", o.class.name))),
        }
    }

    /// call_method() for a name made at run time.
    pub fn call_method_dyn(&self, o: &Rc<Obj>, name: &str, args: Vec<Value>) -> R<Value> {
        let lname = if name.bytes().any(|b| b.is_ascii_uppercase()) { std::borrow::Cow::Owned(name.to_ascii_lowercase()) } else { std::borrow::Cow::Borrowed(name) };
        match o.class.find_method(&lname).cloned() {
            Some(m) => self.invoke_method(Some(o.clone()), &m, o.class.clone(), args, vec![]),
            None if o.class.find_method("__call").is_some() => self.magic_call(o, name, args, vec![]),
            None => Err(self.error("Error", &format!("Call to undefined method {}::{name}()", o.class.name))),
        }
    }

    /// call_method() for a caller that binds the result by reference: a
    /// `function &name()` hands back `Value::Ref`, anything else a value.
    pub fn call_method_ref(&self, o: &Rc<Obj>, name: &'static str, args: Vec<Value>) -> R<Value> {
        let prev = self.want_ref.replace(Some(crate::interp::WANT_REF_ANY));
        let r = self.call_method(o, name, args);
        self.want_ref.set(prev);
        r
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
        let forwarding = is_forwarding(cref);
        let mut buf = [0; 64];
        let lname = crate::class::lower_name(name, &mut buf);
        match cls.find_method(&lname).cloned() {
            Some(m) => {
                self.check_method_visible(f, &cls, &m)?;
                self.static_invoke(f, cref, cls, &m, pos, named)
            }
            None => {
                let this_fits = f.this.as_ref().filter(|o| o.class.is_subclass_of(&cls)).cloned();
                if let (Some(this), Some(_)) = (&this_fits, cls.find_method("__call")) {
                    return self.magic_call(this, name, pos, named);
                }
                if let Some(m) = cls.find_method("__callstatic").cloned() {
                    let args = vec![Value::str(name), magic_args(pos, named)];
                    let sc = if forwarding { f.static_class.clone().unwrap_or(cls.clone()) } else { cls.clone() };
                    return self.invoke_method(None, &m, sc, args, vec![]);
                }
                Err(self.error("Error", &format!("Call to undefined method {}::{name}()", cls.name)))
            }
        }
    }

    /// `C::m()` with `m` found on `cls` and visible: a static call, or an
    /// instance call on a compatible `$this`.
    pub(crate) fn static_invoke(&self, f: &Frame, cref: &'static ast::ClassRef, cls: Rc<Class>, m: &Rc<Method>, pos: Vec<Value>, named: Vec<(Rc<str>, Value)>) -> R<Value> {
        if m.is_static {
            let sc = if is_forwarding(cref) { f.static_class.clone().unwrap_or(cls) } else { cls };
            return self.invoke_method(None, m, sc, pos, named);
        }
        match f.this.as_ref().filter(|o| o.class.is_subclass_of(&cls)).cloned() {
            Some(this) => {
                let sc = this.class.clone();
                self.invoke_method(Some(this), m, sc, pos, named)
            }
            None => Err(self.error("Error", &format!("Non-static method {}::{}() cannot be called statically", cls.name, m.name))),
        }
    }

    pub fn find_static_prop(&self, f: &mut Frame, cref: &'static ast::ClassRef, name: &str) -> R<Rc<StaticProp>> {
        let cls = self.class_ref(f, cref)?;
        match cls.static_props.get(name) {
            Some(sp) => {
                if !self.member_visible(f.class.as_deref(), sp.vis, &sp.declaring) {
                    let word = if matches!(sp.vis, crate::class::Vis::Private) { "private" } else { "protected" };
                    return Err(self.error("Error", &format!("Cannot access {word} property {}::${name}", cls.name)));
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
                let v = self.eval(f, e)?;
                return self.class_name_of_value(&v);
            }
            let cls = self.class_ref(f, cref)?;
            return Ok(cls.name_value.clone());
        }
        let cls = self.class_ref(f, cref)?;
        let v = self.get_class_const(&cls, name)?;
        if !cls.deprecated_consts.is_empty() {
            self.const_deprecation(&cls, name)?;
        }
        Ok(v)
    }

    /// "Constant C::X is deprecated since 8.5, use ... instead".
    #[inline(never)]
    fn const_deprecation(&self, cls: &Rc<Class>, name: &str) -> R<()> {
        let Some((_, owner, attr, ctx)) = cls.deprecated_consts.iter().find(|d| *d.0 == *name).cloned() else { return Ok(()) };
        // Built-in classes raise E_DEPRECATED, user code E_USER_DEPRECATED.
        let internal = ctx.file == crate::interp::PRELUDE_FILE;
        let mut fr = Frame::new(ctx, 0);
        let (mut message, mut since) = (None, None);
        for (i, a) in attr.args.iter().enumerate() {
            let v = self.to_string_lossy(&self.eval(&mut fr, &a.value)?)?;
            match (a.name.as_ref().map(|n| n.name.as_str()), i) {
                (Some("message"), _) | (None, 0) => message = Some(v),
                (Some("since"), _) | (None, 1) => since = Some(v),
                _ => {}
            }
        }
        let mut msg = format!("Constant {owner}::{name} is deprecated");
        if let Some(s) = since.filter(|s| !s.is_empty()) {
            msg.push_str(&format!(" since {s}"));
        }
        if let Some(m) = message.filter(|m| !m.is_empty()) {
            msg.push_str(&format!(", {m}"));
        }
        crate::errors::raise(self, if internal { crate::errors::E_DEPRECATED } else { crate::errors::E_USER_DEPRECATED }, &msg);
        Ok(())
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

    pub fn deps_ready(&self, c: &'static ast::Class, ctx: &Rc<NsCtx>) -> bool {
        // Early binding only uses already-declared classes (no autoload), like PHP.
        let deps = self.class_deps(c, ctx);
        let classes = self.classes.borrow();
        deps.extends.iter().chain(&deps.implements).chain(&deps.traits).all(|(_, l)| classes.contains_key(&**l))
    }

    /// The names a class declaration's parents, interfaces and traits
    /// resolve to in `ctx` (worked out once per declaration and context).
    fn class_deps(&self, c: &'static ast::Class, ctx: &Rc<NsCtx>) -> Rc<ClassDeps> {
        let key = c as *const _ as usize;
        if let Some(d) = CLASS_DEPS.with(|m| m.borrow().get(&key).filter(|d| Rc::ptr_eq(&d.0, ctx)).map(|d| d.1.clone())) {
            return d;
        }
        let resolve = |names: &[String]| {
            names
                .iter()
                .map(|n| {
                    let full = self.resolve_class_name(ctx, n);
                    let lower = self.lower.get(&full);
                    (full, lower)
                })
                .collect::<Vec<_>>()
        };
        let traits: Vec<String> = c.members.iter().filter_map(|m| if let Member::TraitUse { traits, .. } = m { Some(traits) } else { None }).flatten().cloned().collect();
        let d = Rc::new(ClassDeps { extends: resolve(&c.extends), implements: resolve(&c.implements), traits: resolve(&traits) });
        CLASS_DEPS.with(|m| m.borrow_mut().insert(key, (ctx.clone(), d.clone())));
        d
    }

    pub fn declare_anon_class(&self, f: &Frame, c: &'static ast::Class) -> R<Rc<Class>> {
        let key = c as *const _ as usize;
        if let Some(cls) = self.anon_classes.borrow().get(&key) {
            return Ok(cls.clone());
        }
        let ctx = f.ctx.clone();
        // PHP's name: the display name, a NUL, then where it was declared.
        let line = self.line_of(ctx.file, c.span.start);
        let at = format!("{}:{line}${:x}", self.file_path(ctx.file), self.anon_classes.borrow().len());
        let cls = self.build_class(c, &ctx, Some(at))?;
        self.anon_classes.borrow_mut().insert(key, cls.clone());
        Ok(cls)
    }

    pub fn declare_class(&self, c: &'static ast::Class, ctx: &Rc<NsCtx>) -> R<Rc<Class>> {
        let key = c as *const _ as usize;
        let cached = BUILT.with(|b| b.borrow().get(&key).cloned());
        let cls = match cached {
            Some(cls) if self.can_reuse(&cls, c, ctx)? => {
                self.classes.borrow_mut().insert(cls.lname.clone(), cls.clone());
                cls
            }
            _ => {
                let cls = self.build_class(c, ctx, None)?;
                BUILT.with(|b| b.borrow_mut().insert(key, cls.clone()));
                cls
            }
        };
        IN_USE.with(|u| u.borrow_mut().push(cls.clone()));
        self.declared.borrow_mut().insert(key);
        Ok(cls)
    }

    /// Whether a class an earlier request built from this declaration is
    /// what building it now would give: same names in scope, and its
    /// parent, interfaces and traits resolve to the very classes it was
    /// built with (looked up in the order building does, so autoloading
    /// and its errors happen the same way).
    fn can_reuse(&self, cls: &Rc<Class>, c: &'static ast::Class, ctx: &Rc<NsCtx>) -> R<bool> {
        if self.classes.borrow().contains_key(&*cls.lname) || !(Rc::ptr_eq(&cls.decl_ctx, ctx) || *cls.decl_ctx == **ctx) {
            return Ok(false);
        }
        let names = self.class_deps(c, ctx);
        let (parent, ifaces) = match c.kind {
            ClassKind::Interface => (None, &names.extends[..]),
            _ => (names.extends.first(), &names.implements[..]),
        };
        let get = |(full, lower): &(String, Rc<str>)| {
            let known = self.classes.borrow().get(&**lower).cloned();
            match known {
                Some(c) => Ok(c),
                None => self.lookup_class(full),
            }
        };
        if let Some(p) = parent {
            let pc = get(p)?;
            if !cls.parent.as_ref().is_some_and(|q| Rc::ptr_eq(q, &pc)) {
                return Ok(false);
            }
        }
        let mut deps = cls.deps.iter();
        for n in ifaces.iter().chain(&names.traits) {
            let d = get(n)?;
            if !deps.next().is_some_and(|q| Rc::ptr_eq(q, &d)) {
                return Ok(false);
            }
        }
        Ok(deps.next().is_none())
    }

    /// `anon_at`: an anonymous class's declaration site (`file:line$n`).
    fn build_class(&self, c: &'static ast::Class, ctx: &Rc<NsCtx>, anon_at: Option<String>) -> R<Rc<Class>> {
        let name = match &anon_at {
            Some(n) => n.clone(),
            None => qualify(&ctx.ns, &c.name.as_ref().unwrap().name),
        };
        let mut lname = self.lower.get(&name);
        if anon_at.is_none() && self.classes.borrow().contains_key(&*lname) {
            return Err(self.error("Error", &format!("Cannot declare class {name}, because the name is already in use")));
        }

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
        // An anonymous class is named after its parent or first interface
        // ("Base@anonymous"), like PHP's.
        let mut anon_name = None;
        let display: Rc<str> = match &anon_at {
            Some(at) => {
                let base = match (&parent, ifaces.first()) {
                    (Some(p), _) => p.name.to_string(),
                    (None, Some(i)) => self.resolve_class_name(ctx, i),
                    (None, None) => "class".to_string(),
                };
                let full = format!("{base}@anonymous\0{at}");
                lname = self.lower.get(&full);
                anon_name = Some(Rc::from(full));
                Rc::from(format!("{base}@anonymous"))
            }
            None => Rc::from(name.as_str()),
        };
        // Members are declared under the real name (an anonymous class's
        // differs from its display name).
        let decl_name: Rc<str> = anon_name.clone().unwrap_or_else(|| display.clone());
        let mut ancestors: Vec<Rc<str>> = vec![lname.clone()];
        let mut consts: crate::value::FxIndexMap<Rc<str>, ConstSlot> = Default::default();
        let mut consts_init: crate::value::FxIndexMap<Rc<str>, ConstSlot> = Default::default();
        let mut deps: Vec<Rc<Class>> = vec![];
        let mut methods: crate::value::FxIndexMap<Rc<str>, Rc<Method>> = Default::default();
        let mut used_traits: Vec<Rc<str>> = vec![];
        let mut shadowed: Vec<(Rc<str>, Rc<str>, Rc<str>)> = parent.as_ref().map(|p| p.shadowed.clone()).unwrap_or_default();
        let mut trait_keys: Vec<Rc<str>> = vec![];
        let mut props: Vec<PropDecl> = vec![];
        let mut static_props: crate::value::FxIndexMap<Rc<str>, Rc<StaticProp>> = Default::default();
        let mut deprecated_consts = parent.as_ref().map(|p| p.deprecated_consts.clone()).unwrap_or_default();
        if let Some(p) = &parent {
            ancestors.extend(p.ancestors.iter().cloned());
            consts.extend(p.consts.borrow().iter().map(|(k, v)| (k.clone(), v.clone())));
            consts_init.extend(p.consts_init.iter().map(|(k, v)| (k.clone(), v.clone())));
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
            for (k, v) in &iface.consts_init {
                consts_init.entry(k.clone()).or_insert_with(|| v.clone());
            }
            deps.push(iface.clone());
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
        let mut trait_props: Vec<PropDecl> = vec![];
        let mut trait_statics: Vec<(Rc<str>, Rc<StaticProp>)> = vec![];
        for m in &c.members {
            match m {
                Member::TraitUse { traits, adaptations, .. } => {
                    for t in traits {
                        let tc = self.lookup_class(&self.resolve_class_name(ctx, t))?;
                        if tc.kind != ClassKind::Trait {
                            return Err(self.error("Error", &format!("{name} cannot use {} - it is not a trait", tc.name)));
                        }
                        used_traits.push(tc.name.clone());
                        deps.push(tc.clone());
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
                        // Bound after the class's own properties, as PHP does.
                        for p in &tc.props {
                            let mut np = p.clone();
                            np.declaring = decl_name.clone();
                            trait_props.push(np);
                        }
                        for (k, sp) in &tc.static_props {
                            trait_statics.push((
                                k.clone(),
                                Rc::new(StaticProp {
                                    value: std::cell::RefCell::new(None),
                                    init: sp.init,
                                    vis: sp.vis,
                                    declaring: lname.clone(),
                                    ctx: sp.ctx.clone(),
                                }),
                            ));
                        }
                        for (k, v) in tc.consts.borrow().iter() {
                            consts.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                        for (k, v) in &tc.consts_init {
                            consts_init.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                }
                Member::Const { items, attrs, .. } => {
                    let deprecated = attrs.iter().find(|a| self.resolve_class_name(ctx, &a.name).eq_ignore_ascii_case("Deprecated"));
                    for (n, e) in items {
                        let slot = ConstSlot::Pending { expr: e, declaring: lname.clone(), ctx: ctx.clone() };
                        consts_init.insert(Rc::from(&*n.name), slot.clone());
                        consts.insert(Rc::from(&*n.name), slot);
                        deprecated_consts.retain(|d| *d.0 != *n.name);
                        if let Some(a) = deprecated {
                            deprecated_consts.push((Rc::from(&*n.name), display.clone(), a, ctx.clone()));
                        }
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
                                declaring: decl_name.clone(),
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
                                    declaring: decl_name.clone(),
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
                            cache: Default::default(),
                        }),
                    );
                }
                Member::EnumCase { name: cn, value, .. } => {
                    cases.push(Rc::from(&*cn.name));
                    case_values.push((Rc::from(&*cn.name), value.as_ref()));
                }
            }
        }
        // Trait properties: a class's own declaration of the same one wins.
        for np in trait_props {
            if !props.iter().any(|q| q.name == np.name && *q.declaring == *decl_name) {
                upsert_prop(&mut props, np, &mut shadowed);
            }
        }
        for (k, sp) in trait_statics {
            if !static_props.get(&k).is_some_and(|own| *own.declaring == *lname) {
                static_props.insert(k, sp);
            }
        }
        // Trait methods list after the class's own (get_class_methods order).
        for k in trait_keys {
            if let Some(m) = methods.shift_remove(&k) {
                methods.insert(k, m);
            }
        }
        // Native methods: per class, plus the shared enum helpers.
        for ((cl, m), fun) in self.natives.iter() {
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
                        cache: Default::default(),
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
        let magic = crate::expr::MAGIC_NAMES.map(|n| methods.get(n).cloned());
        let plain_new = c.kind == ClassKind::Class && !is_abstract && !ancestors.iter().any(|a| &**a == "throwable");
        for (cn, ve) in case_values {
            let slot = ConstSlot::EnumCase { name: cn.clone(), value: ve, ctx: ctx.clone() };
            consts_init.insert(cn.clone(), slot.clone());
            consts.insert(cn, slot);
        }
        let cls = Rc::new(Class {
            name: display.clone(),
            lname: lname.clone(),
            kind: c.kind,
            parent,
            ancestors,
            is_abstract,
            consts: std::cell::RefCell::new(consts),
            consts_init,
            deps,
            deprecated_consts,
            static_props,
            props,
            has_destruct: methods.contains_key("__destruct"),
            traits: used_traits,
            prop_template: Default::default(),
            name_str: Default::default(),
            internal_calls: Default::default(),
            ctor,
            plain_new,
            shadowed,
            shape,
            methods,
            cases: cases.clone(),
            enum_backed: c.enum_type.is_some(),
            decl: c,
            decl_ctx: ctx.clone(),
            name_value: Value::str(anon_name.as_deref().unwrap_or(&display)),
            magic,
            anon_name,
        });
        // Anonymous classes too, under their unique internal name: methods
        // find their scope through it.
        self.classes.borrow_mut().insert(lname.clone(), cls.clone());
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
            // SAFETY: ASCII lowercasing of UTF-8 leaves it valid UTF-8.
            unsafe { std::str::from_utf8_unchecked(&buf[..n.len()]) }
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
                    uses: HashMap::default(),
                    use_fns: HashMap::default(),
                    use_consts: HashMap::default(),
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
        let func = match kind {
            Include => "include",
            IncludeOnce => "include_once",
            Require => "require",
            RequireOnce => "require_once",
        };
        // A path a user stream wrapper serves (stream_wrapper_register()),
        // like laravel-serializable-closure://: the code is what the
        // wrapper's stream reads, and the URL is its file name.
        if let Some(read) = crate::wrappers::read_for_include(self, path, func) {
            let url = std::path::PathBuf::from(path);
            let src = match read? {
                Ok(src) => src,
                Err(msg) => {
                    if matches!(kind, Require | RequireOnce) {
                        return Err(self.error("Error", &format!("Failed opening required '{path}'")));
                    }
                    self.warn(&msg);
                    return Ok(Value::Bool(false));
                }
            };
            if matches!(kind, IncludeOnce | RequireOnce) && self.included.borrow().contains(&url) {
                return Ok(Value::Bool(true));
            }
            self.included.borrow_mut().insert(url.clone());
            return match self.parse_file(&url, crate::pstr::lossy(&src).into_owned()) {
                Ok((id, file)) => {
                    self.run_file_in(f, file, id)
                }
                Err(msg) => {
                    eprint!("{msg}");
                    Err(Ctrl::Exit(255))
                }
            };
        }
        let p = std::path::Path::new(path);
        // A server worker trusts what it found for this include less than
        // REVALIDATE ago (opcache.revalidate_freq): no stat at all.
        let key = (if p.is_absolute() { usize::MAX } else { f.ctx.file }, path);
        if crate::server::sapi() != "cli" {
            let hit = RESOLVED.with(|r| {
                r.borrow().get(&key as &dyn ResolvedKey).map(|e| {
                    let fresh = e.checked.elapsed() < REVALIDATE;
                    let tree = (fresh && e.file.2 == crate::interp::parse_gen()).then_some((e.file.0, e.file.1));
                    (e.canon.clone(), fresh, tree)
                })
            });
            if let Some((canon, fresh, tree)) = hit {
                // Due for a check: one stat of the file found last time
                // (reparsed if it changed; a vanished file is resolved anew).
                let found = if let Some((id, file)) = tree {
                    self.file_known(id);
                    Some((id, file))
                } else if fresh {
                    self.parsed_file(&canon)
                } else {
                    match self.load_file(&canon, &canon) {
                        Ok(Ok(found)) => {
                            RESOLVED.with(|r| r.borrow_mut().insert((key.0, key.1.to_string()), Resolved::new(canon.clone(), found)));
                            Some(found)
                        }
                        _ => None,
                    }
                };
                if let Some((id, file)) = found {
                    if let Some(v) = self.take_preloaded(&canon) {
                        return Ok(v);
                    }
                    if !self.included.borrow_mut().insert(canon) && matches!(kind, IncludeOnce | RequireOnce) {
                        return Ok(Value::Bool(true));
                    }
                    return self.run_file_in(f, file, id);
                }
            }
        }
        let candidates: Vec<std::path::PathBuf> = if p.is_absolute() {
            vec![p.to_path_buf()]
        } else {
            let cur = self.file_path(f.ctx.file);
            let dir = std::path::Path::new(&cur).parent().map(|d| d.to_path_buf()).unwrap_or_default();
            vec![dir.join(p), p.to_path_buf()]
        };
        let found = candidates.into_iter().find(|c| c.is_file());
        let Some(found) = found else {
            if matches!(kind, Require | RequireOnce) {
                return Err(self.error("Error", &format!("Failed opening required '{path}'")));
            }
            self.warn(&format!("{func}({path}): Failed to open stream: No such file or directory"));
            return Ok(Value::Bool(false));
        };
        let canon = crate::fs::realpath_cached(&found).unwrap_or(found.clone());
        if let Some(v) = self.take_preloaded(&canon) {
            return Ok(v);
        }
        if matches!(kind, IncludeOnce | RequireOnce) && self.included.borrow().contains(&canon) {
            return Ok(Value::Bool(true));
        }
        self.included.borrow_mut().insert(canon.clone());
        // Composer's own autoloader runs by default (apps use its ClassLoader:
        // `$loader = require 'vendor/autoload.php'`, getRegisteredLoaders());
        // PHPX_NATIVE_COMPOSER=1 swaps in the built-in one.
        static NATIVE_COMPOSER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *NATIVE_COMPOSER.get_or_init(|| std::env::var_os("PHPX_NATIVE_COMPOSER").is_some_and(|v| v == "1"))
            && canon.ends_with("vendor/autoload.php")
            && canon.parent().is_some_and(|d| d.join("composer/autoload_psr4.php").is_file())
        {
            return self.composer_boot(f, &canon);
        }
        match self.load_file(&found, &canon).map_err(|e| self.error("Error", &e.to_string()))? {
            Ok((id, file)) => {
                if crate::server::sapi() != "cli" {
                    RESOLVED.with(|r| r.borrow_mut().insert((key.0, key.1.to_string()), Resolved::new(canon, (id, file))));
                }
                self.run_file_in(f, file, id)
            }
            Err(msg) => {
                eprint!("{msg}");
                Err(Ctrl::Exit(255))
            }
        }
    }

    /// A file the snapshot template already ran: its return value, once.
    fn take_preloaded(&self, canon: &std::path::Path) -> Option<Value> {
        if self.preloaded.borrow().is_empty() {
            return None;
        }
        let v = self.preloaded.borrow_mut().remove(canon)?;
        self.included.borrow_mut().insert(canon.to_path_buf());
        Some(v)
    }

        pub fn register_strict(&self, id: usize, file: &ast::File) {
        let strict = crate::interp::declares_strict(&file.stmts);
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
