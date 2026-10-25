//! ext/tokenizer: token_get_all(), token_name() and the scanner behind
//! PhpToken. A standalone lossless scanner modelled on Zend's: it keeps
//! whitespace and comments, and splits interpolated strings the way PHP
//! reports them (T_ENCAPSED_AND_WHITESPACE, T_CURLY_OPEN, ...).

use std::rc::Rc;

use crate::builtins::{int, s};
use crate::interp::{BuiltinFn, Interp};
use crate::value::{PhpArray, Value};

const T_LNUMBER: i32 = 260;
const T_DNUMBER: i32 = 261;
const T_STRING: i32 = 262;
const T_NAME_FULLY_QUALIFIED: i32 = 263;
const T_NAME_RELATIVE: i32 = 264;
const T_NAME_QUALIFIED: i32 = 265;
const T_VARIABLE: i32 = 266;
const T_INLINE_HTML: i32 = 267;
const T_ENCAPSED_AND_WHITESPACE: i32 = 268;
const T_CONSTANT_ENCAPSED_STRING: i32 = 269;
const T_STRING_VARNAME: i32 = 270;
const T_NUM_STRING: i32 = 271;
const T_HALT_COMPILER: i32 = 335;
const T_ENUM: i32 = 339;
const T_YIELD_FROM: i32 = 282;
const T_ATTRIBUTE: i32 = 355;
const T_OBJECT_OPERATOR: i32 = 389;
const T_NULLSAFE_OBJECT_OPERATOR: i32 = 390;
const T_COMMENT: i32 = 392;
const T_DOC_COMMENT: i32 = 393;
const T_OPEN_TAG: i32 = 394;
const T_OPEN_TAG_WITH_ECHO: i32 = 395;
const T_CLOSE_TAG: i32 = 396;
const T_WHITESPACE: i32 = 397;
const T_START_HEREDOC: i32 = 398;
const T_END_HEREDOC: i32 = 399;
const T_DOLLAR_OPEN_CURLY_BRACES: i32 = 400;
const T_CURLY_OPEN: i32 = 401;
const T_NS_SEPARATOR: i32 = 403;
const T_AMPERSAND_FOLLOWED_BY_VAR_OR_VARARG: i32 = 409;
const T_AMPERSAND_NOT_FOLLOWED_BY_VAR_OR_VARARG: i32 = 410;
const T_BAD_CHARACTER: i32 = 411;

/// Names of tokens 260..=411, in id order.
const NAMES: [&str; 152] = [
    "T_LNUMBER", "T_DNUMBER", "T_STRING", "T_NAME_FULLY_QUALIFIED", "T_NAME_RELATIVE", "T_NAME_QUALIFIED", "T_VARIABLE",
    "T_INLINE_HTML", "T_ENCAPSED_AND_WHITESPACE", "T_CONSTANT_ENCAPSED_STRING", "T_STRING_VARNAME", "T_NUM_STRING",
    "T_INCLUDE", "T_INCLUDE_ONCE", "T_EVAL", "T_REQUIRE", "T_REQUIRE_ONCE", "T_LOGICAL_OR", "T_LOGICAL_XOR",
    "T_LOGICAL_AND", "T_PRINT", "T_YIELD", "T_YIELD_FROM", "T_INSTANCEOF", "T_NEW", "T_CLONE", "T_EXIT", "T_IF",
    "T_ELSEIF", "T_ELSE", "T_ENDIF", "T_ECHO", "T_DO", "T_WHILE", "T_ENDWHILE", "T_FOR", "T_ENDFOR", "T_FOREACH",
    "T_ENDFOREACH", "T_DECLARE", "T_ENDDECLARE", "T_AS", "T_SWITCH", "T_ENDSWITCH", "T_CASE", "T_DEFAULT", "T_MATCH",
    "T_BREAK", "T_CONTINUE", "T_GOTO", "T_FUNCTION", "T_FN", "T_CONST", "T_RETURN", "T_TRY", "T_CATCH", "T_FINALLY",
    "T_THROW", "T_USE", "T_INSTEADOF", "T_GLOBAL", "T_STATIC", "T_ABSTRACT", "T_FINAL", "T_PRIVATE", "T_PROTECTED",
    "T_PUBLIC", "T_PRIVATE_SET", "T_PROTECTED_SET", "T_PUBLIC_SET", "T_READONLY", "T_VAR", "T_UNSET", "T_ISSET",
    "T_EMPTY", "T_HALT_COMPILER", "T_CLASS", "T_TRAIT", "T_INTERFACE", "T_ENUM", "T_EXTENDS", "T_IMPLEMENTS",
    "T_NAMESPACE", "T_LIST", "T_ARRAY", "T_CALLABLE", "T_LINE", "T_FILE", "T_DIR", "T_CLASS_C", "T_TRAIT_C",
    "T_METHOD_C", "T_FUNC_C", "T_PROPERTY_C", "T_NS_C", "T_ATTRIBUTE", "T_PLUS_EQUAL", "T_MINUS_EQUAL", "T_MUL_EQUAL",
    "T_DIV_EQUAL", "T_CONCAT_EQUAL", "T_MOD_EQUAL", "T_AND_EQUAL", "T_OR_EQUAL", "T_XOR_EQUAL", "T_SL_EQUAL",
    "T_SR_EQUAL", "T_COALESCE_EQUAL", "T_BOOLEAN_OR", "T_BOOLEAN_AND", "T_IS_EQUAL", "T_IS_NOT_EQUAL",
    "T_IS_IDENTICAL", "T_IS_NOT_IDENTICAL", "T_IS_SMALLER_OR_EQUAL", "T_IS_GREATER_OR_EQUAL", "T_SPACESHIP", "T_SL",
    "T_SR", "T_INC", "T_DEC", "T_INT_CAST", "T_DOUBLE_CAST", "T_STRING_CAST", "T_ARRAY_CAST", "T_OBJECT_CAST",
    "T_BOOL_CAST", "T_UNSET_CAST", "T_VOID_CAST", "T_OBJECT_OPERATOR", "T_NULLSAFE_OBJECT_OPERATOR",
    "T_DOUBLE_ARROW", "T_COMMENT", "T_DOC_COMMENT", "T_OPEN_TAG", "T_OPEN_TAG_WITH_ECHO", "T_CLOSE_TAG",
    "T_WHITESPACE", "T_START_HEREDOC", "T_END_HEREDOC", "T_DOLLAR_OPEN_CURLY_BRACES", "T_CURLY_OPEN",
    "T_DOUBLE_COLON", "T_NS_SEPARATOR", "T_ELLIPSIS", "T_COALESCE", "T_POW", "T_POW_EQUAL", "T_PIPE",
    "T_AMPERSAND_FOLLOWED_BY_VAR_OR_VARARG", "T_AMPERSAND_NOT_FOLLOWED_BY_VAR_OR_VARARG", "T_BAD_CHARACTER",
];

