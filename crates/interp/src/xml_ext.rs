//! ext/xml: xml_parser_create() and the expat-style SAX API, on the event
//! parser shared with DOM (dom/xml_parse.rs). Callbacks, case folding,
//! namespace mode, xml_parse_into_struct() and error strings follow
//! PHP's libxml2 compatibility layer (ext/xml/compat.c, xml.c).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::{has, int, s};
use crate::dom::xml_parse::{run, Sink, TextKind};
use crate::interp::{BuiltinFn, Frame, Interp, R};
use crate::value::{Internal, Key, NativeData, Obj, PhpArray, Value};

const XML_OPTION_CASE_FOLDING: i64 = 1;
const XML_OPTION_TARGET_ENCODING: i64 = 2;
const XML_OPTION_SKIP_TAGSTART: i64 = 3;
const XML_OPTION_SKIP_WHITE: i64 = 4;
const XML_OPTION_PARSE_HUGE: i64 = 5;

/// ext/xml/compat.c's error strings, indexed by libxml2 error code.
const ERRORS: &[&str] = &[
    "No error", "No memory", "Invalid document start", "Empty document", "Not well-formed (invalid token)", "Invalid document end",
    "Invalid hexadecimal character reference", "Invalid decimal character reference", "Invalid character reference", "Invalid character",
    "XML_ERR_CHARREF_AT_EOF", "XML_ERR_CHARREF_IN_PROLOG", "XML_ERR_CHARREF_IN_EPILOG", "XML_ERR_CHARREF_IN_DTD", "XML_ERR_ENTITYREF_AT_EOF",
    "XML_ERR_ENTITYREF_IN_PROLOG", "XML_ERR_ENTITYREF_IN_EPILOG", "XML_ERR_ENTITYREF_IN_DTD", "PEReference at end of document",
    "PEReference in prolog", "PEReference in epilog", "PEReference: forbidden within markup decl in internal subset", "XML_ERR_ENTITYREF_NO_NAME",
    "EntityRef: expecting ';'", "PEReference: no name", "PEReference: expecting ';'", "Undeclared entity error", "Undeclared entity warning",
    "Unparsed Entity", "XML_ERR_ENTITY_IS_EXTERNAL", "XML_ERR_ENTITY_IS_PARAMETER", "Unknown encoding", "Unsupported encoding",
    "String not started expecting ' or \"", "String not closed expecting \" or '", "Namespace declaration error", "EntityValue: \" or ' expected",
    "EntityValue: \" or ' expected", "< in attribute", "Attribute not started", "Attribute not finished", "Attribute without value",
    "Attribute redefined", "SystemLiteral \" or ' expected", "SystemLiteral \" or ' expected", "Comment not finished",
    "Processing Instruction not started", "Processing Instruction not finished", "NOTATION: Name expected here",
    "'>' required to close NOTATION declaration", "'(' required to start ATTLIST enumeration", "'(' required to start ATTLIST enumeration",
    "MixedContentDecl : '|' or ')*' expected", "XML_ERR_MIXED_NOT_FINISHED", "ELEMENT in DTD not started", "ELEMENT in DTD not finished",
    "XML declaration not started", "XML declaration not finished", "XML_ERR_CONDSEC_NOT_STARTED", "XML conditional section not closed",
    "Content error in the external subset", "DOCTYPE not finished", "Sequence ']]>' not allowed in content", "CDATA not finished",
    "Reserved XML Name", "Space required", "XML_ERR_SEPARATOR_REQUIRED", "NmToken expected in ATTLIST enumeration", "XML_ERR_NAME_REQUIRED",
    "MixedContentDecl : '#PCDATA' expected", "SYSTEM or PUBLIC, the URI is missing", "PUBLIC, the Public Identifier is missing", "< required",
    "> required", "</ required", "= required", "Mismatched tag", "Tag not finished", "standalone accepts only 'yes' or 'no'",
    "Invalid XML encoding name", "Comment must not contain '--' (double-hyphen)", "Invalid encoding", "external parsed entities cannot be standalone",
    "XML conditional section '[' expected", "Entity value required", "chunk is not well balanced", "extra content at the end of well balanced chunk",
    "XML_ERR_ENTITY_CHAR_ERROR", "PEReferences forbidden in internal subset", "Detected an entity reference loop", "XML_ERR_ENTITY_BOUNDARY",
    "Invalid URI", "Fragment not allowed", "XML_WAR_CATALOG_PI", "XML_ERR_NO_DTD", "conditional section INCLUDE or IGNORE keyword expected",
    "Version in XML Declaration missing", "XML_WAR_UNKNOWN_VERSION", "XML_WAR_LANG_VALUE", "XML_WAR_NS_URI", "XML_WAR_NS_URI_RELATIVE",
    "Missing encoding in text declaration",
];

