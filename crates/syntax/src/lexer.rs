//! Tokenizer. Keywords are not special here: every word is a `Name` and the
//! parser decides by context (PHP allows keywords as method/const names).

use crate::diag::{Diagnostic, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// Text outside `<?php ... ?>`, echoed as-is.
    InlineHtml(String),
    /// `<?=`: start of a short echo.
    OpenTagEcho,
    /// `?>`: ends a statement like `;`.
    CloseTag,
    /// `$name` (without the `$`).
    Var(String),
    /// Identifier or qualified name: `foo`, `Foo\Bar`, `\Foo`, `namespace\Foo`.
    Name(String),
    Int(i64),
    Float(f64),
    /// Single-quoted string or nowdoc, escapes already processed.
    Str(String),
    /// Double-quoted string or heredoc body. Raw: escapes and `$var` are
    /// handled by the parser, which needs to parse embedded expressions.
    Template { raw: String, heredoc: bool },
    /// `` `cmd` ``: shell exec. Lexed so the parser can reject it nicely.
    Backtick(String),
    /// `(int)`, `(string)`, ... normalized to lowercase canonical name.
    Cast(&'static str),
    Punct(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// `/** ... */` directly before this token.
    pub doc: Option<String>,
    /// Whitespace or a comment came before this token.
    pub space_before: bool,
}

/// Longest first, so `<=>` wins over `<=` and `<`.
const PUNCTS: &[&str] = &[
    "<<=", ">>=", "**=", "...", "<=>", "===", "!==", "??=", "?->", "++", "--", "->", "=>", "::", "==",
    "!=", "<>", "<=", ">=", "&&", "||", "??", "+=", "-=", "*=", "/=", ".=", "%=", "&=", "|=", "^=",
    "<<", ">>", "**", "#[", "+", "-", "*", "/", "%", "=", "<", ">", "!", ".", "(", ")", "[", "]", "{",
    "}", ",", ";", "?", ":", "&", "|", "^", "~", "@", "$", "\\",
];

pub struct Lexer<'a> {
    src: &'a str,
    b: &'a [u8],
    pos: usize,
    /// Added to every span (for lexing an embedded `{$expr}` inside a string).
    base: usize,
    in_php: bool,
    pub diags: Vec<Diagnostic>,
}

pub fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

pub fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

impl<'a> Lexer<'a> {
    /// Lex a whole file, starting in inline-HTML mode.
    pub fn new(src: &'a str) -> Self {
        Lexer { src, b: src.as_bytes(), pos: 0, base: 0, in_php: false, diags: vec![] }
    }

    /// Lex a code fragment (already inside `<?php`) whose first byte is at `base`.
    pub fn new_code(src: &'a str, base: usize) -> Self {
        Lexer { src, b: src.as_bytes(), pos: 0, base, in_php: true, diags: vec![] }
    }

    pub fn tokenize(mut self) -> (Vec<Token>, Vec<Diagnostic>) {
        let mut out = vec![];
        loop {
            let t = self.next_token();
            let eof = t.tok == Tok::Eof;
            out.push(t);
            if eof {
                break;
            }
        }
        (out, self.diags)
    }

    fn span(&self, start: usize) -> Span {
        Span::new(self.base + start, self.base + self.pos)
    }

    fn peek(&self, off: usize) -> u8 {
        *self.b.get(self.pos + off).unwrap_or(&0)
    }

    fn starts_with(&self, s: &str) -> bool {
        self.b[self.pos..].starts_with(s.as_bytes())
    }

    fn err(&mut self, start: usize, msg: impl Into<String>) {
        let span = self.span(start);
        self.diags.push(Diagnostic::error("E0001", span, msg));
    }

    fn next_token(&mut self) -> Token {
        if !self.in_php {
            return self.inline_html();
        }
        let mut doc = None;
        let ws_start = self.pos;
        self.skip_trivia(&mut doc);
        let space_before = self.pos > ws_start;
        let start = self.pos;
        let tok = self.lex_php();
        Token { tok, span: self.span(start), doc, space_before }
    }

    fn inline_html(&mut self) -> Token {
        let start = self.pos;
        let rest = &self.src[self.pos..];
        let (text_len, tag_len, tok_after) = match find_open_tag(rest) {
            Some((i, len, echo)) => (i, len, Some(echo)),
            None => (rest.len(), 0, None),
        };
        if text_len > 0 {
            self.pos += text_len;
            return Token {
                tok: Tok::InlineHtml(rest[..text_len].to_string()),
                span: self.span(start),
                doc: None,
                space_before: false,
            };
        }
        match tok_after {
            None => Token { tok: Tok::Eof, span: self.span(start), doc: None, space_before: false },
            Some(echo) => {
                self.pos += tag_len;
                self.in_php = true;
                if echo {
                    Token { tok: Tok::OpenTagEcho, span: self.span(start), doc: None, space_before: false }
                } else {
                    self.next_token()
                }
            }
        }
    }

    fn skip_trivia(&mut self, doc: &mut Option<String>) {
        loop {
            let c = self.peek(0);
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else if self.starts_with("//") || (c == b'#' && self.peek(1) != b'[') {
                // Line comment ends at newline or `?>`.
                while self.pos < self.b.len() && self.peek(0) != b'\n' && !self.starts_with("?>") {
                    self.pos += 1;
                }
            } else if self.starts_with("/*") {
                let start = self.pos;
                match self.src[self.pos + 2..].find("*/") {
                    Some(i) => self.pos += i + 4,
                    None => {
                        self.pos = self.b.len();
                        self.err(start, "unterminated comment");
                    }
                }
                let text = &self.src[start..self.pos];
                if text.starts_with("/**") && text.len() > 4 {
                    *doc = Some(text.to_string());
                }
            } else {
                return;
            }
        }
    }

    fn lex_php(&mut self) -> Tok {
        let start = self.pos;
        let c = self.peek(0);
        if c == 0 && self.pos >= self.b.len() {
            return Tok::Eof;
        }
        if self.starts_with("?>") {
            self.pos += 2;
            // A single newline right after `?>` is swallowed, like PHP.
            if self.starts_with("\r\n") {
                self.pos += 2;
            } else if self.peek(0) == b'\n' {
                self.pos += 1;
            }
            self.in_php = false;
            return Tok::CloseTag;
        }
        if c == b'$' && is_ident_start(self.peek(1)) {
            self.pos += 1;
            let s = self.pos;
            self.eat_ident();
            return Tok::Var(self.src[s..self.pos].to_string());
        }
        if is_ident_start(c) || (c == b'\\' && is_ident_start(self.peek(1))) {
            return self.name();
        }
        if c.is_ascii_digit() || (c == b'.' && self.peek(1).is_ascii_digit()) {
            return self.number();
        }
        if c == b'\'' {
            return self.single_quoted();
        }
        if c == b'"' {
            return self.double_quoted();
        }
        if c == b'`' {
            self.pos += 1;
            let s = self.pos;
            while self.pos < self.b.len() && self.peek(0) != b'`' {
                self.pos += if self.peek(0) == b'\\' { 2 } else { 1 };
            }
            let text = self.src[s..self.pos.min(self.b.len())].to_string();
            if self.pos >= self.b.len() {
                self.err(start, "unterminated backtick string");
            } else {
                self.pos += 1;
            }
            return Tok::Backtick(text);
        }
        if self.starts_with("<<<") {
            if let Some(t) = self.heredoc() {
                return t;
            }
        }
        if c == b'(' {
            if let Some(t) = self.cast() {
                return t;
            }
        }
        for p in PUNCTS {
            if self.starts_with(p) {
                self.pos += p.len();
                return Tok::Punct(p);
            }
        }
        // Unknown byte: skip one whole char.
        let ch = self.src[self.pos..].chars().next().unwrap();
        self.pos += ch.len_utf8();
        self.err(start, format!("unexpected character `{ch}`"));
        self.lex_php_after_error()
    }

    fn lex_php_after_error(&mut self) -> Tok {
        let mut doc = None;
        self.skip_trivia(&mut doc);
        self.lex_php()
    }

    fn eat_ident(&mut self) {
        while is_ident_char(self.peek(0)) {
            self.pos += 1;
        }
    }

    fn name(&mut self) -> Tok {
        let s = self.pos;
        if self.peek(0) == b'\\' {
            self.pos += 1;
        }
        self.eat_ident();
        // Qualified: `Foo\Bar\baz`.
        while self.peek(0) == b'\\' && is_ident_start(self.peek(1)) {
            self.pos += 1;
            self.eat_ident();
        }
        Tok::Name(self.src[s..self.pos].to_string())
    }

    fn number(&mut self) -> Tok {
        let s = self.pos;
        let radix = match (self.peek(0), self.peek(1) | 0x20) {
            (b'0', b'x') => 16,
            (b'0', b'b') => 2,
            (b'0', b'o') => 8,
            _ => 10,
        };
        if radix != 10 {
            self.pos += 2;
            let ds = self.pos;
            while self.peek(0).is_ascii_hexdigit() || self.peek(0) == b'_' {
                self.pos += 1;
            }
            let digits: String = self.src[ds..self.pos].chars().filter(|&c| c != '_').collect();
            return int_or_float(&digits, radix);
        }
        let mut is_float = false;
        self.eat_digits();
        if self.peek(0) == b'.' && self.peek(1).is_ascii_digit() {
            is_float = true;
            self.pos += 1;
            self.eat_digits();
        } else if self.peek(0) == b'.' && !matches!(self.peek(1), b'.' | b'=') && !is_ident_start(self.peek(1)) {
            // `1.` is a float; `1..2` / `1.=` / `1.foo` are not.
            is_float = true;
            self.pos += 1;
        }
        if self.peek(0) | 0x20 == b'e'
            && (self.peek(1).is_ascii_digit() || (matches!(self.peek(1), b'+' | b'-') && self.peek(2).is_ascii_digit()))
        {
            is_float = true;
            self.pos += 2;
            self.eat_digits();
        }
        let text: String = self.src[s..self.pos].chars().filter(|&c| c != '_').collect();
        if is_float {
            return Tok::Float(text.parse().unwrap_or(0.0));
        }
        // Legacy octal: `0755`.
        if text.len() > 1 && text.starts_with('0') {
            return int_or_float(&text[1..], 8);
        }
        int_or_float(&text, 10)
    }

    fn eat_digits(&mut self) {
        while self.peek(0).is_ascii_digit() || (self.peek(0) == b'_' && self.peek(1).is_ascii_digit()) {
            self.pos += 1;
        }
    }

    fn single_quoted(&mut self) -> Tok {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        let mut seg = self.pos;
        loop {
            match self.peek(0) {
                0 if self.pos >= self.b.len() => {
                    self.err(start, "unterminated string");
                    out.push_str(&self.src[seg..]);
                    return Tok::Str(out);
                }
                b'\\' if matches!(self.peek(1), b'\\' | b'\'') => {
                    out.push_str(&self.src[seg..self.pos]);
                    out.push(self.peek(1) as char);
                    self.pos += 2;
                    seg = self.pos;
                }
                b'\'' => {
                    out.push_str(&self.src[seg..self.pos]);
                    self.pos += 1;
                    return Tok::Str(out);
                }
                _ => self.pos += 1,
            }
        }
    }

    fn double_quoted(&mut self) -> Tok {
        let start = self.pos;
        self.pos += 1;
        let s = self.pos;
        while self.pos < self.b.len() && self.peek(0) != b'"' {
            if self.peek(0) == b'{' && self.peek(1) == b'$' {
                // `{$expr}` may contain quoted strings of its own.
                match skip_braced(&self.b[self.pos..]) {
                    Some(n) => self.pos += n,
                    None => self.pos = self.b.len(),
                }
                continue;
            }
            self.pos += if self.peek(0) == b'\\' { 2 } else { 1 };
        }
        let end = self.pos.min(self.b.len());
        let raw = self.src[s..end].to_string();
        if self.pos >= self.b.len() {
            self.err(start, "unterminated string");
        } else {
            self.pos += 1;
        }
        Tok::Template { raw, heredoc: false }
    }

    /// `<<<EOT` / `<<<"EOT"` (heredoc) or `<<<'EOT'` (nowdoc), PHP 7.3 flexible syntax.
    fn heredoc(&mut self) -> Option<Tok> {
        let start = self.pos;
        let mut p = self.pos + 3;
        while matches!(self.b.get(p), Some(b' ' | b'\t')) {
            p += 1;
        }
        let quote = match self.b.get(p) {
            Some(&q @ (b'\'' | b'"')) => {
                p += 1;
                Some(q)
            }
            _ => None,
        };
        let ls = p;
        while p < self.b.len() && is_ident_char(self.b[p]) {
            p += 1;
        }
        if p == ls || !is_ident_start(self.b[ls]) {
            return None;
        }
        let label = &self.src[ls..p];
        if let Some(q) = quote {
            if self.b.get(p) != Some(&q) {
                return None;
            }
            p += 1;
        }
        // Rest of the opening line must be the newline.
        if self.b.get(p) == Some(&b'\r') {
            p += 1;
        }
        if self.b.get(p) != Some(&b'\n') {
            return None;
        }
        p += 1;
        let body_start = p;

        // Find the closing label: first line whose trimmed start is the label
        // not followed by an identifier char.
        let mut line_start = body_start;
        let (close_line, indent, after) = loop {
            if line_start > self.b.len() {
                self.pos = self.b.len();
                self.err(start, format!("unterminated heredoc, missing `{label}`"));
                return Some(Tok::Str(String::new()));
            }
            let mut q = line_start;
            while matches!(self.b.get(q), Some(b' ' | b'\t')) {
                q += 1;
            }
            if self.src[q..].starts_with(label) && !self.b.get(q + label.len()).is_some_and(|&c| is_ident_char(c)) {
                break (line_start, q - line_start, q + label.len());
            }
            match self.src[line_start..].find('\n') {
                Some(i) => line_start += i + 1,
                None => line_start = self.b.len() + 1,
            }
        };
        // Body excludes the newline before the closing line.
        let mut body_end = close_line;
        if body_end > body_start {
            body_end -= 1;
            if body_end > body_start && self.b[body_end - 1] == b'\r' {
                body_end -= 1;
            }
        }
        let body = if body_end > body_start { &self.src[body_start..body_end] } else { "" };
        let body: String = body
            .split('\n')
            .map(|l| {
                let strip = l.bytes().take(indent).take_while(|c| matches!(c, b' ' | b'\t')).count();
                &l[strip..]
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.pos = after;
        Some(if quote == Some(b'\'') { Tok::Str(body) } else { Tok::Template { raw: body, heredoc: true } })
    }

    /// `(int)`, `( string )` etc. Returns None if it's just a parenthesis.
    fn cast(&mut self) -> Option<Tok> {
        let mut p = self.pos + 1;
        while matches!(self.b.get(p), Some(b' ' | b'\t')) {
            p += 1;
        }
        let ws = p;
        while p < self.b.len() && self.b[p].is_ascii_alphabetic() {
            p += 1;
        }
        let word = self.src[ws..p].to_ascii_lowercase();
        while matches!(self.b.get(p), Some(b' ' | b'\t')) {
            p += 1;
        }
        if self.b.get(p) != Some(&b')') {
            return None;
        }
        let kind = match word.as_str() {
            "int" | "integer" => "int",
            "bool" | "boolean" => "bool",
            "float" | "double" | "real" => "float",
            "string" | "binary" => "string",
            "array" => "array",
            "object" => "object",
            "unset" => "unset",
            _ => return None,
        };
        self.pos = p + 1;
        Some(Tok::Cast(kind))
    }
}

/// Find `<?php` (followed by whitespace/EOF) or `<?=`. Returns (offset, tag length, is_echo).
fn find_open_tag(s: &str) -> Option<(usize, usize, bool)> {
    let mut from = 0;
    while let Some(i) = s[from..].find("<?") {
        let at = from + i;
        let rest = &s.as_bytes()[at + 2..];
        if rest.first() == Some(&b'=') {
            return Some((at, 3, true));
        }
        if rest.len() >= 3 && rest[..3].eq_ignore_ascii_case(b"php") {
            match rest.get(3) {
                None => return Some((at, 5, false)),
                Some(c) if c.is_ascii_whitespace() => {
                    // The whitespace char (one newline) belongs to the tag.
                    let mut len = 6;
                    if *c == b'\r' && rest.get(4) == Some(&b'\n') {
                        len = 7;
                    }
                    return Some((at, len, false));
                }
                _ => {}
            }
        }
        from = at + 2;
    }
    None
}

/// Length of a `{...}` group starting at index 0, skipping nested braces and
/// quoted strings (including double-quoted strings with their own `{$...}`).
pub fn skip_braced(b: &[u8]) -> Option<usize> {
    let mut depth = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            b'\'' => {
                i += 1;
                while i < b.len() && b[i] != b'\'' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'{' && b.get(i + 1) == Some(&b'$') {
                        i += skip_braced(&b[i..])?;
                        continue;
                    }
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// PHP turns integer literals that overflow i64 into floats.
fn int_or_float(digits: &str, radix: u32) -> Tok {
    match i64::from_str_radix(digits, radix) {
        Ok(n) => Tok::Int(n),
        Err(_) if radix == 10 => Tok::Float(digits.parse().unwrap_or(f64::INFINITY)),
        Err(_) => {
            let f = digits.chars().fold(0f64, |acc, c| acc * radix as f64 + c.to_digit(radix).unwrap_or(0) as f64);
            Tok::Float(f)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        let (t, d) = Lexer::new(src).tokenize();
        assert!(d.is_empty(), "{d:?}");
        t.into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn basics() {
        use Tok::*;
        assert_eq!(
            toks("<?php $a = Foo\\bar(1_000, 0x1F, 1.5e3, 'it\\'s') ?->x <=> (int) $b;"),
            vec![
                Var("a".into()),
                Punct("="),
                Name("Foo\\bar".into()),
                Punct("("),
                Int(1000),
                Punct(","),
                Int(31),
                Punct(","),
                Float(1500.0),
                Punct(","),
                Str("it's".into()),
                Punct(")"),
                Punct("?->"),
                Name("x".into()),
                Punct("<=>"),
                Cast("int"),
                Var("b".into()),
                Punct(";"),
                Eof
            ]
        );
    }

    #[test]
    fn inline_html_and_tags() {
        use Tok::*;
        assert_eq!(
            toks("<h1><?= $t ?></h1>\n<?php echo 1;"),
            vec![
                InlineHtml("<h1>".into()),
                OpenTagEcho,
                Var("t".into()),
                CloseTag,
                InlineHtml("</h1>\n".into()),
                Name("echo".into()),
                Int(1),
                Punct(";"),
                Eof
            ]
        );
    }

    #[test]
    fn heredoc_strips_indent() {
        let t = toks("<?php $x = <<<SQL\n    select *\n      from t\n    SQL;");
        assert_eq!(t[2], Tok::Template { raw: "select *\n  from t".into(), heredoc: true });
        let t = toks("<?php $x = <<<'RAW'\n$no\nRAW;\n");
        assert_eq!(t[2], Tok::Str("$no".into()));
    }

    #[test]
    fn overflow_becomes_float() {
        assert_eq!(toks("<?php 9223372036854775808;")[0], Tok::Float(9223372036854775808.0));
        assert_eq!(toks("<?php 0755;")[0], Tok::Int(0o755));
    }

    #[test]
    fn doc_comments_attach() {
        let (t, _) = Lexer::new("<?php /** @var int */ $x; // no\n /* no */ $y;").tokenize();
        assert_eq!(t[0].doc.as_deref(), Some("/** @var int */"));
        assert_eq!(t[2].doc, None);
    }
}
