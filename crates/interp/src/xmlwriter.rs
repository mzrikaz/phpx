//! ext/xmlwriter: XMLWriter and the xmlwriter_* functions, a port of
//! libxml2's xmlwriter.c state machine (its indentation, escaping and
//! empty-element rules decide the output).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::{has, s};
use crate::class::Class;
use crate::interp::{BuiltinFn, Interp, NativeMethod, R};
use crate::value::{Internal, NativeData, Obj, Value};

#[derive(Clone, Copy, PartialEq, Debug)]
enum St {
    Name,
    Attribute,
    Text,
    Pi,
    PiText,
    Cdata,
    Dtd,
    DtdText,
    Comment,
}

struct Entry {
    name: String,
    state: St,
}

enum Sink {
    Memory,
    File(std::fs::File),
    /// php://output: written out on flush() and endDocument().
    Output,
}

struct Writer {
    out: Vec<u8>,
    sink: Option<Sink>,
    nodes: Vec<Entry>,
    /// Namespace declarations waiting for their start tag: (xmlns[:p], uri, depth).
    ns: Vec<(String, String, usize)>,
    indent: bool,
    doindent: bool,
    ichar: String,
    /// Output encoding named by startDocument().
    encoding: Option<String>,
}

impl Writer {
    fn new(sink: Sink) -> Writer {
        Writer { out: vec![], sink: Some(sink), nodes: vec![], ns: vec![], indent: false, doindent: true, ichar: " ".into(), encoding: None }
    }

    fn w(&mut self, s: &str) {
        match self.encoding.as_deref().map(|e| e.to_ascii_uppercase()) {
            Some(e) if e == "ISO-8859-1" || e == "LATIN1" => {
                for ch in s.chars() {
                    if (ch as u32) < 0x100 {
                        self.out.push(ch as u8);
                    } else {
                        self.out.extend_from_slice(format!("&#{};", ch as u32).as_bytes());
                    }
                }
            }
            _ => self.out.extend_from_slice(s.as_bytes()),
        }
    }

    fn top(&self) -> Option<St> {
        self.nodes.last().map(|e| e.state)
    }

    fn set_top(&mut self, st: St) {
        if let Some(e) = self.nodes.last_mut() {
            e.state = st;
        }
    }

    fn write_indent(&mut self) {
        let n = self.nodes.len();
        if n < 1 {
            return;
        }
        let ichar = self.ichar.clone();
        for _ in 0..n - 1 {
            self.w(&ichar);
        }
    }

    fn output_ns(&mut self) {
        let depth = self.nodes.len();
        let mine: Vec<(String, String)> = self.ns.iter().filter(|n| n.2 == depth).map(|n| (n.0.clone(), n.1.clone())).collect();
        self.ns.retain(|n| n.2 != depth);
        for (p, u) in mine {
            self.attribute(&p, &u);
        }
    }

    /// Close an open start tag (">") before content.
    fn close_start(&mut self, newline: bool) {
        if self.top() == Some(St::Attribute) {
            self.end_attribute();
        }
        if self.top() == Some(St::Name) {
            self.output_ns();
            self.w(">");
            if newline && self.indent {
                self.w("\n");
            }
            self.set_top(St::Text);
        }
    }

    fn start_document(&mut self, version: Option<&str>, encoding: Option<&str>, standalone: Option<&str>) -> bool {
        if !self.nodes.is_empty() {
            return false;
        }
        let enc = encoding.filter(|e| !e.is_empty()).map(|e| match e.to_ascii_uppercase().as_str() {
            "UTF-8" | "UTF8" => "UTF-8".to_string(),
            "ISO-8859-1" | "LATIN1" => "ISO-8859-1".to_string(),
            "US-ASCII" | "ASCII" => "US-ASCII".to_string(),
            u => u.to_string(),
        });
        let mut s = format!("<?xml version=\"{}\"", version.filter(|v| !v.is_empty()).unwrap_or("1.0"));
        if let Some(e) = &enc {
            s.push_str(&format!(" encoding=\"{e}\""));
        }
        if let Some(st) = standalone.filter(|v| !v.is_empty()) {
            s.push_str(&format!(" standalone=\"{st}\""));
        }
        s.push_str("?>\n");
        self.w(&s);
        self.encoding = enc;
        true
    }

