//! XML parsing: an event parser with libxml2's well-formedness checks and
//! messages, feeding a `Sink`: the DOM tree builder (DOMDocument::
//! loadXML(), with namespaces, internal DTD entities, CDATA and libxml2's
//! ignorable-whitespace rule for LIBXML_NOBLANKS) or ext/xml's SAX events
//! (character data chunked where libxml2 chunks it).

use std::collections::HashMap;

use super::html_parse::ParseError;
use super::tree::*;

pub const XML_PARSE_RECOVER: i64 = 1;
pub const XML_PARSE_NOENT: i64 = 2;
pub const XML_PARSE_NOBLANKS: i64 = 256;
pub const XML_PARSE_NOCDATA: i64 = 16384;

fn name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':' || (c as u32) >= 0xC0
}

fn name_char(c: char) -> bool {
    name_start(c) || c.is_ascii_digit() || c == '.' || c == '-' || c == '\u{B7}'
}

fn is_char(c: u32) -> bool {
    matches!(c, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

struct Fatal;

#[derive(Clone, Copy, PartialEq)]
pub enum TextKind {
    /// Character data as written.
    Plain,
    /// The expansion of a character or predefined entity reference.
    Ref,
    /// A CDATA section.
    Cdata,
}

/// Receives parse events.
pub trait Sink {
    fn xml_decl(&mut self, _version: Option<String>, _encoding: Option<String>, _standalone: i8) {}
    fn doctype(&mut self, _name: &str, _public: &str, _system: &str, _subset: &str) {}
    /// A start tag; returns namespace errors (not fatal).
    fn start(&mut self, name: &str, attrs: Vec<(String, String)>, line: u32) -> Vec<(i64, String)>;
    fn end(&mut self, name: &str);
    /// Character data; `next` is the input after it (for NOBLANKS).
    fn text(&mut self, s: &str, kind: TextKind, next: (u8, u8));
    /// A reference to an entity declared in the DTD.
    fn entity(&mut self, name: &str, value: &str);
    fn comment(&mut self, s: &str);
    fn pi(&mut self, target: &str, data: &str);
    fn line(&mut self, _line: u32) {}
}

struct P<'a, S: Sink> {
    s: &'a str,
    i: usize,
    sink: &'a mut S,
    /// Open elements: (name, line).
    open: Vec<(String, u32)>,
    errors: Vec<ParseError>,
    entities: HashMap<String, String>,
    line_starts: Vec<usize>,
}

type Res<T> = Result<T, Fatal>;

impl<S: Sink> P<'_, S> {
    fn cur(&self) -> u8 {
        self.s.as_bytes().get(self.i).copied().unwrap_or(0)
    }

    fn at(&self, n: usize) -> u8 {
        self.s.as_bytes().get(self.i + n).copied().unwrap_or(0)
    }

    fn starts(&self, p: &str) -> bool {
        self.s[self.i..].starts_with(p)
    }

    fn line(&self) -> u32 {
        self.line_starts.partition_point(|&s| s <= self.i) as u32
    }

    fn fatal<T>(&mut self, code: i64, msg: String) -> Res<T> {
        self.error(code, msg);
        Err(Fatal)
    }

    fn error(&mut self, code: i64, msg: String) {
        let line = self.line();
        let offset = self.i;
        self.errors.push(ParseError { level: 3, code, message: msg, line, offset });
    }

    fn ns_error(&mut self, code: i64, msg: String) {
        let line = self.line();
        let offset = self.i;
        self.errors.push(ParseError { level: 2, code, message: msg, line, offset });
    }

    fn skip_ws(&mut self) -> bool {
        let st = self.i;
        while matches!(self.cur(), b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
        self.i > st
    }

    fn name(&mut self) -> Option<String> {
        let rest = &self.s[self.i..];
        let mut it = rest.char_indices();
        match it.next() {
            Some((_, c)) if name_start(c) => {}
            _ => return None,
        }
        let end = it.find(|&(_, c)| !name_char(c)).map_or(rest.len(), |(i, _)| i);
        self.i += end;
        Some(rest[..end].to_string())
    }

    fn sync_line(&mut self) {
        let l = self.line();
        self.sink.line(l);
    }

    fn char_ref(&mut self) -> Res<char> {
        let hex = self.at(2) == b'x';
        self.i += if hex { 3 } else { 2 };
        let st = self.i;
        while if hex { self.cur().is_ascii_hexdigit() } else { self.cur().is_ascii_digit() } {
            self.i += 1;
        }
        if self.cur() != b';' || self.i == st {
            return self.fatal(9, "CharRef: invalid decimal value".into());
        }
        let v = u32::from_str_radix(&self.s[st..self.i], if hex { 16 } else { 10 }).unwrap_or(0);
        self.i += 1;
        match char::from_u32(v).filter(|_| is_char(v)) {
            Some(c) => Ok(c),
            None => self.fatal(9, format!("xmlParseCharRef: invalid xmlChar value {v}")),
        }
    }

    /// `&name;` -> its replacement text, or None for an entity kept as a
    /// reference node.
    fn entity_ref(&mut self) -> Res<(String, Option<String>)> {
        self.i += 1;
        let Some(name) = self.name() else {
            return self.fatal(68, "xmlParseEntityRef: no name".into());
        };
        if self.cur() != b';' {
            return self.fatal(23, "EntityRef: expecting ';'".into());
        }
        self.i += 1;
        let v = match name.as_str() {
            "lt" => "<",
            "gt" => ">",
            "amp" => "&",
            "quot" => "\"",
            "apos" => "'",
            _ => {
                return match self.entities.get(&name) {
                    Some(v) => Ok((name.clone(), Some(v.clone()))),
                    None => self.fatal(26, format!("Entity '{name}' not defined")),
                };
            }
        };
        Ok((v.to_string(), None))
    }

    fn att_value(&mut self) -> Res<String> {
        let q = self.cur();
        if q != b'"' && q != b'\'' {
            return self.fatal(39, "AttValue: \" or ' expected".into());
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            match self.cur() {
                0 => return self.fatal(40, "AttValue: ' expected".into()),
                c if c == q => {
                    self.i += 1;
                    return Ok(out);
                }
                b'<' => return self.fatal(38, "Unescaped '<' not allowed in attributes values".into()),
                b'&' if self.at(1) == b'#' => out.push(self.char_ref()?),
                b'&' => {
                    let (v, ent) = self.entity_ref()?;
                    match ent {
                        Some(text) => out.push_str(&text.replace(['\t', '\n', '\r'], " ")),
                        None => out.push_str(&v),
                    }
                }
                b'\t' | b'\n' | b'\r' => {
                    out.push(' ');
                    self.i += 1;
                }
                _ => {
                    let ch = self.s[self.i..].chars().next().unwrap();
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }

    fn comment(&mut self) -> Res<()> {
        self.i += 4;
        match self.s[self.i..].find("--") {
            Some(p) if self.s[self.i + p..].starts_with("-->") => {
                let text = self.s[self.i..self.i + p].to_string();
                self.i += p + 3;
                self.sync_line();
                self.sink.comment(&text);
                Ok(())
            }
            Some(_) => self.fatal(80, "Double hyphen within comment".into()),
            None => self.fatal(45, "Comment not terminated".into()),
        }
    }

    fn pi(&mut self) -> Res<()> {
        self.i += 2;
        let Some(target) = self.name() else {
            return self.fatal(46, "xmlParsePI : no target name".into());
        };
        if target.eq_ignore_ascii_case("xml") {
            return self.fatal(64, "XML declaration allowed only at the start of the document".into());
        }
        let had_ws = self.skip_ws();
        match self.s[self.i..].find("?>") {
            Some(p) => {
                if !had_ws && p > 0 {
                    return self.fatal(65, format!("ParsePI: PI {target} space expected"));
                }
                let data = self.s[self.i..self.i + p].to_string();
                self.i += p + 2;
                self.sync_line();
                self.sink.pi(&target, &data);
                Ok(())
            }
            None => self.fatal(47, format!("ParsePI: PI {target} never end ...")),
        }
    }

    fn cdata(&mut self) -> Res<()> {
        self.i += 9;
        match self.s[self.i..].find("]]>") {
            Some(p) => {
                let text = self.s[self.i..self.i + p].to_string();
                self.i += p + 3;
                self.sync_line();
                let next = (self.cur(), self.at(1));
                self.sink.text(&text, TextKind::Cdata, next);
                Ok(())
            }
            None => self.fatal(63, "CData section not finished".into()),
        }
    }

    fn emit(&mut self, buf: &mut String, kind: TextKind) {
        if !buf.is_empty() {
            self.sync_line();
            let next = (self.cur(), self.at(1));
            self.sink.text(buf, kind, next);
            buf.clear();
        }
    }

    /// Character data up to the next markup. Chunks follow libxml2: an
    /// ASCII run is one chunk; from the first non-ASCII character on,
    /// chunks of at least 300 bytes; references are their own chunks.
    fn text(&mut self) -> Res<()> {
        let mut buf = String::new();
        let mut complex = false;
        loop {
            match self.cur() {
                0 | b'<' => break,
                b'&' => {
                    if self.open.is_empty() {
                        return self.fatal(5, "Extra content at the end of the document".into());
                    }
                    self.emit(&mut buf, TextKind::Plain);
                    if self.at(1) == b'#' {
                        let c = self.char_ref()?;
                        let mut s = c.to_string();
                        self.emit(&mut s, TextKind::Ref);
                    } else {
                        let (v, ent) = self.entity_ref()?;
                        match ent {
                            None => {
                                let mut s = v;
                                self.emit(&mut s, TextKind::Ref);
                            }
                            Some(text) => {
                                self.sync_line();
                                self.sink.entity(&v, &text);
                            }
                        }
                    }
                    return Ok(());
                }
                b']' if self.starts("]]>") => return self.fatal(62, "Sequence ']]>' not allowed in content".into()),
                c => {
                    if c >= 0x80 && !complex {
                        self.emit(&mut buf, TextKind::Plain);
                        complex = true;
                    }
                    let ch = self.s[self.i..].chars().next().unwrap();
                    buf.push(ch);
                    self.i += ch.len_utf8();
                    if complex && buf.len() >= 300 {
                        self.emit(&mut buf, TextKind::Plain);
                    }
                }
            }
        }
        if self.open.is_empty() {
            if buf.trim_matches([' ', '\t', '\n', '\r']).is_empty() {
                return Ok(());
            }
            return self.fatal(5, "Extra content at the end of the document".into());
        }
        self.emit(&mut buf, TextKind::Plain);
        Ok(())
    }

    fn start_tag(&mut self) -> Res<()> {
        self.i += 1;
        let Some(name) = self.name() else {
            return self.fatal(68, "StartTag: invalid element name".into());
        };
        let line = self.line();
        let mut attrs: Vec<(String, String)> = vec![];
        loop {
            let ws = self.skip_ws();
            match self.cur() {
                b'>' | b'/' => break,
                0 => return self.fatal(73, format!("Couldn't find end of Start Tag {name} line {line}")),
                _ => {}
            }
            if !ws {
                return self.fatal(65, "attributes construct error".into());
            }
            let Some(an) = self.name() else {
                return self.fatal(73, format!("Couldn't find end of Start Tag {name} line {line}"));
            };
            self.skip_ws();
            if self.cur() != b'=' {
                return self.fatal(41, format!("Specification mandates value for attribute {an}"));
            }
            self.i += 1;
            self.skip_ws();
            let v = self.att_value()?;
            if attrs.iter().any(|(k, _)| *k == an) {
                return self.fatal(42, format!("Attribute {an} redefined"));
            }
            attrs.push((an, v));
        }
        self.sync_line();
        for (code, msg) in self.sink.start(&name, attrs, line) {
            self.ns_error(code, msg);
        }
        if self.cur() == b'/' {
            if self.at(1) != b'>' {
                return self.fatal(73, format!("Couldn't find end of Start Tag {name} line {line}"));
            }
            self.i += 2;
            self.sink.end(&name);
            return Ok(());
        }
        self.i += 1;
        self.open.push((name, line));
        Ok(())
    }

    fn end_tag(&mut self) -> Res<()> {
        self.i += 2;
        let name = self.name().unwrap_or_default();
        self.skip_ws();
        if self.cur() != b'>' {
            return self.fatal(73, "expected '>'".into());
        }
        self.i += 1;
        let Some((on, ol)) = self.open.last().cloned() else {
            return self.fatal(5, "Extra content at the end of the document".into());
        };
        if on != name {
            return self.fatal(76, format!("Opening and ending tag mismatch: {on} line {ol} and {name}"));
        }
        self.open.pop();
        self.sync_line();
        self.sink.end(&name);
        Ok(())
    }

    fn xml_decl(&mut self) -> Res<()> {
        self.i += 5;
        let get = |p: &mut Self, key: &str| -> Res<Option<String>> {
            p.skip_ws();
            if !p.starts(key) {
                return Ok(None);
            }
            p.i += key.len();
            p.skip_ws();
            if p.cur() != b'=' {
                return p.fatal(75, "Blank needed here".into());
            }
            p.i += 1;
            p.skip_ws();
            let q = p.cur();
            if q != b'"' && q != b'\'' {
                return p.fatal(33, "String not started expecting ' or \"".into());
            }
            let st = p.i + 1;
            let Some(e) = p.s[st..].find(q as char) else { return p.fatal(34, "String not closed expecting \" or '".into()) };
            p.i = st + e + 1;
            Ok(Some(p.s[st..st + e].to_string()))
        };
        let version = get(self, "version")?;
        if version.is_none() {
            return self.fatal(96, "Malformed declaration expecting version".into());
        }
        let encoding = get(self, "encoding")?;
        let standalone = get(self, "standalone")?;
        self.skip_ws();
        if !self.starts("?>") {
            return self.fatal(57, "parsing XML declaration: '?>' expected".into());
        }
        self.i += 2;
        let standalone = match standalone.as_deref() {
            Some("yes") => 1,
            Some("no") => 0,
            _ => -1,
        };
        self.sink.xml_decl(version, encoding, standalone);
        Ok(())
    }

    fn quoted_literal(&mut self) -> Option<String> {
        let q = self.cur();
        if q != b'"' && q != b'\'' {
            return None;
        }
        let st = self.i + 1;
        let e = self.s[st..].find(q as char)?;
        self.i = st + e + 1;
        Some(self.s[st..st + e].to_string())
    }

    fn doctype(&mut self) -> Res<()> {
        self.i += 9;
        self.skip_ws();
        let Some(name) = self.name() else {
            return self.fatal(68, "xmlParseDocTypeDecl : no DOCTYPE name !".into());
        };
        self.skip_ws();
        let (mut public, mut system) = (String::new(), String::new());
        if self.starts("PUBLIC") {
            self.i += 6;
            self.skip_ws();
            public = self.quoted_literal().unwrap_or_default();
            self.skip_ws();
            system = self.quoted_literal().unwrap_or_default();
        } else if self.starts("SYSTEM") {
            self.i += 6;
            self.skip_ws();
            system = self.quoted_literal().unwrap_or_default();
        }
        self.skip_ws();
        let mut subset = String::new();
        if self.cur() == b'[' {
            let st = self.i + 1;
            // The internal subset ends at the first `]` outside quotes,
            // comments and declarations' literals.
            let b = self.s.as_bytes();
            let mut j = st;
            let mut quote = 0u8;
            while j < b.len() {
                let c = b[j];
                if quote != 0 {
                    if c == quote {
                        quote = 0;
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = c;
                } else if self.s[j..].starts_with("<!--") {
                    j = self.s[j..].find("-->").map_or(b.len(), |e| j + e + 2);
                } else if c == b']' {
                    break;
                }
                j += 1;
            }
            subset = self.s[st..j.min(b.len())].to_string();
            self.i = (j + 1).min(b.len());
            self.parse_entities(&subset);
            self.skip_ws();
        }
        if self.cur() != b'>' {
            return self.fatal(61, "DOCTYPE improperly terminated".into());
        }
        self.i += 1;
        self.sink.doctype(&name, &public, &system, subset.trim_start_matches(['\n', '\r']));
        Ok(())
    }

    /// `<!ENTITY name "value">` declarations of the internal subset.
    fn parse_entities(&mut self, subset: &str) {
        let mut rest = subset;
        while let Some(p) = rest.find("<!ENTITY") {
            rest = &rest[p + 8..];
            let t = rest.trim_start();
            if t.starts_with('%') {
                continue;
            }
            let end = t.find(|c: char| c.is_whitespace()).unwrap_or(t.len());
            let name = &t[..end];
            let after = t[end..].trim_start();
            if let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') {
                if let Some(e) = after[1..].find(q) {
                    self.entities.entry(name.to_string()).or_insert_with(|| after[1..1 + e].to_string());
                }
            }
        }
    }

    fn document(&mut self) -> Res<()> {
        if self.starts("<?xml") && matches!(self.at(5), b' ' | b'\t' | b'\n' | b'\r') {
            self.xml_decl()?;
        }
        let mut seen_root = false;
        loop {
            if self.open.is_empty() {
                self.skip_ws();
            }
            let c = self.cur();
            if c == 0 {
                break;
            }
            if c == b'<' {
                if self.starts("<!--") {
                    self.comment()?;
                } else if self.starts("<?") {
                    self.pi()?;
                } else if self.starts("<![CDATA[") {
                    if self.open.is_empty() {
                        return self.fatal(5, "Extra content at the end of the document".into());
                    }
                    self.cdata()?;
                } else if self.starts("<!DOCTYPE") {
                    if seen_root || !self.open.is_empty() {
                        return self.fatal(5, "Extra content at the end of the document".into());
                    }
                    self.doctype()?;
                } else if self.at(1) == b'/' {
                    self.end_tag()?;
                } else {
                    if self.open.is_empty() && seen_root {
                        return self.fatal(5, "Extra content at the end of the document".into());
                    }
                    seen_root = true;
                    self.start_tag()?;
                }
            } else if self.open.is_empty() {
                return self.fatal(if seen_root { 5 } else { 4 }, if seen_root { "Extra content at the end of the document".into() } else { "Start tag expected, '<' not found".into() });
            } else {
                self.text()?;
            }
        }
        if let Some((n, l)) = self.open.last().cloned() {
            return self.fatal(77, format!("Premature end of data in tag {n} line {l}"));
        }
        if !seen_root {
            return self.fatal(4, "Start tag expected, '<' not found".into());
        }
        Ok(())
    }
}

/// The input as UTF-8 per its BOM or declared encoding.
fn decode(input: &[u8]) -> String {
    if let Some(rest) = input.strip_prefix(b"\xEF\xBB\xBF") {
        return crate::pstr::lossy(rest).into_owned();
    }
    if input.starts_with(b"\xFF\xFE") || input.starts_with(b"\xFE\xFF") {
        let le = input[0] == 0xFF;
        let units: Vec<u16> = input[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        return String::from_utf16_lossy(&units);
    }
    let head = &input[..input.len().min(200)];
    if head.starts_with(b"<?xml") {
        let decl_end = memchr::memmem::find(head, b"?>").unwrap_or(head.len());
        let decl = crate::pstr::lossy(&head[..decl_end]).to_ascii_lowercase();
        if let Some(p) = decl.find("encoding") {
            let v = decl[p + 8..].trim_start_matches([' ', '=', '"', '\'']);
            let enc: String = v.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
            if matches!(enc.as_str(), "iso-8859-1" | "latin1" | "iso-latin-1" | "windows-1252" | "cp1252") {
                return input.iter().map(|&b| b as char).collect();
            }
        }
    }
    crate::pstr::lossy(input).into_owned()
}

/// Parse `input` into `sink`. Returns whether the document was
/// well-formed (parsing stops at the first fatal error), the errors, and
/// the length of the decoded input (error offsets index into it).
pub fn run<S: Sink>(input: &[u8], sink: &mut S) -> (bool, Vec<ParseError>, usize) {
    let text = decode(input);
    // Line ends normalize to \n.
    let text = if text.contains('\r') { text.replace("\r\n", "\n").replace('\r', "\n") } else { text };
    let mut line_starts = vec![0];
    line_starts.extend(memchr::memchr_iter(b'\n', text.as_bytes()).map(|i| i + 1));
    let mut p = P { s: &text, i: 0, sink, open: vec![], errors: vec![], entities: HashMap::new(), line_starts };
    let ok = p.document().is_ok();
    (ok, p.errors, text.len())
}

/// Builds the DOM tree.
pub struct TreeSink {
    pub doc: Doc,
    stack: Vec<Id>,
    opts: i64,
}

impl TreeSink {
    pub fn new(opts: i64) -> TreeSink {
        TreeSink { doc: Doc::new(false), stack: vec![], opts }
    }

    fn node(&self) -> Id {
        self.stack.last().copied().unwrap_or(0)
    }

    fn add(&mut self, n: Node) -> Id {
        let id = self.doc.add(n);
        let parent = self.node();
        self.doc.append(parent, id);
        id
    }

    /// libxml2's areBlanks() heuristic for NOBLANKS.
    fn are_blanks(&self, text: &str, next: (u8, u8)) -> bool {
        if self.opts & XML_PARSE_NOBLANKS == 0 || !text.bytes().all(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r')) {
            return false;
        }
        let Some(&node) = self.stack.last() else { return false };
        if next.0 != b'<' && next.0 != b'\r' {
            return false;
        }
        let n = self.doc.n(node);
        if n.first.is_none() && next.0 == b'<' && next.1 == b'/' {
            return false;
        }
        if let Some(l) = n.last {
            if self.doc.n(l).is_text() {
                return false;
            }
        }
        if let Some(f) = n.first {
            if self.doc.n(f).is_text() {
                return false;
            }
        }
        // xml:space="preserve" in scope keeps it.
        for &e in self.stack.iter().rev() {
            if let Some(v) = self.doc.get_attr(e, "xml:space") {
                return v != "preserve";
            }
        }
        true
    }
}

impl Sink for TreeSink {
    fn xml_decl(&mut self, version: Option<String>, encoding: Option<String>, standalone: i8) {
        self.doc.version = version;
        self.doc.encoding = encoding;
        self.doc.standalone = standalone;
    }

    fn doctype(&mut self, name: &str, public: &str, system: &str, subset: &str) {
        let mut n = Node::new(DOCTYPE, name, "");
        n.ids = Some(Box::new((public.to_string(), system.to_string(), subset.to_string())));
        self.add(n);
    }

    fn start(&mut self, name: &str, attrs: Vec<(String, String)>, line: u32) -> Vec<(i64, String)> {
        let mut errors = vec![];
        let mut n = Node::new(ELEMENT, name, "");
        n.line = line;
        let el = self.add(n);
        // Namespace declarations first, then names resolve against them.
        for (k, v) in &attrs {
            if k == "xmlns" {
                self.doc.m(el).ns_defs.push((None, v.clone()));
            } else if let Some(p) = k.strip_prefix("xmlns:") {
                self.doc.m(el).ns_defs.push((Some(p.to_string()), v.clone()));
            }
        }
        let prefix = name.split_once(':').map(|(p, _)| p.to_string());
        let ns = self.doc.lookup_ns(el, prefix.as_deref());
        if ns.is_none() {
            if let Some(p) = &prefix {
                errors.push((201, format!("Namespace prefix {p} on {} is not defined", &name[p.len() + 1..])));
            }
        }
        self.doc.m(el).ns = ns;
        for (k, v) in attrs {
            if k == "xmlns" || k.starts_with("xmlns:") {
                continue;
            }
            let a = self.doc.set_attr(el, &k, &v);
            if let Some((p, l)) = k.split_once(':') {
                match self.doc.lookup_ns(el, Some(p)) {
                    Some(uri) => self.doc.m(a).ns = Some(uri),
                    None => errors.push((201, format!("Namespace prefix {p} for {l} on {name} is not defined"))),
                }
            }
        }
        self.stack.push(el);
        errors
    }

    fn end(&mut self, _name: &str) {
        self.stack.pop();
    }

    fn text(&mut self, s: &str, kind: TextKind, next: (u8, u8)) {
        let parent = self.node();
        match kind {
            TextKind::Cdata if self.opts & XML_PARSE_NOCDATA == 0 => {
                self.add(Node::new(CDATA, "#cdata-section", s));
            }
            TextKind::Plain if self.are_blanks(s, next) => {}
            _ => self.doc.append_text(parent, TEXT, s),
        }
    }

    fn entity(&mut self, name: &str, value: &str) {
        if self.opts & XML_PARSE_NOENT != 0 {
            let parent = self.node();
            self.doc.append_text(parent, TEXT, value);
        } else {
            self.add(Node::new(ENTITY_REF, name, ""));
        }
    }

    fn comment(&mut self, s: &str) {
        self.add(Node::new(COMMENT, "#comment", s));
    }

    fn pi(&mut self, target: &str, data: &str) {
        self.add(Node::new(PI, target, data));
    }
}

pub fn parse(input: &[u8], opts: i64) -> (Option<Doc>, Vec<ParseError>) {
    let mut sink = TreeSink::new(opts);
    let (ok, errors, _) = run(input, &mut sink);
    if ok || opts & XML_PARSE_RECOVER != 0 {
        (Some(sink.doc), errors)
    } else {
        (None, errors)
    }
}