pub fn token_name(id: i64) -> &'static str {
    match id {
        260..=411 => NAMES[(id - 260) as usize],
        _ => "UNKNOWN",
    }
}

fn id_of(name: &str) -> i32 {
    260 + NAMES.iter().position(|n| *n == name).expect("token name") as i32
}

/// Reserved words, lowercase, and their tokens.
fn keyword(w: &[u8]) -> Option<i32> {
    let lower = w.to_ascii_lowercase();
    let name = match &lower[..] {
        b"exit" | b"die" => "T_EXIT",
        b"function" => "T_FUNCTION",
        b"fn" => "T_FN",
        b"const" => "T_CONST",
        b"return" => "T_RETURN",
        b"yield" => "T_YIELD",
        b"try" => "T_TRY",
        b"catch" => "T_CATCH",
        b"finally" => "T_FINALLY",
        b"throw" => "T_THROW",
        b"if" => "T_IF",
        b"elseif" => "T_ELSEIF",
        b"endif" => "T_ENDIF",
        b"else" => "T_ELSE",
        b"while" => "T_WHILE",
        b"endwhile" => "T_ENDWHILE",
        b"do" => "T_DO",
        b"for" => "T_FOR",
        b"endfor" => "T_ENDFOR",
        b"foreach" => "T_FOREACH",
        b"endforeach" => "T_ENDFOREACH",
        b"declare" => "T_DECLARE",
        b"enddeclare" => "T_ENDDECLARE",
        b"instanceof" => "T_INSTANCEOF",
        b"as" => "T_AS",
        b"switch" => "T_SWITCH",
        b"match" => "T_MATCH",
        b"endswitch" => "T_ENDSWITCH",
        b"case" => "T_CASE",
        b"default" => "T_DEFAULT",
        b"break" => "T_BREAK",
        b"continue" => "T_CONTINUE",
        b"goto" => "T_GOTO",
        b"echo" => "T_ECHO",
        b"print" => "T_PRINT",
        b"class" => "T_CLASS",
        b"interface" => "T_INTERFACE",
        b"trait" => "T_TRAIT",
        b"extends" => "T_EXTENDS",
        b"implements" => "T_IMPLEMENTS",
        b"new" => "T_NEW",
        b"clone" => "T_CLONE",
        b"var" => "T_VAR",
        b"eval" => "T_EVAL",
        b"include" => "T_INCLUDE",
        b"include_once" => "T_INCLUDE_ONCE",
        b"require" => "T_REQUIRE",
        b"require_once" => "T_REQUIRE_ONCE",
        b"namespace" => "T_NAMESPACE",
        b"use" => "T_USE",
        b"insteadof" => "T_INSTEADOF",
        b"global" => "T_GLOBAL",
        b"isset" => "T_ISSET",
        b"empty" => "T_EMPTY",
        b"__halt_compiler" => "T_HALT_COMPILER",
        b"static" => "T_STATIC",
        b"abstract" => "T_ABSTRACT",
        b"final" => "T_FINAL",
        b"private" => "T_PRIVATE",
        b"protected" => "T_PROTECTED",
        b"public" => "T_PUBLIC",
        b"readonly" => "T_READONLY",
        b"unset" => "T_UNSET",
        b"list" => "T_LIST",
        b"array" => "T_ARRAY",
        b"callable" => "T_CALLABLE",
        b"or" => "T_LOGICAL_OR",
        b"and" => "T_LOGICAL_AND",
        b"xor" => "T_LOGICAL_XOR",
        b"__class__" => "T_CLASS_C",
        b"__trait__" => "T_TRAIT_C",
        b"__function__" => "T_FUNC_C",
        b"__method__" => "T_METHOD_C",
        b"__line__" => "T_LINE",
        b"__file__" => "T_FILE",
        b"__dir__" => "T_DIR",
        b"__namespace__" => "T_NS_C",
        b"__property__" => "T_PROPERTY_C",
        _ => return None,
    };
    Some(id_of(name))
}

