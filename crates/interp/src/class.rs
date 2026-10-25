//! Runtime class model: flattened at declaration time (inherited members
//! copied in), so lookups never walk the parent chain.

use std::cell::RefCell;
use std::rc::Rc;

use indexmap::IndexMap;
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
    pub vis: Vis,
    /// Lowercase name of the declaring class (`self::` inside the body).
    pub declaring: Rc<str>,
    pub ctx: Rc<NsCtx>,
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
    pub consts: RefCell<IndexMap<Rc<str>, ConstSlot>>,
    /// Shared with the parent unless redeclared (PHP semantics).
    pub static_props: IndexMap<Rc<str>, Rc<StaticProp>>,
    pub props: Vec<PropDecl>,
    pub methods: crate::value::FxIndexMap<Rc<str>, Rc<Method>>,
    /// Enum case names in declaration order.
    pub cases: Vec<Rc<str>>,
    pub enum_backed: bool,
    pub has_destruct: bool,
    /// Traits used directly by this class.
    pub traits: Vec<Rc<str>>,
    /// Initial property map of a new instance (built on first instantiation).
    pub prop_template: RefCell<Option<crate::value::FxIndexMap<Rc<str>, crate::value::Value>>>,
}

impl Class {
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
