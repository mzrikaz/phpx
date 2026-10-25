//! XPath 1.0 over the DOM tree (DOMXPath::query()/evaluate()): the full
//! expression grammar, all axes but namespace, the core function library
//! and XPath's comparison and conversion rules.

use std::collections::HashMap;

use super::tree::*;

#[derive(Clone, Debug)]
pub enum XValue {
    Nodes(Vec<Id>),
    Bool(bool),
    Num(f64),
    Str(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Axis {
    Child,
    Descendant,
    DescendantOrSelf,
    Parent,
    Ancestor,
    AncestorOrSelf,
    FollowingSibling,
    PrecedingSibling,
    Following,
    Preceding,
    Attribute,
    SelfAxis,
    Namespace,
}

#[derive(Clone, Debug)]
enum Test {
    /// Name test: optional prefix, local name or "*".
    Name(Option<String>, String),
    Node,
    Text,
    Comment,
    Pi(Option<String>),
}

#[derive(Clone, Debug)]
struct Step {
    axis: Axis,
    test: Test,
    preds: Vec<Expr>,
}

#[derive(Clone, Debug)]
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Cmp(&'static str, Box<Expr>, Box<Expr>),
    Arith(char, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    Union(Box<Expr>, Box<Expr>),
    Num(f64),
    Str(String),
    Var(String),
    Call(String, Vec<Expr>),
    /// Absolute (from the root) or relative location path.
    Path { abs: bool, steps: Vec<Step> },
    /// A filter expression with predicates, then optional path steps.
    Filter { primary: Box<Expr>, preds: Vec<Expr>, steps: Vec<Step> },
}

// ---- lexer -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Slash,
    DSlash,
    LBracket,
    RBracket,
    LParen,
    RParen,
    At,
    Comma,
    Dot,
    DDot,
    DColon,
    Pipe,
    Op(&'static str),
    Star,
    Name(String),
    Num(f64),
    Lit(String),
    Var(String),
}

fn name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || (c as u32) >= 0xC0
}

fn name_char(c: char) -> bool {
    name_start(c) || c.is_ascii_digit() || c == '.' || c == '-' || c == '\u{B7}'
}

fn lex(s: &str) -> Result<Vec<Tok>, ()> {
    let cs: Vec<char> = s.chars().collect();
    let mut out: Vec<Tok> = vec![];
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        let next = cs.get(i + 1).copied();
        // An operator name or `*` is an operator only after an operand.
        let after_operand = matches!(
            out.last(),
            Some(Tok::RBracket | Tok::RParen | Tok::Dot | Tok::DDot | Tok::Star | Tok::Name(_) | Tok::Num(_) | Tok::Lit(_) | Tok::Var(_))
        ) && !matches!(out.last(), Some(Tok::Name(n)) if n.ends_with(':'));
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
                continue;
            }
            '/' if next == Some('/') => {
                out.push(Tok::DSlash);
                i += 2;
            }
            '/' => {
                out.push(Tok::Slash);
                i += 1;
            }
            '[' => {
                out.push(Tok::LBracket);
                i += 1;
            }
            ']' => {
                out.push(Tok::RBracket);
                i += 1;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            '@' => {
                out.push(Tok::At);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '|' => {
                out.push(Tok::Pipe);
                i += 1;
            }
            ':' if next == Some(':') => {
                out.push(Tok::DColon);
                i += 2;
            }
            '.' if next == Some('.') => {
                out.push(Tok::DDot);
                i += 2;
            }
            '.' if !next.is_some_and(|n| n.is_ascii_digit()) => {
                out.push(Tok::Dot);
                i += 1;
            }
            '0'..='9' | '.' => {
                let st = i;
                while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                    i += 1;
                }
                let t: String = cs[st..i].iter().collect();
                out.push(Tok::Num(t.parse().map_err(|_| ())?));
            }
            '"' | '\'' => {
                let st = i + 1;
                let e = cs[st..].iter().position(|&x| x == c).ok_or(())? + st;
                out.push(Tok::Lit(cs[st..e].iter().collect()));
                i = e + 1;
            }
            '$' => {
                let st = i + 1;
                i += 1;
                while i < cs.len() && (name_char(cs[i]) || cs[i] == ':') {
                    i += 1;
                }
                out.push(Tok::Var(cs[st..i].iter().collect()));
            }
            '=' => {
                out.push(Tok::Op("="));
                i += 1;
            }
            '!' if next == Some('=') => {
                out.push(Tok::Op("!="));
                i += 2;
            }
            '<' | '>' => {
                let op = match (c, next) {
                    ('<', Some('=')) => "<=",
                    ('>', Some('=')) => ">=",
                    ('<', _) => "<",
                    _ => ">",
                };
                i += op.len();
                out.push(Tok::Op(op));
            }
            '+' => {
                out.push(Tok::Op("+"));
                i += 1;
            }
            '-' => {
                out.push(Tok::Op("-"));
                i += 1;
            }
            '*' if after_operand => {
                out.push(Tok::Op("*"));
                i += 1;
            }
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            c if name_start(c) => {
                let st = i;
                while i < cs.len() && name_char(cs[i]) {
                    i += 1;
                }
                let mut name: String = cs[st..i].iter().collect();
                // prefix:local or prefix:*
                if i < cs.len() && cs[i] == ':' && cs.get(i + 1) != Some(&':') {
                    if cs.get(i + 1) == Some(&'*') {
                        name.push_str(":*");
                        i += 2;
                    } else if cs.get(i + 1).is_some_and(|&x| name_start(x)) {
                        let st2 = i + 1;
                        i += 1;
                        while i < cs.len() && name_char(cs[i]) {
                            i += 1;
                        }
                        name.push(':');
                        name.extend(&cs[st2..i]);
                    }
                }
                let op = match name.as_str() {
                    "and" | "or" | "div" | "mod" if after_operand => Some(match name.as_str() {
                        "and" => "and",
                        "or" => "or",
                        "div" => "div",
                        _ => "mod",
                    }),
                    _ => None,
                };
                match op {
                    Some(o) => out.push(Tok::Op(o)),
                    None => out.push(Tok::Name(name)),
                }
            }
            _ => return Err(()),
        }
    }
    Ok(out)
}

