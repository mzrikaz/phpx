//! Runtime class model: flattened at declaration time (inherited members
//! copied in), so lookups never walk the parent chain.

use std::cell::RefCell;
use std::rc::Rc;

use phpx_syntax::ast;

use crate::interp::{NativeMethod, NsCtx};
use crate::value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vis {
    Public,
    Protected,
    Private,
}

impl Vis {
    pub fn of(m: &ast::Modifiers) -> Vis {
        if m.private {
            Vis::Private
        } else if m.protected {
            Vis::Protected
        } else {
            Vis::Public
        }
    }
}

#[derive(Clone)]
pub enum PropDefault {
    /// Typed property without default: uninitialized until assigned.
    Uninit,
    Value(Value),
    Expr(&'static ast::Expr),
}

#[derive(Clone)]
pub struct PropDecl {
    pub name: Rc<str>,
    pub default: PropDefault,
    pub ty: Option<&'static ast::Type>,
    pub vis: Vis,
    pub readonly: bool,
    /// Class that declared it (shown for private props in dumps).
    pub declaring: Rc<str>,
    pub ctx: Rc<NsCtx>,
}

#[derive(Clone)]
pub enum MethodBody {
    User(&'static ast::Function),
    Native(NativeMethod),
}

#[derive(Clone)]
pub struct Method {
    pub name: Rc<str>,
    pub body: MethodBody,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    pub vis: Vis,
    /// Lowercase name of the declaring class (`self::` inside the body).
    pub declaring: Rc<str>,
    pub ctx: Rc<NsCtx>,
    /// The declaring class and `Class::method` display name, looked up on
    /// the first call.
    pub cache: MethodCache,
}

/// Per-method lookups done once. Cloning a method (a trait's, copied into
/// a class) starts it empty: the copy has another declaring class.
#[derive(Default)]
pub struct MethodCache(std::cell::OnceCell<(std::rc::Weak<Class>, Rc<str>)>);

impl Clone for MethodCache {
    fn clone(&self) -> Self {
        MethodCache::default()
    }
}

impl Method {
    /// (declaring class, `Class::method`).
    pub fn declared(&self, classes: &rustc_hash::FxHashMap<Rc<str>, Rc<Class>>) -> (Option<Rc<Class>>, Rc<str>) {
        if let Some((w, d)) = self.cache.0.get() {
            if let Some(c) = w.upgrade() {
                return (Some(c), d.clone());
            }
        }
        let class = classes.get(&*self.declaring).cloned();
        let display: Rc<str> = Rc::from(format!("{}::{}", class.as_ref().map(|c| &*c.name).unwrap_or(""), self.name));
        if let Some(c) = &class {
            let _ = self.cache.0.set((Rc::downgrade(c), display.clone()));
        }
        (class, display)
    }
}

#[derive(Clone)]
pub enum ConstSlot {
    Pending { expr: &'static ast::Expr, declaring: Rc<str>, ctx: Rc<NsCtx> },
    Evaluating,
    Ready(Value),
    /// Enum case, created on first access (PHP allocates the object lazily).
    EnumCase { name: Rc<str>, value: Option<&'static ast::Expr>, ctx: Rc<NsCtx> },
}

pub struct StaticProp {
    /// None until first access, then the evaluated default.
    pub value: RefCell<Option<Value>>,
    pub init: Option<&'static ast::Expr>,
    pub vis: Vis,
    pub declaring: Rc<str>,
    pub ctx: Rc<NsCtx>,
}

pub struct Class {
    pub name: Rc<str>,
    pub lname: Rc<str>,
    pub kind: ast::ClassKind,
    pub parent: Option<Rc<Class>>,
    /// Lowercase names of this class, every ancestor and every interface.
    pub ancestors: Vec<Rc<str>>,
    pub is_abstract: bool,
    pub consts: RefCell<crate::value::FxIndexMap<Rc<str>, ConstSlot>>,
    /// The constants before any was evaluated (a class reused by a later
    /// request starts from these).
    pub consts_init: crate::value::FxIndexMap<Rc<str>, ConstSlot>,
    /// The interfaces and traits it was built with, as resolved: a later
    /// request reuses the class only when they resolve the same.
    pub deps: Vec<Rc<Class>>,
    /// Constants marked `#[\Deprecated]`: name -> (declaring class name,
    /// the attribute, its namespace context).
    pub deprecated_consts: Vec<(Rc<str>, Rc<str>, &'static ast::Attribute, Rc<NsCtx>)>,
    /// Shared with the parent unless redeclared (PHP semantics).
    pub static_props: crate::value::FxIndexMap<Rc<str>, Rc<StaticProp>>,
    pub props: Vec<PropDecl>,
    pub methods: crate::value::FxIndexMap<Rc<str>, Rc<Method>>,
    /// Enum case names in declaration order.
    pub cases: Vec<Rc<str>>,
    pub enum_backed: bool,
    pub has_destruct: bool,
    /// Traits used directly by this class.
    pub traits: Vec<Rc<str>>,
    /// Initial property map of a new instance (built on first instantiation).
    pub prop_template: RefCell<Option<Vec<Option<crate::value::Value>>>>,
    /// Private properties of ancestors that a subclass redeclared: each
    /// object keeps both, the ancestor's under a mangled key
    /// (declaring lname, plain name, mangled key).
    pub shadowed: Vec<(Rc<str>, Rc<str>, Rc<str>)>,
    /// `__construct` (own or inherited), resolved once.
    pub ctor: Option<Rc<Method>>,
    /// Can be `new`ed (a concrete class) and is not Throwable: the fast path.
    pub plain_new: bool,
    /// Shape of new instances: the declared properties in order.
    pub shape: Rc<crate::props::Shape>,
    /// The declaration and its namespace context (Reflection: attributes,
    /// doc comments, lines, modifiers).
    pub decl: &'static ast::Class,
    pub decl_ctx: Rc<NsCtx>,
    /// An anonymous class's real name, `Base@anonymous\0file:line$n`
    /// (get_class(), ::class); `name` is what messages and dumps show.
    pub anon_name: Option<Rc<str>>,
    /// __get, __set, __unset, __isset (indexed like MAGIC_NAMES).
    pub magic: [Option<Rc<Method>>; 4],
    /// `php_name()` as a PHP string (::class, get_class()).
    pub name_value: crate::value::Value,
}

/// A class name as messages and dumps show it: an anonymous class's real
/// name stops at its NUL (PHP prints C strings there).
pub fn display_name(name: &str) -> &str {
    name.split('\0').next().unwrap_or(name)
}

impl Class {
    /// The name PHP code sees (get_class(), ::class, Reflection).
    pub fn php_name(&self) -> &Rc<str> {
        self.anon_name.as_ref().unwrap_or(&self.name)
    }

    pub fn is_enum(&self) -> bool {
        self.kind == ast::ClassKind::Enum
    }

    /// Methods for get_class_methods(): own first, then inherited.
    pub fn method_order(&self) -> Vec<Rc<Method>> {
        let own = self.methods.values().filter(|m| m.declaring == self.lname).cloned();
        let inherited = self.methods.values().filter(|m| m.declaring != self.lname).cloned();
        own.chain(inherited).collect()
    }

    pub fn find_method(&self, lname: &str) -> Option<&Rc<Method>> {
        self.methods.get(lname)
    }

    /// The method `$obj->name()` runs when called from `scope`: private
    /// methods belong to their class, so the scope's own private method
    /// wins over a same-named one of a subclass (PHP semantics).
    pub fn find_method_from<'a>(&'a self, lname: &str, scope: Option<&'a Class>) -> Option<&'a Rc<Method>> {
        if let Some(sc) = scope {
            if let Some(m) = sc.methods.get(lname) {
                if m.vis == Vis::Private && *m.declaring == *sc.lname && !std::ptr::eq(sc, self) && self.is_subclass_of(sc) {
                    return Some(m);
                }
            }
        }
        self.methods.get(lname)
    }

    pub fn find_prop(&self, name: &str) -> Option<&PropDecl> {
        self.props.iter().find(|p| &*p.name == name)
    }

    /// `instanceof` by lowercase name.
    pub fn is_a(&self, lname: &str) -> bool {
        self.ancestors.iter().any(|a| &**a == lname)
    }

    pub fn is_subclass_of(&self, other: &Class) -> bool {
        self.is_a(&other.lname)
    }
}