    fn end_document(&mut self) -> bool {
        while let Some(st) = self.top() {
            match st {
                St::Name | St::Attribute | St::Text => {
                    self.end_element(false);
                }
                St::Pi | St::PiText => {
                    self.end_pi();
                }
                St::Cdata => {
                    self.end_cdata();
                }
                St::Dtd | St::DtdText => {
                    self.end_dtd();
                }
                St::Comment => {
                    self.end_comment();
                }
            }
        }
        if !self.indent {
            self.w("\n");
        }
        true
    }

    fn start_element(&mut self, name: &str) -> bool {
        match self.top() {
            Some(St::Attribute) | Some(St::Name) => self.close_start(true),
            _ => {}
        }
        self.nodes.push(Entry { name: name.to_string(), state: St::Name });
        if self.indent {
            self.write_indent();
        }
        self.w("<");
        self.w(name);
        true
    }

    fn start_element_ns(&mut self, prefix: Option<&str>, name: &str, uri: Option<&str>) -> bool {
        let qname = match prefix {
            Some(p) => format!("{p}:{name}"),
            None => name.to_string(),
        };
        self.start_element(&qname);
        if let Some(u) = uri {
            let decl = match prefix {
                Some(p) => format!("xmlns:{p}"),
                None => "xmlns".to_string(),
            };
            let depth = self.nodes.len();
            self.ns.push((decl, u.to_string(), depth));
        }
        true
    }

    fn end_element(&mut self, full: bool) -> bool {
        let Some(st) = self.top() else { return false };
        if st == St::Attribute {
            self.end_attribute();
        }
        let st = self.top().unwrap();
        match st {
            St::Name if !full => {
                self.output_ns();
                if self.indent {
                    self.doindent = true;
                }
                self.w("/>");
            }
            St::Name | St::Text => {
                if st == St::Name {
                    self.output_ns();
                    self.w(">");
                    if self.indent {
                        self.doindent = false;
                    }
                }
                if self.indent && self.doindent {
                    self.write_indent();
                }
                self.doindent = true;
                let name = self.nodes.last().unwrap().name.clone();
                self.w("</");
                self.w(&name);
                self.w(">");
            }
            _ => return false,
        }
        if self.indent {
            self.w("\n");
        }
        self.nodes.pop();
        true
    }

    fn start_attribute(&mut self, name: &str) -> bool {
        match self.top() {
            Some(St::Attribute) => {
                self.end_attribute();
            }
            Some(St::Name) => {}
            _ => return false,
        }
        self.w(" ");
        self.w(name);
        self.w("=\"");
        self.set_top(St::Attribute);
        true
    }

    fn end_attribute(&mut self) -> bool {
        if self.top() != Some(St::Attribute) {
            return false;
        }
        self.set_top(St::Name);
        self.w("\"");
        true
    }

    fn attribute(&mut self, name: &str, value: &str) -> bool {
        self.start_attribute(name) && self.string(value) && self.end_attribute()
    }

    fn start_attribute_ns(&mut self, prefix: Option<&str>, name: &str, uri: Option<&str>) -> bool {
        if let (Some(u), Some(p)) = (uri, prefix) {
            let decl = format!("xmlns:{p}");
            let depth = self.nodes.len();
            match self.ns.iter().find(|n| n.0 == decl && n.2 == depth) {
                Some(n) if n.1 != u => return false,
                Some(_) => {}
                None => self.ns.push((decl, u.to_string(), depth)),
            }
        } else if let (Some(u), None) = (uri, prefix) {
            let depth = self.nodes.len();
            if !self.ns.iter().any(|n| n.0 == "xmlns" && n.2 == depth) {
                self.ns.push(("xmlns".into(), u.to_string(), depth));
            }
        }
        let qname = match prefix {
            Some(p) => format!("{p}:{name}"),
            None => name.to_string(),
        };
        self.start_attribute(&qname)
    }

