//! HTML parsing as libxml2 2.13's HTMLparser.c does it (DOMDocument::
//! loadHTML): implied html/head/body/p, start tags that close open
//! elements, end-tag priorities, whitespace dropping, HTML 4 entities.

use super::tables::{ALLOW_PCDATA, ENTITIES, START_CLOSE};
use super::tree::*;

pub const HTML_PARSE_RECOVER: i64 = 1;
pub const HTML_PARSE_NODEFDTD: i64 = 4;
pub const HTML_PARSE_NOBLANKS: i64 = 256;
pub const HTML_PARSE_NOIMPLIED: i64 = 8192;

/// A parse error as libxml2 reports it.
pub struct ParseError {
    pub level: u8,
    pub code: i64,
    pub message: String,
    pub line: u32,
    /// Where in the input (byte offset).
    pub offset: usize,
}

pub fn entity_value(name: &str) -> Option<u32> {
    ENTITIES.iter().find(|e| e.1 == name).map(|e| e.0)
}

fn end_priority(name: &str) -> u32 {
    match name {
        "div" => 150,
        "td" | "th" => 160,
        "tr" => 170,
        "thead" | "tbody" | "tfoot" => 180,
        "table" => 190,
        "head" | "body" => 200,
        "html" => 220,
        _ => 100,
    }
}

fn closes(newtag: &str, oldtag: &str) -> bool {
    START_CLOSE.binary_search_by(|e| (e.0, e.1).cmp(&(oldtag, newtag))).is_ok()
}

pub fn is_boolean_attr(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "checked" | "compact" | "declare" | "defer" | "disabled" | "ismap" | "multiple" | "nohref" | "noresize" | "noshade" | "nowrap" | "readonly" | "selected"
    )
}

fn is_char(c: u32) -> bool {
    matches!(c, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

fn name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b':' || c >= 0x80
}

fn name_char(c: u8) -> bool {
    name_start(c) || c.is_ascii_digit() || c == b'.' || c == b'-'
}

fn blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

/// The input as UTF-8: a BOM or `<?xm` means UTF-8, a meta charset names
/// the encoding, and anything else is ISO-8859-1 (libxml2's default).
pub fn decode(input: &[u8]) -> (String, Option<String>) {
    if let Some(rest) = input.strip_prefix(b"\xEF\xBB\xBF") {
        return (crate::pstr::lossy(rest).into_owned(), None);
    }
    if input.starts_with(b"<?xm") {
        return (crate::pstr::lossy(input).into_owned(), Some("UTF-8".into()));
    }
    match meta_charset(input) {
        Some(cs) if cs.eq_ignore_ascii_case("utf-8") || cs.eq_ignore_ascii_case("utf8") => (crate::pstr::lossy(input).into_owned(), Some(cs)),
        cs => {
            // libxml2 settles on ISO-8859-1 at the first non-ASCII byte.
            let cs = cs.or_else(|| (!input.is_ascii()).then(|| "ISO-8859-1".to_string()));
            (input.iter().map(|&b| b as char).collect(), cs)
        }
    }
}

/// charset of the first `<meta>` that declares one.
fn meta_charset(input: &[u8]) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    let mut from = 0;
    while let Some(p) = memchr::memmem::find(&lower[from..], b"<meta") {
        let start = from + p;
        let end = memchr::memchr(b'>', &lower[start..]).map_or(lower.len(), |e| start + e);
        let tag = &lower[start..end];
        if let Some(c) = memchr::memmem::find(tag, b"charset") {
            let mut i = c + 7;
            while i < tag.len() && blank(tag[i]) {
                i += 1;
            }
            if i < tag.len() && tag[i] == b'=' {
                i += 1;
                while i < tag.len() && (blank(tag[i]) || tag[i] == b'"' || tag[i] == b'\'') {
                    i += 1;
                }
                let s = i;
                while i < tag.len() && (tag[i].is_ascii_alphanumeric() || tag[i] == b'-' || tag[i] == b'_') {
                    i += 1;
                }
                if i > s {
                    return Some(crate::pstr::lossy(&input[start + s..start + i]).into_owned());
                }
            }
        }
        from = end;
    }
    None
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
    doc: Doc,
    /// Open elements: (name, node).
    stack: Vec<(String, Id)>,
    /// libxml2's ctxt->html: 3 after <head>, 10 after <body>.
    level: u8,
    /// Ignored misplaced html/head/body tags.
    depth: i32,
    opts: i64,
    errors: Vec<ParseError>,
    line_starts: Vec<usize>,
}