/// Multi-character operators, longest first.
const OPERATORS: &[(&[u8], &str)] = &[
    (b"<<=", "T_SL_EQUAL"),
    (b">>=", "T_SR_EQUAL"),
    (b"===", "T_IS_IDENTICAL"),
    (b"!==", "T_IS_NOT_IDENTICAL"),
    (b"<=>", "T_SPACESHIP"),
    (b"**=", "T_POW_EQUAL"),
    (b"...", "T_ELLIPSIS"),
    (b"??=", "T_COALESCE_EQUAL"),
    (b"==", "T_IS_EQUAL"),
    (b"!=", "T_IS_NOT_EQUAL"),
    (b"<>", "T_IS_NOT_EQUAL"),
    (b"<=", "T_IS_SMALLER_OR_EQUAL"),
    (b">=", "T_IS_GREATER_OR_EQUAL"),
    (b"+=", "T_PLUS_EQUAL"),
    (b"-=", "T_MINUS_EQUAL"),
    (b"*=", "T_MUL_EQUAL"),
    (b"/=", "T_DIV_EQUAL"),
    (b".=", "T_CONCAT_EQUAL"),
    (b"%=", "T_MOD_EQUAL"),
    (b"&=", "T_AND_EQUAL"),
    (b"|=", "T_OR_EQUAL"),
    (b"^=", "T_XOR_EQUAL"),
    (b"||", "T_BOOLEAN_OR"),
    (b"&&", "T_BOOLEAN_AND"),
    (b"<<", "T_SL"),
    (b">>", "T_SR"),
    (b"++", "T_INC"),
    (b"--", "T_DEC"),
    (b"=>", "T_DOUBLE_ARROW"),
    (b"::", "T_DOUBLE_COLON"),
    (b"??", "T_COALESCE"),
    (b"**", "T_POW"),
    (b"|>", "T_PIPE"),
];

const CASTS: &[(&[u8], &str)] = &[
    (b"int", "T_INT_CAST"),
    (b"integer", "T_INT_CAST"),
    (b"float", "T_DOUBLE_CAST"),
    (b"double", "T_DOUBLE_CAST"),
    (b"string", "T_STRING_CAST"),
    (b"binary", "T_STRING_CAST"),
    (b"array", "T_ARRAY_CAST"),
    (b"object", "T_OBJECT_CAST"),
    (b"bool", "T_BOOL_CAST"),
    (b"boolean", "T_BOOL_CAST"),
    (b"unset", "T_UNSET_CAST"),
    (b"void", "T_VOID_CAST"),
];

fn label_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn label_char(c: u8) -> bool {
    label_start(c) || c.is_ascii_digit()
}

#[derive(Clone)]
enum St {
    Initial,
    Script,
    /// Inside "..." with interpolation.
    Dq,
    /// Inside `...`.
    Bq,
    Heredoc(Rc<Vec<u8>>),
    Nowdoc(Rc<Vec<u8>>),
    /// The closing label of a heredoc is next, after this much indentation.
    EndHeredoc(usize, usize),
    /// After `->`: a label is a property name.
    LookProp,
    /// After `${`: a variable name, or an expression.
    LookVarname,
    /// After `"$a[`.
    VarOffset,
    /// After __halt_compiler(); everything is inline data.
    Halted,
}