    /// Raw output after resolving the open construct (">" for a start
    /// tag, " " after a PI target, " [" in a DTD).
    fn raw(&mut self, content: &str) -> bool {
        match self.top() {
            Some(St::Name) => {
                self.output_ns();
                self.w(">");
                self.set_top(St::Text);
            }
            Some(St::Pi) => {
                self.w(" ");
                self.set_top(St::PiText);
            }
            Some(St::Dtd) => {
                self.w(" [");
                self.set_top(St::DtdText);
            }
            _ => {}
        }
        if self.indent {
            self.doindent = false;
        }
        self.w(content);
        true
    }

    fn string(&mut self, content: &str) -> bool {
        match self.top() {
            Some(St::Name) | Some(St::Text) => {
                let esc = escape_text(content);
                self.raw(&esc)
            }
            Some(St::Attribute) => {
                let esc = escape_attr(content);
                self.w(&esc);
                true
            }
            _ => self.raw(content),
        }
    }

    fn start_comment(&mut self) -> bool {
        match self.top() {
            None | Some(St::Text) => {}
            Some(St::Name) | Some(St::Attribute) => self.close_start(true),
            _ => return false,
        }
        self.nodes.push(Entry { name: String::new(), state: St::Comment });
        if self.indent {
            self.write_indent();
        }
        self.w("<!--");
        true
    }

    fn end_comment(&mut self) -> bool {
        if self.top() != Some(St::Comment) {
            return false;
        }
        self.w("-->");
        if self.indent {
            self.w("\n");
        }
        self.nodes.pop();
        true
    }

    fn start_pi(&mut self, target: &str) -> bool {
        if target.eq_ignore_ascii_case("xml") {
            return false;
        }
        match self.top() {
            Some(St::Attribute) | Some(St::Name) => self.close_start(false),
            None | Some(St::Text) | Some(St::Dtd) => {}
            _ => return false,
        }
        self.nodes.push(Entry { name: target.to_string(), state: St::Pi });
        self.w("<?");
        self.w(target);
        true
    }

    fn end_pi(&mut self) -> bool {
        if !matches!(self.top(), Some(St::Pi) | Some(St::PiText)) {
            return false;
        }
        self.w("?>");
        if self.indent {
            self.w("\n");
        }
        self.nodes.pop();
        true
    }

    fn start_cdata(&mut self) -> bool {
        match self.top() {
            None | Some(St::Text) | Some(St::Pi) | Some(St::PiText) => {}
            Some(St::Attribute) | Some(St::Name) => self.close_start(false),
            _ => return false,
        }
        self.nodes.push(Entry { name: String::new(), state: St::Cdata });
        self.w("<![CDATA[");
        true
    }

    fn end_cdata(&mut self) -> bool {
        if self.top() != Some(St::Cdata) {
            return false;
        }
        self.w("]]>");
        self.nodes.pop();
        true
    }

    fn start_dtd(&mut self, name: &str, public: Option<&str>, system: Option<&str>) -> bool {
        if !self.nodes.is_empty() {
            return false;
        }
        self.nodes.push(Entry { name: name.to_string(), state: St::Dtd });
        self.w("<!DOCTYPE ");
        self.w(name);
        if let Some(p) = public {
            self.w(&format!(" PUBLIC \"{p}\""));
            if let Some(sy) = system {
                if self.indent {
                    self.w("\n       ");
                } else {
                    self.w(" ");
                }
                self.w(&format!("\"{sy}\""));
            }
        } else if let Some(sy) = system {
            self.w(&format!(" SYSTEM \"{sy}\""));
        }
        true
    }

    fn end_dtd(&mut self) -> bool {
        match self.top() {
            Some(St::DtdText) => self.w("]"),
            Some(St::Dtd) => {}
            _ => return false,
        }
        self.w(">");
        if self.indent {
            self.w("\n");
        }
        self.nodes.pop();
        true
    }