#[derive(Default)]
struct Handlers {
    start: Option<Value>,
    end: Option<Value>,
    cdata: Option<Value>,
    pi: Option<Value>,
    default: Option<Value>,
    unparsed: Option<Value>,
    notation: Option<Value>,
    external: Option<Value>,
    start_ns: Option<Value>,
    end_ns: Option<Value>,
}

struct Parser {
    ns_sep: Option<String>,
    case_folding: bool,
    target: String,
    skip_white: bool,
    skip_tagstart: usize,
    object: Option<Value>,
    h: Handlers,
    buffer: Vec<u8>,
    /// Events already delivered (incremental xml_parse()).
    delivered: usize,
    error: i64,
    line: u32,
    finished: bool,
    /// xml_parse_into_struct() state.
    level: usize,
    ltags: Vec<String>,
    lastwasopen: bool,
}

struct ParserData(RefCell<Parser>);

impl NativeData for ParserData {
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

#[derive(Clone, Debug)]
enum Ev {
    Start(String, Vec<(String, String)>),
    End(String),
    /// Character data; true when it ran to the end of the input (it may
    /// continue in the next chunk).
    Text(String, bool),
    /// Text for the default handler (comments, unhandled constructs).
    Default(String),
    Pi(String, String),
    Entity(String, String),
    StartNs(Option<String>, String),
}

/// Collects SAX events, resolving namespaces in namespace mode.
struct SaxSink {
    events: Vec<(Ev, u32)>,
    line: u32,
    ns_sep: Option<String>,
    scopes: Vec<Vec<(Option<String>, String)>>,
}

impl SaxSink {
    fn lookup(&self, prefix: Option<&str>) -> Option<String> {
        for scope in self.scopes.iter().rev() {
            if let Some((_, u)) = scope.iter().rev().find(|(p, _)| p.as_deref() == prefix) {
                return if u.is_empty() { None } else { Some(u.clone()) };
            }
        }
        None
    }

    /// "uri<sep>local", or the local name (namespace mode).
    fn qualify(&self, name: &str, is_attr: bool) -> String {
        let Some(sep) = &self.ns_sep else { return name.to_string() };
        let (prefix, local) = match name.split_once(':') {
            Some((p, l)) => (Some(p), l),
            None => (None, name),
        };
        if prefix.is_none() && is_attr {
            return name.to_string();
        }
        match self.lookup(prefix) {
            Some(u) => format!("{u}{sep}{local}"),
            None => local.to_string(),
        }
    }
}

impl Sink for SaxSink {
    fn start(&mut self, name: &str, attrs: Vec<(String, String)>, _line: u32) -> Vec<(i64, String)> {
        if self.ns_sep.is_some() {
            let mut scope = vec![];
            for (k, v) in &attrs {
                if k == "xmlns" {
                    scope.push((None, v.clone()));
                } else if let Some(p) = k.strip_prefix("xmlns:") {
                    scope.push((Some(p.to_string()), v.clone()));
                }
            }
            for (p, u) in &scope {
                self.events.push((Ev::StartNs(p.clone(), u.clone()), self.line));
            }
            self.scopes.push(scope);
            let attrs: Vec<(String, String)> = attrs.into_iter().filter(|(k, _)| k != "xmlns" && !k.starts_with("xmlns:")).map(|(k, v)| (self.qualify(&k, true), v)).collect();
            let q = self.qualify(name, false);
            self.events.push((Ev::Start(q, attrs), self.line));
        } else {
            self.events.push((Ev::Start(name.to_string(), attrs), self.line));
        }
        vec![]
    }

    fn end(&mut self, name: &str) {
        let q = self.qualify(name, false);
        if self.ns_sep.is_some() {
            self.scopes.pop();
        }
        self.events.push((Ev::End(q), self.line));
    }

    fn text(&mut self, s: &str, kind: TextKind, next: (u8, u8)) {
        let at_end = kind == TextKind::Plain && next.0 == 0;
        self.events.push((Ev::Text(s.to_string(), at_end), self.line));
    }