// ---- parser ------------------------------------------------------------------------------

struct Parser {
    t: Vec<Tok>,
    i: usize,
}

type P<T> = Result<T, ()>;

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }

    fn peek2(&self) -> Option<&Tok> {
        self.t.get(self.i + 1)
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn op(&mut self, ops: &[&'static str]) -> Option<&'static str> {
        if let Some(Tok::Op(o)) = self.peek() {
            if let Some(&m) = ops.iter().find(|x| *x == o) {
                self.i += 1;
                return Some(m);
            }
        }
        None
    }

    fn expr(&mut self) -> P<Expr> {
        let mut l = self.and()?;
        while self.op(&["or"]).is_some() {
            l = Expr::Or(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }

    fn and(&mut self) -> P<Expr> {
        let mut l = self.equality()?;
        while self.op(&["and"]).is_some() {
            l = Expr::And(Box::new(l), Box::new(self.equality()?));
        }
        Ok(l)
    }

    fn equality(&mut self) -> P<Expr> {
        let mut l = self.relational()?;
        while let Some(o) = self.op(&["=", "!="]) {
            l = Expr::Cmp(o, Box::new(l), Box::new(self.relational()?));
        }
        Ok(l)
    }

    fn relational(&mut self) -> P<Expr> {
        let mut l = self.additive()?;
        while let Some(o) = self.op(&["<", "<=", ">", ">="]) {
            l = Expr::Cmp(o, Box::new(l), Box::new(self.additive()?));
        }
        Ok(l)
    }

    fn additive(&mut self) -> P<Expr> {
        let mut l = self.multiplicative()?;
        while let Some(o) = self.op(&["+", "-"]) {
            l = Expr::Arith(o.chars().next().unwrap(), Box::new(l), Box::new(self.multiplicative()?));
        }
        Ok(l)
    }

    fn multiplicative(&mut self) -> P<Expr> {
        let mut l = self.unary()?;
        while let Some(o) = self.op(&["*", "div", "mod"]) {
            let c = match o {
                "*" => '*',
                "div" => '/',
                _ => '%',
            };
            l = Expr::Arith(c, Box::new(l), Box::new(self.unary()?));
        }
        Ok(l)
    }

    fn unary(&mut self) -> P<Expr> {
        if self.op(&["-"]).is_some() {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.union()
    }

    fn union(&mut self) -> P<Expr> {
        let mut l = self.path()?;
        while self.eat(&Tok::Pipe) {
            l = Expr::Union(Box::new(l), Box::new(self.path()?));
        }
        Ok(l)
    }

    fn is_primary_start(&self) -> bool {
        match self.peek() {
            Some(Tok::LParen | Tok::Lit(_) | Tok::Num(_) | Tok::Var(_)) => true,
            Some(Tok::Name(n)) => {
                self.peek2() == Some(&Tok::LParen) && !matches!(n.as_str(), "node" | "text" | "comment" | "processing-instruction")
            }
            _ => false,
        }
    }

    fn path(&mut self) -> P<Expr> {
        if self.is_primary_start() {
            let primary = self.primary()?;
            let mut preds = vec![];
            while self.peek() == Some(&Tok::LBracket) {
                preds.push(self.predicate()?);
            }
            let mut steps = vec![];
            loop {
                if self.eat(&Tok::Slash) {
                    steps.push(self.step()?);
                } else if self.eat(&Tok::DSlash) {
                    steps.push(Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: vec![] });
                    steps.push(self.step()?);
                } else {
                    break;
                }
            }
            if preds.is_empty() && steps.is_empty() {
                return Ok(primary);
            }
            return Ok(Expr::Filter { primary: Box::new(primary), preds, steps });
        }
        let mut steps = vec![];
        let abs = match self.peek() {
            Some(Tok::Slash) => {
                self.i += 1;
                // A lone `/` is the root.
                let starts_step = matches!(self.peek(), Some(Tok::Name(_) | Tok::Star | Tok::At | Tok::Dot | Tok::DDot));
                if !starts_step {
                    return Ok(Expr::Path { abs: true, steps });
                }
                true
            }
            Some(Tok::DSlash) => {
                self.i += 1;
                steps.push(Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: vec![] });
                true
            }
            _ => false,
        };
        steps.push(self.step()?);
        loop {
            if self.eat(&Tok::Slash) {
                steps.push(self.step()?);
            } else if self.eat(&Tok::DSlash) {
                steps.push(Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: vec![] });
                steps.push(self.step()?);
            } else {
                break;
            }
        }
        Ok(Expr::Path { abs, steps })
    }

    fn predicate(&mut self) -> P<Expr> {
        if !self.eat(&Tok::LBracket) {
            return Err(());
        }
        let e = self.expr()?;
        if !self.eat(&Tok::RBracket) {
            return Err(());
        }
        Ok(e)
    }

    fn step(&mut self) -> P<Step> {
        if self.eat(&Tok::Dot) {
            return Ok(Step { axis: Axis::SelfAxis, test: Test::Node, preds: vec![] });
        }
        if self.eat(&Tok::DDot) {
            return Ok(Step { axis: Axis::Parent, test: Test::Node, preds: vec![] });
        }
        let mut axis = Axis::Child;
        if self.eat(&Tok::At) {
            axis = Axis::Attribute;
        } else if let (Some(Tok::Name(n)), Some(Tok::DColon)) = (self.peek(), self.peek2()) {
            axis = match n.as_str() {
                "child" => Axis::Child,
                "descendant" => Axis::Descendant,
                "descendant-or-self" => Axis::DescendantOrSelf,
                "parent" => Axis::Parent,
                "ancestor" => Axis::Ancestor,
                "ancestor-or-self" => Axis::AncestorOrSelf,
                "following-sibling" => Axis::FollowingSibling,
                "preceding-sibling" => Axis::PrecedingSibling,
                "following" => Axis::Following,
                "preceding" => Axis::Preceding,
                "attribute" => Axis::Attribute,
                "self" => Axis::SelfAxis,
                "namespace" => Axis::Namespace,
                _ => return Err(()),
            };
            self.i += 2;
        }
        let test = match self.peek().cloned() {
            Some(Tok::Star) => {
                self.i += 1;
                Test::Name(None, "*".into())
            }
            Some(Tok::Name(n)) if self.peek2() == Some(&Tok::LParen) => {
                self.i += 2;
                let t = match n.as_str() {
                    "node" => Test::Node,
                    "text" => Test::Text,
                    "comment" => Test::Comment,
                    "processing-instruction" => {
                        let lit = match self.peek().cloned() {
                            Some(Tok::Lit(l)) => {
                                self.i += 1;
                                Some(l)
                            }
                            _ => None,
                        };
                        Test::Pi(lit)
                    }
                    _ => return Err(()),
                };
                if !self.eat(&Tok::RParen) {
                    return Err(());
                }
                t
            }
            Some(Tok::Name(n)) => {
                self.i += 1;
                match n.split_once(':') {
                    Some((p, l)) => Test::Name(Some(p.to_string()), l.to_string()),
                    None => Test::Name(None, n),
                }
            }
            _ => return Err(()),
        };
        let mut preds = vec![];
        while self.peek() == Some(&Tok::LBracket) {
            preds.push(self.predicate()?);
        }
        Ok(Step { axis, test, preds })
    }

    fn primary(&mut self) -> P<Expr> {
        match self.peek().cloned() {
            Some(Tok::LParen) => {
                self.i += 1;
                let e = self.expr()?;
                if !self.eat(&Tok::RParen) {
                    return Err(());
                }
                Ok(e)
            }
            Some(Tok::Lit(s)) => {
                self.i += 1;
                Ok(Expr::Str(s))
            }
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Var(v)) => {
                self.i += 1;
                Ok(Expr::Var(v))
            }
            Some(Tok::Name(n)) => {
                self.i += 2;
                let mut args = vec![];
                if !self.eat(&Tok::RParen) {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(&Tok::Comma) {
                            continue;
                        }
                        if self.eat(&Tok::RParen) {
                            break;
                        }
                        return Err(());
                    }
                }
                Ok(Expr::Call(n, args))
            }
            _ => Err(()),
        }
    }
}

