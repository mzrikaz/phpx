//! Interpreter state, frames, and statement execution.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use phpx_syntax::ast::{self, Stmt, StmtKind};
use rustc_hash::FxHashMap;

use crate::class::{Class, StaticProp};
use crate::value::{Lower, Obj, Value};

/// Non-local exits that unwind through Rust frames.
/// `Interp::want_ref` for whichever call comes next.
pub const WANT_REF_ANY: u32 = u32::MAX;

#[derive(Clone)]
pub enum Ctrl {
    Throw(Rc<Obj>),
    Exit(i32),
    /// A generator was destroyed while suspended: unwind its body (running
    /// `finally` blocks) without an exception.
    GenClose,
}

pub type R<T> = Result<T, Ctrl>;

/// What a statement did to control flow.
pub enum Flow {
    Normal,
    Break(u32),
    Continue(u32),
    Return(Value),
    /// `goto label`: unwinds until the statement list that holds the label.
    Goto(&'static str),
}

/// Result of a task started with `async()`: its value or what it threw.
pub type Outcome = Result<Value, Ctrl>;

pub use crate::value::Slot;

pub fn slot(v: Value) -> Slot {
    Rc::new(RefCell::new(v))
}

/// File id of the prelude (built-in classes written in PHP).
pub const PRELUDE_FILE: usize = 0;

/// Statement being executed (for error messages). `repr(C)` so native code
/// can update it with plain stores.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Pos {
    pub file: u32,
    pub off: u32,
}

/// Name-resolution context of one `namespace` block in one file.
pub struct NsCtx {
    pub ns: String,
    /// Lowercase alias -> full class/namespace name.
    pub uses: HashMap<String, String>,
    pub use_fns: HashMap<String, String>,
    pub use_consts: HashMap<String, String>,
    pub file: usize,
    pub strict: bool,
}

pub struct FileInfo {
    pub path: PathBuf,
    pub src: &'static str,
    /// Byte offsets where lines start, built on first use (line numbers
    /// for traces and errors).
    lines: std::cell::OnceCell<Vec<u32>>,
}

impl FileInfo {
    /// 1-based line of byte offset `off`.
    pub fn line(&self, off: usize) -> usize {
        let starts = self.lines.get_or_init(|| {
            let mut v = vec![0u32];
            v.extend(memchr::memchr_iter(b'\n', self.src.as_bytes()).map(|i| i as u32 + 1));
            v
        });
        starts.partition_point(|&s| s as usize <= off.min(self.src.len()))
    }
}

pub struct UserFn {
    pub name: Rc<str>,
    pub decl: &'static ast::Function,
    pub ctx: Rc<NsCtx>,
}