    /// Write pending output to a file sink; php://output's comes back to
    /// be echoed.
    fn drain_to_sink(&mut self) -> Option<Vec<u8>> {
        match &mut self.sink {
            Some(Sink::File(file)) => {
                use std::io::Write;
                let data = std::mem::take(&mut self.out);
                let _ = file.write_all(&data);
                None
            }
            Some(Sink::Output) => Some(std::mem::take(&mut self.out)),
            _ => None,
        }
    }

    /// Take what has been written (outputMemory / flush).
    fn take(&mut self, empty: bool) -> Vec<u8> {
        if empty {
            std::mem::take(&mut self.out)
        } else {
            self.out.clone()
        }
    }
}

fn escape_text(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '&' => o.push_str("&amp;"),
            '"' => o.push_str("&quot;"),
            '\r' => o.push_str("&#13;"),
            _ => o.push(c),
        }
    }
    o
}

/// Attribute text (non-ASCII stays as is).
fn escape_attr(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => o.push_str("&#10;"),
            '\r' => o.push_str("&#13;"),
            '\t' => o.push_str("&#9;"),
            '"' => o.push_str("&quot;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '&' => o.push_str("&amp;"),
            c => o.push(c),
        }
    }
    o
}

struct WriterData(RefCell<Writer>);

impl NativeData for WriterData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        None
    }
}

fn valid_name(n: &str) -> bool {
    let mut cs = n.chars();
    matches!(cs.next(), Some(c) if c.is_alphabetic() || c == '_' || c == ':' || (c as u32) >= 0xC0) && cs.all(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '.' | '-' | '\u{B7}') || (c as u32) >= 0xC0)
}

fn text(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(String::from_utf8_lossy(&s(it, a, i)?).into_owned())
}

fn opt(it: &Interp, a: &[Value], i: usize) -> R<Option<String>> {
    if !has(a, i) || a[i].is_null() {
        return Ok(None);
    }
    text(it, a, i).map(Some)
}

/// Run `f` on the writer of `obj`.
fn with_writer<T>(it: &Interp, obj: &Rc<Obj>, func: &str, f: impl FnOnce(&mut Writer) -> T) -> R<T> {
    match &*obj.internal() {
        Internal::Data(d) => match d.as_any().downcast_ref::<WriterData>() {
            Some(w) => Ok(f(&mut w.0.borrow_mut())),
            None => Err(it.error("Error", &format!("{func}(): Invalid or uninitialized XMLWriter object"))),
        },
        _ => Err(it.error("Error", &format!("{func}(): Invalid or uninitialized XMLWriter object"))),
    }
}

fn name_arg(it: &Interp, a: &[Value], i: usize, func: &str, what: &str) -> R<String> {
    let n = text(it, a, i)?;
    if !valid_name(&n) {
        // PHP numbers the argument as in the procedural form.
        return Err(it.error("ValueError", &format!("{func}(): Argument #{} must be a valid {what} name, \"{n}\" given", i + 2)));
    }
    Ok(n)
}

type Op = fn(&Interp, &Rc<Obj>, &[Value], &str) -> R<Value>;