fn parse(s: &str) -> P<Expr> {
    let t = lex(s)?;
    if t.is_empty() {
        return Err(());
    }
    let mut p = Parser { t, i: 0 };
    let e = p.expr()?;
    if p.i != p.t.len() {
        return Err(());
    }
    Ok(e)
}

// ---- evaluation ----------------------------------------------------------------------------

pub struct Ctx<'a> {
    pub doc: &'a Doc,
    /// Registered prefix -> namespace URI.
    pub ns: &'a HashMap<String, String>,
    /// Document order of every node (built on first need).
    order: std::cell::OnceCell<HashMap<Id, usize>>,
}

pub enum Error {
    /// Invalid expression (PHP: "Invalid expression").
    Syntax,
    /// Undefined namespace prefix, unknown function, ...
    Eval(String),
}

/// XPath string value of a node.
fn string_value(d: &Doc, id: Id) -> String {
    let n = d.n(id);
    match n.kind {
        ELEMENT | DOCUMENT | HTML_DOCUMENT | FRAGMENT => d.text_content(id),
        _ => n.value.clone(),
    }
}

fn str_to_num(s: &str) -> f64 {
    let t = s.trim_matches([' ', '\t', '\n', '\r']);
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') || t.matches('-').count() > 1 || (t.contains('-') && !t.starts_with('-')) || t.matches('.').count() > 1 || t == "-" || t == "." || t == "-." {
        return f64::NAN;
    }
    t.parse().unwrap_or(f64::NAN)
}