impl P<'_> {
    fn cur(&self) -> u8 {
        self.s.get(self.i).copied().unwrap_or(0)
    }

    fn nxt(&self, n: usize) -> u8 {
        self.s.get(self.i + n).copied().unwrap_or(0)
    }

    fn line(&self) -> u32 {
        self.line_starts.partition_point(|&s| s <= self.i) as u32
    }

    fn err(&mut self, code: i64, msg: String) {
        let line = self.line();
        let offset = self.i;
        self.errors.push(ParseError { level: 2, code, message: msg, line, offset });
    }

    fn name(&self) -> Option<&str> {
        self.stack.last().map(|(n, _)| n.as_str())
    }

    fn node(&self) -> Option<Id> {
        self.stack.last().map(|e| e.1)
    }

    fn skip_blanks(&mut self) {
        while blank(self.cur()) {
            self.i += 1;
        }
    }

    // ---- SAX ---------------------------------------------------------------

    fn start_element(&mut self, name: &str, attrs: Vec<(String, Option<String>)>) {
        if name == "head" && self.level < 3 {
            self.level = 3;
        }
        if name == "body" && self.level < 10 {
            self.level = 10;
        }
        let mut n = Node::new(ELEMENT, name, "");
        n.line = self.line();
        let el = self.doc.add(n);
        for (k, v) in attrs {
            let (v, novalue) = match v {
                Some(v) => (v, false),
                None if is_boolean_attr(&k) => (k.clone(), false),
                None => (String::new(), true),
            };
            let a = self.doc.set_attr(el, &k, &v);
            self.doc.m(a).raw = novalue;
        }
        let parent = self.node().unwrap_or(0);
        self.doc.append(parent, el);
        self.stack.push((name.to_string(), el));
    }

    fn end_element(&mut self) {
        self.stack.pop();
    }

    fn characters(&mut self, s: &str) {
        if let Some(n) = self.node() {
            self.doc.append_text(n, TEXT, s);
        }
    }

    fn cdata(&mut self, s: &str) {
        if let Some(n) = self.node() {
            self.doc.append_text(n, CDATA, s);
        }
    }

    fn add_misc(&mut self, node: Node) {
        let id = self.doc.add(node);
        let parent = self.node().unwrap_or(0);
        self.doc.append(parent, id);
    }

    fn doctype(&mut self, name: &str, public: Option<String>, system: Option<String>) {
        if self.doc.doctype().is_some() {
            return;
        }
        let mut n = Node::new(DOCTYPE, name, "");
        n.ids = Some(Box::new((public.unwrap_or_default(), system.unwrap_or_default(), String::new())));
        let id = self.doc.add(n);
        let first = self.doc.n(0).first;
        self.doc.insert(0, id, first);
    }

    // ---- structure -----------------------------------------------------------

    fn auto_close(&mut self, newtag: &str) {
        while let Some(name) = self.name() {
            if !closes(newtag, name) {
                break;
            }
            self.end_element();
        }
    }

    fn check_implied(&mut self, newtag: &str) {
        if self.opts & HTML_PARSE_NOIMPLIED != 0 || newtag == "html" {
            return;
        }
        if self.stack.is_empty() {
            self.start_element("html", vec![]);
        }
        if newtag == "body" || newtag == "head" {
            return;
        }
        if self.stack.len() <= 1 && matches!(newtag, "script" | "style" | "meta" | "link" | "title" | "base") {
            if self.level >= 3 {
                return;
            }
            self.start_element("head", vec![]);
        } else if !matches!(newtag, "noframes" | "frame" | "frameset") {
            if self.level >= 10 {
                return;
            }
            if self.stack.iter().any(|(n, _)| n == "body" || n == "head") {
                return;
            }
            self.start_element("body", vec![]);
        }
    }

    fn check_paragraph(&mut self) {
        match self.name() {
            None => {}
            Some("html") | Some("head") => {}
            Some(_) => return,
        }
        self.auto_close("p");
        self.check_implied("p");
        self.start_element("p", vec![]);
    }

    fn auto_close_on_close(&mut self, newtag: &str) {
        let pri = end_priority(newtag);
        let mut found = false;
        for (n, _) in self.stack.iter().rev() {
            if n == newtag {
                found = true;
                break;
            }
            if end_priority(n) > pri {
                return;
            }
        }
        if !found {
            return;
        }
        while self.name() != Some(newtag) {
            let cur = self.name().unwrap_or_default().to_string();
            if html_info(&cur).is_some_and(|i| i.0 == 3) {
                self.err(76, format!("Opening and ending tag mismatch: {newtag} and {cur}"));
            }
            self.end_element();
        }
    }

    fn are_blanks(&self, text: &str) -> bool {
        if !text.bytes().all(blank) {
            return false;
        }
        match self.cur() {
            0 => return true,
            b'<' => {}
            _ => return false,
        }
        let Some(name) = self.name() else { return true };
        if name == "html" || name == "head" {
            return true;
        }
        if name == "body" {
            if let Some(dt) = self.doc.doctype() {
                let public = &self.doc.n(dt).ids.as_ref().unwrap().0;
                if public.eq_ignore_ascii_case("-//W3O//DTD W3 HTML Strict 3.0//EN") || public.eq_ignore_ascii_case("-//W3C//DTD HTML 4.0 Strict//EN") {
                    return true;
                }
            }
        }
        let Some(node) = self.node() else { return false };
        let mut last = self.doc.n(node).last;
        while let Some(l) = last {
            if self.doc.n(l).kind != COMMENT {
                break;
            }
            last = self.doc.n(l).prev;
        }
        match last {
            None => !ALLOW_PCDATA.contains(&name),
            Some(l) if self.doc.n(l).kind == TEXT => false,
            Some(l) => !ALLOW_PCDATA.contains(&self.doc.n(l).name.as_str()),
        }
    }

    // ---- lexical -------------------------------------------------------------

    /// Tag or attribute name, lowercased.
    fn html_name(&mut self) -> Option<String> {
        let c = self.cur();
        if !(c.is_ascii_alphabetic() || c == b'_' || c == b':' || c == b'.') {
            return None;
        }
        let st = self.i;
        while self.cur().is_ascii_alphanumeric() || matches!(self.cur(), b':' | b'-' | b'_' | b'.') {
            self.i += 1;
        }
        Some(crate::pstr::lossy(&self.s[st..self.i]).to_ascii_lowercase())
    }

    /// An XML Name (entity names, PI targets, doctype names).
    fn xml_name(&mut self) -> Option<String> {
        if !name_start(self.cur()) {
            return None;
        }
        let st = self.i;
        while name_char(self.cur()) {
            self.i += 1;
        }
        Some(crate::pstr::lossy(&self.s[st..self.i]).into_owned())
    }

    fn char_ref(&mut self) -> u32 {
        let mut val: u32 = 0;
        let hex = matches!(self.nxt(2), b'x' | b'X');
        self.i += if hex { 3 } else { 2 };
        loop {
            let c = self.cur();
            if c == b';' {
                self.i += 1;
                break;
            }
            let d = if hex { (c as char).to_digit(16) } else { (c as char).to_digit(10) };
            match d {
                Some(d) => {
                    if val < 0x110000 {
                        val = val * if hex { 16 } else { 10 } + d;
                    }
                    self.i += 1;
                }
                None => {
                    self.err(1, "htmlParseCharRef: missing semicolon".into());
                    break;
                }
            }
        }
        if is_char(val) {
            val
        } else {
            self.err(9, format!("htmlParseCharRef: invalid xmlChar value {val}"));
            0
        }
    }

    /// `&name;`: (entity value, name).
    fn entity_ref(&mut self) -> (Option<u32>, Option<String>) {
        self.i += 1;
        let Some(name) = self.xml_name() else {
            self.err(68, "htmlParseEntityRef: no name".into());
            return (None, None);
        };
        if self.cur() == b';' {
            let v = entity_value(&name);
            if v.is_some() {
                self.i += 1;
            }
            (v, Some(name))
        } else {
            self.err(23, "htmlParseEntityRef: expecting ';'".into());
            (None, Some(name))
        }
    }

    fn attr_value_until(&mut self, stop: u8) -> String {
        let mut out = String::new();
        loop {
            let c = self.cur();
            if c == 0 || c == stop || (stop == 0 && (c == b'>' || blank(c))) {
                break;
            }
            if c == b'&' {
                if self.nxt(1) == b'#' {
                    let v = self.char_ref();
                    if let Some(ch) = char::from_u32(v).filter(|_| v != 0) {
                        out.push(ch);
                    }
                } else {
                    match self.entity_ref() {
                        (_, None) => out.push('&'),
                        (None, Some(n)) => {
                            out.push('&');
                            out.push_str(&n);
                        }
                        (Some(v), _) => out.push(char::from_u32(v).unwrap_or('\u{FFFD}')),
                    }
                }
            } else {
                let len = utf8_len(c);
                out.push_str(std::str::from_utf8(&self.s[self.i..(self.i + len).min(self.s.len())]).unwrap_or("\u{FFFD}"));
                self.i += len;
            }
        }
        out
    }

    fn attr_value(&mut self) -> String {
        match self.cur() {
            q @ (b'"' | b'\'') => {
                self.i += 1;
                let v = self.attr_value_until(q);
                if self.cur() == q {
                    self.i += 1;
                } else {
                    self.err(39, "AttValue: \" expected".into());
                }
                v
            }
            _ => self.attr_value_until(0),
        }
    }

    // ---- constructs ------------------------------------------------------------

    /// Returns false when no element was opened.
    fn start_tag(&mut self) -> bool {
        self.i += 1;
        let Some(name) = self.html_name() else {
            self.err(68, "htmlParseStartTag: invalid element name".into());
            while self.cur() != 0 && self.cur() != b'>' {
                self.i += 1;
            }
            return false;
        };
        self.auto_close(&name);
        self.check_implied(&name);
        let mut discard = false;
        if !self.stack.is_empty() && name == "html" {
            self.err(800, "htmlParseStartTag: misplaced <html> tag".into());
            discard = true;
            self.depth += 1;
        }
        if self.stack.len() != 1 && name == "head" {
            self.err(800, "htmlParseStartTag: misplaced <head> tag".into());
            discard = true;
            self.depth += 1;
        }
        if name == "body" {
            let n = self.stack.iter().filter(|(n, _)| n == "body").count();
            if n > 0 {
                self.err(800, "htmlParseStartTag: misplaced <body> tag".into());
                discard = true;
                self.depth += n as i32;
            }
        }
        self.skip_blanks();
        let mut attrs: Vec<(String, Option<String>)> = vec![];
        while self.cur() != 0 && self.cur() != b'>' && !(self.cur() == b'/' && self.nxt(1) == b'>') {
            match self.html_name() {
                Some(an) => {
                    self.skip_blanks();
                    let val = if self.cur() == b'=' {
                        self.i += 1;
                        self.skip_blanks();
                        Some(self.attr_value())
                    } else {
                        None
                    };
                    if attrs.iter().any(|(k, _)| *k == an) {
                        self.err(42, format!("Attribute {an} redefined"));
                    } else {
                        attrs.push((an, val));
                    }
                }
                None => {
                    self.err(68, "error parsing attribute name".into());
                    while self.cur() != 0 && !blank(self.cur()) && self.cur() != b'>' && !(self.cur() == b'/' && self.nxt(1) == b'>') {
                        self.i += 1;
                    }
                }
            }
            self.skip_blanks();
        }
        if discard {
            return true;
        }
        if html_info(&name).is_none() {
            self.err(801, format!("Tag {name} invalid"));
        }
        self.start_element(&name, attrs);
        true
    }

    fn element(&mut self) {
        let opened = self.start_tag();
        let Some(name) = self.name().map(str::to_string) else {
            if self.cur() == b'>' {
                self.i += 1;
            }
            return;
        };
        if !opened {
            if self.cur() == b'>' {
                self.i += 1;
            }
            return;
        }
        if self.cur() == b'/' && self.nxt(1) == b'>' {
            self.i += 2;
            self.end_element();
            return;
        }
        if self.cur() == b'>' {
            self.i += 1;
        } else {
            self.err(73, format!("Couldn't find end of Start Tag {name}"));
            self.end_element();
            return;
        }
        if html_info(&name).is_some_and(|i| i.2) {
            self.end_element();
        }
    }

    /// Returns true when it closed the current element.
    fn end_tag(&mut self) -> bool {
        self.i += 2;
        let Some(name) = self.html_name() else { return false };
        self.skip_blanks();
        if self.cur() != b'>' {
            self.err(73, "End tag : expected '>'".into());
            while self.cur() != 0 && self.cur() != b'>' {
                self.i += 1;
            }
        }
        if self.cur() == b'>' {
            self.i += 1;
        }
        if self.depth > 0 && matches!(name.as_str(), "html" | "body" | "head") {
            self.depth -= 1;
            return false;
        }
        if !self.stack.iter().any(|(n, _)| *n == name) {
            self.err(76, format!("Unexpected end tag : {name}"));
            return false;
        }
        self.auto_close_on_close(&name);
        if self.name() == Some(name.as_str()) {
            self.end_element();
            true
        } else {
            false
        }
    }

    fn script(&mut self) {
        let st = self.i;
        while self.cur() != 0 {
            if self.cur() == b'<' && self.nxt(1) == b'/' {
                if self.opts & HTML_PARSE_RECOVER != 0 {
                    let name = self.name().unwrap_or_default();
                    let n = name.len();
                    if self.s.get(self.i + 2..self.i + 2 + n).is_some_and(|w| w.eq_ignore_ascii_case(name.as_bytes())) {
                        break;
                    }
                } else if self.nxt(2).is_ascii_alphabetic() {
                    break;
                }
            }
            self.i += 1;
        }
        if self.i > st {
            let text = crate::pstr::lossy(&self.s[st..self.i]).into_owned();
            self.cdata(&text);
        }
    }

    fn char_data(&mut self) {
        let st = self.i;
        while !matches!(self.cur(), b'<' | b'&' | 0) {
            self.i += 1;
        }
        let text = crate::pstr::lossy(&self.s[st..self.i]).into_owned();
        if self.are_blanks(&text) {
            if self.opts & HTML_PARSE_NOBLANKS == 0 {
                self.characters(&text);
            }
        } else {
            self.check_paragraph();
            self.characters(&text);
        }
    }

    fn reference(&mut self) {
        let mut buf = [0u8; 4];
        if self.nxt(1) == b'#' {
            let v = self.char_ref();
            if v == 0 {
                return;
            }
            self.check_paragraph();
            let s = char::from_u32(v).unwrap_or('\u{FFFD}').encode_utf8(&mut buf).to_string();
            self.characters(&s);
        } else {
            match self.entity_ref() {
                (_, None) => {
                    self.check_paragraph();
                    self.characters("&");
                }
                (None, Some(n)) => {
                    self.check_paragraph();
                    self.characters(&format!("&{n}"));
                }
                (Some(v), _) => {
                    self.check_paragraph();
                    let s = char::from_u32(v).unwrap_or('\u{FFFD}').encode_utf8(&mut buf).to_string();
                    self.characters(&s);
                }
            }
        }
    }

    fn comment(&mut self) {
        self.i += 4;
        let st = self.i;
        // `<!-->` and `<!--->` are empty comments.
        if self.cur() == b'>' {
            self.i += 1;
            return self.add_misc(Node::new(COMMENT, "#comment", ""));
        }
        if self.cur() == b'-' && self.nxt(1) == b'>' {
            self.i += 2;
            return self.add_misc(Node::new(COMMENT, "#comment", ""));
        }
        let mut j = st;
        while j < self.s.len() {
            if self.s[j..].starts_with(b"-->") {
                let text = crate::pstr::lossy(&self.s[st..j]).into_owned();
                self.i = j + 3;
                return self.add_misc(Node::new(COMMENT, "#comment", &text));
            }
            if self.s[j..].starts_with(b"--!>") {
                let text = crate::pstr::lossy(&self.s[st..j]).into_owned();
                self.i = j + 4;
                return self.add_misc(Node::new(COMMENT, "#comment", &text));
            }
            j += 1;
        }
        self.err(45, "Comment not terminated".into());
        self.i = self.s.len();
    }

    fn pi(&mut self) {
        self.i += 2;
        let Some(target) = self.xml_name() else {
            self.err(46, "PI is not started correctly".into());
            return;
        };
        if self.cur() == b'>' {
            self.i += 1;
            return self.add_misc(Node::new(PI, &target, ""));
        }
        self.skip_blanks();
        let st = self.i;
        while self.cur() != 0 && self.cur() != b'>' {
            self.i += 1;
        }
        if self.cur() != b'>' {
            self.err(47, format!("PI {target} never end ..."));
            return;
        }
        let data = crate::pstr::lossy(&self.s[st..self.i]).into_owned();
        self.i += 1;
        self.add_misc(Node::new(PI, &target, &data));
    }

    fn literal(&mut self) -> Option<String> {
        let q = self.cur();
        if q != b'"' && q != b'\'' {
            return None;
        }
        self.i += 1;
        let st = self.i;
        while self.cur() != 0 && self.cur() != q {
            self.i += 1;
        }
        let v = crate::pstr::lossy(&self.s[st..self.i]).into_owned();
        if self.cur() == q {
            self.i += 1;
        }
        Some(v)
    }

    fn doctype_decl(&mut self) {
        self.i += 9;
        self.skip_blanks();
        let name = self.xml_name().unwrap_or_default();
        self.skip_blanks();
        let (mut public, mut system) = (None, None);
        let upper: Vec<u8> = self.s[self.i..(self.i + 6).min(self.s.len())].to_ascii_uppercase();
        if upper == b"SYSTEM" {
            self.i += 6;
            self.skip_blanks();
            system = self.literal();
        } else if upper == b"PUBLIC" {
            self.i += 6;
            self.skip_blanks();
            public = self.literal();
            self.skip_blanks();
            if matches!(self.cur(), b'"' | b'\'') {
                system = self.literal();
            }
        }
        self.skip_blanks();
        while self.cur() != 0 && self.cur() != b'>' {
            self.i += 1;
        }
        if self.cur() == b'>' {
            self.i += 1;
        }
        self.doctype(&name, public, system);
    }

    fn is_doctype(&self) -> bool {
        self.s.get(self.i + 2..self.i + 9).is_some_and(|w| w.eq_ignore_ascii_case(b"DOCTYPE"))
    }

    fn content(&mut self) {
        loop {
            let c = self.cur();
            if c == b'<' && self.nxt(1) == b'/' {
                self.end_tag();
                continue;
            }
            if c != 0 && matches!(self.name(), Some("script") | Some("style")) {
                self.script();
                continue;
            }
            if c == b'<' && self.nxt(1) == b'!' {
                if self.is_doctype() {
                    self.err(800, "Misplaced DOCTYPE declaration".into());
                    self.doctype_decl();
                } else if self.nxt(2) == b'-' && self.nxt(3) == b'-' {
                    self.comment();
                } else {
                    while self.cur() != 0 && self.cur() != b'>' {
                        self.i += 1;
                    }
                    if self.cur() == b'>' {
                        self.i += 1;
                    }
                }
            } else if c == b'<' && self.nxt(1) == b'?' {
                self.pi();
            } else if c == b'<' && self.nxt(1).is_ascii_alphabetic() {
                self.element();
            } else if c == b'<' {
                self.characters("<");
                self.i += 1;
            } else if c == b'&' {
                self.reference();
            } else if c == 0 {
                self.stack.clear();
                break;
            } else {
                self.char_data();
            }
        }
    }

    fn document(&mut self) {
        self.skip_blanks();
        if self.cur() == 0 {
            self.err(4, "Document is empty".into());
        }
        let misc = |p: &mut Self| {
            while p.cur() == b'<' && ((p.nxt(1) == b'!' && p.nxt(2) == b'-' && p.nxt(3) == b'-') || p.nxt(1) == b'?') {
                if p.nxt(1) == b'!' {
                    p.comment();
                } else {
                    p.pi();
                }
                p.skip_blanks();
            }
        };
        misc(self);
        if self.cur() == b'<' && self.nxt(1) == b'!' && self.is_doctype() {
            self.doctype_decl();
        }
        self.skip_blanks();
        misc(self);
        self.content();
        if self.opts & HTML_PARSE_NODEFDTD == 0 && self.doc.doctype().is_none() {
            self.doctype("html", Some("-//W3C//DTD HTML 4.0 Transitional//EN".into()), Some("http://www.w3.org/TR/REC-html40/loose.dtd".into()));
        }
    }
}

fn utf8_len(b: u8) -> usize {
    match b {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

pub fn parse(input: &[u8], opts: i64) -> (Doc, Vec<ParseError>) {
    let (text, encoding) = decode(input);
    // libxml2 stops at a NUL byte.
    let s = text.as_bytes();
    let s = &s[..memchr::memchr(0, s).unwrap_or(s.len())];
    let mut line_starts = vec![0];
    line_starts.extend(memchr::memchr_iter(b'\n', s).map(|i| i + 1));
    let mut doc = Doc::new(true);
    doc.encoding = encoding;
    doc.standalone = 1;
    let mut p = P { s, i: 0, doc, stack: vec![], level: 1, depth: 0, opts, errors: vec![], line_starts };
    p.document();
    (p.doc, p.errors)
}