/// Every operation, shared by the methods and the functions (which take
/// the writer as their first argument).
const OPS: &[(&str, Op)] = &[
    ("openmemory", |_, o, _, _| {
        o.set_internal(Internal::Data(Box::new(WriterData(RefCell::new(Writer::new(Sink::Memory))))));
        Ok(Value::Bool(true))
    }),
    ("openuri", |it, o, a, f| {
        let path = text(it, a, 0)?;
        if path.is_empty() {
            return Err(it.error("ValueError", &format!("{f}(): Argument #1 ($uri) cannot be empty")));
        }
        let sink = if path == "php://output" || path == "php://stdout" {
            Sink::Output
        } else {
            match std::fs::File::create(path.strip_prefix("file://").unwrap_or(&path)) {
                Ok(file) => Sink::File(file),
                Err(_) => {
                    it.warn(&format!("{f}(): Unable to resolve file path"));
                    return Ok(Value::Bool(false));
                }
            }
        };
        o.set_internal(Internal::Data(Box::new(WriterData(RefCell::new(Writer::new(sink))))));
        Ok(Value::Bool(true))
    }),
    ("setindent", |it, o, a, f| {
        let on = a.first().is_some_and(|v| v.truthy());
        with_writer(it, o, f, |w| {
            w.indent = on;
            Value::Bool(true)
        })
    }),
    ("setindentstring", |it, o, a, f| {
        let s = text(it, a, 0)?;
        with_writer(it, o, f, |w| {
            w.ichar = s;
            Value::Bool(true)
        })
    }),
    ("startdocument", |it, o, a, f| {
        let (v, e, s) = (opt(it, a, 0)?, opt(it, a, 1)?, opt(it, a, 2)?);
        with_writer(it, o, f, |w| Value::Bool(w.start_document(v.as_deref(), e.as_deref(), s.as_deref())))
    }),
    ("enddocument", |it, o, _, f| {
        let (r, out) = with_writer(it, o, f, |w| {
            let r = w.end_document();
            (r, w.drain_to_sink())
        })?;
        if let Some(b) = out {
            it.echo(&b);
        }
        Ok(Value::Bool(r))
    }),
    ("startelement", |it, o, a, f| {
        let n = name_arg(it, a, 0, f, "element")?;
        with_writer(it, o, f, |w| Value::Bool(w.start_element(&n)))
    }),
    ("startelementns", |it, o, a, f| {
        let (p, n, u) = (opt(it, a, 0)?, name_arg(it, a, 1, f, "element")?, opt(it, a, 2)?);
        with_writer(it, o, f, |w| Value::Bool(w.start_element_ns(p.as_deref().filter(|p| !p.is_empty()), &n, u.as_deref())))
    }),
    ("endelement", |it, o, _, f| with_writer(it, o, f, |w| Value::Bool(w.end_element(false)))),
    ("fullendelement", |it, o, _, f| with_writer(it, o, f, |w| Value::Bool(w.end_element(true)))),
    ("writeelement", |it, o, a, f| {
        let (n, c) = (name_arg(it, a, 0, f, "element")?, opt(it, a, 1)?);
        with_writer(it, o, f, |w| {
            let ok = w.start_element(&n) && c.as_deref().is_none_or(|c| w.string(c)) && w.end_element(false);
            Value::Bool(ok)
        })
    }),
    ("writeelementns", |it, o, a, f| {
        let (p, n, u, c) = (opt(it, a, 0)?, name_arg(it, a, 1, f, "element")?, opt(it, a, 2)?, opt(it, a, 3)?);
        with_writer(it, o, f, |w| {
            let ok = w.start_element_ns(p.as_deref().filter(|p| !p.is_empty()), &n, u.as_deref()) && c.as_deref().is_none_or(|c| w.string(c)) && w.end_element(false);
            Value::Bool(ok)
        })
    }),
    ("startattribute", |it, o, a, f| {
        let n = name_arg(it, a, 0, f, "attribute")?;
        with_writer(it, o, f, |w| Value::Bool(w.start_attribute(&n)))
    }),
    ("startattributens", |it, o, a, f| {
        let (p, n, u) = (opt(it, a, 0)?, name_arg(it, a, 1, f, "attribute")?, opt(it, a, 2)?);
        with_writer(it, o, f, |w| Value::Bool(w.start_attribute_ns(p.as_deref().filter(|p| !p.is_empty()), &n, u.as_deref())))
    }),
    ("endattribute", |it, o, _, f| with_writer(it, o, f, |w| Value::Bool(w.end_attribute()))),
    ("writeattribute", |it, o, a, f| {
        let (n, v) = (name_arg(it, a, 0, f, "attribute")?, text(it, a, 1)?);
        with_writer(it, o, f, |w| Value::Bool(w.attribute(&n, &v)))
    }),
    ("writeattributens", |it, o, a, f| {
        let (p, n, u, v) = (opt(it, a, 0)?, name_arg(it, a, 1, f, "attribute")?, opt(it, a, 2)?, text(it, a, 3)?);
        with_writer(it, o, f, |w| Value::Bool(w.start_attribute_ns(p.as_deref().filter(|p| !p.is_empty()), &n, u.as_deref()) && w.string(&v) && w.end_attribute()))
    }),
    ("text", |it, o, a, f| {
        let t = text(it, a, 0)?;
        with_writer(it, o, f, |w| Value::Bool(w.string(&t)))
    }),
    ("writeraw", |it, o, a, f| {
        let t = text(it, a, 0)?;
        with_writer(it, o, f, |w| Value::Bool(w.raw(&t)))
    }),
    ("startcomment", |it, o, _, f| with_writer(it, o, f, |w| Value::Bool(w.start_comment()))),
    ("endcomment", |it, o, _, f| with_writer(it, o, f, |w| Value::Bool(w.end_comment()))),
    ("writecomment", |it, o, a, f| {
        let t = text(it, a, 0)?;
        with_writer(it, o, f, |w| Value::Bool(w.start_comment() && w.string(&t) && w.end_comment()))
    }),
    ("startpi", |it, o, a, f| {
        let t = name_arg(it, a, 0, f, "PI target")?;
        with_writer(it, o, f, |w| Value::Bool(w.start_pi(&t)))
    }),
    ("endpi", |it, o, _, f| with_writer(it, o, f, |w| Value::Bool(w.end_pi()))),
    ("writepi", |it, o, a, f| {
        let (t, c) = (name_arg(it, a, 0, f, "PI target")?, text(it, a, 1)?);
        with_writer(it, o, f, |w| Value::Bool(w.start_pi(&t) && w.string(&c) && w.end_pi()))
    }),
    ("startcdata", |it, o, _, f| with_writer(it, o, f, |w| Value::Bool(w.start_cdata()))),
    ("endcdata", |it, o, _, f| with_writer(it, o, f, |w| Value::Bool(w.end_cdata()))),
    ("writecdata", |it, o, a, f| {
        let t = text(it, a, 0)?;
        with_writer(it, o, f, |w| Value::Bool(w.start_cdata() && w.raw(&t) && w.end_cdata()))
    }),
    ("startdtd", |it, o, a, f| {
        let (n, p, s) = (name_arg(it, a, 0, f, "element")?, opt(it, a, 1)?, opt(it, a, 2)?);
        with_writer(it, o, f, |w| Value::Bool(w.start_dtd(&n, p.as_deref(), s.as_deref())))
    }),
    ("enddtd", |it, o, _, f| with_writer(it, o, f, |w| Value::Bool(w.end_dtd()))),
    ("writedtd", |it, o, a, f| {
        let (n, p, s, sub) = (name_arg(it, a, 0, f, "element")?, opt(it, a, 1)?, opt(it, a, 2)?, opt(it, a, 3)?);
        with_writer(it, o, f, |w| {
            let ok = w.start_dtd(&n, p.as_deref(), s.as_deref()) && sub.as_deref().is_none_or(|x| w.raw(x)) && w.end_dtd();
            Value::Bool(ok)
        })
    }),
    ("outputmemory", |it, o, a, f| {
        let flush = a.first().is_none_or(|v| v.truthy());
        with_writer(it, o, f, |w| Value::bytes(w.take(flush)))
    }),
    ("flush", |it, o, a, f| {
        let empty = a.first().is_none_or(|v| v.truthy());
        let (v, out) = with_writer(it, o, f, |w| match &w.sink {
            Some(Sink::Memory) | None => (Value::bytes(w.take(empty)), None),
            _ => {
                let n = w.out.len();
                (Value::Int(n as i64), w.drain_to_sink())
            }
        })?;
        if let Some(b) = out {
            it.echo(&b);
        }
        Ok(v)
    }),
];