/// XPath number -> string, as libxml2 formats it (15 significant digits;
/// exponent form outside [1e-5, 1e9)).
pub fn num_to_str(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if n == n.trunc() && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let a = n.abs();
    if (1e-5..1e9).contains(&a) {
        let int_digits = if a >= 1.0 { a.log10().floor() as usize + 1 } else { 0 };
        let frac = 15usize.saturating_sub(int_digits.max(1));
        let s = format!("{n:.frac$}");
        return s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    let s = format!("{n:.14e}");
    let (m, e) = s.split_once('e').unwrap();
    let m = m.trim_end_matches('0').trim_end_matches('.');
    let e: i32 = e.parse().unwrap_or(0);
    format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
}

impl<'a> Ctx<'a> {
    pub fn new(doc: &'a Doc, ns: &'a HashMap<String, String>) -> Ctx<'a> {
        Ctx { doc, ns, order: std::cell::OnceCell::new() }
    }

    fn order(&self) -> &HashMap<Id, usize> {
        self.order.get_or_init(|| {
            let d = self.doc;
            let mut m = HashMap::new();
            let mut i = 0;
            let mut visit = |id: Id, m: &mut HashMap<Id, usize>| {
                m.insert(id, i);
                i += 1;
                for &a in &d.n(id).attrs {
                    m.insert(a, i);
                    i += 1;
                }
            };
            // Every tree: the document, then detached roots by id.
            let mut roots: Vec<Id> = vec![0];
            for (k, n) in d.nodes.iter().enumerate() {
                if n.parent.is_none() && k != 0 && n.kind != ATTRIBUTE {
                    roots.push(k as Id);
                }
            }
            for r in roots {
                visit(r, &mut m);
                for x in d.descendants(r) {
                    visit(x, &mut m);
                }
            }
            m
        })
    }

    fn sort(&self, v: &mut Vec<Id>) {
        let o = self.order();
        v.sort_by_key(|id| o.get(id).copied().unwrap_or(usize::MAX));
        v.dedup();
    }

    fn to_str(&self, v: &XValue) -> String {
        match v {
            XValue::Str(s) => s.clone(),
            XValue::Num(n) => num_to_str(*n),
            XValue::Bool(b) => if *b { "true" } else { "false" }.into(),
            XValue::Nodes(ns) => ns.first().map(|&n| string_value(self.doc, n)).unwrap_or_default(),
        }
    }

    fn to_num(&self, v: &XValue) -> f64 {
        match v {
            XValue::Num(n) => *n,
            XValue::Bool(b) => *b as i64 as f64,
            _ => str_to_num(&self.to_str(v)),
        }
    }

    fn to_bool(v: &XValue) -> bool {
        match v {
            XValue::Bool(b) => *b,
            XValue::Num(n) => *n != 0.0 && !n.is_nan(),
            XValue::Str(s) => !s.is_empty(),
            XValue::Nodes(n) => !n.is_empty(),
        }
    }

    fn axis(&self, id: Id, axis: Axis) -> Vec<Id> {
        let d = self.doc;
        let n = d.n(id);
        match axis {
            Axis::SelfAxis => vec![id],
            Axis::Child => {
                if n.kind == ATTRIBUTE {
                    vec![]
                } else {
                    d.children(id).collect()
                }
            }
            Axis::Descendant => {
                if n.kind == ATTRIBUTE {
                    vec![]
                } else {
                    d.descendants(id)
                }
            }
            Axis::DescendantOrSelf => {
                let mut v = vec![id];
                if n.kind != ATTRIBUTE {
                    v.extend(d.descendants(id));
                }
                v
            }
            Axis::Parent => n.parent.into_iter().collect(),
            Axis::Ancestor | Axis::AncestorOrSelf => {
                let mut v = vec![];
                if axis == Axis::AncestorOrSelf {
                    v.push(id);
                }
                let mut cur = n.parent;
                while let Some(p) = cur {
                    v.push(p);
                    cur = d.n(p).parent;
                }
                v
            }
            Axis::FollowingSibling | Axis::PrecedingSibling => {
                if n.kind == ATTRIBUTE {
                    return vec![];
                }
                let mut v = vec![];
                let mut cur = if axis == Axis::FollowingSibling { n.next } else { n.prev };
                while let Some(c) = cur {
                    v.push(c);
                    cur = if axis == Axis::FollowingSibling { d.n(c).next } else { d.n(c).prev };
                }
                v
            }
            Axis::Following => {
                // Everything after this node in document order, minus
                // descendants.
                let start = if n.kind == ATTRIBUTE { n.parent.unwrap_or(id) } else { id };
                let mut v = vec![];
                let mut cur = start;
                loop {
                    let mut sib = d.n(cur).next;
                    while let Some(s) = sib {
                        v.push(s);
                        v.extend(d.descendants(s));
                        sib = d.n(s).next;
                    }
                    match d.n(cur).parent {
                        Some(p) => cur = p,
                        None => break,
                    }
                }
                if n.kind == ATTRIBUTE {
                    let mut kids = d.descendants(start);
                    kids.extend(v);
                    v = kids;
                }
                v
            }
            Axis::Preceding => {
                let start = if n.kind == ATTRIBUTE { n.parent.unwrap_or(id) } else { id };
                let ancestors: Vec<Id> = self.axis(start, Axis::Ancestor);
                let root = d.root(start);
                let mut all = vec![root];
                all.extend(d.descendants(root));
                let pos = all.iter().position(|&x| x == start).unwrap_or(0);
                let mut v: Vec<Id> = all[..pos].iter().copied().filter(|x| !ancestors.contains(x)).collect();
                v.reverse();
                v
            }
            Axis::Attribute => {
                if n.kind == ELEMENT {
                    n.attrs.clone()
                } else {
                    vec![]
                }
            }
            Axis::Namespace => vec![],
        }
    }

    fn matches(&self, id: Id, axis: Axis, test: &Test) -> Result<bool, Error> {
        let n = self.doc.n(id);
        Ok(match test {
            Test::Node => true,
            Test::Text => n.is_text(),
            Test::Comment => n.kind == COMMENT,
            Test::Pi(t) => n.kind == PI && t.as_ref().is_none_or(|t| *t == n.name),
            Test::Name(prefix, local) => {
                let principal = if axis == Axis::Attribute { ATTRIBUTE } else { ELEMENT };
                if n.kind != principal {
                    return Ok(false);
                }
                let want_ns = match prefix {
                    Some(p) => Some(self.ns.get(p).cloned().ok_or_else(|| Error::Eval(format!("Undefined namespace prefix")))?),
                    None => None,
                };
                if local == "*" {
                    return Ok(prefix.is_none() || n.ns == want_ns);
                }
                n.local() == local && n.ns == want_ns
            }
        })
    }

    fn step(&self, input: &[Id], step: &Step) -> Result<Vec<Id>, Error> {
        let mut out = vec![];
        for &c in input {
            let mut cands = vec![];
            for x in self.axis(c, step.axis) {
                if self.matches(x, step.axis, &step.test)? {
                    cands.push(x);
                }
            }
            // Predicates see positions in axis order (reverse axes count
            // backwards from the context node).
            for p in &step.preds {
                cands = self.filter(cands, p)?;
            }
            out.extend(cands);
        }
        self.sort(&mut out);
        Ok(out)
    }

    fn filter(&self, nodes: Vec<Id>, pred: &Expr) -> Result<Vec<Id>, Error> {
        let size = nodes.len();
        let mut out = vec![];
        for (i, &n) in nodes.iter().enumerate() {
            let v = self.eval(pred, n, i + 1, size)?;
            let keep = match v {
                XValue::Num(x) => x == (i + 1) as f64,
                other => Self::to_bool(&other),
            };
            if keep {
                out.push(n);
            }
        }
        Ok(out)
    }

    fn compare(&self, op: &str, a: &XValue, b: &XValue) -> bool {
        use XValue::*;
        let rel = |x: f64, y: f64| match op {
            "<" => x < y,
            "<=" => x <= y,
            ">" => x > y,
            ">=" => x >= y,
            "=" => x == y,
            _ => x != y,
        };
        match (a, b) {
            (Nodes(x), Nodes(y)) => {
                let ys: Vec<String> = y.iter().map(|&n| string_value(self.doc, n)).collect();
                x.iter().any(|&n| {
                    let s = string_value(self.doc, n);
                    ys.iter().any(|t| if op == "=" || op == "!=" { (s == *t) == (op == "=") } else { rel(str_to_num(&s), str_to_num(t)) })
                })
            }
            (Nodes(x), other) | (other, Nodes(x)) => {
                let flipped = matches!(a, Nodes(_)) == false;
                x.iter().any(|&n| {
                    let s = string_value(self.doc, n);
                    let sv = Str(s.clone());
                    match other {
                        Num(v) => {
                            let (l, r) = if flipped { (*v, str_to_num(&s)) } else { (str_to_num(&s), *v) };
                            rel(l, r)
                        }
                        Bool(bv) => {
                            let nb = Self::to_bool(&Nodes(vec![n]));
                            if op == "=" || op == "!=" {
                                (nb == *bv) == (op == "=")
                            } else {
                                let (l, r) = if flipped { (*bv as i64 as f64, nb as i64 as f64) } else { (nb as i64 as f64, *bv as i64 as f64) };
                                rel(l, r)
                            }
                        }
                        Str(t) => {
                            if op == "=" || op == "!=" {
                                (s == *t) == (op == "=")
                            } else {
                                let (l, r) = if flipped { (str_to_num(t), str_to_num(&s)) } else { (str_to_num(&s), str_to_num(t)) };
                                rel(l, r)
                            }
                        }
                        Nodes(_) => {
                            let _ = sv;
                            false
                        }
                    }
                })
            }
            _ if op == "=" || op == "!=" => {
                let eq = match (a, b) {
                    (Bool(_), _) | (_, Bool(_)) => Self::to_bool(a) == Self::to_bool(b),
                    (Num(_), _) | (_, Num(_)) => self.to_num(a) == self.to_num(b),
                    _ => self.to_str(a) == self.to_str(b),
                };
                eq == (op == "=")
            }
            _ => rel(self.to_num(a), self.to_num(b)),
        }
    }

    fn nodes(&self, v: XValue) -> Result<Vec<Id>, Error> {
        match v {
            XValue::Nodes(n) => Ok(n),
            _ => Err(Error::Eval("Invalid type".into())),
        }
    }

    fn eval(&self, e: &Expr, node: Id, pos: usize, size: usize) -> Result<XValue, Error> {
        use XValue::*;
        Ok(match e {
            Expr::Or(a, b) => Bool(Self::to_bool(&self.eval(a, node, pos, size)?) || Self::to_bool(&self.eval(b, node, pos, size)?)),
            Expr::And(a, b) => Bool(Self::to_bool(&self.eval(a, node, pos, size)?) && Self::to_bool(&self.eval(b, node, pos, size)?)),
            Expr::Cmp(op, a, b) => {
                let (x, y) = (self.eval(a, node, pos, size)?, self.eval(b, node, pos, size)?);
                Bool(self.compare(op, &x, &y))
            }
            Expr::Arith(op, a, b) => {
                let (x, y) = (self.to_num(&self.eval(a, node, pos, size)?), self.to_num(&self.eval(b, node, pos, size)?));
                Num(match op {
                    '+' => x + y,
                    '-' => x - y,
                    '*' => x * y,
                    '/' => x / y,
                    _ => x % y,
                })
            }
            Expr::Neg(a) => Num(-self.to_num(&self.eval(a, node, pos, size)?)),
            Expr::Union(a, b) => {
                let mut x = self.nodes(self.eval(a, node, pos, size)?)?;
                x.extend(self.nodes(self.eval(b, node, pos, size)?)?);
                self.sort(&mut x);
                Nodes(x)
            }
            Expr::Num(n) => Num(*n),
            Expr::Str(s) => Str(s.clone()),
            Expr::Var(v) => return Err(Error::Eval(format!("Undefined variable {v}"))),
            Expr::Path { abs, steps } => {
                let mut cur = vec![if *abs { self.doc.root(node) } else { node }];
                for s in steps {
                    cur = self.step(&cur, s)?;
                }
                Nodes(cur)
            }
            Expr::Filter { primary, preds, steps } => {
                let mut cur = self.nodes(self.eval(primary, node, pos, size)?)?;
                for p in preds {
                    cur = self.filter(cur, p)?;
                }
                for s in steps {
                    cur = self.step(&cur, s)?;
                }
                Nodes(cur)
            }
            Expr::Call(name, args) => self.call(name, args, node, pos, size)?,
        })
    }

    fn call(&self, name: &str, args: &[Expr], node: Id, pos: usize, size: usize) -> Result<XValue, Error> {
        use XValue::*;
        let d = self.doc;
        let arg = |i: usize| -> Result<XValue, Error> { self.eval(&args[i], node, pos, size) };
        let sarg = |i: usize| -> Result<String, Error> { Ok(self.to_str(&arg(i)?)) };
        let narg = |i: usize| -> Result<f64, Error> { Ok(self.to_num(&arg(i)?)) };
        let ctx_or_arg = |i: usize| -> Result<Option<Id>, Error> {
            if args.len() > i {
                Ok(self.nodes(arg(i)?)?.first().copied())
            } else {
                Ok(Some(node))
            }
        };
        let arity = |lo: usize, hi: usize| -> Result<(), Error> {
            if args.len() < lo || args.len() > hi {
                Err(Error::Eval(format!("Invalid number of arguments")))
            } else {
                Ok(())
            }
        };
        Ok(match name {
            "last" => Num(size as f64),
            "position" => Num(pos as f64),
            "count" => {
                arity(1, 1)?;
                Num(self.nodes(arg(0)?)?.len() as f64)
            }
            "id" => {
                arity(1, 1)?;
                let wanted: Vec<String> = match arg(0)? {
                    Nodes(ns) => ns.iter().flat_map(|&n| string_value(d, n).split_whitespace().map(str::to_string).collect::<Vec<_>>()).collect(),
                    v => self.to_str(&v).split_whitespace().map(str::to_string).collect(),
                };
                let mut out: Vec<Id> = d
                    .descendants(0)
                    .into_iter()
                    .filter(|&e| {
                        d.n(e).kind == ELEMENT
                            && d.n(e).attrs.iter().any(|&a| {
                                let an = d.n(a);
                                (an.is_id || an.name == "xml:id" || (d.html && an.name == "id")) && wanted.contains(&an.value)
                            })
                    })
                    .collect();
                self.sort(&mut out);
                Nodes(out)
            }
            "local-name" => {
                arity(0, 1)?;
                Str(ctx_or_arg(0)?.map(|n| match d.n(n).kind {
                    ELEMENT | ATTRIBUTE => d.n(n).local().to_string(),
                    PI => d.n(n).name.clone(),
                    _ => String::new(),
                }).unwrap_or_default())
            }
            "namespace-uri" => {
                arity(0, 1)?;
                Str(ctx_or_arg(0)?.and_then(|n| d.n(n).ns.clone()).unwrap_or_default())
            }
            "name" => {
                arity(0, 1)?;
                Str(ctx_or_arg(0)?.map(|n| match d.n(n).kind {
                    ELEMENT | ATTRIBUTE | PI => d.n(n).name.clone(),
                    _ => String::new(),
                }).unwrap_or_default())
            }
            "string" => {
                arity(0, 1)?;
                if args.is_empty() {
                    Str(string_value(d, node))
                } else {
                    Str(sarg(0)?)
                }
            }
            "concat" => {
                if args.len() < 2 {
                    return Err(Error::Eval("Invalid number of arguments".into()));
                }
                let mut s = String::new();
                for i in 0..args.len() {
                    s.push_str(&sarg(i)?);
                }
                Str(s)
            }
            "starts-with" => {
                arity(2, 2)?;
                Bool(sarg(0)?.starts_with(&sarg(1)?))
            }
            "ends-with" => {
                arity(2, 2)?;
                Bool(sarg(0)?.ends_with(&sarg(1)?))
            }
            "contains" => {
                arity(2, 2)?;
                Bool(sarg(0)?.contains(&sarg(1)?))
            }
            "substring-before" => {
                arity(2, 2)?;
                let (s, t) = (sarg(0)?, sarg(1)?);
                Str(s.find(&t).map(|p| s[..p].to_string()).unwrap_or_default())
            }
            "substring-after" => {
                arity(2, 2)?;
                let (s, t) = (sarg(0)?, sarg(1)?);
                Str(s.find(&t).map(|p| s[p + t.len()..].to_string()).unwrap_or_default())
            }
            "substring" => {
                arity(2, 3)?;
                let s: Vec<char> = sarg(0)?.chars().collect();
                let start = narg(1)?.round();
                let end = if args.len() == 3 { start + narg(2)?.round() } else { f64::INFINITY };
                Str(s.iter().enumerate().filter(|(i, _)| {
                    let p = (*i + 1) as f64;
                    p >= start && p < end
                }).map(|(_, c)| *c).collect())
            }
            "string-length" => {
                arity(0, 1)?;
                let s = if args.is_empty() { string_value(d, node) } else { sarg(0)? };
                Num(s.chars().count() as f64)
            }
            "normalize-space" => {
                arity(0, 1)?;
                let s = if args.is_empty() { string_value(d, node) } else { sarg(0)? };
                Str(s.split_whitespace().collect::<Vec<_>>().join(" "))
            }
            "translate" => {
                arity(3, 3)?;
                let (s, from, to) = (sarg(0)?, sarg(1)?.chars().collect::<Vec<_>>(), sarg(2)?.chars().collect::<Vec<_>>());
                Str(s.chars().filter_map(|c| match from.iter().position(|&f| f == c) {
                    Some(i) => to.get(i).copied(),
                    None => Some(c),
                }).collect())
            }
            "boolean" => {
                arity(1, 1)?;
                Bool(Self::to_bool(&arg(0)?))
            }
            "not" => {
                arity(1, 1)?;
                Bool(!Self::to_bool(&arg(0)?))
            }
            "true" => Bool(true),
            "false" => Bool(false),
            "lang" => {
                arity(1, 1)?;
                let want = sarg(0)?.to_ascii_lowercase();
                let mut cur = Some(node);
                let mut found = None;
                while let Some(c) = cur {
                    if let Some(v) = d.get_attr(c, "xml:lang") {
                        found = Some(v.to_ascii_lowercase());
                        break;
                    }
                    cur = d.n(c).parent;
                }
                Bool(found.is_some_and(|l| l == want || l.starts_with(&format!("{want}-"))))
            }
            "number" => {
                arity(0, 1)?;
                if args.is_empty() {
                    Num(str_to_num(&string_value(d, node)))
                } else {
                    Num(narg(0)?)
                }
            }
            "sum" => {
                arity(1, 1)?;
                Num(self.nodes(arg(0)?)?.iter().map(|&n| str_to_num(&string_value(d, n))).sum())
            }
            "floor" => {
                arity(1, 1)?;
                Num(narg(0)?.floor())
            }
            "ceiling" => {
                arity(1, 1)?;
                Num(narg(0)?.ceil())
            }
            "round" => {
                arity(1, 1)?;
                let n = narg(0)?;
                Num(if n.is_nan() || n.is_infinite() { n } else { (n + 0.5).floor() })
            }
            _ => return Err(Error::Eval(format!("Unregistered function"))),
        })
    }
}

/// Evaluate `expr` with `node` as context.
pub fn evaluate(doc: &Doc, ns: &HashMap<String, String>, expr: &str, node: Id) -> Result<XValue, Error> {
    let e = parse(expr).map_err(|_| Error::Syntax)?;
    let cx = Ctx::new(doc, ns);
    cx.eval(&e, node, 1, 1)
}