pub struct Token {
    /// Token id; below 256, a single character.
    pub id: i32,
    pub start: usize,
    pub end: usize,
    pub line: i64,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: i64,
    cur: St,
    stack: Vec<St>,
    out: Vec<Token>,
    /// Tokens seen since T_HALT_COMPILER (`(`, `)`, `;`).
    halt: Option<u8>,
}

impl<'a> Lexer<'a> {
    fn at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    fn starts(&self, i: usize, s: &[u8]) -> bool {
        self.src.get(i..i + s.len()) == Some(s)
    }

    fn starts_ci(&self, i: usize, s: &[u8]) -> bool {
        self.src.get(i..i + s.len()).is_some_and(|w| w.eq_ignore_ascii_case(s))
    }

    fn emit(&mut self, id: i32, end: usize) {
        let start = self.pos;
        self.out.push(Token { id, start, end, line: self.line });
        self.line += self.src[start..end].iter().filter(|&&c| c == b'\n').count() as i64;
        // A lone \r counts as a newline too.
        self.line += self.src[start..end].windows(2).filter(|w| w[0] == b'\r' && w[1] != b'\n').count() as i64;
        if end > start && self.src[end - 1] == b'\r' {
            self.line += 1;
        }
        self.pos = end;
    }

    fn push(&mut self, s: St) {
        let old = std::mem::replace(&mut self.cur, s);
        self.stack.push(old);
    }

    fn pop(&mut self) {
        self.cur = self.stack.pop().unwrap_or(St::Script);
    }

    fn label_end(&self, mut i: usize) -> usize {
        while label_char(self.at(i)) {
            i += 1;
        }
        i
    }