fn op(name: &str) -> Op {
    OPS.iter().find(|(n, _)| *n == name).map(|(_, f)| *f).expect("xmlwriter op")
}

fn method_name(lname: &str) -> String {
    let display = match lname {
        "openmemory" => "openMemory",
        "openuri" => "openUri",
        "setindent" => "setIndent",
        "setindentstring" => "setIndentString",
        "startdocument" => "startDocument",
        "enddocument" => "endDocument",
        "startelement" => "startElement",
        "startelementns" => "startElementNs",
        "endelement" => "endElement",
        "fullendelement" => "fullEndElement",
        "writeelement" => "writeElement",
        "writeelementns" => "writeElementNs",
        "startattribute" => "startAttribute",
        "startattributens" => "startAttributeNs",
        "endattribute" => "endAttribute",
        "writeattribute" => "writeAttribute",
        "writeattributens" => "writeAttributeNs",
        "writeraw" => "writeRaw",
        "startcomment" => "startComment",
        "endcomment" => "endComment",
        "writecomment" => "writeComment",
        "startpi" => "startPi",
        "endpi" => "endPi",
        "writepi" => "writePi",
        "startcdata" => "startCdata",
        "endcdata" => "endCdata",
        "writecdata" => "writeCdata",
        "startdtd" => "startDtd",
        "enddtd" => "endDtd",
        "writedtd" => "writeDtd",
        "outputmemory" => "outputMemory",
        n => n,
    };
    format!("XMLWriter::{display}")
}

