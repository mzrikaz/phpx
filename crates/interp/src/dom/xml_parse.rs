//! XML parsing for DOMDocument::loadXML(): well-formedness checks with
//! libxml2's messages, namespaces, internal DTD entities, CDATA, and
//! libxml2's ignorable-whitespace rule for LIBXML_NOBLANKS.

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

struct P<'a> {
    s: &'a str,
    i: usize,
    doc: Doc,
    stack: Vec<Id>,
    opts: i64,
    errors: Vec<ParseError>,
    entities: HashMap<String, String>,
    line_starts: Vec<usize>,
}

type Res<T> = Result<T, Fatal>;

impl P<'_> {
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
        self.errors.push(ParseError { level: 3, code, message: msg, line });
    }

    fn ns_error(&mut self, code: i64, msg: String) {
        let line = self.line();
        self.errors.push(ParseError { level: 2, code, message: msg, line });
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

    fn node(&self) -> Id {
        self.stack.last().copied().unwrap_or(0)
    }

    fn add(&mut self, n: Node) -> Id {
        let id = self.doc.add(n);
        let parent = self.node();
        self.doc.append(parent, id);
        id
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
                self.add(Node::new(COMMENT, "#comment", &text));
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
                self.add(Node::new(PI, &target, &data));
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
                let parent = self.node();
                if self.opts & XML_PARSE_NOCDATA != 0 {
                    self.doc.append_text(parent, TEXT, &text);
                } else {
                    self.add(Node::new(CDATA, "#cdata-section", &text));
                }
                Ok(())
            }
            None => self.fatal(63, "CData section not finished".into()),
        }
    }

    /// libxml2's areBlanks() heuristic for NOBLANKS.
    fn are_blanks(&self, text: &str) -> bool {
        if self.opts & XML_PARSE_NOBLANKS == 0 || !text.bytes().all(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r')) {
            return false;
        }
        let Some(&node) = self.stack.last() else { return false };
        if self.cur() != b'<' && self.cur() != b'\r' {
            return false;
        }
        let n = self.doc.n(node);
        if n.first.is_none() && self.cur() == b'<' && self.at(1) == b'/' {
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

    fn text(&mut self) -> Res<()> {
        let st = self.i;
        let mut buf = String::new();
        loop {
            match self.cur() {
                0 | b'<' => break,
                b'&' => {
                    if !buf.is_empty() || self.i > st {
                        let parent = self.node();
                        self.doc.append_text(parent, TEXT, &buf);
                        buf.clear();
                    }
                    if self.at(1) == b'#' {
                        let c = self.char_ref()?;
                        let parent = self.node();
                        self.doc.append_text(parent, TEXT, c.encode_utf8(&mut [0; 4]));
                    } else {
                        let (v, ent) = self.entity_ref()?;
                        let parent = self.node();
                        match ent {
                            None => self.doc.append_text(parent, TEXT, &v),
                            Some(text) if self.opts & XML_PARSE_NOENT != 0 => self.doc.append_text(parent, TEXT, &text),
                            Some(_) => {
                                self.add(Node::new(ENTITY_REF, &v, ""));
                            }
                        }
                    }
                    return Ok(());
                }
                b']' if self.starts("]]>") => return self.fatal(62, "Sequence ']]>' not allowed in content".into()),
                _ => {
                    let ch = self.s[self.i..].chars().next().unwrap();
                    buf.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
        if self.stack.is_empty() {
            if buf.trim_matches([' ', '\t', '\n', '\r']).is_empty() {
                return Ok(());
            }
            return self.fatal(5, "Extra content at the end of the document".into());
        }
        if !self.are_blanks(&buf) {
            let parent = self.node();
            self.doc.append_text(parent, TEXT, &buf);
        }
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
        let mut n = Node::new(ELEMENT, &name, "");
        n.line = line;
        let el = self.doc.add(n);
        let parent = self.node();
        self.doc.append(parent, el);
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
                self.ns_error(201, format!("Namespace prefix {p} on {} is not defined", &name[p.len() + 1..]));
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
                    None => self.ns_error(201, format!("Namespace prefix {p} for {l} on {name} is not defined")),
                }
            }
        }
        if self.cur() == b'/' {
            if self.at(1) != b'>' {
                return self.fatal(73, format!("Couldn't find end of Start Tag {name} line {line}"));
            }
            self.i += 2;
            return Ok(());
        }
        self.i += 1;
        self.stack.push(el);
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
        let Some(&open) = self.stack.last() else {
            return self.fatal(5, "Extra content at the end of the document".into());
        };
        if self.doc.n(open).name != name {
            let (on, ol) = (self.doc.n(open).name.clone(), self.doc.n(open).line);
            return self.fatal(76, format!("Opening and ending tag mismatch: {on} line {ol} and {name}"));
        }
        self.stack.pop();
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
        self.doc.version = version;
        self.doc.encoding = encoding;
        self.doc.standalone = match standalone.as_deref() {
            Some("yes") => 1,
            Some("no") => 0,
            _ => -1,
        };
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
        let mut n = Node::new(DOCTYPE, &name, "");
        n.ids = Some(Box::new((public, system, subset.trim_start_matches(['\n', '\r']).to_string())));
        self.add(n);
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
            if self.stack.is_empty() {
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
                    if self.stack.is_empty() {
                        return self.fatal(5, "Extra content at the end of the document".into());
                    }
                    self.cdata()?;
                } else if self.starts("<!DOCTYPE") {
                    if seen_root || !self.stack.is_empty() {
                        return self.fatal(5, "Extra content at the end of the document".into());
                    }
                    self.doctype()?;
                } else if self.at(1) == b'/' {
                    self.end_tag()?;
                } else {
                    if self.stack.is_empty() && seen_root {
                        return self.fatal(5, "Extra content at the end of the document".into());
                    }
                    seen_root = true;
                    self.start_tag()?;
                }
            } else if self.stack.is_empty() {
                return self.fatal(if seen_root { 5 } else { 4 }, if seen_root { "Extra content at the end of the document".into() } else { "Start tag expected, '<' not found".into() });
            } else {
                self.text()?;
            }
        }
        if let Some(&open) = self.stack.last() {
            let (n, l) = (self.doc.n(open).name.clone(), self.doc.n(open).line);
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
        return String::from_utf8_lossy(rest).into_owned();
    }
    if input.starts_with(b"\xFF\xFE") || input.starts_with(b"\xFE\xFF") {
        let le = input[0] == 0xFF;
        let units: Vec<u16> = input[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        return String::from_utf16_lossy(&units);
    }
    let head = &input[..input.len().min(200)];
    if head.starts_with(b"<?xml") {
        let decl_end = memchr::memmem::find(head, b"?>").unwrap_or(head.len());
        let decl = String::from_utf8_lossy(&head[..decl_end]).to_ascii_lowercase();
        if let Some(p) = decl.find("encoding") {
            let v = decl[p + 8..].trim_start_matches([' ', '=', '"', '\'']);
            let enc: String = v.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
            if matches!(enc.as_str(), "iso-8859-1" | "latin1" | "iso-latin-1" | "windows-1252" | "cp1252") {
                return input.iter().map(|&b| b as char).collect();
            }
        }
    }
    String::from_utf8_lossy(input).into_owned()
}

pub fn parse(input: &[u8], opts: i64) -> (Option<Doc>, Vec<ParseError>) {
    let text = decode(input);
    // Line ends normalize to \n.
    let text = if text.contains('\r') { text.replace("\r\n", "\n").replace('\r', "\n") } else { text };
    let mut line_starts = vec![0];
    line_starts.extend(memchr::memchr_iter(b'\n', text.as_bytes()).map(|i| i + 1));
    let mut p = P { s: &text, i: 0, doc: Doc::new(false), stack: vec![], opts, errors: vec![], entities: HashMap::new(), line_starts };
    let ok = p.document().is_ok();
    if ok || opts & XML_PARSE_RECOVER != 0 {
        (Some(p.doc), p.errors)
    } else {
        (None, p.errors)
    }
}