    fn whitespace_end(&self, mut i: usize) -> usize {
        while matches!(self.at(i), b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        i
    }

    /// Past whitespace and comments.
    fn skip_trivia(&self, mut i: usize) -> usize {
        loop {
            let j = self.whitespace_end(i);
            if self.starts(j, b"/*") {
                i = self.find(j + 2, b"*/").map_or(self.src.len(), |e| e + 2);
            } else if self.starts(j, b"//") || (self.at(j) == b'#' && self.at(j + 1) != b'[') {
                i = self.line_comment_end(j);
            } else {
                return j;
            }
        }
    }

    fn find(&self, from: usize, pat: &[u8]) -> Option<usize> {
        if from > self.src.len() {
            return None;
        }
        memchr::memmem::find(&self.src[from..], pat).map(|i| i + from)
    }

    /// End of a `//` or `#` comment: before the newline or `?>`.
    fn line_comment_end(&self, mut i: usize) -> usize {
        while i < self.src.len() {
            match self.src[i] {
                b'\n' | b'\r' => break,
                b'?' if self.at(i + 1) == b'>' => break,
                _ => i += 1,
            }
        }
        i
    }

    fn run(&mut self) {
        while self.pos < self.src.len() {
            match self.cur.clone() {
                St::Initial => self.initial(),
                St::Script => self.script(),
                St::Dq => self.interp_part(b'"'),
                St::Bq => self.interp_part(b'`'),
                St::Heredoc(l) => self.heredoc(&l, true),
                St::Nowdoc(l) => self.heredoc(&l, false),
                St::EndHeredoc(indent, len) => {
                    self.cur = St::Script;
                    self.emit(T_END_HEREDOC, self.pos + indent + len);
                }
                St::LookProp => self.look_prop(),
                St::LookVarname => self.look_varname(),
                St::VarOffset => self.var_offset(),
                St::Halted => {
                    let end = self.src.len();
                    self.emit(T_INLINE_HTML, end);
                }
            }
        }
    }

    fn initial(&mut self) {
        let mut i = self.pos;
        loop {
            match self.find(i, b"<?") {
                None => {
                    let end = self.src.len();
                    return self.emit(T_INLINE_HTML, end);
                }
                Some(p) => {
                    let open = if self.at(p + 2) == b'=' {
                        Some((T_OPEN_TAG_WITH_ECHO, p + 3))
                    } else if self.starts_ci(p + 2, b"php") {
                        match self.at(p + 5) {
                            b'\r' if self.at(p + 6) == b'\n' => Some((T_OPEN_TAG, p + 7)),
                            b' ' | b'\t' | b'\n' | b'\r' => Some((T_OPEN_TAG, p + 6)),
                            0 if p + 5 == self.src.len() => Some((T_OPEN_TAG, p + 5)),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    match open {
                        Some((id, end)) => {
                            if p > self.pos {
                                self.emit(T_INLINE_HTML, p);
                            }
                            self.emit(id, end);
                            self.cur = St::Script;
                            return;
                        }
                        None => i = p + 2,
                    }
                }
            }
        }
    }

    fn script(&mut self) {
        let p = self.pos;
        let c = self.src[p];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' => {
                let e = self.whitespace_end(p);
                self.emit(T_WHITESPACE, e)
            }
            b'#' if self.at(p + 1) == b'[' => self.emit(T_ATTRIBUTE, p + 2),
            b'#' => {
                let e = self.line_comment_end(p);
                self.emit(T_COMMENT, e)
            }
            b'/' if self.at(p + 1) == b'/' => {
                let e = self.line_comment_end(p);
                self.emit(T_COMMENT, e)
            }
            b'/' if self.at(p + 1) == b'*' => {
                let doc = self.at(p + 2) == b'*' && matches!(self.at(p + 3), b' ' | b'\t' | b'\n' | b'\r');
                let e = self.find(p + 2, b"*/").map_or(self.src.len(), |e| e + 2);
                self.emit(if doc { T_DOC_COMMENT } else { T_COMMENT }, e)
            }
            b'?' if self.at(p + 1) == b'>' => {
                let mut e = p + 2;
                if self.at(e) == b'\n' {
                    e += 1;
                } else if self.at(e) == b'\r' {
                    e += if self.at(e + 1) == b'\n' { 2 } else { 1 };
                }
                self.emit(T_CLOSE_TAG, e);
                self.cur = St::Initial;
            }
            b'?' if self.starts(p, b"?->") => {
                self.push(St::LookProp);
                self.emit(T_NULLSAFE_OBJECT_OPERATOR, p + 3)
            }
            b'-' if self.at(p + 1) == b'>' => {
                self.push(St::LookProp);
                self.emit(T_OBJECT_OPERATOR, p + 2)
            }
            b'$' if label_start(self.at(p + 1)) => {
                let e = self.label_end(p + 1);
                self.emit(T_VARIABLE, e)
            }
            b'0'..=b'9' => self.number(),
            b'.' if self.at(p + 1).is_ascii_digit() => self.number(),
            b'\'' => {
                let e = self.single_quoted_end(p + 1);
                self.emit(T_CONSTANT_ENCAPSED_STRING, e)
            }
            b'"' => self.double_quote(p + 1),
            b'b' | b'B' if self.at(p + 1) == b'\'' => {
                let e = self.single_quoted_end(p + 2);
                self.emit(T_CONSTANT_ENCAPSED_STRING, e)
            }
            b'b' | b'B' if self.at(p + 1) == b'"' => self.double_quote(p + 2),
            b'b' | b'B' if self.starts(p + 1, b"<<<") && self.heredoc_start(p + 1) => {}
            b'<' if self.starts(p, b"<<<") && self.heredoc_start(p) => {}
            b'`' => {
                self.cur = St::Bq;
                self.emit(b'`' as i32, p + 1)
            }
            b'{' => {
                self.push(St::Script);
                self.emit(b'{' as i32, p + 1)
            }
            b'}' => {
                if !self.stack.is_empty() {
                    self.pop();
                }
                self.emit(b'}' as i32, p + 1)
            }
            b'(' => {
                if let Some((id, e)) = self.cast(p) {
                    return self.emit(id, e);
                }
                self.emit(b'(' as i32, p + 1)
            }
            b'&' if !matches!(self.at(p + 1), b'&' | b'=') => {
                let j = self.whitespace_end(p + 1);
                let id = if self.at(j) == b'$' || self.starts(j, b"...") {
                    T_AMPERSAND_FOLLOWED_BY_VAR_OR_VARARG
                } else {
                    T_AMPERSAND_NOT_FOLLOWED_BY_VAR_OR_VARARG
                };
                self.emit(id, p + 1)
            }
            b'\\' if label_start(self.at(p + 1)) => {
                let e = self.name_end(p + 1);
                self.emit(T_NAME_FULLY_QUALIFIED, e)
            }
            b'\\' => self.emit(T_NS_SEPARATOR, p + 1),
            _ if label_start(c) => self.word(),
            _ => {
                for (op, name) in OPERATORS {
                    if self.starts(p, op) {
                        return self.emit(id_of(name), p + op.len());
                    }
                }
                if b";:,.|^+-/*=%!~$<>?@[]()\"".contains(&c) {
                    self.emit(c as i32, p + 1)
                } else {
                    self.emit(T_BAD_CHARACTER, p + 1)
                }
            }
        }
        if let Some(n) = self.halt {
            self.after_halt(n);
        }
    }

    /// __halt_compiler ( ) ; then the rest is data.
    fn after_halt(&mut self, n: u8) {
        let Some(t) = self.out.last() else { return };
        let ignorable = matches!(t.id, T_WHITESPACE | T_COMMENT | T_DOC_COMMENT);
        let want = [b'(' as i32, b')' as i32, b';' as i32];
        if t.id == T_HALT_COMPILER || ignorable {
            return;
        }
        if t.id == want[n as usize] || (n == 2 && t.id == T_CLOSE_TAG) {
            if n == 2 {
                self.halt = None;
                self.cur = St::Halted;
            } else {
                self.halt = Some(n + 1);
            }
        } else {
            self.halt = None;
        }
    }

    /// Past `Foo\Bar\Baz` (from the first label).
    fn name_end(&self, i: usize) -> usize {
        let mut e = self.label_end(i);
        while self.at(e) == b'\\' && label_start(self.at(e + 1)) {
            e = self.label_end(e + 1);
        }
        e
    }

    fn word(&mut self) {
        let p = self.pos;
        let e = self.label_end(p);
        let w = &self.src[p..e];
        // Qualified names win over keywords.
        if self.at(e) == b'\\' && label_start(self.at(e + 1)) {
            let end = self.name_end(p);
            let id = if w.eq_ignore_ascii_case(b"namespace") { T_NAME_RELATIVE } else { T_NAME_QUALIFIED };
            return self.emit(id, end);
        }
        let lower = w.to_ascii_lowercase();
        match &lower[..] {
            b"yield" => {
                let j = self.skip_trivia(e);
                if j > e && self.starts_ci(j, b"from") && !label_char(self.at(j + 4)) {
                    return self.emit(T_YIELD_FROM, j + 4);
                }
            }
            b"enum" => {
                let j = self.skip_trivia(e);
                let next = self.label_end(j);
                let after = self.src[j..next].to_ascii_lowercase();
                let id = if j > e && next > j && after != b"extends" && after != b"implements" { T_ENUM } else { T_STRING };
                return self.emit(id, e);
            }
            b"private" | b"protected" | b"public" => {
                let j = self.whitespace_end(e);
                if self.at(j) == b'(' {
                    let k = self.whitespace_end(j + 1);
                    if self.starts_ci(k, b"set") {
                        let m = self.whitespace_end(k + 3);
                        if self.at(m) == b')' {
                            let id = id_of(match &lower[..] {
                                b"private" => "T_PRIVATE_SET",
                                b"protected" => "T_PROTECTED_SET",
                                _ => "T_PUBLIC_SET",
                            });
                            return self.emit(id, m + 1);
                        }
                    }
                }
            }
            _ => {}
        }
        let id = keyword(w).unwrap_or(T_STRING);
        if id == T_HALT_COMPILER {
            self.halt = Some(0);
        }
        self.emit(id, e)
    }

    fn cast(&self, p: usize) -> Option<(i32, usize)> {
        let skip = |mut i: usize| {
            while matches!(self.at(i), b' ' | b'\t') {
                i += 1;
            }
            i
        };
        let i = skip(p + 1);
        let e = self.label_end(i);
        let w = self.src[i..e].to_ascii_lowercase();
        let (_, name) = CASTS.iter().find(|(c, _)| *c == &w[..])?;
        let j = skip(e);
        (self.at(j) == b')').then(|| (id_of(name), j + 1))
    }

    fn single_quoted_end(&self, mut i: usize) -> usize {
        while i < self.src.len() {
            match self.src[i] {
                b'\\' => i += 2,
                b'\'' => return i + 1,
                _ => i += 1,
            }
        }
        self.src.len()
    }

    /// `"` ending just before `body`: a constant string unless it
    /// interpolates.
    fn double_quote(&mut self, body: usize) {
        let mut i = body;
        while i < self.src.len() {
            match self.src[i] {
                b'"' => return self.emit(T_CONSTANT_ENCAPSED_STRING, i + 1),
                b'\\' => i += 2,
                b'$' if label_start(self.at(i + 1)) || self.at(i + 1) == b'{' => break,
                b'{' if self.at(i + 1) == b'$' => break,
                _ => i += 1,
            }
        }
        self.cur = St::Dq;
        self.emit(b'"' as i32, body);
    }

    /// `<<<LABEL\n` at `p` (after an optional `b`).
    fn heredoc_start(&mut self, p: usize) -> bool {
        let mut i = p + 3;
        while matches!(self.at(i), b' ' | b'\t') {
            i += 1;
        }
        let quote = match self.at(i) {
            q @ (b'\'' | b'"') => {
                i += 1;
                Some(q)
            }
            _ => None,
        };
        if !label_start(self.at(i)) {
            return false;
        }
        let e = self.label_end(i);
        let label = self.src[i..e].to_vec();
        let mut j = e;
        if let Some(q) = quote {
            if self.at(j) != q {
                return false;
            }
            j += 1;
        }
        match self.at(j) {
            b'\n' => j += 1,
            b'\r' => j += if self.at(j + 1) == b'\n' { 2 } else { 1 },
            _ => return false,
        }
        let label = Rc::new(label);
        self.cur = if quote == Some(b'\'') { St::Nowdoc(label.clone()) } else { St::Heredoc(label.clone()) };
        self.emit(T_START_HEREDOC, j);
        // Closing label right away: an empty heredoc.
        if let Some(indent) = self.closing_label_at(self.pos, &label) {
            self.cur = St::EndHeredoc(indent, label.len());
        }
        true
    }

    /// The closing label at line start `i`: its indentation.
    fn closing_label_at(&self, i: usize, label: &[u8]) -> Option<usize> {
        let mut j = i;
        while matches!(self.at(j), b' ' | b'\t') {
            j += 1;
        }
        (self.starts(j, label) && !label_char(self.at(j + label.len()))).then_some(j - i)
    }

    fn heredoc(&mut self, label: &Rc<Vec<u8>>, interpolate: bool) {
        let p = self.pos;
        if interpolate && self.interpolation(p) {
            return;
        }
        let mut i = p;
        let n = self.src.len();
        while i < n {
            match self.src[i] {
                b'\n' | b'\r' => {
                    i += if self.src[i] == b'\r' && self.at(i + 1) == b'\n' { 2 } else { 1 };
                    if let Some(indent) = self.closing_label_at(i, label) {
                        self.emit(T_ENCAPSED_AND_WHITESPACE, i);
                        self.cur = St::EndHeredoc(indent, label.len());
                        return;
                    }
                }
                b'$' if interpolate && (label_start(self.at(i + 1)) || self.at(i + 1) == b'{') => break,
                b'{' if interpolate && self.at(i + 1) == b'$' => break,
                b'\\' if interpolate && !matches!(self.at(i + 1), b'\n' | b'\r') => i += 2,
                _ => i += 1,
            }
        }
        let e = i.min(n);
        if e > p {
            self.emit(T_ENCAPSED_AND_WHITESPACE, e);
        }
    }

    /// `$var`, `${` or `{$` at `p` inside a string: emit it and enter the
    /// matching state.
    fn interpolation(&mut self, p: usize) -> bool {
        match self.src[p] {
            b'$' if label_start(self.at(p + 1)) => {
                let e = self.label_end(p + 1);
                if self.at(e) == b'[' {
                    self.push(St::VarOffset);
                } else if (self.starts(e, b"->") && label_start(self.at(e + 2))) || (self.starts(e, b"?->") && label_start(self.at(e + 3))) {
                    self.push(St::LookProp);
                }
                self.emit(T_VARIABLE, e);
                true
            }
            b'$' if self.at(p + 1) == b'{' => {
                self.push(St::LookVarname);
                self.emit(T_DOLLAR_OPEN_CURLY_BRACES, p + 2);
                true
            }
            b'{' if self.at(p + 1) == b'$' => {
                self.push(St::Script);
                self.emit(T_CURLY_OPEN, p + 1);
                true
            }
            _ => false,
        }
    }

    /// Inside "..." or `...`.
    fn interp_part(&mut self, close: u8) {
        let p = self.pos;
        if self.src[p] == close {
            self.cur = St::Script;
            return self.emit(close as i32, p + 1);
        }
        if self.interpolation(p) {
            return;
        }
        let mut i = p;
        while i < self.src.len() {
            match self.src[i] {
                c if c == close => break,
                b'\\' => i += 2,
                b'$' if label_start(self.at(i + 1)) || self.at(i + 1) == b'{' => break,
                b'{' if self.at(i + 1) == b'$' => break,
                _ => i += 1,
            }
        }
        let e = i.min(self.src.len());
        self.emit(T_ENCAPSED_AND_WHITESPACE, e)
    }

    fn look_prop(&mut self) {
        let p = self.pos;
        match self.src[p] {
            b' ' | b'\t' | b'\n' | b'\r' => {
                let e = self.whitespace_end(p);
                self.emit(T_WHITESPACE, e)
            }
            b'-' if self.at(p + 1) == b'>' => self.emit(T_OBJECT_OPERATOR, p + 2),
            b'?' if self.starts(p, b"?->") => self.emit(T_NULLSAFE_OBJECT_OPERATOR, p + 3),
            c if label_start(c) => {
                self.pop();
                let e = self.label_end(p);
                self.emit(T_STRING, e)
            }
            _ => self.pop(),
        }
    }

    fn look_varname(&mut self) {
        let p = self.pos;
        let e = self.label_end(p);
        self.pop();
        self.push(St::Script);
        if e > p && matches!(self.at(e), b'[' | b'}') {
            self.emit(T_STRING_VARNAME, e);
        }
    }

    fn var_offset(&mut self) {
        let p = self.pos;
        let c = self.src[p];
        match c {
            b'0'..=b'9' => {
                let mut e = p + 1;
                while self.at(e).is_ascii_alphanumeric() || self.at(e) == b'_' {
                    e += 1;
                }
                self.emit(T_NUM_STRING, e)
            }
            b'$' if label_start(self.at(p + 1)) => {
                let e = self.label_end(p + 1);
                self.emit(T_VARIABLE, e)
            }
            b']' => {
                self.pop();
                self.emit(b']' as i32, p + 1)
            }
            _ if label_start(c) => {
                let e = self.label_end(p);
                self.emit(T_STRING, e)
            }
            b' ' | b'\n' | b'\r' | b'\t' | b'\\' | b'\'' | b'#' => self.pop(),
            _ if b";:,.|^&+-/*=%!~$<>?@[](){}\"`".contains(&c) => self.emit(c as i32, p + 1),
            _ => self.emit(T_BAD_CHARACTER, p + 1),
        }
    }

    fn number(&mut self) {
        let p = self.pos;
        let digits = |mut i: usize, ok: fn(u8) -> bool| {
            while ok(self.at(i)) || (self.at(i) == b'_' && ok(self.at(i + 1))) {
                i += 1;
            }
            i
        };
        let lower = self.at(p + 1).to_ascii_lowercase();
        if self.at(p) == b'0' && matches!(lower, b'x' | b'b' | b'o') {
            let ok: fn(u8) -> bool = match lower {
                b'x' => |c: u8| c.is_ascii_hexdigit(),
                b'b' => |c: u8| c == b'0' || c == b'1',
                _ => |c: u8| (b'0'..=b'7').contains(&c),
            };
            if ok(self.at(p + 2)) {
                let e = digits(p + 2, ok);
                let radix = match lower {
                    b'x' => 16,
                    b'b' => 2,
                    _ => 8,
                };
                let text: String = self.src[p + 2..e].iter().filter(|&&c| c != b'_').map(|&c| c as char).collect();
                let id = if i64::from_str_radix(&text, radix).is_ok() { T_LNUMBER } else { T_DNUMBER };
                return self.emit(id, e);
            }
        }
        let dec = |c: u8| c.is_ascii_digit();
        let int_end = digits(p, dec);
        let mut e = int_end;
        let mut float = false;
        if self.at(e) == b'.' && (e > p || dec(self.at(e + 1))) {
            float = true;
            e = digits(e + 1, dec);
        }
        if matches!(self.at(e), b'e' | b'E') {
            let mut j = e + 1;
            if matches!(self.at(j), b'+' | b'-') {
                j += 1;
            }
            if dec(self.at(j)) {
                float = true;
                e = digits(j, dec);
            }
        }
        if !float {
            let text: String = self.src[p..e].iter().filter(|&&c| c != b'_').map(|&c| c as char).collect();
            let fits = if text.len() > 1 && text.starts_with('0') {
                text.bytes().all(|c| c < b'8') && i64::from_str_radix(&text, 8).is_ok()
            } else {
                text.parse::<i64>().is_ok()
            };
            float = !fits;
        }
        self.emit(if float { T_DNUMBER } else { T_LNUMBER }, e)
    }
}

pub fn tokenize(src: &[u8]) -> Vec<Token> {
    let mut lx = Lexer { src, pos: 0, line: 1, cur: St::Initial, stack: vec![], out: vec![], halt: None };
    lx.run();
    lx.out
}

pub fn register(t: &mut std::collections::HashMap<&'static str, BuiltinFn>) {
    t.insert("token_get_all", |it, _, a| {
        let src = s(it, a, 0)?;
        let mut out = PhpArray::new();
        for t in tokenize(&src) {
            let text = Value::bytes(src[t.start..t.end].to_vec());
            if t.id < 256 {
                out.push(text);
            } else {
                out.push(Value::array(PhpArray::from_list([Value::Int(t.id as i64), text, Value::Int(t.line)])));
            }
        }
        Ok(Value::array(out))
    });
    t.insert("token_name", |it, _, a| {
        let id = int(it, a, 0)?;
        Ok(Value::str(token_name(id)))
    });
    // [id, text, line, pos] rows for PhpToken::tokenize().
    t.insert("__phpx_tokenize", |it: &Interp, _, a| {
        let src = s(it, a, 0)?;
        let mut out = PhpArray::new();
        for t in tokenize(&src) {
            let text = Value::bytes(src[t.start..t.end].to_vec());
            out.push(Value::array(PhpArray::from_list([Value::Int(t.id as i64), text, Value::Int(t.line), Value::Int(t.start as i64)])));
        }
        Ok(Value::array(out))
    });
}