macro_rules! natives {
    ($($n:literal),*) => {
        vec![$((
            "xmlwriter",
            $n,
            (|it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>| -> R<Value> {
                let o = this.ok_or_else(|| it.error("Error", "Non-static method called statically"))?;
                op($n)(it, o, &a, &method_name($n))
            }) as NativeMethod,
        )),*]
    };
}

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    let mut v = natives!(
        "openmemory", "openuri", "setindent", "setindentstring", "startdocument", "enddocument", "startelement", "startelementns", "endelement",
        "fullendelement", "writeelement", "writeelementns", "startattribute", "startattributens", "endattribute", "writeattribute",
        "writeattributens", "text", "writeraw", "startcomment", "endcomment", "writecomment", "startpi", "endpi", "writepi", "startcdata",
        "endcdata", "writecdata", "startdtd", "enddtd", "writedtd", "outputmemory", "flush"
    );
    v.push(("xmlwriter", "static tomemory", |it, _, cls, _| {
        let o = it.new_instance(cls)?;
        o.set_internal(Internal::Data(Box::new(WriterData(RefCell::new(Writer::new(Sink::Memory))))));
        Ok(Value::Object(o))
    }));
    v.push(("xmlwriter", "static touri", |it, _, cls, a| {
        let o = it.new_instance(cls)?;
        match op("openuri")(it, &o, &a, "XMLWriter::toUri")? {
            Value::Bool(true) => Ok(Value::Object(o)),
            _ => Err(it.error("Error", "XMLWriter::toUri(): Unable to resolve file path")),
        }
    }));
    v
}

macro_rules! functions {
    ($t:expr, $($n:literal),*) => {
        $(
            $t.insert(concat!("xmlwriter_", $n), |it: &Interp, _: &mut crate::interp::Frame, a: &mut [Value]| -> R<Value> {
                let fname = concat!("xmlwriter_", $n);
                if $n == "open_memory" || $n == "open_uri" {
                    let cls = it.lookup_class("XMLWriter")?;
                    let o = it.new_instance(&cls)?;
                    let r = op($n.replace('_', "").as_str())(it, &o, a, fname)?;
                    return Ok(if r.truthy() { Value::Object(o) } else { Value::Bool(false) });
                }
                let Some(Value::Object(o)) = a.first().map(|v| v.deref()) else {
                    return Err(it.error("TypeError", &format!("{fname}(): Argument #1 ($writer) must be of type XMLWriter")));
                };
                op($n.replace('_', "").as_str())(it, &o, &a[1..], fname)
            });
        )*
    };
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    functions!(
        t, "open_memory", "open_uri", "set_indent", "set_indent_string", "start_document", "end_document", "start_element", "start_element_ns",
        "end_element", "full_end_element", "write_element", "write_element_ns", "start_attribute", "start_attribute_ns", "end_attribute",
        "write_attribute", "write_attribute_ns", "text", "write_raw", "start_comment", "end_comment", "write_comment", "start_pi", "end_pi",
        "write_pi", "start_cdata", "end_cdata", "write_cdata", "start_dtd", "end_dtd", "write_dtd", "output_memory", "flush"
    );
}