    fn entity(&mut self, name: &str, value: &str) {
        self.events.push((Ev::Entity(name.to_string(), value.to_string()), self.line));
    }

    fn comment(&mut self, s: &str) {
        self.events.push((Ev::Default(format!("<!--{s}-->")), self.line));
    }

    fn pi(&mut self, target: &str, data: &str) {
        self.events.push((Ev::Pi(target.to_string(), data.to_string()), self.line));
    }

    fn line(&mut self, line: u32) {
        self.line = line;
    }
}

fn parser_of(it: &Interp, v: &Value, func: &str) -> R<Rc<Obj>> {
    if let Value::Object(o) = v.deref() {
        if let Internal::Data(d) = &*o.internal() {
            if d.as_any().downcast_ref::<ParserData>().is_some() {
                return Ok(o.clone());
            }
        }
    }
    Err(it.error("TypeError", &format!("{func}(): Argument #1 ($parser) must be of type XMLParser, {} given", v.debug_type())))
}

fn with<T>(o: &Rc<Obj>, f: impl FnOnce(&mut Parser) -> T) -> T {
    match &*o.internal() {
        Internal::Data(d) => f(&mut d.as_any().downcast_ref::<ParserData>().unwrap().0.borrow_mut()),
        _ => unreachable!(),
    }
}

fn create(it: &Interp, a: &[Value], ns_sep: Option<String>) -> R<Value> {
    if has(a, 0) && !a[0].is_null() {
        let enc = String::from_utf8_lossy(&s(it, a, 0)?).to_ascii_uppercase();
        if !matches!(enc.as_str(), "ISO-8859-1" | "UTF-8" | "US-ASCII" | "") {
            return Err(it.error("ValueError", &format!("xml_parser_create(): Argument #1 ($encoding) is not a supported source encoding")));
        }
    }
    let cls = it.lookup_class("XMLParser")?;
    let o = it.new_instance(&cls)?;
    o.set_internal(Internal::Data(Box::new(ParserData(RefCell::new(Parser {
        ns_sep,
        case_folding: true,
        target: "UTF-8".into(),
        skip_white: false,
        skip_tagstart: 0,
        object: None,
        h: Handlers::default(),
        buffer: vec![],
        delivered: 0,
        error: 0,
        line: 1,
        finished: false,
        level: 0,
        ltags: vec![],
        lastwasopen: false,
    })))));
    Ok(Value::Object(o))
}

/// Output conversion (XML_OPTION_TARGET_ENCODING) and case folding.
fn decode(p: &Parser, s: &str) -> Value {
    match p.target.as_str() {
        "ISO-8859-1" => Value::bytes(s.chars().map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' }).collect()),
        "US-ASCII" => Value::bytes(s.chars().map(|c| if c.is_ascii() { c as u8 } else { b'?' }).collect()),
        _ => Value::str(s),
    }
}

fn tag(p: &Parser, name: &str) -> String {
    let n = if p.case_folding { name.to_ascii_uppercase() } else { name.to_string() };
    if p.skip_tagstart >= n.len() {
        if p.skip_tagstart == 0 { n } else { String::new() }
    } else {
        n[p.skip_tagstart..].to_string()
    }
}

/// Call a handler: a method name of the xml_set_object() object, or a
/// callable.
fn call(it: &Interp, obj: &Option<Value>, h: &Value, args: Vec<Value>) -> R<()> {
    let target = match (obj, h) {
        (Some(o), Value::Str(name)) => Value::array(PhpArray::from_list([o.clone(), Value::Str(name.clone())])),
        _ => h.clone(),
    };
    let f = Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
    let c = it.to_callable(&f, &target)?;
    it.call_callable(&c, args, vec![])?;
    Ok(())
}

/// Deliver events to the handlers (and xml_parse_into_struct's arrays).
fn dispatch(it: &Interp, po: &Rc<Obj>, events: &[(Ev, u32)], mut out: Option<(&mut PhpArray, &mut PhpArray)>) -> R<()> {
    let me = Value::Object(po.clone());
    for (ev, line) in events {
        let (obj, handler) = with(po, |p| {
            p.line = *line;
            let h = match ev {
                Ev::Start(..) => p.h.start.clone(),
                Ev::End(..) => p.h.end.clone(),
                Ev::Text(..) => p.h.cdata.clone().or_else(|| p.h.default.clone()),
                Ev::Default(..) => p.h.default.clone(),
                Ev::Pi(..) => p.h.pi.clone().or_else(|| p.h.default.clone()),
                Ev::Entity(..) => p.h.default.clone().or_else(|| p.h.cdata.clone()),
                Ev::StartNs(..) => p.h.start_ns.clone(),
            };
            (p.object.clone(), h)
        });
        match ev {
            Ev::Start(name, attrs) => {
                let (t, args) = with(po, |p| {
                    p.level += 1;
                    let t = tag(p, name);
                    let mut arr = PhpArray::new();
                    for (k, v) in attrs {
                        let k = if p.case_folding { k.to_ascii_uppercase() } else { k.clone() };
                        arr.insert(Key::from_bytes(k.as_bytes()), decode(p, v));
                    }
                    (t, arr)
                });
                if let Some(h) = &handler {
                    if with(po, |p| p.h.start.is_some()) {
                        call(it, &obj, h, vec![me.clone(), Value::str(&t), Value::array(args.clone())])?;
                    } else {
                        // No start handler: the default handler gets the tag.
                        let mut raw = format!("<{name}");
                        for (k, v) in attrs {
                            raw.push_str(&format!(" {k}=\"{v}\""));
                        }
                        raw.push('>');
                        call(it, &obj, h, vec![me.clone(), Value::str(&raw)])?;
                    }
                } else if let Some(d) = with(po, |p| p.h.default.clone()) {
                    let mut raw = format!("<{name}");
                    for (k, v) in attrs {
                        raw.push_str(&format!(" {k}=\"{v}\""));
                    }
                    raw.push('>');
                    call(it, &obj, &d, vec![me.clone(), Value::str(&raw)])?;
                }
                if let Some((vals, idx)) = out.as_mut() {
                    with(po, |p| {
                        let mut e = PhpArray::new();
                        e.insert(Key::from_bytes(b"tag"), Value::str(&t));
                        e.insert(Key::from_bytes(b"type"), Value::str("open"));
                        e.insert(Key::from_bytes(b"level"), Value::Int(p.level as i64));
                        if args.len() > 0 {
                            e.insert(Key::from_bytes(b"attributes"), Value::array(args));
                        }
                        add_index(idx, &t, vals.len());
                        vals.push(Value::array(e));
                        if p.ltags.len() < p.level {
                            p.ltags.resize(p.level, String::new());
                        }
                        p.ltags[p.level - 1] = t.clone();
                        p.lastwasopen = true;
                    });
                }
            }
            Ev::End(name) => {
                let t = with(po, |p| tag(p, name));
                if let Some(h) = &handler {
                    call(it, &obj, h, vec![me.clone(), Value::str(&t)])?;
                } else if let Some(d) = with(po, |p| p.h.default.clone()) {
                    call(it, &obj, &d, vec![me.clone(), Value::str(&format!("</{name}>"))])?;
                }
                if let Some((vals, idx)) = out.as_mut() {
                    with(po, |p| {
                        if p.lastwasopen {
                            if let Some(Value::Array(last)) = vals.get_mut(&Key::Int(vals.len() as i64 - 1)) {
                                std::rc::Rc::make_mut(last).insert(Key::from_bytes(b"type"), Value::str("complete"));
                            }
                        } else {
                            let mut e = PhpArray::new();
                            e.insert(Key::from_bytes(b"tag"), Value::str(&t));
                            e.insert(Key::from_bytes(b"type"), Value::str("close"));
                            e.insert(Key::from_bytes(b"level"), Value::Int(p.level as i64));
                            add_index(idx, &t, vals.len());
                            vals.push(Value::array(e));
                        }
                        p.lastwasopen = false;
                    });
                }
                with(po, |p| p.level = p.level.saturating_sub(1));
            }
            Ev::Text(s, _) | Ev::Entity(_, s) => {
                let is_entity = matches!(ev, Ev::Entity(..));
                let v = with(po, |p| decode(p, s));
                if let Some(h) = &handler {
                    if is_entity && with(po, |p| p.h.default.is_some()) {
                        let Ev::Entity(n, _) = ev else { unreachable!() };
                        call(it, &obj, h, vec![me.clone(), Value::str(&format!("&{n};"))])?;
                    } else {
                        call(it, &obj, h, vec![me.clone(), v.clone()])?;
                    }
                }
                if let Some((vals, idx)) = out.as_mut() {
                    with(po, |p| add_cdata(p, vals, idx, s, &v));
                }
            }
            Ev::Default(s) => {
                if let Some(h) = &handler {
                    call(it, &obj, h, vec![me.clone(), Value::str(s)])?;
                }
            }
            Ev::Pi(t, d) => {
                if let Some(h) = &handler {
                    if with(po, |p| p.h.pi.is_some()) {
                        call(it, &obj, h, vec![me.clone(), Value::str(t), Value::str(d)])?;
                    } else {
                        call(it, &obj, h, vec![me.clone(), Value::str(&format!("<?{t} {d}?>"))])?;
                    }
                }
            }
            Ev::StartNs(prefix, uri) => {
                if let Some(h) = &handler {
                    let p = prefix.as_deref().map_or(Value::Bool(false), Value::str);
                    call(it, &obj, h, vec![me.clone(), p, Value::str(uri)])?;
                }
            }
        }
    }
    Ok(())
}

fn add_index(idx: &mut PhpArray, tag: &str, pos: usize) {
    let k = Key::from_bytes(tag.as_bytes());
    let mut list = match idx.get(&k) {
        Some(Value::Array(a)) => (*a).clone(),
        _ => PhpArray::new(),
    };
    list.push(Value::Int(pos as i64));
    idx.insert(k, Value::array(list));
}

/// xml.c's character data rule for xml_parse_into_struct().
fn add_cdata(p: &Parser, vals: &mut PhpArray, idx: &mut PhpArray, raw: &str, v: &Value) {
    let doprint = !p.skip_white || raw.bytes().any(|c| !matches!(c, b' ' | b'\t' | b'\n'));
    let n = vals.len() as i64;
    let append = |e: &mut PhpArray| -> bool {
        if let Some(Value::Str(old)) = e.get(&Key::from_bytes(b"value")) {
            let mut b = old.to_vec();
            b.extend_from_slice(&v.scalar_bytes().unwrap_or_default());
            e.insert(Key::from_bytes(b"value"), Value::bytes(b));
            return true;
        }
        false
    };
    if p.lastwasopen {
        if let Some(Value::Array(last)) = vals.get_mut(&Key::Int(n - 1)) {
            let last = std::rc::Rc::make_mut(last);
            if !append(last) && doprint {
                last.insert(Key::from_bytes(b"value"), v.clone());
            }
        }
        return;
    }
    if let Some(Value::Array(last)) = vals.get_mut(&Key::Int(n - 1)) {
        if matches!(last.get(&Key::from_bytes(b"type")), Some(Value::Str(t)) if &t[..] == b"cdata") {
            let last = std::rc::Rc::make_mut(last);
            if append(last) {
                return;
            }
        }
    }
    if p.level > 0 && doprint {
        let t = p.ltags.get(p.level - 1).cloned().unwrap_or_default();
        let mut e = PhpArray::new();
        e.insert(Key::from_bytes(b"tag"), Value::str(&t));
        e.insert(Key::from_bytes(b"value"), v.clone());
        e.insert(Key::from_bytes(b"type"), Value::str("cdata"));
        e.insert(Key::from_bytes(b"level"), Value::Int(p.level as i64));
        add_index(idx, &t, vals.len());
        vals.push(Value::array(e));
    }
}

/// Parse the buffered input; deliver the events not delivered yet.
/// Returns 1 or 0 like xml_parse().
fn parse(it: &Interp, po: &Rc<Obj>, data: &[u8], is_final: bool, out: Option<(&mut PhpArray, &mut PhpArray)>) -> R<Value> {
    let (buffer, ns_sep, delivered) = with(po, |p| {
        p.buffer.extend_from_slice(data);
        (p.buffer.clone(), p.ns_sep.clone(), p.delivered)
    });
    let mut sink = SaxSink { events: vec![], line: 1, ns_sep, scopes: vec![] };
    let (ok, errors, len) = run(&buffer, &mut sink);
    // An error at the end of a partial document is just "need more input".
    let real_error = if ok { None } else { errors.iter().find(|e| e.level == 3).filter(|e| is_final || e.offset < len) };
    let mut events = sink.events;
    if !is_final && real_error.is_none() {
        // Text running into the end of a partial chunk may continue.
        while matches!(events.last(), Some((Ev::Text(_, true), _))) {
            events.pop();
        }
    }
    let new: Vec<(Ev, u32)> = events.into_iter().skip(delivered).collect();
    with(po, |p| p.delivered += new.len());
    dispatch(it, po, &new, out)?;
    with(po, |p| {
        if let Some(e) = real_error {
            p.error = e.code;
            p.line = e.line;
        } else if is_final {
            p.finished = true;
            p.line = buffer.iter().filter(|&&c| c == b'\n').count() as u32 + 1;
        }
    });
    Ok(Value::Int(if real_error.is_some() { 0 } else { 1 }))
}

/// A handler argument: None to unset; a string that is not callable is
/// a method name for xml_set_object() (deprecated since 8.4; reported
/// once per call).
fn handler_arg(it: &Interp, v: Value, func: &str, warned: &mut bool) -> Option<Value> {
    match v.into_deref() {
        Value::Null => None,
        Value::Str(s) if s.is_empty() => None,
        v @ Value::Str(_) => {
            let f = Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
            if !*warned && it.to_callable(&f, &v).is_err() {
                it.deprecated(&format!("{func}(): Passing non-callable strings is deprecated since 8.4"));
                *warned = true;
            }
            Some(v)
        }
        v => Some(v),
    }
}

fn set_handler(it: &Interp, a: &[Value], func: &str, f: fn(&mut Handlers, Option<Value>)) -> R<Value> {
    let po = parser_of(it, &a[0], func)?;
    let v = handler_arg(it, a.get(1).cloned().unwrap_or_default(), func, &mut false);
    with(&po, |p| f(&mut p.h, v));
    Ok(Value::Bool(true))
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("xml_parser_create", |it, _, a| create(it, a, None));
    t.insert("xml_parser_create_ns", |it, _, a| {
        let sep = if has(a, 1) { String::from_utf8_lossy(&s(it, a, 1)?).into_owned() } else { ":".into() };
        let sep: String = sep.chars().take(1).collect();
        create(it, a, Some(if sep.is_empty() { ":".into() } else { sep }))
    });
    t.insert("xml_parser_free", |it, _, a| {
        parser_of(it, &a[0], "xml_parser_free")?;
        Ok(Value::Bool(true))
    });
    t.insert("xml_set_object", |it, _, a| {
        it.deprecated("Function xml_set_object() is deprecated since 8.4, provide a proper method callable to xml_set_*_handler() functions");
        let po = parser_of(it, &a[0], "xml_set_object")?;
        let obj = a.get(1).cloned().unwrap_or_default().into_deref();
        with(&po, |p| p.object = Some(obj));
        Ok(Value::Bool(true))
    });
    t.insert("xml_set_element_handler", |it, _, a| {
        let po = parser_of(it, &a[0], "xml_set_element_handler")?;
        let mut warned = false;
        let st = handler_arg(it, a.get(1).cloned().unwrap_or_default(), "xml_set_element_handler", &mut warned);
        let en = handler_arg(it, a.get(2).cloned().unwrap_or_default(), "xml_set_element_handler", &mut warned);
        with(&po, |p| {
            p.h.start = st;
            p.h.end = en;
        });
        Ok(Value::Bool(true))
    });
    t.insert("xml_set_character_data_handler", |it, _, a| set_handler(it, a, "xml_set_character_data_handler", |h, v| h.cdata = v));
    t.insert("xml_set_processing_instruction_handler", |it, _, a| set_handler(it, a, "xml_set_processing_instruction_handler", |h, v| h.pi = v));
    t.insert("xml_set_default_handler", |it, _, a| set_handler(it, a, "xml_set_default_handler", |h, v| h.default = v));
    t.insert("xml_set_unparsed_entity_decl_handler", |it, _, a| set_handler(it, a, "xml_set_unparsed_entity_decl_handler", |h, v| h.unparsed = v));
    t.insert("xml_set_notation_decl_handler", |it, _, a| set_handler(it, a, "xml_set_notation_decl_handler", |h, v| h.notation = v));
    t.insert("xml_set_external_entity_ref_handler", |it, _, a| set_handler(it, a, "xml_set_external_entity_ref_handler", |h, v| h.external = v));
    t.insert("xml_set_start_namespace_decl_handler", |it, _, a| set_handler(it, a, "xml_set_start_namespace_decl_handler", |h, v| h.start_ns = v));
    t.insert("xml_set_end_namespace_decl_handler", |it, _, a| set_handler(it, a, "xml_set_end_namespace_decl_handler", |h, v| h.end_ns = v));
    t.insert("xml_parse", |it, _, a| {
        let po = parser_of(it, &a[0], "xml_parse")?;
        let data = s(it, a, 1)?.to_vec();
        let is_final = a.get(2).is_some_and(|v| v.truthy());
        if with(&po, |p| p.finished || p.error != 0) {
            return Ok(Value::Int(0));
        }
        parse(it, &po, &data, is_final, None)
    });
    t.insert("xml_parse_into_struct", |it, _, a| {
        let po = parser_of(it, &a[0], "xml_parse_into_struct")?;
        let data = s(it, a, 1)?.to_vec();
        let mut vals = PhpArray::new();
        let mut idx = PhpArray::new();
        let r = parse(it, &po, &data, true, Some((&mut vals, &mut idx)))?;
        if a.len() > 2 {
            a[2] = Value::array(vals);
        }
        if a.len() > 3 {
            a[3] = Value::array(idx);
        }
        Ok(r)
    });
    t.insert("xml_get_error_code", |it, _, a| {
        let po = parser_of(it, &a[0], "xml_get_error_code")?;
        Ok(Value::Int(with(&po, |p| p.error)))
    });
    t.insert("xml_error_string", |it, _, a| {
        let code = int(it, a, 0)?;
        Ok(match usize::try_from(code).ok().and_then(|c| ERRORS.get(c)) {
            Some(s) => Value::str(s),
            None => Value::str("Unknown"),
        })
    });
    t.insert("xml_get_current_line_number", |it, _, a| {
        let po = parser_of(it, &a[0], "xml_get_current_line_number")?;
        Ok(Value::Int(with(&po, |p| p.line) as i64))
    });
    t.insert("xml_get_current_column_number", |it, _, a| {
        parser_of(it, &a[0], "xml_get_current_column_number")?;
        Ok(Value::Int(0))
    });
    t.insert("xml_get_current_byte_index", |it, _, a| {
        let po = parser_of(it, &a[0], "xml_get_current_byte_index")?;
        Ok(Value::Int(with(&po, |p| p.buffer.len()) as i64))
    });
    t.insert("xml_parser_set_option", |it, _, a| {
        let po = parser_of(it, &a[0], "xml_parser_set_option")?;
        let opt = int(it, a, 1)?;
        let v = a.get(2).cloned().unwrap_or_default();
        match opt {
            XML_OPTION_CASE_FOLDING => with(&po, |p| p.case_folding = v.truthy()),
            XML_OPTION_SKIP_WHITE => with(&po, |p| p.skip_white = v.truthy()),
            XML_OPTION_SKIP_TAGSTART => {
                let n = it.to_int(&v)?;
                if n < 0 {
                    return Err(it.error("ValueError", "xml_parser_set_option(): Argument #3 ($value) must be between 0 and 2147483647 for option XML_OPTION_SKIP_TAGSTART"));
                }
                with(&po, |p| p.skip_tagstart = n as usize)
            }
            XML_OPTION_TARGET_ENCODING => {
                let e = String::from_utf8_lossy(&it.to_str(&v)?).to_ascii_uppercase();
                if !matches!(e.as_str(), "ISO-8859-1" | "UTF-8" | "US-ASCII") {
                    return Err(it.error("ValueError", "xml_parser_set_option(): Argument #3 ($value) is not a supported target encoding"));
                }
                with(&po, |p| p.target = e)
            }
            XML_OPTION_PARSE_HUGE => {}
            _ => return Err(it.error("ValueError", "xml_parser_set_option(): Argument #2 ($option) must be a XML_OPTION_* constant")),
        }
        Ok(Value::Bool(true))
    });
    t.insert("xml_parser_get_option", |it, _, a| {
        let po = parser_of(it, &a[0], "xml_parser_get_option")?;
        let opt = int(it, a, 1)?;
        Ok(match opt {
            XML_OPTION_CASE_FOLDING => Value::Bool(with(&po, |p| p.case_folding)),
            XML_OPTION_SKIP_WHITE => Value::Bool(with(&po, |p| p.skip_white)),
            XML_OPTION_SKIP_TAGSTART => Value::Int(with(&po, |p| p.skip_tagstart) as i64),
            XML_OPTION_TARGET_ENCODING => Value::str(&with(&po, |p| p.target.clone())),
            XML_OPTION_PARSE_HUGE => Value::Bool(false),
            _ => return Err(it.error("ValueError", "xml_parser_get_option(): Argument #2 ($option) must be a XML_OPTION_* constant")),
        })
    });
}
