//! Recursive-descent parser for statements, Pratt parser for expressions.
//!
//! Errors never abort: each one becomes a `Diagnostic`, the parser skips to
//! the next `;` or `}` and carries on, so one run reports every problem.

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::lexer::{is_ident_char, is_ident_start, skip_braced, Lexer, Tok, Token};

/// `Err(())` means a diagnostic was already recorded.
type PResult<T> = Result<T, ()>;

const ASSIGN_RBP: u8 = 7;
const UNARY_RBP: u8 = 41;
const NOT_RBP: u8 = 37;
const TERNARY_LBP: u8 = 10;
const INSTANCEOF_LBP: u8 = 39;

/// Parse a whole `.php` file.
pub fn parse(src: &str) -> (File, Vec<Diagnostic>) {
    crate::stack::with_default_budget(|| parse_inner(src))
}

fn parse_inner(src: &str) -> (File, Vec<Diagnostic>) {
    let (toks, mut diags) = Lexer::new(src).tokenize();
    let mut p = Parser::from_tokens(toks);
    let stmts = p.stmt_list(|_| false);
    diags.append(&mut p.diags);
    diags.sort_by_key(|d| d.span.start);
    (File { stmts }, diags)
}

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    pub diags: Vec<Diagnostic>,
    /// One entry per function body being parsed: did it contain `yield`?
    gen_stack: Vec<bool>,
    /// Doc comment in front of the last attribute group (`/** */ #[A]
    /// class X`): it belongs to the declaration after the attributes.
    attr_doc: Option<String>,
}

fn binary_op(t: &Tok) -> Option<(u8, u8, BinOp)> {
    use BinOp::*;
    Some(match t {
        Tok::Name(n) if n.eq_ignore_ascii_case("or") => (1, 2, Or),
        Tok::Name(n) if n.eq_ignore_ascii_case("xor") => (3, 4, Xor),
        Tok::Name(n) if n.eq_ignore_ascii_case("and") => (5, 6, And),
        Tok::Punct(p) => match *p {
            "??" => (14, 13, Coalesce),
            "||" => (15, 16, Or),
            "&&" => (17, 18, And),
            "|" => (19, 20, BitOr),
            "^" => (21, 22, BitXor),
            "&" => (23, 24, BitAnd),
            "==" => (25, 26, Eq),
            "!=" | "<>" => (25, 26, NotEq),
            "===" => (25, 26, Identical),
            "!==" => (25, 26, NotIdentical),
            "<=>" => (25, 26, Spaceship),
            "<" => (27, 28, Lt),
            "<=" => (27, 28, LtEq),
            ">" => (27, 28, Gt),
            ">=" => (27, 28, GtEq),
            "." => (29, 30, Concat),
            "<<" => (31, 32, Shl),
            ">>" => (31, 32, Shr),
            "+" => (33, 34, Add),
            "-" => (33, 34, Sub),
            "*" => (35, 36, Mul),
            "/" => (35, 36, Div),
            "%" => (35, 36, Mod),
            "**" => (44, 43, Pow),
            _ => return None,
        },
        _ => return None,
    })
}

/// `None` inside `Some` means plain `=`.
fn assign_op(t: &Tok) -> Option<Option<BinOp>> {
    use BinOp::*;
    let Tok::Punct(p) = t else { return None };
    Some(match *p {
        "=" => None,
        "+=" => Some(Add),
        "-=" => Some(Sub),
        "*=" => Some(Mul),
        "/=" => Some(Div),
        "%=" => Some(Mod),
        "**=" => Some(Pow),
        ".=" => Some(Concat),
        "&=" => Some(BitAnd),
        "|=" => Some(BitOr),
        "^=" => Some(BitXor),
        "<<=" => Some(Shl),
        ">>=" => Some(Shr),
        "??=" => Some(Coalesce),
        _ => return None,
    })
}

const MAGIC_CONSTS: &[&str] =
    &["__LINE__", "__FILE__", "__DIR__", "__FUNCTION__", "__CLASS__", "__TRAIT__", "__METHOD__", "__NAMESPACE__", "__PROPERTY__"];

impl Parser {
    pub fn from_tokens(toks: Vec<Token>) -> Self {
        Parser { toks, pos: 0, diags: vec![], gen_stack: vec![], attr_doc: None }
    }

    // ---- token helpers -------------------------------------------------

