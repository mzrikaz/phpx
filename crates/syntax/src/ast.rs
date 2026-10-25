//! Syntax tree for the phpx subset. Names are kept as written; resolution
//! against `namespace`/`use` happens in a later pass.

use crate::diag::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub stmts: Vec<Stmt>,
}

/// Name as written: `Foo`, `Foo\Bar`, `\Foo\Bar`, `namespace\Foo`.
pub type Name = String;

#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Expr(Expr),
    Echo(Vec<Expr>),
    InlineHtml(String),
    If { cond: Expr, then: Vec<Stmt>, elseifs: Vec<(Expr, Vec<Stmt>)>, else_: Option<Vec<Stmt>> },
    While { cond: Expr, body: Vec<Stmt> },
    DoWhile { body: Vec<Stmt>, cond: Expr },
    For { init: Vec<Expr>, cond: Vec<Expr>, step: Vec<Expr>, body: Vec<Stmt> },
    /// `by_ref`: `foreach ($a as &$v)` aliases each element.
    Foreach { subject: Expr, key: Option<Expr>, value: Expr, by_ref: bool, body: Vec<Stmt> },
    Switch { subject: Expr, cases: Vec<Case> },
    Break(Option<Expr>),
    Continue(Option<Expr>),
    Return(Option<Expr>),
    Function(Box<Function>),
    Class(Box<Class>),
    Try { body: Vec<Stmt>, catches: Vec<Catch>, finally: Option<Vec<Stmt>> },
    /// `namespace Foo;` (body None) or `namespace Foo { ... }`.
    Namespace { name: Option<Name>, body: Option<Vec<Stmt>> },
    Use { kind: UseKind, items: Vec<UseItem> },
    Const(Vec<(Ident, Expr)>),
    /// `static $count = 0;` inside a function.
    StaticVar(Vec<(Ident, Option<Expr>)>),
    /// `global $a, $$b;`: bind locals to the global variables (Var / VarVar items).
    Global(Vec<Expr>),
    Declare { items: Vec<(Ident, Expr)>, body: Option<Vec<Stmt>> },
    Unset(Vec<Expr>),
    Block(Vec<Stmt>),
    /// `name:` target of a `goto`.
    Label(Ident),
    /// `goto name;` (only out of or within blocks, never into loops: PHP rule).
    Goto(Ident),
    /// `__halt_compiler();` and empty `;`.
    Nop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UseKind {
    Normal,
    Function,
    Const,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UseItem {
    pub name: Name,
    pub alias: Option<Ident>,
    /// Only inside group use: `use Foo\{function bar}`.
    pub kind: Option<UseKind>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    /// None for `default:`.
    pub test: Option<Expr>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Catch {
    pub types: Vec<Name>,
    pub var: Option<Ident>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: Name,
    pub args: Vec<Arg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Modifiers {
    pub public: bool,
    pub protected: bool,
    pub private: bool,
    pub static_: bool,
    pub abstract_: bool,
    pub final_: bool,
    pub readonly: bool,
    /// Asymmetric visibility (PHP 8.4): `public private(set)`.
    pub set_visibility: Option<String>,
}

impl Modifiers {
    pub fn is_empty(&self) -> bool {
        *self == Modifiers::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// `int`, `Foo\Bar`, `self`, `static`, `null`, `mixed`...
    Named(Name),
    Nullable(Box<Type>),
    Union(Vec<Type>),
    Intersection(Vec<Type>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: Ident,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
    pub variadic: bool,
    /// `&$x`: binds to the caller's variable.
    pub by_ref: bool,
    /// Constructor promotion: `public function __construct(private int $x)`.
    pub promote: Option<Modifiers>,
    pub attrs: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    /// None for abstract/interface methods.
    pub body: Option<Vec<Stmt>>,
    /// `function &f()`: returns by reference (treated as by value for now).
    pub by_ref: bool,
    /// Body contains `yield`: calling it returns a Generator.
    pub is_generator: bool,
    pub attrs: Vec<Attribute>,
    pub doc: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClassKind {
    Class,
    Interface,
    Trait,
    Enum,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub kind: ClassKind,
    /// None for anonymous classes.
    pub name: Option<Ident>,
    pub modifiers: Modifiers,
    /// Parent class, or parent interfaces for an interface.
    pub extends: Vec<Name>,
    pub implements: Vec<Name>,
    /// Backed enum type: `enum Suit: string`.
    pub enum_type: Option<Type>,
    pub members: Vec<Member>,
    pub attrs: Vec<Attribute>,
    pub doc: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Member {
    Property {
        modifiers: Modifiers,
        ty: Option<Type>,
        items: Vec<(Ident, Option<Expr>)>,
        /// PHP 8.4 property hooks are parsed but not supported yet.
        has_hooks: bool,
        attrs: Vec<Attribute>,
        doc: Option<String>,
        span: Span,
    },
    Method { modifiers: Modifiers, func: Function },
    Const { modifiers: Modifiers, ty: Option<Type>, items: Vec<(Ident, Expr)>, attrs: Vec<Attribute>, doc: Option<String>, span: Span },
    TraitUse { traits: Vec<Name>, adaptations: Vec<TraitAdaptation>, span: Span },
    EnumCase { name: Ident, value: Option<Expr>, attrs: Vec<Attribute>, doc: Option<String>, span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TraitAdaptation {
    /// `A::foo insteadof B, C;`
    Insteadof { trait_: Name, method: Ident, instead_of: Vec<Name> },
    /// `foo as protected bar;` / `A::foo as baz;`
    Alias { trait_: Option<Name>, method: Ident, visibility: Option<String>, alias: Option<Ident> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    /// Scratch cell for executors (inline cache): the interpreter keeps a
    /// variable's last slot index here. Never affects meaning.
    pub hint: std::cell::Cell<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub name: Option<Ident>,
    pub value: Expr,
    pub spread: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayItem {
    pub key: Option<Expr>,
    pub value: Expr,
    pub spread: bool,
    /// `[&$x]`: the element aliases the variable.
    pub by_ref: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InterpPart {
    Lit(Vec<u8>),
    Expr(Expr),
}

/// Member name after `->` or `::`: usually an identifier, but `$obj->$name`
/// and `$obj->{expr}` are allowed (dynamic dispatch, not variable variables).
#[derive(Debug, Clone, PartialEq)]
pub enum MemberName {
    Ident(Ident),
    Dynamic(Box<Expr>),
}

/// The class in `new X`, `X::foo()`, `$x instanceof X`.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassRef {
    /// Includes `self`, `parent`, `static`.
    Name(Name),
    Dynamic(Box<Expr>),
    Anonymous(Box<Class>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod, Pow, Concat,
    BitAnd, BitOr, BitXor, Shl, Shr,
    And, Or, Xor,
    Eq, NotEq, Identical, NotIdentical, Lt, LtEq, Gt, GtEq, Spaceship,
    Coalesce,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
    BitNot,
    /// `@expr`: parsed, warned about, then ignored.
    Silence,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IncludeKind {
    Include,
    IncludeOnce,
    Require,
    RequireOnce,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    /// None for `default`.
    pub conds: Option<Vec<Expr>>,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Closure {
    pub params: Vec<Param>,
    pub uses: Vec<ClosureUse>,
    pub ret: Option<Type>,
    pub body: Vec<Stmt>,
    pub by_ref: bool,
    pub is_generator: bool,
    pub is_static: bool,
    pub attrs: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureUse {
    pub var: Ident,
    /// `use (&$x)`: shares the variable with the enclosing scope.
    pub by_ref: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrowFn {
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Expr,
    pub by_ref: bool,
    pub is_generator: bool,
    pub is_static: bool,
    pub attrs: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Str(String),
    /// A double-quoted string whose escapes produced non-UTF-8 bytes.
    Bytes(Vec<u8>),
    Interp(Vec<InterpPart>),
    Bool(bool),
    Null,
    Var(String),
    /// `$$name` / `${expr}`: the variable named by the value of the expression.
    VarVar(Box<Expr>),
    /// `$GLOBALS[expr]`: the global variable named by the expression.
    GlobalVar(Box<Expr>),
    /// Constant or bare function name: `PHP_EOL`, `strlen` in `strlen($x)`.
    Name(Name),
    /// `__DIR__`, `__CLASS__`, ...
    MagicConst(String),
    /// `[...]` / `array(...)` / `list(...)`. `None` items are holes: `[, $b] = $x`.
    Array(Vec<Option<ArrayItem>>),
    Assign { target: Box<Expr>, value: Box<Expr> },
    /// `$a = &$b`: make `$a` an alias of `$b`.
    AssignRef { target: Box<Expr>, value: Box<Expr> },
    AssignOp { op: BinOp, target: Box<Expr>, value: Box<Expr> },
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    Unary { op: UnOp, expr: Box<Expr> },
    IncDec { prefix: bool, inc: bool, expr: Box<Expr> },
    /// `a ? b : c`, or `a ?: c` when `then` is None.
    Ternary { cond: Box<Expr>, then: Option<Box<Expr>>, else_: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Arg> },
    /// `strlen(...)`, `$obj->method(...)`: first-class callable.
    CallableRef(Box<Expr>),
    Prop { object: Box<Expr>, name: MemberName, nullsafe: bool },
    MethodCall { object: Box<Expr>, name: MemberName, args: Vec<Arg>, nullsafe: bool },
    StaticProp { class: ClassRef, name: Ident },
    StaticCall { class: ClassRef, name: MemberName, args: Vec<Arg> },
    /// `Foo::BAR`, `Foo::class`.
    ClassConst { class: ClassRef, name: Ident },
    /// `$a[i]`, or `$a[]` (append) when `index` is None.
    Index { base: Box<Expr>, index: Option<Box<Expr>> },
    New { class: ClassRef, args: Vec<Arg> },
    Clone(Box<Expr>),
    Closure(Box<Closure>),
    ArrowFn(Box<ArrowFn>),
    Match { subject: Box<Expr>, arms: Vec<MatchArm> },
    Instanceof { expr: Box<Expr>, class: ClassRef },
    Cast { ty: &'static str, expr: Box<Expr> },
    Isset(Vec<Expr>),
    Empty(Box<Expr>),
    Throw(Box<Expr>),
    Print(Box<Expr>),
    Exit(Option<Box<Expr>>),
    Include { kind: IncludeKind, path: Box<Expr> },
    Yield { key: Option<Box<Expr>>, value: Option<Box<Expr>> },
    YieldFrom(Box<Expr>),
    /// Placeholder after a syntax error, so parsing can continue.
    Error,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Expr { kind, span, hint: std::cell::Cell::new(u32::MAX) }
    }

    pub fn is_lvalue(&self) -> bool {
        matches!(
            self.kind,
            ExprKind::Var(_)
                | ExprKind::VarVar(_)
                | ExprKind::GlobalVar(_)
                | ExprKind::Index { .. }
                | ExprKind::Prop { .. }
                | ExprKind::StaticProp { .. }
                | ExprKind::Array(_)
                // Already reported; don't cascade "cannot assign" errors.
                | ExprKind::Error
        )
    }
}