pub enum ClosureKind {
    Closure(&'static ast::Closure),
    Arrow(&'static ast::ArrowFn),
    /// `strlen(...)`, `Closure::fromCallable()`.
    Wrap(Callable),
}

pub struct ClosureData {
    pub kind: ClosureKind,
    /// By-value captures hold the value; `use (&$x)` holds `Value::Ref` of
    /// the caller's slot (and the flag is set).
    pub captured: Vec<(&'static str, Value, bool)>,
    pub this: Option<Rc<Obj>>,
    pub scope: Option<Rc<Class>>,
    pub static_class: Option<Rc<Class>>,
    pub ctx: Rc<NsCtx>,
    /// `{closure:<where>:<line>}` (PHP 8.4+ naming).
    pub name: Rc<str>,
}

pub type BuiltinFn = fn(&Interp, &mut Frame, &mut [Value]) -> R<Value>;
pub type NativeMethod = fn(&Interp, Option<&Rc<Obj>>, &Rc<Class>, Vec<Value>) -> R<Value>;

#[derive(Clone)]
pub enum Callable {
    User(Rc<UserFn>),
    Builtin(&'static str, BuiltinFn),
    Closure(Rc<ClosureData>),
    Method { this: Option<Rc<Obj>>, class: Rc<Class>, method: Rc<crate::class::Method>, static_class: Rc<Class> },
    /// `[$obj, 'name']` / `'C::name'` for a method only __call or
    /// __callStatic answers.
    Magic { this: Option<Rc<Obj>>, class: Rc<Class>, name: Rc<str> },
}

thread_local! {
    static EMPTY_NAME: Rc<str> = Rc::from("");
    /// Parsed files by path (syntax trees are immutable, so they outlive
    /// any one interpreter on this thread).
    static PARSED: RefCell<HashMap<PathBuf, (&'static str, &'static ast::File)>> = RefCell::new(HashMap::new());
}

/// A function's variables. Keys borrow from the (leaked) AST: phpx has no
/// `$$var`, so every name is known statically. Small frames use a linear
/// scan (no hashing, one allocation); big ones (top-level scripts) add an index.
#[derive(Default)]
pub struct Vars {
    items: Vec<(&'static str, Slot)>,
    index: Option<FxHashMap<&'static str, usize>>,
}

const VARS_INDEX_AT: usize = 24;

impl Vars {
    fn pos(&self, name: &str) -> Option<usize> {
        match &self.index {
            Some(ix) => ix.get(name).copied(),
            None => self.items.iter().position(|(n, _)| *n == name),
        }
    }

    pub fn get(&self, name: &str) -> Option<&Slot> {
        self.pos(name).map(|i| &self.items[i].1)
    }

    /// Lookup with an inline-cache hint (the slot index this AST node found
    /// last time). The hint is verified by name, so a stale one is harmless.
    #[inline]
    pub fn get_hinted(&self, name: &str, hint: &std::cell::Cell<u32>) -> Option<&Slot> {
        if let Some((n, s)) = self.items.get(hint.get() as usize) {
            if *n == name {
                return Some(s);
            }
        }
        let i = self.pos(name)?;
        hint.set(i as u32);
        Some(&self.items[i].1)
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.pos(name).is_some()
    }

    pub fn insert(&mut self, name: &'static str, s: Slot) {
        if let Some(i) = self.pos(name) {
            self.items[i].1 = s;
            return;
        }
        self.items.push((name, s));
        if let Some(ix) = &mut self.index {
            ix.insert(name, self.items.len() - 1);
        } else if self.items.len() > VARS_INDEX_AT {
            self.index = Some(self.items.iter().enumerate().map(|(i, (n, _))| (*n, i)).collect());
        }
    }

    pub fn remove(&mut self, name: &str) {
        if let Some(i) = self.pos(name) {
            self.items.remove(i);
            if self.index.is_some() {
                self.index = Some(self.items.iter().enumerate().map(|(i, (n, _))| (*n, i)).collect());
            }
        }
    }

    /// Variables in definition order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &Slot)> {
        self.items.iter().map(|(n, s)| (*n, s))
    }

    pub fn with_capacity(n: usize) -> Vars {
        Vars { items: Vec::with_capacity(n), index: None }
    }
}

pub struct Frame {
    pub vars: Vars,
    pub this: Option<Rc<Obj>>,
    /// `self` scope (declaring class of the running method).
    pub class: Option<Rc<Class>>,
    /// `static` (late static binding).
    pub static_class: Option<Rc<Class>>,
    pub ctx: Rc<NsCtx>,
    /// Function name for `__FUNCTION__` and errors; empty at top level.
    pub func: Rc<str>,
    pub args: Vec<Value>,
    pub depth: usize,
    /// Address of the running function's AST node (keys `static $x`).
    pub fn_id: usize,
    /// Set while running a generator body (where `yield` is valid).
    pub gen: Option<Rc<crate::generator::GenShared>>,
    /// The running closure, when called through a native call site (its
    /// captured values are bound straight into native locals).
    pub closure: Option<Rc<ClosureData>>,
}

/// Offset of a `Vec`'s length word (probed: std doesn't fix the layout).
fn vec_len_offset() -> Option<usize> {
    let mut v: Vec<u64> = Vec::with_capacity(5);
    v.extend([1, 2, 3]);
    let words: [usize; 3] = unsafe { std::mem::transmute_copy(&v) };
    let i = words.iter().position(|&w| w == 3)?;
    (words.iter().filter(|&&w| w == 3).count() == 1).then_some(i * 8)
}

impl Frame {
    /// Offsets of the frame's variable count and argument count, for
    /// generated code that skips resetting an untouched frame.
    pub fn len_offsets() -> Option<(usize, usize)> {
        if std::mem::size_of::<Vec<u64>>() != 24 {
            return None;
        }
        let l = vec_len_offset()?;
        Some((std::mem::offset_of!(Frame, vars) + std::mem::offset_of!(Vars, items) + l, std::mem::offset_of!(Frame, args) + l))
    }

    /// Frame for a call: named function, no interpreter variables yet.
    pub fn for_call(ctx: Rc<NsCtx>, func: Rc<str>, fn_id: usize) -> Frame {
        Frame {
            vars: Vars::default(),
            this: None,
            class: None,
            static_class: None,
            ctx,
            func,
            args: vec![],
            depth: 0,
            fn_id,
            gen: None,
            closure: None,
        }
    }

    pub fn new(ctx: Rc<NsCtx>, depth: usize) -> Frame {
        Frame {
            vars: Vars::default(),
            this: None,
            class: None,
            static_class: None,
            ctx,
            func: EMPTY_NAME.with(|e| e.clone()),
            args: vec![],
            depth,
            fn_id: 0,
            gen: None,
            closure: None,
        }
    }

    /// Make a site-cached frame ready for the next call: drop what this
    /// call left behind (variables, `$this`, arguments).
    pub fn reset_after_call(&mut self) {
        if !self.vars.items.is_empty() {
            self.vars = Vars::default();
        }
        self.this = None;
        if !self.args.is_empty() {
            self.args.clear();
        }
        self.gen = None;
        self.closure = None;
    }

    pub fn var_slot(&mut self, name: &'static str) -> Slot {
        if let Some(s) = self.vars.get(name) {
            return s.clone();
        }
        let s = slot(Value::Null);
        self.vars.insert(name, s.clone());
        s
    }
}

pub struct Interp {
    pub files: RefCell<Vec<FileInfo>>,
    pub functions: RefCell<HashMap<Rc<str>, Rc<UserFn>>>,
    pub builtins: HashMap<&'static str, BuiltinFn>,
    pub natives: HashMap<(&'static str, &'static str), NativeMethod>,
    pub classes: RefCell<FxHashMap<Rc<str>, Rc<Class>>>,
    pub constants: RefCell<HashMap<String, Value>>,
    /// Function `static $x` slots, keyed by declaration address + class.
    pub statics: RefCell<HashMap<(usize, Rc<str>), HashMap<Rc<str>, Slot>>>,
    /// Output buffer stack; index 0 is flushed to `writer`.
    pub out: RefCell<Vec<Vec<u8>>>,
    pub writer: RefCell<Box<dyn Write>>,
    pub arrow_caps: RefCell<HashMap<usize, Rc<Vec<&'static str>>>>,
    /// spl_autoload_register() callbacks, tried in order on unknown classes.
    pub autoloaders: RefCell<Vec<Callable>>,
    pub composer: RefCell<Option<crate::composer::Composer>>,
    pub generators: RefCell<Vec<std::rc::Weak<crate::generator::GenShared>>>,
    /// Classes being autoloaded right now (stops autoload recursion).
    pub autoloading: RefCell<HashSet<String>>,
    /// Call site (AST address) -> resolved user function.
    pub call_cache: RefCell<FxHashMap<usize, Callable>>,
    /// `$obj->name()` call sites (by the name's AST address): the method
    /// last found for (class, calling scope), visible from there.
    pub method_sites: RefCell<FxHashMap<usize, (usize, usize, Rc<crate::class::Method>)>>,
    pub included: RefCell<HashSet<PathBuf>>,
    /// Declared classes per AST node, so hoisted declarations aren't redone.
    pub declared: RefCell<HashSet<usize>>,
    /// (file, byte offset) of the statement running now, for error lines.
    pub pos: Cell<Pos>,
    /// The main script's frame (the global scope), for `global`,
    /// `$GLOBALS`. Null when no script is running.
    pub globals: Cell<*mut Frame>,
    /// Headers, response code, shutdown functions.
    pub response: RefCell<crate::misc::Response>,
    pub include_path: RefCell<String>,
    /// Where output first reached the client (headers_sent()).
    output_start: RefCell<Option<(String, usize)>>,
    /// Classes resolved from source names, keyed by the AST name's address.
    pub class_cache: RefCell<FxHashMap<usize, Rc<Class>>>,
    /// Class references standing for a class computed at runtime
    /// (`$x::m()` in compiled code), one per class.
    pub dyn_class_refs: RefCell<FxHashMap<usize, &'static ast::ClassRef>>,
    /// Class names in type declarations, resolved and lowercased, keyed by
    /// the AST name's address (a name always resolves the same way).
    pub type_names: RefCell<FxHashMap<usize, Rc<str>>>,
    /// Closure names by AST node, with the enclosing (function, class)
    /// they were computed for.
    pub closure_names: RefCell<FxHashMap<usize, ((usize, usize), Rc<str>)>>,
    /// Position of the innermost running built-in's call: a user function
    /// bound while `pos` still equals it was called by the built-in (a
    /// callback), not from user code.
    pub builtin_pos: Cell<Pos>,
    pub ini: RefCell<crate::ini::Ini>,
    /// Constant array literals, built once (None: not constant).
    pub const_arrays: RefCell<FxHashMap<usize, Option<Value>>>,
    /// eval() sources already parsed, by call site + code.
    pub eval_cache: RefCell<HashMap<String, (usize, &'static ast::File)>>,
    /// `@` nesting depth: warnings are suppressed while nonzero.
    pub silence: Cell<u32>,
    /// Variable names created at run time (`$$x`, extract()).
    names: RefCell<HashSet<&'static str>>,
    pub lower: Lower,
    pub anon_classes: RefCell<HashMap<usize, Rc<Class>>>,
    /// `declare(strict_types=1)` per file id.
    pub file_strict: RefCell<Vec<bool>>,
    /// Exception raised inside native code, handed back with status 1.
    pub pending: RefCell<Option<Ctrl>>,
    /// Set by a caller that binds a call's result by reference: the call
    /// site's offset (`$x = &f()`; not calls nested in its arguments), or
    /// `WANT_REF_ANY` for the next call (ArrayAccess writes). Taken by the
    /// callee's entry.
    pub want_ref: Cell<Option<u32>>,
    /// Depth of the frame whose `return` yields a reference (usize::MAX: none).
    pub ref_return_depth: Cell<usize>,
    /// Magic property methods running now: (object id, property, MAGIC_*).
    /// PHP's guards: inside `__get('x')`, `$this->x` is a plain access.
    pub magic_guards: RefCell<Vec<(u32, Rc<str>, u8)>>,
    /// See `file_ctx`.
    pub file_ctxs: RefCell<Vec<Option<Rc<NsCtx>>>>,
    /// Native compiler for whole functions (None when disabled).
    pub native: RefCell<Option<crate::compiler::Native>>,
    /// Native compiler for hot numeric functions (None when disabled).
    pub jit: RefCell<Option<phpx_jit::Jit>>,
    /// Function AST -> its namespace context (for the JIT's name lookups).
    pub decl_ctx: RefCell<FxHashMap<usize, Rc<NsCtx>>>,
    /// Self handle, so tasks spawned by `async()` can own the interpreter.
    pub me: RefCell<std::rc::Weak<Interp>>,
}

impl Interp {
    pub fn new(writer: Box<dyn Write>) -> Rc<Interp> {
        let it = Rc::new(Interp {
            files: RefCell::new(vec![]),
            functions: RefCell::new(HashMap::new()),
            builtins: crate::builtins::table(),
            natives: crate::native::table(),
            classes: RefCell::new(FxHashMap::default()),
            constants: RefCell::new(crate::builtins::constants()),
            statics: RefCell::new(HashMap::new()),
            out: RefCell::new(vec![vec![]]),
            writer: RefCell::new(writer),
            arrow_caps: RefCell::new(HashMap::new()),
            call_cache: RefCell::new(FxHashMap::default()),
            method_sites: RefCell::new(FxHashMap::default()),
            autoloaders: RefCell::new(vec![]),
            composer: RefCell::new(None),
            generators: RefCell::new(vec![]),
            autoloading: RefCell::new(HashSet::new()),
            included: RefCell::new(HashSet::new()),
            declared: RefCell::new(HashSet::new()),
            pos: Cell::new(Pos { file: 0, off: 0 }),
            globals: Cell::new(std::ptr::null_mut()),
            silence: Cell::new(0),
            eval_cache: RefCell::new(HashMap::new()),
            ini: RefCell::new(crate::ini::Ini::new()),
            const_arrays: RefCell::new(FxHashMap::default()),
            class_cache: RefCell::new(FxHashMap::default()),
            dyn_class_refs: RefCell::new(FxHashMap::default()),
            type_names: RefCell::new(FxHashMap::default()),
            closure_names: RefCell::new(FxHashMap::default()),
            builtin_pos: Cell::new(Pos { file: u32::MAX, off: 0 }),
            response: RefCell::new(Default::default()),
            include_path: RefCell::new(".:/usr/share/php".into()),
            output_start: RefCell::new(None),
            names: RefCell::new(HashSet::new()),
            lower: Lower::default(),
            anon_classes: RefCell::new(HashMap::new()),
            file_strict: RefCell::new(vec![]),
            me: RefCell::new(std::rc::Weak::new()),
            jit: RefCell::new(if std::env::var("PHPX_NOJIT").is_ok() {
                None
            } else {
                phpx_jit::Jit::new().map(|mut j| {
                    j.set_interrupt_flag(&crate::value::DESTRUCT_PENDING as *const _ as usize);
                    j
                })
            }),
            pending: RefCell::new(None),
            want_ref: Cell::new(None),
            ref_return_depth: Cell::new(usize::MAX),
            magic_guards: RefCell::new(Vec::new()),
            file_ctxs: RefCell::new(vec![]),
            native: RefCell::new(crate::compiler::Native::from_env()),
            decl_ctx: RefCell::new(FxHashMap::default()),
        });
        *it.me.borrow_mut() = Rc::downgrade(&it);
        it.load_prelude();
        it
    }

    fn load_prelude(&self) {
        assert_eq!(self.files.borrow().len(), PRELUDE_FILE, "the prelude must be the first file");
        let src = concat!(include_str!("prelude.php"), include_str!("prelude_spl.php"), include_str!("prelude_reflection.php"), include_str!("prelude_date.php"), include_str!("prelude_splfs.php"), include_str!("prelude_tokenizer.php"), include_str!("prelude_dom.php"), include_str!("prelude_redis.php"), include_str!("prelude_pdo.php"), include_str!("prelude_random.php"), include_str!("prelude_intl.php"));
        let (file, diags) = phpx_syntax::check(src);
        assert!(!diags.iter().any(|d| d.is_error()), "prelude has errors: {diags:?}");
        let file: &'static ast::File = Box::leak(Box::new(file));
        let id = self.add_file(PathBuf::from("[prelude]"), src);
        let mut frame = Frame::new(self.top_ctx(id, false), 0);
        if self.run_stmts_top(&mut frame, &file.stmts, id).is_err() {
            panic!("prelude failed to load");
        }
    }

    pub fn rc(&self) -> Rc<Interp> {
        self.me.borrow().upgrade().expect("interpreter dropped")
    }

    pub fn add_file(&self, path: PathBuf, src: &'static str) -> usize {
        let mut files = self.files.borrow_mut();
        files.push(FileInfo { path, src, lines: Default::default() });
        files.len() - 1
    }

    /// Global-namespace context for `file`, shared (built once per file).
    pub fn file_ctx(&self, file: usize) -> Rc<NsCtx> {
        if let Some(Some(c)) = self.file_ctxs.borrow().get(file) {
            return c.clone();
        }
        let c = self.top_ctx(file, false);
        let mut v = self.file_ctxs.borrow_mut();
        if v.len() <= file {
            v.resize(file + 1, None);
        }
        v[file] = Some(c.clone());
        c
    }

    pub fn top_ctx(&self, file: usize, strict: bool) -> Rc<NsCtx> {
        Rc::new(NsCtx {
            ns: String::new(),
            uses: HashMap::new(),
            use_fns: HashMap::new(),
            use_consts: HashMap::new(),
            file,
            strict,
        })
    }

    pub fn file_path(&self, file: usize) -> String {
        self.files.borrow()[file].path.display().to_string()
    }

    /// Current file and line, for error messages.
    pub fn here(&self) -> (String, usize) {
        let Pos { file, off } = self.pos.get();
        let file = file as usize;
        let files = self.files.borrow();
        let f = &files[file.min(files.len() - 1)];
        (f.path.display().to_string(), f.line(off as usize))
    }

    pub fn line_of(&self, file: usize, off: u32) -> usize {
        let files = self.files.borrow();
        files[file].line(off as usize)
    }

    // ---- output ------------------------------------------------------

    pub fn output_started(&self) -> Option<(String, usize)> {
        self.output_start.borrow().clone()
    }

    pub fn echo(&self, b: &[u8]) {
        if !b.is_empty() && self.out.borrow().len() == 1 && self.output_start.borrow().is_none() {
            *self.output_start.borrow_mut() = Some(self.here());
        }
        let mut out = self.out.borrow_mut();
        out.last_mut().unwrap().extend_from_slice(b);
        if out.len() == 1 && out[0].len() > 64 * 1024 {
            let buf = std::mem::take(&mut out[0]);
            drop(out);
            let _ = self.writer.borrow_mut().write_all(&buf);
        }
    }

    pub fn flush(&self) {
        let mut out = self.out.borrow_mut();
        // Unclosed ob_start() buffers are flushed at exit, like PHP.
        let mut all = vec![];
        for b in out.iter_mut() {
            all.append(b);
        }
        drop(out);
        let mut w = self.writer.borrow_mut();
        let _ = w.write_all(&all);
        let _ = w.flush();
    }

    /// php://stdout: the process's stdout, past any ob_start() buffers
    /// (in order with echo output that is not buffered).
    pub fn write_stdout(&self, data: &[u8]) {
        if crate::server::sapi() == "cli-server" {
            let _ = std::io::stdout().write_all(data);
            return;
        }
        let base = std::mem::take(&mut self.out.borrow_mut()[0]);
        let mut w = self.writer.borrow_mut();
        let _ = w.write_all(&base);
        let _ = w.write_all(data);
        let _ = w.flush();
    }

    /// `return e;`: the value, or e's cell when this frame was called for a
    /// reference (`Interp::want_ref`).
    #[inline]
    fn return_value(&self, f: &mut Frame, e: &'static ast::Expr) -> R<Value> {
        if self.ref_return_depth.get() == f.depth && matches!(e.kind, ast::ExprKind::Var(_) | ast::ExprKind::Index { .. } | ast::ExprKind::Prop { .. } | ast::ExprKind::StaticProp { .. }) {
            return Ok(Value::Ref(self.make_ref(f, e)?));
        }
        self.eval(f, e)
    }

    pub fn warn(&self, msg: &str) {
        crate::errors::raise(self, crate::errors::E_WARNING, msg);
    }

    /// PHP's fatal "Allowed memory size ... exhausted" (not catchable).
    pub fn memory_fatal(&self, limit: usize, tried: usize) -> Ctrl {
        crate::alloc::set_limit(None);
        self.flush();
        let (file, line) = self.here();
        eprintln!("PHP Fatal error:  Allowed memory size of {limit} bytes exhausted (tried to allocate {tried} bytes) in {file} on line {line}");
        Ctrl::Exit(255)
    }

    pub fn notice(&self, msg: &str) {
        crate::errors::raise(self, crate::errors::E_NOTICE, msg);
    }

    pub fn deprecated(&self, msg: &str) {
        crate::errors::raise(self, crate::errors::E_DEPRECATED, msg);
    }

    // ---- running files -----------------------------------------------

    /// Parse a file and return its AST (leaked: code lives for the process).
    pub fn parse_file(&self, path: &Path, src: String) -> Result<(usize, &'static ast::File), String> {
        // Same file, same content: reuse its syntax tree (across includes,
        // and across the requests a server worker thread runs).
        let cached = PARSED.with(|p| p.borrow().get(path).filter(|(s, _)| **s == *src).copied());
        if let Some((s, file)) = cached {
            return Ok((self.add_file(path.to_path_buf(), s), file));
        }
        // Parsed code lives for the whole run (like opcache): not charged
        // to memory_limit.
        let before = crate::alloc::raw_used();
        let src: &'static str = Box::leak(src.into_boxed_str());
        // Inside a task, parse within the stack it has left (eval/include
        // can run deep in recursion).
        let (file, diags) = match phpx_runtime::stack_remaining() {
            Some(r) => phpx_syntax::stack::with_budget(r.saturating_sub(256 * 1024), || phpx_syntax::check(src)),
            None => phpx_syntax::check(src),
        };
        crate::alloc::exclude(crate::alloc::raw_used().saturating_sub(before));
        let id = self.add_file(path.to_path_buf(), src);
        let errors: Vec<String> = diags
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.render(&path.display().to_string(), src))
            .collect();
        if !errors.is_empty() {
            return Err(errors.concat());
        }
        let file: &'static ast::File = Box::leak(Box::new(file));
        PARSED.with(|p| p.borrow_mut().insert(path.to_path_buf(), (src, file)));
        Ok((id, file))
    }

    /// Execute a file's top level in `frame` (shared scope, like `include`).
    pub fn run_file_in(&self, frame: &mut Frame, file: &'static ast::File, id: usize) -> R<Value> {
        let saved = frame.ctx.clone();
        let r = self.run_stmts_top(frame, &file.stmts, id);
        frame.ctx = saved;
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(Value::Int(1)),
        }
    }

    /// Top-level statements: build namespace contexts, hoist declarations, run.
    pub(crate) fn run_stmts_top(&self, frame: &mut Frame, stmts: &'static [Stmt], file: usize) -> R<Flow> {
        let strict = stmts.iter().any(|s| match &s.kind {
            StmtKind::Declare { items, .. } => items.iter().any(|(k, v)| {
                k.name.eq_ignore_ascii_case("strict_types") && matches!(v.kind, ast::ExprKind::Int(1))
            }),
            _ => false,
        });
        {
            let mut fs = self.file_strict.borrow_mut();
            if fs.len() <= file {
                fs.resize(file + 1, false);
            }
            fs[file] = strict;
        }
        let segments = self.namespace_segments(stmts, file, strict);
        for (body, ctx) in &segments {
            self.hoist(body, ctx)?;
        }
        for (body, ctx) in segments {
            frame.ctx = ctx;
            let flow = match self.native_top_level(frame, body) {
                Some(r) => r?,
                None => self.exec_block(frame, body)?,
            };
            match flow {
                Flow::Normal => {}
                f => return Ok(f),
            }
        }
        Ok(Flow::Normal)
    }

    /// Split top-level statements into (statements, namespace context) runs.
    fn namespace_segments(&self, stmts: &'static [Stmt], file: usize, strict: bool) -> Vec<(&'static [Stmt], Rc<NsCtx>)> {
        let mut out = vec![];
        let mut start = 0;
        let mut ctx = NsCtx { ns: String::new(), uses: HashMap::new(), use_fns: HashMap::new(), use_consts: HashMap::new(), file, strict };
        let finish = |ctx: &NsCtx| NsCtx {
            ns: ctx.ns.clone(),
            uses: ctx.uses.clone(),
            use_fns: ctx.use_fns.clone(),
            use_consts: ctx.use_consts.clone(),
            file,
            strict,
        };
        for (i, s) in stmts.iter().enumerate() {
            match &s.kind {
                StmtKind::Namespace { name, body: None } => {
                    out.push((&stmts[start..i], Rc::new(finish(&ctx))));
                    start = i + 1;
                    ctx = NsCtx {
                        ns: name.clone().unwrap_or_default(),
                        uses: HashMap::new(),
                        use_fns: HashMap::new(),
                        use_consts: HashMap::new(),
                        file,
                        strict,
                    };
                }
                StmtKind::Namespace { name, body: Some(b) } => {
                    out.push((&stmts[start..i], Rc::new(finish(&ctx))));
                    start = i + 1;
                    let mut inner = NsCtx {
                        ns: name.clone().unwrap_or_default(),
                        uses: HashMap::new(),
                        use_fns: HashMap::new(),
                        use_consts: HashMap::new(),
                        file,
                        strict,
                    };
                    collect_uses(b, &mut inner);
                    out.push((&b[..], Rc::new(inner)));
                }
                StmtKind::Use { .. } => collect_uses(std::slice::from_ref(s), &mut ctx),
                _ => {}
            }
        }
        out.push((&stmts[start..], Rc::new(finish(&ctx))));
        out.retain(|(b, _)| !b.is_empty());
        out
    }

    /// Declare top-level functions, then classes in as many passes as it
    /// takes for parents to appear (PHP's early binding).
    fn hoist(&self, stmts: &'static [Stmt], ctx: &Rc<NsCtx>) -> R<()> {
        let mut pending = vec![];
        for s in stmts {
            match &s.kind {
                StmtKind::Function(f) => self.declare_function(f, ctx),
                StmtKind::Class(c) => pending.push(&**c),
                _ => {}
            }
        }
        loop {
            let before = pending.len();
            pending.retain(|c| !(self.deps_ready(c, ctx) && self.declare_class(c, ctx).is_ok()));
            if pending.len() == before {
                // The rest are declared when execution reaches them.
                return Ok(());
            }
        }
    }

    pub fn declare_function(&self, f: &'static ast::Function, ctx: &Rc<NsCtx>) {
        let full = qualify(&ctx.ns, &f.name.name);
        let key = self.lower.get(&full);
        self.decl_ctx.borrow_mut().insert(f as *const ast::Function as usize, ctx.clone());
        if !ctx.ns.is_empty() && self.builtins.contains_key(&*f.name.name.to_ascii_lowercase()) {
            // A namespaced function shadows a built-in for unqualified calls
            // in that namespace: compiled code stops inlining built-ins.
            crate::value::BUILTIN_SHADOWED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.functions.borrow_mut().insert(key, Rc::new(UserFn { name: Rc::from(full), decl: f, ctx: ctx.clone() }));
    }

    // ---- statements --------------------------------------------------

    pub fn exec_block(&self, f: &mut Frame, stmts: &'static [Stmt]) -> R<Flow> {
        if stmts.is_empty() && crate::value::DESTRUCT_PENDING.load(std::sync::atomic::Ordering::Relaxed) {
            // Empty loop bodies still notice time/memory limits.
            self.run_destructors()?;
        }
        let mut i = 0;
        while i < stmts.len() {
            let flow = self.exec(f, &stmts[i]);
            // An error handler that threw surfaces after the statement.
            match crate::errors::check(flow)? {
                Flow::Normal => i += 1,
                Flow::Goto(label) => match label_index(stmts, label) {
                    // Jump within this list (backward or forward).
                    Some(at) => i = at + 1,
                    None => return Ok(Flow::Goto(label)),
                },
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    /// register_shutdown_function() callbacks (they may register more).
    /// Some(code) when one called exit().
    pub fn run_shutdown(&self) -> R<Option<i32>> {
        loop {
            let next = {
                let mut r = self.response.borrow_mut();
                if r.shutdown.is_empty() {
                    return Ok(None);
                }
                r.shutdown.remove(0)
            };
            match self.call_callable(&next.0, next.1, vec![]) {
                Ok(_) => {}
                Err(Ctrl::Exit(c)) => return Ok(Some(c)),
                Err(e) => return Err(e),
            }
        }
    }

    /// Run queued `__destruct` calls (objects whose last reference died).
    /// Run queued destructors now (not the other statement-boundary work):
    /// a temporary object dies as soon as the call made on it returns.
    pub fn destruct_now(&self) -> R<()> {
        use crate::value::DESTRUCT_QUEUE;
        let q = DESTRUCT_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
        if q.is_empty() {
            return Ok(());
        }
        let saved = self.pos.get();
        let mut r = Ok(());
        for o in q {
            if r.is_ok() {
                r = self.call_method(&o, "__destruct", vec![]).map(drop);
            }
        }
        self.pos.set(saved);
        r
    }

    pub fn run_destructors(&self) -> R<()> {
        use crate::value::{DESTRUCT_PENDING, DESTRUCT_QUEUE};
        while DESTRUCT_PENDING.swap(false, std::sync::atomic::Ordering::Relaxed) {
            if let Some((limit, tried)) = crate::alloc::take_over() {
                return Err(self.memory_fatal(limit, tried));
            }
            if let Some(secs) = crate::timeout::take_expired() {
                self.flush();
                let (file, line) = self.here();
                eprintln!("PHP Fatal error:  Maximum execution time of {secs} second{} exceeded in {file} on line {line}", if secs == 1 { "" } else { "s" });
                return Err(Ctrl::Exit(255));
            }
            if crate::gc::take_due() {
                crate::gc::collect();
            }
            let q = DESTRUCT_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
            let saved = self.pos.get();
            let mut r = Ok(());
            for o in q {
                if r.is_ok() {
                    r = self.call_method(&o, "__destruct", vec![]).map(drop);
                }
            }
            self.pos.set(saved);
            r?;
        }
        Ok(())
    }

    fn loop_flow(flow: Flow) -> Option<Flow> {
        // None = keep looping; Some(f) = leave the loop with f.
        match flow {
            Flow::Normal | Flow::Continue(1) => None,
            Flow::Break(1) => Some(Flow::Normal),
            Flow::Break(n) => Some(Flow::Break(n - 1)),
            Flow::Continue(n) => Some(Flow::Continue(n - 1)),
            r @ (Flow::Return(_) | Flow::Goto(_)) => Some(r),
        }
    }

    /// Hot statements only; see `eval` for why the rest is split out.
    pub fn exec(&self, f: &mut Frame, s: &'static Stmt) -> R<Flow> {
        if crate::value::DESTRUCT_PENDING.load(std::sync::atomic::Ordering::Relaxed) {
            self.run_destructors()?;
        }
        // Built-in classes written in PHP (the prelude, file 0) behave like
        // internal code: diagnostics point at the caller's statement.
        if f.ctx.file != PRELUDE_FILE {
            self.pos.set(Pos { file: f.ctx.file as u32, off: s.span.start });
        }
        match &s.kind {
            // A bare `$var;` is a no-op in PHP: no undefined-variable warning.
            StmtKind::Expr(e) if matches!(e.kind, ast::ExprKind::Var(_)) => Ok(Flow::Normal),
            StmtKind::Expr(e) => {
                self.eval(f, e)?;
                Ok(Flow::Normal)
            }
            StmtKind::Return(Some(e)) => Ok(Flow::Return(self.return_value(f, e)?)),
            StmtKind::If { cond, then, elseifs, else_ } if elseifs.is_empty() => {
                if self.eval(f, cond)?.truthy() {
                    self.exec_block(f, then)
                } else if let Some(b) = else_ {
                    self.exec_block(f, b)
                } else {
                    Ok(Flow::Normal)
                }
            }
            _ => self.exec_rest(f, s),
        }
    }

    /// A variable name made at run time, as a `'static` key.
    pub fn intern(&self, s: &str) -> &'static str {
        if let Some(n) = self.names.borrow().get(s) {
            return n;
        }
        let n: &'static str = Box::leak(s.into());
        self.names.borrow_mut().insert(n);
        n
    }

    /// The name a `$$x` / `$GLOBALS[x]` / `global $$x` refers to.
    pub fn var_name(&self, f: &mut Frame, e: &'static ast::Expr) -> R<&'static str> {
        match &e.kind {
            ast::ExprKind::Str(s) => Ok(s),
            _ => {
                let v = self.eval(f, e)?;
                let s = self.to_str(&v)?;
                Ok(self.intern(&String::from_utf8_lossy(&s)))
            }
        }
    }

    /// The global scope's variables. `f` is the running frame (which may be
    /// the global one). The main frame outlives every call it makes, and
    /// nothing else holds its `vars` across this closure.
    pub fn with_globals<T>(&self, f: &mut Frame, g: impl FnOnce(&mut Vars) -> T) -> T {
        let p = self.globals.get();
        if p.is_null() || std::ptr::eq(p, f) {
            g(&mut f.vars)
        } else {
            g(unsafe { &mut (*p).vars })
        }
    }

    pub fn global_slot(&self, f: &mut Frame, name: &'static str) -> Slot {
        self.with_globals(f, |v| match v.get(name) {
            Some(s) => s.clone(),
            None => {
                let s = slot(Value::Null);
                v.insert(name, s.clone());
                s
            }
        })
    }

    /// `$GLOBALS` as a value: a read-only copy (PHP 8.1+).
    pub fn globals_array(&self, f: &mut Frame) -> Value {
        let mut out = crate::value::PhpArray::new();
        self.with_globals(f, |v| {
            for (n, s) in v.iter() {
                out.insert(crate::value::Key::Str(crate::value::pstr(n)), s.borrow().clone());
            }
        });
        Value::array(out)
    }

    #[inline(never)]
    fn exec_rest(&self, f: &mut Frame, s: &'static Stmt) -> R<Flow> {
        match &s.kind {
            StmtKind::Global(vars) => {
                for e in vars {
                    let name = match &e.kind {
                        ast::ExprKind::VarVar(n) => self.var_name(f, n)?,
                        ast::ExprKind::Var(n) => n,
                        _ => continue,
                    };
                    let s = self.global_slot(f, name);
                    f.vars.insert(name, s);
                }
            }
            StmtKind::Expr(e) => {
                self.eval(f, e)?;
            }
            StmtKind::Echo(es) => {
                for e in es {
                    let v = self.eval(f, e)?;
                    let b = self.to_str(&v)?;
                    self.echo(&b);
                }
            }
            StmtKind::InlineHtml(h) => self.echo(h.as_bytes()),
            StmtKind::If { cond, then, elseifs, else_ } => {
                if self.eval(f, cond)?.truthy() {
                    return self.exec_block(f, then);
                }
                for (c, b) in elseifs {
                    if self.eval(f, c)?.truthy() {
                        return self.exec_block(f, b);
                    }
                }
                if let Some(b) = else_ {
                    return self.exec_block(f, b);
                }
            }
            StmtKind::While { cond, body } => {
                let mut n = 0u32;
                while self.eval(f, cond)?.truthy() {
                    if let Some(fl) = Self::loop_flow(self.exec_block(f, body)?) {
                        return Ok(fl);
                    }
                    n += 1;
                    if n == crate::compiler::OSR_ITERATIONS {
                        if let Some(r) = self.try_osr(f, s) {
                            return Ok(Flow::Return(r?));
                        }
                    }
                }
            }
            StmtKind::DoWhile { body, cond } => {
                let mut n = 0u32;
                loop {
                    if let Some(fl) = Self::loop_flow(self.exec_block(f, body)?) {
                        return Ok(fl);
                    }
                    n += 1;
                    if n == crate::compiler::OSR_ITERATIONS {
                        if let Some(r) = self.try_osr(f, s) {
                            return Ok(Flow::Return(r?));
                        }
                    }
                    if !self.eval(f, cond)?.truthy() {
                        break;
                    }
                }
            }
            StmtKind::For { init, cond, step, body } => {
                for e in init {
                    self.eval(f, e)?;
                }
                let mut n = 0u32;
                loop {
                    // Only the last condition expression decides.
                    let mut ok = true;
                    for (i, c) in cond.iter().enumerate() {
                        let v = self.eval(f, c)?;
                        if i == cond.len() - 1 {
                            ok = v.truthy();
                        }
                    }
                    if !ok {
                        break;
                    }
                    if let Some(fl) = Self::loop_flow(self.exec_block(f, body)?) {
                        return Ok(fl);
                    }
                    for e in step {
                        self.eval(f, e)?;
                    }
                    n += 1;
                    if n == crate::compiler::OSR_ITERATIONS {
                        if let Some(r) = self.try_osr(f, s) {
                            return Ok(Flow::Return(r?));
                        }
                    }
                }
            }
            StmtKind::Foreach { subject, key, value, by_ref: true, body } => {
                return self.exec_foreach_ref(f, subject, key.as_ref(), value, body)
            }
            StmtKind::Foreach { subject, key, value, body, .. } => return self.exec_foreach(f, subject, key.as_ref(), value, body),
            StmtKind::Switch { subject, cases } => {
                let v = self.eval(f, subject)?;
                let mut matched = None;
                for (i, c) in cases.iter().enumerate() {
                    if let Some(t) = &c.test {
                        if crate::value::loose_eq(&v, &self.eval(f, t)?) {
                            matched = Some(i);
                            break;
                        }
                    }
                }
                let start = matched.or_else(|| cases.iter().position(|c| c.test.is_none()));
                if let Some(start) = start {
                    for c in &cases[start..] {
                        match self.exec_block(f, &c.body)? {
                            Flow::Normal => {}
                            // `switch` counts as a loop level for break and continue.
                            Flow::Break(1) | Flow::Continue(1) => return Ok(Flow::Normal),
                            Flow::Break(n) => return Ok(Flow::Break(n - 1)),
                            Flow::Continue(n) => return Ok(Flow::Continue(n - 1)),
                            r => return Ok(r),
                        }
                    }
                }
            }
            StmtKind::Break(lvl) => return Ok(Flow::Break(self.level(f, lvl)?)),
            StmtKind::Continue(lvl) => return Ok(Flow::Continue(self.level(f, lvl)?)),
            StmtKind::Return(e) => {
                let v = match e {
                    Some(e) => self.return_value(f, e)?,
                    None => Value::Null,
                };
                return Ok(Flow::Return(v));
            }
            StmtKind::Function(func) => {
                let full = qualify(&f.ctx.ns, &func.name.name);
                if !self.functions.borrow().contains_key(&*self.lower.get(&full)) {
                    self.declare_function(func, &f.ctx);
                }
            }
            StmtKind::Class(c) => {
                if !self.declared.borrow().contains(&(&**c as *const ast::Class as usize)) {
                    let ctx = f.ctx.clone();
                    self.declare_class(c, &ctx)?;
                }
            }
            StmtKind::Try { body, catches, finally } => return self.exec_try(f, body, catches, finally.as_deref()),
            StmtKind::Goto(label) => return Ok(Flow::Goto(&label.name)),
            StmtKind::Namespace { .. }
            | StmtKind::Use { .. }
            | StmtKind::Nop
            | StmtKind::Label(_)
            | StmtKind::Declare { body: None, .. } => {}
            StmtKind::Declare { body: Some(b), .. } | StmtKind::Block(b) => return self.exec_block(f, b),
            StmtKind::Const(items) => {
                for (name, e) in items {
                    let v = self.eval(f, e)?;
                    self.constants.borrow_mut().insert(qualify(&f.ctx.ns, &name.name), v);
                }
            }
            StmtKind::StaticVar(vars) => {
                let key = (f.fn_id, f.class.as_ref().map(|c| c.lname.clone()).unwrap_or_else(|| Rc::from("")));
                for (name, init) in vars {
                    let existing = self.statics.borrow().get(&key).and_then(|m| m.get(&*name.name).cloned());
                    let s = match existing {
                        Some(s) => s,
                        None => {
                            let v = match init {
                                Some(e) => self.eval(f, e)?,
                                None => Value::Null,
                            };
                            let s = slot(v);
                            self.statics.borrow_mut().entry(key.clone()).or_default().insert(Rc::from(&*name.name), s.clone());
                            s
                        }
                    };
                    f.vars.insert(&name.name, s);
                }
            }
            StmtKind::Unset(es) => {
                for e in es {
                    self.unset(f, e)?;
                }
            }
        }
        Ok(Flow::Normal)
    }

    fn level(&self, f: &mut Frame, lvl: &'static Option<ast::Expr>) -> R<u32> {
        match lvl {
            None => Ok(1),
            Some(e) => match self.eval(f, e)? {
                Value::Int(n) if n >= 1 => Ok(n as u32),
                _ => Err(self.error("Error", "'break' operator accepts only positive integers")),
            },
        }
    }

    fn exec_try(
        &self,
        f: &mut Frame,
        body: &'static [Stmt],
        catches: &'static [ast::Catch],
        finally: Option<&'static [Stmt]>,
    ) -> R<Flow> {
        let mut result = self.exec_block(f, body);
        if let Err(Ctrl::Throw(exc)) = &result {
            let exc = exc.clone();
            'outer: for c in catches {
                for t in &c.types {
                    let name = self.resolve_class_name(&f.ctx, t);
                    let lname = self.lower.get(&name);
                    if exc.class.is_a(&lname) {
                        if let Some(v) = &c.var {
                            *f.var_slot(&v.name).borrow_mut() = Value::Object(exc.clone());
                        }
                        result = self.exec_block(f, &c.body);
                        break 'outer;
                    }
                }
            }
        }
        if let Some(fin) = finally {
            // Exit skips finally, like PHP's exit().
            if matches!(result, Err(Ctrl::Exit(_))) {
                return result;
            }
            match self.exec_block(f, fin)? {
                Flow::Normal => {}
                // A return/break in finally overrides everything, even a throw.
                other => return Ok(other),
            }
        }
        result
    }

    fn exec_foreach(
        &self,
        f: &mut Frame,
        subject: &'static ast::Expr,
        key: Option<&'static ast::Expr>,
        value: &'static ast::Expr,
        body: &'static [Stmt],
    ) -> R<Flow> {
        let subj = self.eval(f, subject)?;
        match subj {
            Value::Array(arr) => {
                // By-value foreach iterates a snapshot: writes to the array
                // inside the loop don't affect iteration.
                for i in 0..arr.len() {
                    let (k, v) = arr.get_index(i).unwrap();
                    if let Some(ke) = key {
                        self.assign_to(f, ke, k.to_value())?;
                    }
                    self.assign_to(f, value, v.clone())?;
                    if let Some(fl) = Self::loop_flow(self.exec_block(f, body)?) {
                        return Ok(fl);
                    }
                }
                Ok(Flow::Normal)
            }
            Value::Object(o) => {
                if o.class.is_a("iteratoraggregate") {
                    let it = self.call_method(&o, "getIterator", vec![])?;
                    let Value::Object(it) = it else {
                        return Err(self.error("TypeError", "getIterator() must return a Traversable"));
                    };
                    return self.foreach_iterator(f, &it, key, value, body);
                }
                if o.class.is_a("iterator") {
                    return self.foreach_iterator(f, &o, key, value, body);
                }
                // Plain object: visible properties.
                let props: Vec<(Rc<str>, Value)> = o
                    .props
                    .borrow()
                    .iter()
                    .filter(|(n, _)| self.prop_visible(f, &o, n))
                    .map(|(n, v)| (Rc::from(crate::call::plain_prop(n)), v.clone()))
                    .collect();
                for (n, v) in props {
                    if let Some(ke) = key {
                        self.assign_to(f, ke, Value::str(&n))?;
                    }
                    self.assign_to(f, value, v)?;
                    if let Some(fl) = Self::loop_flow(self.exec_block(f, body)?) {
                        return Ok(fl);
                    }
                }
                Ok(Flow::Normal)
            }
            other => {
                self.warn(&format!("foreach() argument must be of type array|object, {} given", other.type_name()));
                Ok(Flow::Normal)
            }
        }
    }

    /// `foreach ($arr as &$v)`: iterates the live array (appends during the
    /// loop are visited) and binds `$v` to each element's cell. Like PHP,
    /// `$v` still references the last element after the loop.
    fn exec_foreach_ref(
        &self,
        f: &mut Frame,
        subject: &'static ast::Expr,
        key: Option<&'static ast::Expr>,
        value: &'static ast::Expr,
        body: &'static [Stmt],
    ) -> R<Flow> {
        let place = if subject.is_lvalue() {
            self.place(f, subject)?
        } else {
            // Temporary (e.g. a function result): iterate a private copy.
            let v = self.eval(f, subject)?;
            crate::expr::Place { root: crate::expr::Root::Var(slot(v)), path: vec![] }
        };
        let mut i = 0;
        loop {
            let k = match self.place_get(f, &place)? {
                Some(Value::Array(a)) => match a.get_index(i) {
                    Some((k, _)) => k.clone(),
                    None => break,
                },
                Some(Value::Object(_)) => {
                    return Err(self.error("Error", "phpx: foreach by reference over objects is not supported yet"));
                }
                Some(Value::Null) | None if i == 0 => {
                    self.warn("foreach() argument must be of type array|object, null given");
                    break;
                }
                _ => break,
            };
            let mut elem = place.clone();
            elem.path.push(Some(crate::expr::Off::Key(k.clone())));
            let cell = self.place_ref(&elem)?;
            self.bind_ref(f, value, cell)?;
            if let Some(ke) = key {
                self.assign_to(f, ke, k.to_value())?;
            }
            if let Some(fl) = Self::loop_flow(self.exec_block(f, body)?) {
                return Ok(fl);
            }
            i += 1;
        }
        Ok(Flow::Normal)
    }

    fn foreach_iterator(
        &self,
        f: &mut Frame,
        it: &Rc<Obj>,
        key: Option<&'static ast::Expr>,
        value: &'static ast::Expr,
        body: &'static [Stmt],
    ) -> R<Flow> {
        self.call_method(it, "rewind", vec![])?;
        while self.call_method(it, "valid", vec![])?.truthy() {
            let v = self.call_method(it, "current", vec![])?;
            if let Some(ke) = key {
                let k = self.call_method(it, "key", vec![])?;
                self.assign_to(f, ke, k)?;
            }
            self.assign_to(f, value, v)?;
            if let Some(fl) = Self::loop_flow(self.exec_block(f, body)?) {
                return Ok(fl);
            }
            self.call_method(it, "next", vec![])?;
        }
        Ok(Flow::Normal)
    }

    // ---- errors --------------------------------------------------------

    /// Build an exception object of a built-in class, ready to throw.
    pub fn error(&self, class: &str, msg: &str) -> Ctrl {
        let lname = class.to_ascii_lowercase();
        let cls = self.classes.borrow().get(&*lname).cloned().expect("built-in exception class missing");
        let o = Obj::new(cls.clone(), crate::value::Internal::None);
        self.init_props(&o, &cls).ok();
        o.set_prop("message", Value::str(msg));
        self.set_throw_site(&o);
        static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *TRACE.get_or_init(|| std::env::var_os("PHPX_TRACE_THROW").is_some()) {
            let (file, line) = self.here();
            eprintln!("[throw] {class}: {msg} at {file}:{line}");
            if std::env::var("PHPX_TRACE_THROW").as_deref() == Ok("2") {
                for e in crate::trace::entries(self, false, 0).iter().take(12) {
                    eprintln!("    {}:{} {}", e.file.as_deref().unwrap_or("[internal]"), e.line, e.function);
                }
            }
        }
        Ctrl::Throw(o)
    }

    pub fn set_throw_site(&self, o: &Rc<Obj>) {
        let (file, line) = self.here();
        o.set_prop("file", Value::str(&file));
        o.set_prop("line", Value::Int(line as i64));
        let args = self.ini_get("zend.exception_ignore_args").is_none_or(|v| v != "1" && !v.eq_ignore_ascii_case("on"));
        o.set_prop("trace", crate::trace::as_array(&crate::trace::entries(self, args, 0), false, args));
    }

    /// Render an uncaught exception like PHP's fatal error message.
    pub fn uncaught_message(&self, o: &Rc<Obj>) -> String {
        let s = |n: &str| o.get_prop(n).and_then(|v| v.scalar_bytes()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let (file, line) = (s("file"), s("line"));
        // PHP prints the exception's string form (its chain and traces).
        let text = match self.call_method(o, "__toString", vec![]) {
            Ok(v) => v.scalar_bytes().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default(),
            Err(_) => format!("{}: {} in {file}:{line}\nStack trace:\n#0 {{main}}", o.class.name, s("message")),
        };
        format!("PHP Fatal error:  Uncaught {text}\n  thrown in {file} on line {line}")
    }

    pub fn static_prop_slot(&self, sp: &Rc<StaticProp>) -> R<Value> {
        if let Some(v) = sp.value.borrow().as_ref() {
            return Ok(v.deref());
        }
        let v = match sp.init {
            Some(e) => {
                let decl = self.classes.borrow().get(&*sp.declaring).cloned();
                let mut fr = Frame::new(sp.ctx.clone(), 0);
                fr.class = decl.clone();
                fr.static_class = decl;
                self.eval(&mut fr, e)?
            }
            None => Value::Null,
        };
        *sp.value.borrow_mut() = Some(v.clone());
        Ok(v)
    }
}

fn collect_uses(stmts: &[Stmt], ctx: &mut NsCtx) {
    for s in stmts {
        let StmtKind::Use { kind, items } = &s.kind else { continue };
        for it in items {
            let name = it.name.trim_start_matches('\\').to_string();
            let alias = it
                .alias
                .as_ref()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| name.rsplit('\\').next().unwrap().to_string());
            match it.kind.unwrap_or(*kind) {
                ast::UseKind::Normal => ctx.uses.insert(alias.to_ascii_lowercase(), name),
                ast::UseKind::Function => ctx.use_fns.insert(alias.to_ascii_lowercase(), name),
                ast::UseKind::Const => ctx.use_consts.insert(alias, name),
            };
        }
    }
}

fn label_index(stmts: &[Stmt], label: &str) -> Option<usize> {
    stmts.iter().position(|s| matches!(&s.kind, StmtKind::Label(l) if l.name == label))
}

pub fn qualify(ns: &str, name: &str) -> String {
    if ns.is_empty() { name.to_string() } else { format!("{ns}\\{name}") }
}