    fn tok(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek(&self) -> &Tok {
        &self.tok().tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.tok().span
    }

    fn prev_end(&self) -> Span {
        let i = self.pos.saturating_sub(1);
        let e = self.toks[i].span.end;
        Span { start: e, end: e }
    }

    fn since(&self, start: Span) -> Span {
        start.to(self.prev_end())
    }

    fn bump(&mut self) -> Token {
        let t = self.tok().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at_eof(&self) -> bool {
        *self.peek() == Tok::Eof
    }

    fn at_p(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }

    fn at_p_at(&self, n: usize, p: &str) -> bool {
        matches!(self.peek_at(n), Tok::Punct(q) if *q == p)
    }

    fn eat_p(&mut self, p: &str) -> bool {
        if self.at_p(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_p(&mut self, p: &str) -> PResult<Span> {
        if self.at_p(p) {
            Ok(self.bump().span)
        } else {
            self.expected(&format!("`{p}`"))
        }
    }

    fn at_kw(&self, kw: &str) -> bool {
        is_kw(self.peek(), kw)
    }

    fn at_kw_at(&self, n: usize, kw: &str) -> bool {
        is_kw(self.peek_at(n), kw)
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_kw(&mut self, kw: &str) -> PResult<()> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            self.expected(&format!("`{kw}`"))
        }
    }

    /// Lowercase keyword at the cursor, if the token is an unqualified word.
    fn kw(&self) -> Option<String> {
        match self.peek() {
            Tok::Name(n) if !n.contains('\\') => Some(n.to_ascii_lowercase()),
            _ => None,
        }
    }

    fn expected<T>(&mut self, what: &str) -> PResult<T> {
        let found = describe(self.peek());
        let span = self.span();
        self.diags.push(Diagnostic::error("E0002", span, format!("expected {what}, found {found}")));
        Err(())
    }


    /// Any single word: identifiers, keywords (allowed as member names).
    fn ident(&mut self) -> PResult<Ident> {
        match self.peek().clone() {
            Tok::Name(n) if !n.contains('\\') => {
                let span = self.bump().span;
                Ok(Ident { name: n, span })
            }
            _ => self.expected("identifier"),
        }
    }

    fn name(&mut self) -> PResult<Name> {
        match self.peek().clone() {
            Tok::Name(n) => {
                self.bump();
                Ok(n)
            }
            _ => self.expected("name"),
        }
    }

    fn var_ident(&mut self) -> PResult<Ident> {
        match self.peek().clone() {
            Tok::Var(n) => {
                let span = self.bump().span;
                Ok(Ident { name: n, span })
            }
            _ => self.expected("variable"),
        }
    }

    /// `;` or `?>` ends a statement.
    fn end_stmt(&mut self) -> PResult<()> {
        if self.eat_p(";") || matches!(self.peek(), Tok::CloseTag | Tok::Eof) {
            if *self.peek() == Tok::CloseTag {
                self.bump();
            }
            return Ok(());
        }
        self.expected("`;`")
    }

    /// Skip to just after the next `;`, or to a `}` that closes an outer block.
    fn recover(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::Punct(";") if depth == 0 => {
                    self.bump();
                    return;
                }
                Tok::Punct("{") => depth += 1,
                Tok::Punct("}") => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    // ---- statements ----------------------------------------------------

    fn stmt_list(&mut self, end: impl Fn(&Self) -> bool) -> Vec<Stmt> {
        let mut out = vec![];
        while !self.at_eof() && !end(self) {
            let before = self.pos;
            match self.statement() {
                Ok(s) => out.push(s),
                Err(()) => self.recover(),
            }
            if self.pos == before {
                self.bump();
            }
        }
        out
    }

    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect_p("{")?;
        let body = self.stmt_list(|p| p.at_p("}"));
        self.expect_p("}")?;
        Ok(body)
    }

    /// Body of `if`/`while`/...: a block or a single statement.
    fn stmt_body(&mut self) -> PResult<Vec<Stmt>> {
        if self.at_p("{") {
            self.block()
        } else {
            Ok(vec![self.statement()?])
        }
    }

    /// Body that may use alternative syntax: `while (x): ... endwhile;`.
    fn body_or_alt(&mut self, end_kw: &str) -> PResult<Vec<Stmt>> {
        if self.eat_p(":") {
            let body = self.stmt_list(|p| p.at_kw(end_kw));
            self.expect_kw(end_kw)?;
            self.end_stmt()?;
            Ok(body)
        } else {
            self.stmt_body()
        }
    }

    fn paren_expr(&mut self) -> PResult<Expr> {
        self.expect_p("(")?;
        let e = self.expr()?;
        self.expect_p(")")?;
        Ok(e)
    }

    /// Nested too deeply for the stack budget: give up on the rest of the
    /// file (the caller reports `stack::TOO_DEEP`).
    fn too_deep(&mut self) -> bool {
        if crate::stack::exhausted() {
            self.pos = self.toks.len() - 1;
            return true;
        }
        false
    }

    pub fn statement(&mut self) -> PResult<Stmt> {
        if self.too_deep() {
            return Err(());
        }
        let start = self.span();
        let kind = self.statement_kind()?;
        Ok(Stmt { kind, span: self.since(start) })
    }

    fn statement_kind(&mut self) -> PResult<StmtKind> {
        match self.peek().clone() {
            Tok::InlineHtml(h) => {
                self.bump();
                return Ok(StmtKind::InlineHtml(h));
            }
            Tok::OpenTagEcho => {
                self.bump();
                let exprs = self.expr_list()?;
                self.end_stmt()?;
                return Ok(StmtKind::Echo(exprs));
            }
            Tok::CloseTag => {
                self.bump();
                return Ok(StmtKind::Nop);
            }
            Tok::Punct("{") => return Ok(StmtKind::Block(self.block()?)),
            Tok::Punct(";") => {
                self.bump();
                return Ok(StmtKind::Nop);
            }
            Tok::Punct("#[") => {
                let attrs = self.attributes()?;
                return self.decl_after_attrs(attrs);
            }
            _ => {}
        }
        let Some(kw) = self.kw() else { return self.expr_stmt() };
        match kw.as_str() {
            "echo" => {
                self.bump();
                let exprs = self.expr_list()?;
                self.end_stmt()?;
                Ok(StmtKind::Echo(exprs))
            }
            "if" => self.if_stmt(),
            "while" => {
                self.bump();
                let cond = self.paren_expr()?;
                let body = self.body_or_alt("endwhile")?;
                Ok(StmtKind::While { cond, body })
            }
            "do" => {
                self.bump();
                let body = self.stmt_body()?;
                self.expect_kw("while")?;
                let cond = self.paren_expr()?;
                self.end_stmt()?;
                Ok(StmtKind::DoWhile { body, cond })
            }
            "for" => {
                self.bump();
                self.expect_p("(")?;
                let init = self.expr_list_until(";")?;
                self.expect_p(";")?;
                let cond = self.expr_list_until(";")?;
                self.expect_p(";")?;
                let step = self.expr_list_until(")")?;
                self.expect_p(")")?;
                let body = self.body_or_alt("endfor")?;
                Ok(StmtKind::For { init, cond, step, body })
            }
            "foreach" => self.foreach_stmt(),
            "switch" => self.switch_stmt(),
            "break" | "continue" => {
                self.bump();
                let level = if self.at_p(";") || matches!(self.peek(), Tok::CloseTag) { None } else { Some(self.expr()?) };
                self.end_stmt()?;
                Ok(if kw == "break" { StmtKind::Break(level) } else { StmtKind::Continue(level) })
            }
            "return" => {
                self.bump();
                let e = if self.at_p(";") || matches!(self.peek(), Tok::CloseTag | Tok::Eof) { None } else { Some(self.expr()?) };
                self.end_stmt()?;
                Ok(StmtKind::Return(e))
            }
            "function" if self.is_named_function() => {
                let f = self.function_decl(vec![])?;
                Ok(StmtKind::Function(Box::new(f)))
            }
            "abstract" | "final" | "readonly" | "class" | "interface" | "trait" | "enum" if self.is_class_decl() => {
                Ok(StmtKind::Class(Box::new(self.class_decl(vec![])?)))
            }
            "try" => self.try_stmt(),
            "namespace" if !self.at_p_at(1, "\\") => self.namespace_stmt(),
            "use" => self.use_stmt(),
            "const" => {
                self.bump();
                let items = self.const_items()?;
                self.end_stmt()?;
                Ok(StmtKind::Const(items))
            }
            "static" if matches!(self.peek_at(1), Tok::Var(_)) => {
                self.bump();
                let mut vars = vec![];
                loop {
                    let v = self.var_ident()?;
                    let init = if self.eat_p("=") { Some(self.expr()?) } else { None };
                    vars.push((v, init));
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.end_stmt()?;
                Ok(StmtKind::StaticVar(vars))
            }
            "global" if matches!(self.peek_at(1), Tok::Var(_) | Tok::Punct("$")) => {
                self.bump();
                let mut vars = vec![];
                loop {
                    let e = match self.peek().clone() {
                        Tok::Var(n) => {
                            let span = self.bump().span;
                            Expr::new(ExprKind::Var(n), span)
                        }
                        _ => self.variable_variable()?,
                    };
                    vars.push(e);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.end_stmt()?;
                Ok(StmtKind::Global(vars))
            }
            "goto" => {
                self.bump();
                let label = self.ident()?;
                self.end_stmt()?;
                Ok(StmtKind::Goto(label))
            }
            "declare" => {
                self.bump();
                self.expect_p("(")?;
                let mut items = vec![];
                loop {
                    let k = self.ident()?;
                    self.expect_p("=")?;
                    items.push((k, self.expr()?));
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(")")?;
                let body = if self.at_p("{") {
                    Some(self.block()?)
                } else if self.eat_p(":") {
                    let b = self.stmt_list(|p| p.at_kw("enddeclare"));
                    self.expect_kw("enddeclare")?;
                    self.end_stmt()?;
                    Some(b)
                } else {
                    self.end_stmt()?;
                    None
                };
                Ok(StmtKind::Declare { items, body })
            }
            "unset" if self.at_p_at(1, "(") => {
                self.bump();
                self.bump();
                let args = self.expr_list_until(")")?;
                self.eat_p(",");
                self.expect_p(")")?;
                self.end_stmt()?;
                Ok(StmtKind::Unset(args))
            }
            "__halt_compiler" => {
                // Everything after this is raw data.
                while !self.at_eof() {
                    self.bump();
                }
                Ok(StmtKind::Nop)
            }
            _ if self.at_p_at(1, ":") && !kw.contains('\\') && !matches!(kw.as_str(), "default" | "else") => {
                let label = self.ident()?;
                self.bump();
                Ok(StmtKind::Label(label))
            }
            _ => self.expr_stmt(),
        }
    }

    fn expr_stmt(&mut self) -> PResult<StmtKind> {
        let e = self.expr()?;
        self.end_stmt()?;
        Ok(StmtKind::Expr(e))
    }

    fn is_named_function(&self) -> bool {
        match self.peek_at(1) {
            Tok::Name(_) => true,
            Tok::Punct("&") => matches!(self.peek_at(2), Tok::Name(_)),
            _ => false,
        }
    }

    fn is_class_decl(&self) -> bool {
        let mut i = 0;
        while ["abstract", "final", "readonly"].iter().any(|m| self.at_kw_at(i, m)) {
            i += 1;
        }
        let Some(Tok::Name(kw)) = Some(self.peek_at(i)) else { return false };
        let kw = kw.to_ascii_lowercase();
        let named = matches!(self.peek_at(i + 1), Tok::Name(_));
        match kw.as_str() {
            "class" | "interface" | "trait" => named,
            // `enum` is also a valid function/const name, so look further.
            "enum" => named && (self.at_p_at(i + 2, "{") || self.at_p_at(i + 2, ":") || self.at_kw_at(i + 2, "implements")),
            _ => false,
        }
    }

    fn decl_after_attrs(&mut self, attrs: Vec<Attribute>) -> PResult<StmtKind> {
        if self.at_kw("function") && self.is_named_function() {
            return Ok(StmtKind::Function(Box::new(self.function_decl(attrs)?)));
        }
        if self.is_class_decl() {
            return Ok(StmtKind::Class(Box::new(self.class_decl(attrs)?)));
        }
        // Attributes on a closure / arrow fn expression statement.
        let start = self.span();
        let e = self.closure_like(attrs, start)?;
        let e = self.postfix(e)?;
        let e = self.infix(e, 0)?;
        self.end_stmt()?;
        Ok(StmtKind::Expr(e))
    }

    fn if_stmt(&mut self) -> PResult<StmtKind> {
        self.bump();
        let cond = self.paren_expr()?;
        let mut elseifs = vec![];
        let mut else_ = None;
        if self.eat_p(":") {
            let then = self.stmt_list(|p| p.at_kw("elseif") || p.at_kw("else") || p.at_kw("endif"));
            loop {
                if self.eat_kw("elseif") {
                    let c = self.paren_expr()?;
                    self.expect_p(":")?;
                    let b = self.stmt_list(|p| p.at_kw("elseif") || p.at_kw("else") || p.at_kw("endif"));
                    elseifs.push((c, b));
                } else if self.eat_kw("else") {
                    self.expect_p(":")?;
                    else_ = Some(self.stmt_list(|p| p.at_kw("endif")));
                } else {
                    break;
                }
            }
            self.expect_kw("endif")?;
            self.end_stmt()?;
            return Ok(StmtKind::If { cond, then, elseifs, else_ });
        }
        let then = self.stmt_body()?;
        loop {
            if self.eat_kw("elseif") || (self.at_kw("else") && self.at_kw_at(1, "if") && {
                self.bump();
                self.bump();
                true
            }) {
                let c = self.paren_expr()?;
                let b = self.stmt_body()?;
                elseifs.push((c, b));
            } else if self.eat_kw("else") {
                else_ = Some(self.stmt_body()?);
                break;
            } else {
                break;
            }
        }
        Ok(StmtKind::If { cond, then, elseifs, else_ })
    }

    fn foreach_stmt(&mut self) -> PResult<StmtKind> {
        self.bump();
        self.expect_p("(")?;
        let subject = self.expr()?;
        self.expect_kw("as")?;
        let mut by_ref = self.eat_p("&");
        let mut value = self.expr()?;
        let mut key = None;
        if self.eat_p("=>") {
            if by_ref {
                let span = value.span;
                self.diags.push(Diagnostic::error("E0002", span, "key element cannot be a reference"));
            }
            key = Some(value);
            by_ref = self.eat_p("&");
            value = self.expr()?;
        }
        self.expect_p(")")?;
        let body = self.body_or_alt("endforeach")?;
        Ok(StmtKind::Foreach { subject, key, value, by_ref, body })
    }

    fn switch_stmt(&mut self) -> PResult<StmtKind> {
        self.bump();
        let subject = self.paren_expr()?;
        let alt = if self.eat_p(":") {
            true
        } else {
            self.expect_p("{")?;
            false
        };
        let end = |p: &Self| if alt { p.at_kw("endswitch") } else { p.at_p("}") };
        let mut cases = vec![];
        while !self.at_eof() && !end(self) {
            let test = if self.eat_kw("default") {
                None
            } else {
                self.expect_kw("case")?;
                Some(self.expr()?)
            };
            if !self.eat_p(":") && !self.eat_p(";") {
                return self.expected("`:`");
            }
            let body = self.stmt_list(|p| p.at_kw("case") || p.at_kw("default") || end(p));
            cases.push(Case { test, body });
        }
        if alt {
            self.expect_kw("endswitch")?;
            self.end_stmt()?;
        } else {
            self.expect_p("}")?;
        }
        Ok(StmtKind::Switch { subject, cases })
    }

    fn try_stmt(&mut self) -> PResult<StmtKind> {
        self.bump();
        let body = self.block()?;
        let mut catches = vec![];
        while self.eat_kw("catch") {
            self.expect_p("(")?;
            let mut types = vec![self.name()?];
            while self.eat_p("|") {
                types.push(self.name()?);
            }
            let var = if matches!(self.peek(), Tok::Var(_)) { Some(self.var_ident()?) } else { None };
            self.expect_p(")")?;
            catches.push(Catch { types, var, body: self.block()? });
        }
        let finally = if self.eat_kw("finally") { Some(self.block()?) } else { None };
        if catches.is_empty() && finally.is_none() {
            return self.expected("`catch` or `finally`");
        }
        Ok(StmtKind::Try { body, catches, finally })
    }

    fn namespace_stmt(&mut self) -> PResult<StmtKind> {
        self.bump();
        let name = if matches!(self.peek(), Tok::Name(_)) { Some(self.name()?) } else { None };
        if self.at_p("{") {
            return Ok(StmtKind::Namespace { name, body: Some(self.block()?) });
        }
        self.end_stmt()?;
        Ok(StmtKind::Namespace { name, body: None })
    }

    fn use_kind(&mut self) -> Option<UseKind> {
        // `use function foo;` but not `use function;` (a class named Function).
        let next_is_name = matches!(self.peek_at(1), Tok::Name(_));
        if self.at_kw("function") && next_is_name {
            self.bump();
            Some(UseKind::Function)
        } else if self.at_kw("const") && next_is_name {
            self.bump();
            Some(UseKind::Const)
        } else {
            None
        }
    }

    fn use_stmt(&mut self) -> PResult<StmtKind> {
        self.bump();
        let kind = self.use_kind().unwrap_or(UseKind::Normal);
        let mut items = vec![];
        loop {
            let name = self.name()?;
            if self.eat_p("\\") {
                // Group use: `use App\{Foo, Bar as Baz, function helper};`
                self.expect_p("{")?;
                while !self.at_p("}") {
                    let k = self.use_kind();
                    let sub = self.name()?;
                    let alias = if self.eat_kw("as") { Some(self.ident()?) } else { None };
                    items.push(UseItem { name: format!("{name}\\{sub}"), alias, kind: k });
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p("}")?;
            } else {
                let alias = if self.eat_kw("as") { Some(self.ident()?) } else { None };
                items.push(UseItem { name, alias, kind: None });
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.end_stmt()?;
        Ok(StmtKind::Use { kind, items })
    }

    fn const_items(&mut self) -> PResult<Vec<(Ident, Expr)>> {
        let mut items = vec![];
        loop {
            let name = self.ident()?;
            self.expect_p("=")?;
            items.push((name, self.expr()?));
            if !self.eat_p(",") {
                break;
            }
        }
        Ok(items)
    }

    // ---- declarations --------------------------------------------------

    fn attributes(&mut self) -> PResult<Vec<Attribute>> {
        let mut attrs = vec![];
        self.attr_doc = self.tok().doc.clone();
        while self.eat_p("#[") {
            loop {
                if self.at_p("]") {
                    break;
                }
                let start = self.span();
                let name = self.name()?;
                let args = if self.at_p("(") { self.args()?.unwrap_or_default() } else { vec![] };
                attrs.push(Attribute { name, args, span: self.since(start) });
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p("]")?;
        }
        Ok(attrs)
    }

    fn function_decl(&mut self, attrs: Vec<Attribute>) -> PResult<Function> {
        let start = self.span();
        let doc = self.tok().doc.clone().or_else(|| if attrs.is_empty() { None } else { self.attr_doc.take() });
        self.expect_kw("function")?;
        let by_ref = self.eat_p("&");
        let name = self.ident()?;
        let params = self.params()?;
        let ret = self.return_type()?;
        let (body, is_generator) = self.function_body(|p| p.block())?;
        let body = Some(body);
        Ok(Function { name, params, ret, body, by_ref, is_generator, attrs, doc, span: self.since(start) })
    }

    fn return_type(&mut self) -> PResult<Option<Type>> {
        if self.eat_p(":") {
            Ok(Some(self.parse_type()?))
        } else {
            Ok(None)
        }
    }

    fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect_p("(")?;
        let mut params = vec![];
        while !self.at_p(")") {
            params.push(self.param()?);
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(params)
    }

    fn param(&mut self) -> PResult<Param> {
        let attrs = if self.at_p("#[") { self.attributes()? } else { vec![] };
        let mods = self.modifiers();
        let promote = if mods.is_empty() { None } else { Some(mods) };
        let ty = if matches!(self.peek(), Tok::Name(_)) || self.at_p("?") || self.at_p("(") {
            Some(self.parse_type()?)
        } else {
            None
        };
        let by_ref = self.eat_p("&");
        let variadic = self.eat_p("...");
        let name = self.var_ident()?;
        let default = if self.eat_p("=") { Some(self.expr()?) } else { None };
        Ok(Param { name, ty, default, variadic, by_ref, promote, attrs })
    }

    pub fn parse_type(&mut self) -> PResult<Type> {
        if self.eat_p("?") {
            return Ok(Type::Nullable(Box::new(self.type_atom()?)));
        }
        let first = self.type_atom()?;
        if self.at_p("|") {
            let mut ts = vec![first];
            while self.eat_p("|") {
                ts.push(self.type_atom()?);
            }
            return Ok(Type::Union(ts));
        }
        if self.at_intersection_amp() {
            let mut ts = vec![first];
            while self.at_intersection_amp() {
                self.bump();
                ts.push(self.type_atom()?);
            }
            return Ok(Type::Intersection(ts));
        }
        Ok(first)
    }

    /// `A&B` is an intersection; `A &$x` / `A &...$x` is a by-ref param.
    fn at_intersection_amp(&self) -> bool {
        self.at_p("&") && matches!(self.peek_at(1), Tok::Name(_))
    }

    fn type_atom(&mut self) -> PResult<Type> {
        if self.eat_p("(") {
            // DNF: `(A&B)|null`
            let t = self.parse_type()?;
            self.expect_p(")")?;
            return Ok(t);
        }
        Ok(Type::Named(self.name()?))
    }

    /// Visibility and friends. Stops at the first non-modifier word.
    fn modifiers(&mut self) -> Modifiers {
        let mut m = Modifiers::default();
        loop {
            let Some(kw) = self.kw() else { break };
            // `static fn` / `static::` / `readonly function` are not modifiers here.
            if matches!(self.peek_at(1), Tok::Punct("::" | "(")) && kw != "private" && kw != "protected" && kw != "public" {
                break;
            }
            match kw.as_str() {
                "public" | "protected" | "private" => {
                    self.bump();
                    // Asymmetric visibility: `private(set)`.
                    if self.at_p("(") && self.at_kw_at(1, "set") && self.at_p_at(2, ")") {
                        self.bump();
                        self.bump();
                        self.bump();
                        m.set_visibility = Some(kw);
                        continue;
                    }
                    match kw.as_str() {
                        "public" => m.public = true,
                        "protected" => m.protected = true,
                        _ => m.private = true,
                    }
                }
                "var" => {
                    self.bump();
                    m.public = true;
                }
                "static" => {
                    self.bump();
                    m.static_ = true;
                }
                "abstract" => {
                    self.bump();
                    m.abstract_ = true;
                }
                "final" => {
                    self.bump();
                    m.final_ = true;
                }
                "readonly" => {
                    self.bump();
                    m.readonly = true;
                }
                _ => break,
            }
        }
        m
    }

    fn name_list(&mut self) -> PResult<Vec<Name>> {
        let mut names = vec![self.name()?];
        while self.eat_p(",") {
            names.push(self.name()?);
        }
        Ok(names)
    }

    fn class_decl(&mut self, attrs: Vec<Attribute>) -> PResult<Class> {
        let start = self.span();
        let doc = self.tok().doc.clone().or_else(|| if attrs.is_empty() { None } else { self.attr_doc.take() });
        let modifiers = self.modifiers();
        let kind = match self.kw().as_deref() {
            Some("class") => ClassKind::Class,
            Some("interface") => ClassKind::Interface,
            Some("trait") => ClassKind::Trait,
            Some("enum") => ClassKind::Enum,
            _ => return self.expected("`class`"),
        };
        self.bump();
        let name = Some(self.ident()?);
        self.class_rest(kind, name, modifiers, attrs, doc, start)
    }

    /// Everything after the class name (shared with anonymous classes).
    fn class_rest(
        &mut self,
        kind: ClassKind,
        name: Option<Ident>,
        modifiers: Modifiers,
        attrs: Vec<Attribute>,
        doc: Option<String>,
        start: Span,
    ) -> PResult<Class> {
        let enum_type = if kind == ClassKind::Enum && self.eat_p(":") { Some(self.parse_type()?) } else { None };
        let extends = if self.eat_kw("extends") { self.name_list()? } else { vec![] };
        let implements = if self.eat_kw("implements") { self.name_list()? } else { vec![] };
        self.expect_p("{")?;
        let mut members = vec![];
        while !self.at_p("}") && !self.at_eof() {
            let before = self.pos;
            match self.member() {
                Ok(m) => members.push(m),
                Err(()) => self.recover(),
            }
            if self.pos == before {
                self.bump();
            }
        }
        self.expect_p("}")?;
        Ok(Class { kind, name, modifiers, extends, implements, enum_type, members, attrs, doc, span: self.since(start) })
    }

    fn member(&mut self) -> PResult<Member> {
        let start = self.span();
        let doc = self.tok().doc.clone();
        let attrs = if self.at_p("#[") { self.attributes()? } else { vec![] };
        let doc = doc.or_else(|| self.tok().doc.clone());
        if self.eat_kw("use") {
            let traits = self.name_list()?;
            let mut adaptations = vec![];
            if self.eat_p("{") {
                while !self.at_p("}") && !self.at_eof() {
                    adaptations.push(self.trait_adaptation()?);
                }
                self.expect_p("}")?;
            } else {
                self.end_stmt()?;
            }
            return Ok(Member::TraitUse { traits, adaptations, span: self.since(start) });
        }
        if self.at_kw("case") {
            self.bump();
            let name = self.ident()?;
            let value = if self.eat_p("=") { Some(self.expr()?) } else { None };
            self.end_stmt()?;
            return Ok(Member::EnumCase { name, value, attrs, doc, span: self.since(start) });
        }
        let modifiers = self.modifiers();
        if self.eat_kw("const") {
            // Typed constant (8.3) if the name isn't directly followed by `=`.
            let ty = if self.at_p_at(1, "=") { None } else { Some(self.parse_type()?) };
            let items = self.const_items()?;
            self.end_stmt()?;
            return Ok(Member::Const { modifiers, ty, items, attrs, doc, span: self.since(start) });
        }
        if self.at_kw("function") {
            self.bump();
            let by_ref = self.eat_p("&");
            let name = self.ident()?;
            let params = self.params()?;
            let ret = self.return_type()?;
            let (body, is_generator) = if self.at_p("{") {
                let (b, g) = self.function_body(|p| p.block())?;
                (Some(b), g)
            } else {
                self.end_stmt()?;
                (None, false)
            };
            let func = Function { name, params, ret, body, by_ref, is_generator, attrs, doc, span: self.since(start) };
            return Ok(Member::Method { modifiers, func });
        }
        // Property: [type] $a [= x], $b ...;
        let ty = if matches!(self.peek(), Tok::Var(_)) { None } else { Some(self.parse_type()?) };
        let mut items = vec![];
        loop {
            let v = self.var_ident()?;
            let default = if self.eat_p("=") { Some(self.expr()?) } else { None };
            items.push((v, default));
            if !self.eat_p(",") {
                break;
            }
        }
        let mut has_hooks = false;
        if self.at_p("{") {
            // PHP 8.4 property hooks: keep the span, skip the body.
            has_hooks = true;
            self.skip_balanced();
        } else {
            self.end_stmt()?;
        }
        Ok(Member::Property { modifiers, ty, items, has_hooks, attrs, doc, span: self.since(start) })
    }

    fn skip_balanced(&mut self) {
        let mut depth = 0;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::Punct("{") => depth += 1,
                Tok::Punct("}") => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn trait_adaptation(&mut self) -> PResult<TraitAdaptation> {
        let first = self.name()?;
        let (trait_, method) = if self.eat_p("::") {
            (Some(first), self.ident()?)
        } else {
            (None, Ident { name: first, span: self.prev_end() })
        };
        if self.eat_kw("insteadof") {
            let instead_of = self.name_list()?;
            self.end_stmt()?;
            let Some(trait_) = trait_ else { return self.expected("`Trait::method` before insteadof") };
            return Ok(TraitAdaptation::Insteadof { trait_, method, instead_of });
        }
        self.expect_kw("as")?;
        let visibility = match self.kw().as_deref() {
            Some(v @ ("public" | "protected" | "private")) => {
                let v = v.to_string();
                self.bump();
                Some(v)
            }
            _ => None,
        };
        let alias = if matches!(self.peek(), Tok::Name(_)) { Some(self.ident()?) } else { None };
        self.end_stmt()?;
        Ok(TraitAdaptation::Alias { trait_, method, visibility, alias })
    }

    // ---- expressions ---------------------------------------------------

    pub fn expr(&mut self) -> PResult<Expr> {
        self.expr_bp(0)
    }

    fn expr_list(&mut self) -> PResult<Vec<Expr>> {
        let mut v = vec![self.expr()?];
        while self.eat_p(",") {
            v.push(self.expr()?);
        }
        Ok(v)
    }

    /// Comma list that may be empty (for `for (;;)`).
    fn expr_list_until(&mut self, end: &str) -> PResult<Vec<Expr>> {
        if self.at_p(end) {
            return Ok(vec![]);
        }
        let mut v = vec![self.expr()?];
        while self.eat_p(",") {
            if self.at_p(end) {
                break;
            }
            v.push(self.expr()?);
        }
        Ok(v)
    }

    fn expr_bp(&mut self, min_bp: u8) -> PResult<Expr> {
        if self.too_deep() {
            return Err(());
        }
        let lhs = self.prefix()?;
        let lhs = self.postfix(lhs)?;
        self.infix(lhs, min_bp)
    }

    fn infix(&mut self, mut lhs: Expr, min_bp: u8) -> PResult<Expr> {
        loop {
            let t = self.peek().clone();
            // Assignment binds to the nearest lvalue no matter the context:
            // `!$a = f()` is `!($a = f())`, `$x && $y = 1` is `$x && ($y = 1)`.
            if let Some(op) = assign_op(&t) {
                if !lhs.is_lvalue() {
                    if min_bp > 0 {
                        break;
                    }
                    let span = self.span();
                    self.diags.push(Diagnostic::error("E0002", span, "cannot assign to this expression"));
                    return Err(());
                }
                self.bump();
                if op.is_none() && self.eat_p("&") {
                    let value = self.expr_bp(ASSIGN_RBP)?;
                    let span = lhs.span.to(value.span);
                    lhs = Expr::new(ExprKind::AssignRef { target: Box::new(lhs), value: Box::new(value) }, span);
                    continue;
                }
                let value = self.expr_bp(ASSIGN_RBP)?;
                let span = lhs.span.to(value.span);
                lhs = Expr::new(
                    match op {
                        None => ExprKind::Assign { target: Box::new(lhs), value: Box::new(value) },
                        Some(op) => ExprKind::AssignOp { op, target: Box::new(lhs), value: Box::new(value) },
                    },
                    span,
                );
                continue;
            }
            if t == Tok::Punct("?") {
                if TERNARY_LBP < min_bp {
                    break;
                }
                self.bump();
                let then = if self.eat_p(":") {
                    None
                } else {
                    let e = self.expr_bp(0)?;
                    self.expect_p(":")?;
                    Some(Box::new(e))
                };
                let else_ = self.expr_bp(TERNARY_LBP + 1)?;
                let span = lhs.span.to(else_.span);
                lhs = Expr::new(ExprKind::Ternary { cond: Box::new(lhs), then, else_: Box::new(else_) }, span);
                continue;
            }
            if is_kw(&t, "instanceof") {
                if INSTANCEOF_LBP < min_bp {
                    break;
                }
                self.bump();
                let class = self.class_ref(false)?;
                let span = self.since(lhs.span);
                lhs = Expr::new(ExprKind::Instanceof { expr: Box::new(lhs), class }, span);
                continue;
            }
            let Some((lbp, rbp, op)) = binary_op(&t) else { break };
            if lbp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.expr_bp(rbp)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr::new(ExprKind::Binary { op, left: Box::new(lhs), right: Box::new(rhs) }, span);
        }
        Ok(lhs)
    }

    fn unary(&mut self, op: UnOp, rbp: u8) -> PResult<Expr> {
        let start = self.bump().span;
        let e = self.expr_bp(rbp)?;
        Ok(Expr::new(ExprKind::Unary { op, expr: Box::new(e) }, start.to(self.prev_end())))
    }

    /// Operand only (no postfix chain): `new`, prefix operators and primaries.
    fn prefix(&mut self) -> PResult<Expr> {
        let start = self.span();
        let mk = |p: &Self, kind| Ok(Expr::new(kind, p.since(start)));
        match self.peek().clone() {
            Tok::Punct("!") => return self.unary(UnOp::Not, NOT_RBP),
            Tok::Punct("-") => return self.unary(UnOp::Neg, UNARY_RBP),
            Tok::Punct("+") => return self.unary(UnOp::Plus, UNARY_RBP),
            Tok::Punct("~") => return self.unary(UnOp::BitNot, UNARY_RBP),
            Tok::Punct("@") => return self.unary(UnOp::Silence, UNARY_RBP),
            Tok::Punct(p @ ("++" | "--")) => {
                self.bump();
                let e = self.prefix()?;
                let e = self.postfix(e)?;
                return mk(self, ExprKind::IncDec { prefix: true, inc: p == "++", expr: Box::new(e) });
            }
            Tok::Cast(ty) => {
                self.bump();
                if ty == "unset" {
                    self.diags.push(Diagnostic::error("E0002", start, "the (unset) cast was removed in PHP 8"));
                }
                let e = self.expr_bp(UNARY_RBP)?;
                return mk(self, ExprKind::Cast { ty, expr: Box::new(e) });
            }
            Tok::Punct("&") => {
                self.bump();
                self.diags.push(Diagnostic::error(
                    "E0002",
                    start,
                    "`&` is only valid in parameters, foreach, `=&` assignments and array items",
                ));
                return self.expr_bp(UNARY_RBP);
            }
            Tok::Punct("$") => return self.variable_variable(),
            Tok::Punct("#[") => {
                let attrs = self.attributes()?;
                return self.closure_like(attrs, start);
            }
            Tok::Backtick(raw) => {
                // `cmd` is shell_exec("cmd") (deprecated in PHP 8.5; the
                // run-time helper reports that).
                let span = self.span();
                self.bump();
                let cmd = self.template(&raw, false, span);
                let callee = Expr { kind: ExprKind::Name("\\__phpx_backtick".into()), span, hint: Default::default() };
                return mk(self, ExprKind::Call { callee: Box::new(callee), args: vec![Arg { name: None, value: cmd, spread: false }] });
            }
            Tok::Name(n) => {
                let lower = n.to_ascii_lowercase();
                match lower.as_str() {
                    "new" => return self.new_expr(),
                    "clone" => {
                        self.bump();
                        // `clone($x)` and `clone $x` (8.5 made it function-like).
                        let e = self.expr_bp(UNARY_RBP)?;
                        return mk(self, ExprKind::Clone(Box::new(e)));
                    }
                    "print" => {
                        self.bump();
                        let e = self.expr_bp(ASSIGN_RBP)?;
                        return mk(self, ExprKind::Print(Box::new(e)));
                    }
                    "throw" => {
                        self.bump();
                        let e = self.expr_bp(0)?;
                        return mk(self, ExprKind::Throw(Box::new(e)));
                    }
                    "yield" => return self.yield_expr(),
                    "include" | "include_once" | "require" | "require_once" => {
                        self.bump();
                        let kind = match lower.as_str() {
                            "include" => IncludeKind::Include,
                            "include_once" => IncludeKind::IncludeOnce,
                            "require" => IncludeKind::Require,
                            _ => IncludeKind::RequireOnce,
                        };
                        let path = self.expr_bp(ASSIGN_RBP)?;
                        return mk(self, ExprKind::Include { kind, path: Box::new(path) });
                    }
                    "function" | "fn" => return self.closure_like(vec![], start),
                    "static" if self.at_kw_at(1, "function") || self.at_kw_at(1, "fn") => {
                        return self.closure_like(vec![], start)
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        self.primary()
    }

    /// `$$name`, `$$$name`, `${expr}` (binds tighter than `[` / `->`).
    fn variable_variable(&mut self) -> PResult<Expr> {
        let start = self.bump().span;
        let name = if self.eat_p("{") {
            let e = self.expr()?;
            self.expect_p("}")?;
            e
        } else {
            match self.peek().clone() {
                Tok::Var(n) => {
                    let span = self.bump().span;
                    Expr::new(ExprKind::Var(n), span)
                }
                Tok::Punct("$") => self.variable_variable()?,
                _ => return self.expected("variable name"),
            }
        };
        Ok(Expr::new(ExprKind::VarVar(Box::new(name)), self.since(start)))
    }

    /// Parse a function body, reporting whether it contains `yield`
    /// (which makes the function a generator).
    fn function_body<T>(&mut self, body: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<(T, bool)> {
        self.gen_stack.push(false);
        let r = body(self);
        let is_gen = self.gen_stack.pop().unwrap_or(false);
        Ok((r?, is_gen))
    }

    fn yield_expr(&mut self) -> PResult<Expr> {
        if let Some(top) = self.gen_stack.last_mut() {
            *top = true;
        }
        let start = self.bump().span;
        if self.eat_kw("from") {
            let e = self.expr_bp(ASSIGN_RBP)?;
            return Ok(Expr::new(ExprKind::YieldFrom(Box::new(e)), self.since(start)));
        }
        if matches!(self.peek(), Tok::Punct(";" | ")" | "," | "]") | Tok::CloseTag) {
            return Ok(Expr::new(ExprKind::Yield { key: None, value: None }, start));
        }
        let first = self.expr_bp(ASSIGN_RBP)?;
        let (key, value) = if self.eat_p("=>") {
            (Some(Box::new(first)), Box::new(self.expr_bp(ASSIGN_RBP)?))
        } else {
            (None, Box::new(first))
        };
        Ok(Expr::new(ExprKind::Yield { key, value: Some(value) }, self.since(start)))
    }

    fn closure_like(&mut self, attrs: Vec<Attribute>, start: Span) -> PResult<Expr> {
        let is_static = self.eat_kw("static");
        if self.eat_kw("fn") {
            let by_ref = self.eat_p("&");
            let params = self.params()?;
            let ret = self.return_type()?;
            self.expect_p("=>")?;
            let (body, is_generator) = self.function_body(|p| p.expr_bp(0))?;
            let f = ArrowFn { span: start, params, ret, body, by_ref, is_generator, is_static, attrs };
            return Ok(Expr::new(ExprKind::ArrowFn(Box::new(f)), self.since(start)));
        }
        self.expect_kw("function")?;
        let by_ref = self.eat_p("&");
        let params = self.params()?;
        let mut uses = vec![];
        if self.eat_kw("use") {
            self.expect_p("(")?;
            while !self.at_p(")") {
                let by_ref_use = self.eat_p("&");
                let var = self.var_ident()?;
                uses.push(ClosureUse { var, by_ref: by_ref_use });
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p(")")?;
        }
        let ret = self.return_type()?;
        let (body, is_generator) = self.function_body(|p| p.block())?;
        let c = Closure { span: start, params, uses, ret, body, by_ref, is_generator, is_static, attrs };
        Ok(Expr::new(ExprKind::Closure(Box::new(c)), self.since(start)))
    }

    fn new_expr(&mut self) -> PResult<Expr> {
        let start = self.bump().span;
        if self.at_kw("class") || (self.at_p("#[") && self.peek_is_anon_class_after_attrs()) {
            let attrs = if self.at_p("#[") { self.attributes()? } else { vec![] };
            let cstart = self.span();
            self.bump();
            let args = if self.at_p("(") { self.args()?.unwrap_or_default() } else { vec![] };
            let class = self.class_rest(ClassKind::Class, None, Modifiers::default(), attrs, None, cstart)?;
            return Ok(Expr::new(ExprKind::New { class: ClassRef::Anonymous(Box::new(class)), args }, self.since(start)));
        }
        if self.at_kw("readonly") && self.at_kw_at(1, "class") {
            self.bump();
            let cstart = self.span();
            self.bump();
            let args = if self.at_p("(") { self.args()?.unwrap_or_default() } else { vec![] };
            let mods = Modifiers { readonly: true, ..Default::default() };
            let class = self.class_rest(ClassKind::Class, None, mods, vec![], None, cstart)?;
            return Ok(Expr::new(ExprKind::New { class: ClassRef::Anonymous(Box::new(class)), args }, self.since(start)));
        }
        let class = self.class_ref(true)?;
        let args = if self.at_p("(") { self.args()?.unwrap_or_default() } else { vec![] };
        Ok(Expr::new(ExprKind::New { class, args }, self.since(start)))
    }

    fn peek_is_anon_class_after_attrs(&self) -> bool {
        let mut i = 0;
        let mut depth = 0;
        loop {
            match self.peek_at(i) {
                Tok::Eof => return false,
                Tok::Punct("#[" | "[") => depth += 1,
                Tok::Punct("]") => depth -= 1,
                Tok::Name(n) if depth == 0 => return n.eq_ignore_ascii_case("class"),
                _ if depth == 0 => return false,
                _ => {}
            }
            i += 1;
        }
    }

    /// Class after `new` / `instanceof`: a name, or a variable with property
    /// and index fetches (no calls), or a parenthesized expression.
    fn class_ref(&mut self, for_new: bool) -> PResult<ClassRef> {
        match self.peek().clone() {
            Tok::Name(n) if !(self.at_p_at(1, "::") && matches!(self.peek_at(2), Tok::Var(_))) => {
                self.bump();
                Ok(ClassRef::Name(n))
            }
            Tok::Name(_) | Tok::Var(_) => {
                let start = self.span();
                let mut e = if let Tok::Name(n) = self.peek().clone() {
                    // `new static::$builder`: the class named by a static property.
                    self.bump();
                    self.bump();
                    let v = self.var_ident()?;
                    let name = Ident { name: v.name, span: v.span };
                    Expr::new(ExprKind::StaticProp { class: ClassRef::Name(n), name }, self.since(start))
                } else {
                    let v = self.var_ident()?;
                    Expr::new(ExprKind::Var(v.name), v.span)
                };
                loop {
                    if self.at_p("->") || self.at_p("?->") {
                        let nullsafe = self.bump().tok == Tok::Punct("?->");
                        let name = self.member_name()?;
                        e = Expr::new(ExprKind::Prop { object: Box::new(e), name, nullsafe }, self.since(start));
                    } else if self.at_p("::") && matches!(self.peek_at(1), Tok::Var(_)) {
                        self.bump();
                        let v = self.var_ident()?;
                        let name = Ident { name: v.name, span: v.span };
                        e = Expr::new(ExprKind::StaticProp { class: ClassRef::Dynamic(Box::new(e)), name }, self.since(start));
                    } else if self.at_p("[") {
                        self.bump();
                        let idx = self.expr()?;
                        self.expect_p("]")?;
                        e = Expr::new(ExprKind::Index { base: Box::new(e), index: Some(Box::new(idx)) }, self.since(start));
                    } else {
                        break;
                    }
                }
                Ok(ClassRef::Dynamic(Box::new(e)))
            }
            Tok::Punct("(") => {
                let e = self.paren_expr()?;
                Ok(ClassRef::Dynamic(Box::new(e)))
            }
            Tok::Punct("$") => Ok(ClassRef::Dynamic(Box::new(self.variable_variable()?))),
            _ if for_new => self.expected("class name after `new`"),
            _ => self.expected("class name"),
        }
    }

    fn member_name(&mut self) -> PResult<MemberName> {
        match self.peek().clone() {
            Tok::Name(_) => Ok(MemberName::Ident(self.ident()?)),
            Tok::Var(n) => {
                let span = self.bump().span;
                Ok(MemberName::Dynamic(Box::new(Expr::new(ExprKind::Var(n), span))))
            }
            Tok::Punct("{") => {
                self.bump();
                let e = self.expr()?;
                self.expect_p("}")?;
                Ok(MemberName::Dynamic(Box::new(e)))
            }
            Tok::Punct("$") => Ok(MemberName::Dynamic(Box::new(self.variable_variable()?))),
            _ => self.expected("member name"),
        }
    }

    /// Call arguments. `Ok(None)` means first-class callable syntax `(...)`.
    fn args(&mut self) -> PResult<Option<Vec<Arg>>> {
        self.expect_p("(")?;
        if self.at_p("...") && self.at_p_at(1, ")") {
            self.bump();
            self.bump();
            return Ok(None);
        }
        let mut args = vec![];
        while !self.at_p(")") {
            let name = if matches!(self.peek(), Tok::Name(_)) && self.at_p_at(1, ":") {
                let n = self.ident()?;
                self.bump();
                Some(n)
            } else {
                None
            };
            let spread = self.eat_p("...");
            if self.at_p("&") {
                let s = self.bump().span;
                self.diags.push(
                    Diagnostic::error("E0002", s, "call-time pass-by-reference was removed in PHP 5.4")
                        .with_help("declare the parameter as `&$x` in the function instead"),
                );
            }
            let value = self.expr()?;
            args.push(Arg { name, value, spread });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(Some(args))
    }

    fn call(&mut self, callee: Expr) -> PResult<Expr> {
        let start = callee.span;
        Ok(match self.args()? {
            Some(args) => Expr::new(ExprKind::Call { callee: Box::new(callee), args }, self.since(start)),
            None => Expr::new(ExprKind::CallableRef(Box::new(callee)), self.since(start)),
        })
    }

    /// `->`, `?->`, `::`, `[...]`, `(...)`, `++`, `--` after an operand.
    fn postfix(&mut self, mut e: Expr) -> PResult<Expr> {
        loop {
            let start = e.span;
            match self.peek().clone() {
                Tok::Punct("[") => {
                    self.bump();
                    let index = if self.at_p("]") { None } else { Some(Box::new(self.expr()?)) };
                    self.expect_p("]")?;
                    e = match (e.kind, index) {
                        (ExprKind::Var(n), Some(i)) if n == "GLOBALS" => Expr::new(ExprKind::GlobalVar(i), self.since(start)),
                        (kind, index) => Expr::new(ExprKind::Index { base: Box::new(Expr { kind, ..e }), index }, self.since(start)),
                    };
                }
                Tok::Punct("{") if matches!(e.kind, ExprKind::Var(_)) && !self.tok().space_before => {
                    self.diags.push(Diagnostic::error(
                        "E0002",
                        self.span(),
                        "`$str{0}` offset syntax was removed in PHP 8, use `$str[0]`",
                    ));
                    return Err(());
                }
                Tok::Punct(p @ ("->" | "?->")) => {
                    self.bump();
                    let nullsafe = p == "?->";
                    let name = self.member_name()?;
                    if self.at_p("(") {
                        e = match self.args()? {
                            Some(args) => Expr::new(
                                ExprKind::MethodCall { object: Box::new(e), name, args, nullsafe },
                                self.since(start),
                            ),
                            None => {
                                let m = Expr::new(ExprKind::Prop { object: Box::new(e), name, nullsafe }, self.since(start));
                                Expr::new(ExprKind::CallableRef(Box::new(m)), self.since(start))
                            }
                        };
                    } else {
                        e = Expr::new(ExprKind::Prop { object: Box::new(e), name, nullsafe }, self.since(start));
                    }
                }
                Tok::Punct("::") => {
                    self.bump();
                    let class = match e.kind {
                        ExprKind::Name(ref n) => ClassRef::Name(n.clone()),
                        _ => ClassRef::Dynamic(Box::new(e.clone())),
                    };
                    match self.peek().clone() {
                        Tok::Var(n) => {
                            let span = self.bump().span;
                            if self.at_p("(") {
                                // `Foo::$method()`: call the static method named by $method.
                                let name = MemberName::Dynamic(Box::new(Expr::new(ExprKind::Var(n), span)));
                                let args = self.args()?.unwrap_or_default();
                                e = Expr::new(ExprKind::StaticCall { class, name, args }, self.since(start));
                            } else {
                                e = Expr::new(ExprKind::StaticProp { class, name: Ident { name: n, span } }, self.since(start));
                            }
                        }
                        Tok::Punct("{") => {
                            self.bump();
                            let ne = self.expr()?;
                            self.expect_p("}")?;
                            let args = self.args()?.unwrap_or_default();
                            let name = MemberName::Dynamic(Box::new(ne));
                            e = Expr::new(ExprKind::StaticCall { class, name, args }, self.since(start));
                        }
                        Tok::Punct("$") => {
                            self.variable_variable()?;
                            e = Expr::new(ExprKind::Error, self.since(start));
                        }
                        _ => {
                            let name = self.ident()?;
                            if self.at_p("(") {
                                e = match self.args()? {
                                    Some(args) => Expr::new(
                                        ExprKind::StaticCall { class, name: MemberName::Ident(name), args },
                                        self.since(start),
                                    ),
                                    None => {
                                        let r = Expr::new(ExprKind::ClassConst { class, name }, self.since(start));
                                        Expr::new(ExprKind::CallableRef(Box::new(r)), self.since(start))
                                    }
                                };
                            } else {
                                e = Expr::new(ExprKind::ClassConst { class, name }, self.since(start));
                            }
                        }
                    }
                }
                Tok::Punct("(") => e = self.call(e)?,
                Tok::Punct(p @ ("++" | "--")) => {
                    self.bump();
                    e = Expr::new(ExprKind::IncDec { prefix: false, inc: p == "++", expr: Box::new(e) }, self.since(start));
                }
                _ => return Ok(e),
            }
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.span();
        let t = self.bump();
        let kind = match t.tok {
            // Superglobals are the global variables, from any scope.
            Tok::Var(n) if SUPERGLOBALS.contains(&n.as_str()) => {
                ExprKind::GlobalVar(Box::new(Expr::new(ExprKind::Str(n), t.span)))
            }
            Tok::Var(n) => ExprKind::Var(n),
            Tok::Int(n) => ExprKind::Int(n),
            Tok::Float(f) => ExprKind::Float(f),
            Tok::Str(s) => ExprKind::Str(s),
            Tok::Template { raw, heredoc } => return Ok(self.template(&raw, heredoc, t.span)),
            Tok::Punct("(") => {
                let e = self.expr()?;
                self.expect_p(")")?;
                return Ok(Expr::new(e.kind, self.since(start)));
            }
            Tok::Punct("[") => ExprKind::Array(self.array_items("]")?),
            Tok::Name(n) => return self.name_expr(n, start),
            _ => {
                self.pos -= 1;
                return self.expected("expression");
            }
        };
        Ok(Expr::new(kind, self.since(start)))
    }

    fn name_expr(&mut self, n: String, start: Span) -> PResult<Expr> {
        let lower = n.to_ascii_lowercase();
        let mk = |p: &Self, kind| Ok(Expr::new(kind, p.since(start)));
        let call_follows = self.at_p("(");
        match lower.as_str() {
            "true" => return mk(self, ExprKind::Bool(true)),
            "false" => return mk(self, ExprKind::Bool(false)),
            "null" => return mk(self, ExprKind::Null),
            "array" | "list" if call_follows => {
                self.bump();
                let items = self.array_items(")")?;
                return mk(self, ExprKind::Array(items));
            }
            "isset" if call_follows => {
                self.bump();
                let args = self.expr_list_until(")")?;
                self.expect_p(")")?;
                return mk(self, ExprKind::Isset(args));
            }
            "empty" if call_follows => {
                self.bump();
                let e = self.expr()?;
                self.expect_p(")")?;
                return mk(self, ExprKind::Empty(Box::new(e)));
            }
            "exit" | "die" => {
                let arg = if self.eat_p("(") {
                    let a = if self.at_p(")") { None } else { Some(Box::new(self.expr()?)) };
                    self.expect_p(")")?;
                    a
                } else {
                    None
                };
                return mk(self, ExprKind::Exit(arg));
            }
            "match" if call_follows => return self.match_expr(start),
            _ => {}
        }
        let upper = n.to_ascii_uppercase();
        if MAGIC_CONSTS.contains(&upper.as_str()) {
            return mk(self, ExprKind::MagicConst(upper));
        }
        mk(self, ExprKind::Name(n))
    }

    fn match_expr(&mut self, start: Span) -> PResult<Expr> {
        let subject = self.paren_expr()?;
        self.expect_p("{")?;
        let mut arms = vec![];
        while !self.at_p("}") && !self.at_eof() {
            let conds = if self.at_kw("default") && (self.at_p_at(1, "=>") || self.at_p_at(1, ",")) {
                self.bump();
                self.eat_p(",");
                None
            } else {
                let mut cs = vec![self.expr()?];
                while self.eat_p(",") {
                    if self.at_p("=>") {
                        break;
                    }
                    cs.push(self.expr()?);
                }
                Some(cs)
            };
            self.expect_p("=>")?;
            let body = self.expr()?;
            arms.push(MatchArm { conds, body });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        Ok(Expr::new(ExprKind::Match { subject: Box::new(subject), arms }, self.since(start)))
    }

    fn array_items(&mut self, close: &str) -> PResult<Vec<Option<ArrayItem>>> {
        let mut items = vec![];
        loop {
            if self.eat_p(close) {
                return Ok(items);
            }
            if self.at_p(",") {
                // Hole in a destructuring list: `[, $b] = ...`
                self.bump();
                items.push(None);
                continue;
            }
            let spread = self.eat_p("...");
            let mut by_ref = self.eat_p("&");
            let first = self.expr()?;
            let item = if self.eat_p("=>") {
                by_ref = self.eat_p("&");
                ArrayItem { key: Some(first), value: self.expr()?, spread, by_ref }
            } else {
                ArrayItem { key: None, value: first, spread, by_ref }
            };
            items.push(Some(item));
            if !self.eat_p(",") {
                self.expect_p(close)?;
                return Ok(items);
            }
        }
    }

    // ---- string interpolation -----------------------------------------

    /// Turn the raw body of `"..."` / heredoc into literal parts and expressions.
    fn template(&mut self, raw: &str, heredoc: bool, span: Span) -> Expr {
        // Heredoc bodies had indentation stripped, so inner offsets are approximate.
        let base = if heredoc { span.start as usize } else { span.start as usize + 1 };
        let at = |i: usize| if heredoc { span } else { Span::new(base + i, base + i + 1) };
        let b = raw.as_bytes();
        let mut parts: Vec<InterpPart> = vec![];
        let mut lit: Vec<u8> = vec![];
        let mut i = 0;
        let flush = |lit: &mut Vec<u8>, parts: &mut Vec<InterpPart>| {
            if !lit.is_empty() {
                parts.push(InterpPart::Lit(std::mem::take(lit)));
            }
        };
        while i < b.len() {
            match b[i] {
                b'\\' if i + 1 < b.len() => {
                    i += 1 + unescape(&raw[i + 1..], heredoc, &mut lit);
                }
                b'$' if i + 1 < b.len() && is_ident_start(b[i + 1]) => {
                    flush(&mut lit, &mut parts);
                    let (e, used) = self.simple_interp(&raw[i..], base + i, heredoc.then_some(span));
                    parts.push(InterpPart::Expr(e));
                    i += used;
                }
                b'{' if b.get(i + 1) == Some(&b'$') => {
                    flush(&mut lit, &mut parts);
                    let Some(close) = find_close_brace(&raw[i..]) else {
                        self.diags.push(Diagnostic::error("E0002", at(i), "unclosed `{` in string"));
                        return Expr::new(ExprKind::Error, span);
                    };
                    let inner = &raw[i + 1..i + close];
                    let e = self.sub_expr(inner, if heredoc { span.start as usize } else { base + i + 1 }, at(i));
                    parts.push(InterpPart::Expr(e));
                    i += close + 1;
                }
                b'$' if b.get(i + 1) == Some(&b'{') => {
                    // `${name}`, `${name[expr]}` or `${expr}` (variable variable).
                    flush(&mut lit, &mut parts);
                    let Some(c) = find_close_brace(&raw[i + 1..]) else {
                        self.diags.push(Diagnostic::error("E0002", at(i), "unclosed `${` in string"));
                        return Expr::new(ExprKind::Error, span);
                    };
                    let inner = &raw[i + 2..i + 1 + c];
                    let off = if heredoc { span.start as usize } else { base + i + 2 };
                    let ident_len = inner.bytes().take_while(|&c| c == b'_' || c.is_ascii_alphanumeric() || c >= 0x80).count();
                    let e = if ident_len > 0 && is_ident_start(inner.as_bytes()[0]) && (ident_len == inner.len() || inner.as_bytes()[ident_len] == b'[') {
                        self.sub_expr(&format!("${inner}"), off, at(i))
                    } else {
                        let name = self.sub_expr(inner, off, at(i));
                        Expr::new(ExprKind::VarVar(Box::new(name)), at(i))
                    };
                    parts.push(InterpPart::Expr(e));
                    i += c + 2;
                }
                _ => {
                    let ch = raw[i..].chars().next().unwrap();
                    lit.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                    i += ch.len_utf8();
                }
            }
        }
        if parts.is_empty() {
            return match String::from_utf8(lit) {
                Ok(s) => Expr::new(ExprKind::Str(s), span),
                Err(e) => Expr::new(ExprKind::Bytes(e.into_bytes()), span),
            };
        }
        flush(&mut lit, &mut parts);
        Expr::new(ExprKind::Interp(parts), span)
    }

    /// `$name`, `$name[key]`, `$name->prop` inside a string. Returns bytes used.
    fn simple_interp(&mut self, s: &str, base: usize, heredoc_span: Option<Span>) -> (Expr, usize) {
        let b = s.as_bytes();
        let sp = |a: usize, z: usize| heredoc_span.unwrap_or(Span::new(base + a, base + z));
        let mut i = 1;
        while i < b.len() && is_ident_char(b[i]) {
            i += 1;
        }
        let var = Expr::new(ExprKind::Var(s[1..i].to_string()), sp(0, i));
        if b.get(i) == Some(&b'[') {
            let ks = i + 1;
            let Some(ke) = s[ks..].find(']').map(|k| ks + k) else { return (var, i) };
            let key = &s[ks..ke];
            let kind = if let Some(v) = key.strip_prefix('$') {
                ExprKind::Var(v.to_string())
            } else if let Ok(n) = key.parse::<i64>() {
                ExprKind::Int(n)
            } else {
                ExprKind::Str(key.to_string())
            };
            let idx = Expr::new(kind, sp(ks, ke));
            let e = Expr::new(ExprKind::Index { base: Box::new(var), index: Some(Box::new(idx)) }, sp(0, ke + 1));
            return (e, ke + 1);
        }
        let arrow = if s[i..].starts_with("->") { 2 } else if s[i..].starts_with("?->") { 3 } else { 0 };
        if arrow > 0 && b.get(i + arrow).is_some_and(|&c| is_ident_start(c)) {
            let ps = i + arrow;
            let mut pe = ps;
            while pe < b.len() && is_ident_char(b[pe]) {
                pe += 1;
            }
            let name = MemberName::Ident(Ident { name: s[ps..pe].to_string(), span: sp(ps, pe) });
            let e = Expr::new(ExprKind::Prop { object: Box::new(var), name, nullsafe: arrow == 3 }, sp(0, pe));
            return (e, pe);
        }
        (var, i)
    }

    /// Parse `{$expr}` contents as a standalone expression.
    fn sub_expr(&mut self, src: &str, base: usize, err_span: Span) -> Expr {
        let (toks, mut ld) = Lexer::new_code(src, base).tokenize();
        self.diags.append(&mut ld);
        let mut p = Parser::from_tokens(toks);
        let e = p.expr();
        if e.is_ok() && !p.at_eof() {
            let _: PResult<()> = p.expected("`}`");
        }
        self.diags.append(&mut p.diags);
        e.unwrap_or(Expr::new(ExprKind::Error, err_span))
    }
}

pub const SUPERGLOBALS: &[&str] = &["_SERVER", "_GET", "_POST", "_COOKIE", "_FILES", "_REQUEST", "_ENV", "_SESSION"];

fn is_kw(t: &Tok, kw: &str) -> bool {
    matches!(t, Tok::Name(n) if n.eq_ignore_ascii_case(kw))
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Eof => "end of file".into(),
        Tok::Name(n) => format!("`{n}`"),
        Tok::Var(n) => format!("`${n}`"),
        Tok::Punct(p) => format!("`{p}`"),
        Tok::Int(n) => format!("`{n}`"),
        Tok::Float(f) => format!("`{f}`"),
        Tok::Str(_) | Tok::Template { .. } => "string".into(),
        Tok::InlineHtml(_) => "inline HTML".into(),
        Tok::OpenTagEcho => "`<?=`".into(),
        Tok::CloseTag => "`?>`".into(),
        Tok::Cast(c) => format!("`({c})`"),
        Tok::Backtick(_) => "backtick string".into(),
    }
}

/// Byte index of the `}` matching the `{` at index 0.
fn find_close_brace(s: &str) -> Option<usize> {
    skip_braced(s.as_bytes()).map(|n| n - 1)
}

/// Process one escape (text after the backslash). Returns bytes consumed.
fn unescape(s: &str, heredoc: bool, out: &mut Vec<u8>) -> usize {
    let b = s.as_bytes();
    let simple = match b[0] {
        b'n' => Some('\n'),
        b't' => Some('\t'),
        b'r' => Some('\r'),
        b'v' => Some('\x0B'),
        b'e' => Some('\x1B'),
        b'f' => Some('\x0C'),
        b'\\' => Some('\\'),
        b'$' => Some('$'),
        b'"' if !heredoc => Some('"'),
        _ => None,
    };
    if let Some(c) = simple {
        out.push(c as u8);
        return 1;
    }
    if b[0].is_ascii_digit() && b[0] < b'8' {
        let n = b.iter().take(3).take_while(|c| (b'0'..b'8').contains(c)).count();
        let v = u32::from_str_radix(&s[..n], 8).unwrap_or(0) & 0xFF;
        push_byte(out, v as u8);
        return n;
    }
    if b[0] == b'x' {
        let n = b[1..].iter().take(2).take_while(|c| c.is_ascii_hexdigit()).count();
        if n > 0 {
            push_byte(out, u8::from_str_radix(&s[1..1 + n], 16).unwrap_or(0));
            return 1 + n;
        }
    }
    if s.starts_with("u{") {
        if let Some(end) = s.find('}') {
            if let Some(c) = u32::from_str_radix(&s[2..end], 16).ok().and_then(char::from_u32) {
                out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                return end + 1;
            }
        }
    }
    // Unknown escape: PHP keeps the backslash.
    out.push(b'\\');
    0
}

fn push_byte(out: &mut Vec<u8>, b: u8) {
    out.push(b);
}
