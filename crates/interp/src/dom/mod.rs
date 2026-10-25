//! ext/dom and ext/libxml: a native node tree (tree.rs) with libxml2's
//! HTML and XML parsers and serializers, behind PHP's legacy DOM classes
//! (DOMDocument, DOMElement, ...). PHP objects are thin wrappers holding
//! (document, node id); the same node always maps to the same object.

mod document;
mod html_parse;
mod node;
mod props;
mod save;
mod tables;
mod tree;
mod xml_parse;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::{has, s};
use crate::interp::{BuiltinFn, Ctrl, Interp, NativeMethod, R};
use crate::value::{Internal, NativeData, Obj, PhpArray, Value};

pub use tree::{Doc, DocRef, Id};
use tree::*;

/// A PHP DOMNode (or DOMDocument) wrapper's state.
pub struct NodeData {
    pub doc: DocRef,
    pub id: Id,
}

impl NativeData for NodeData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        // `clone $node`: a deep copy in the same document.
        let copy = self.doc.borrow_mut().copy_within(self.id, true);
        Some(Box::new(NodeData { doc: self.doc.clone(), id: copy }))
    }
    fn view_props(&self) -> Vec<(Rc<str>, Value)> {
        props::dump(&self.doc.borrow(), self.id)
    }
}

impl Doc {
    /// A copy for reading while the original is borrowed mutably.
    pub fn clone_tree(&self) -> Doc {
        let mut d = self.clone();
        d.wrappers.clear();
        d
    }
}

/// Which nodes a DOMNodeList / DOMNamedNodeMap holds.
#[derive(Clone)]
pub enum ListKind {
    Children(Id),
    /// getElementsByTagName(NS): namespace filter (None: any), local or
    /// qualified name ("*": any).
    Tag { root: Id, ns: Option<Option<String>>, name: String },
    Attrs(Id),
    Empty,
}

pub struct ListData {
    pub doc: Option<DocRef>,
    pub kind: ListKind,
    cache: RefCell<Option<(u64, Rc<Vec<Id>>)>>,
}

impl NativeData for ListData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        None
    }
    fn view_props(&self) -> Vec<(Rc<str>, Value)> {
        vec![(Rc::from("length"), Value::Int(self.ids().len() as i64))]
    }
}

impl ListData {
    pub fn new(doc: Option<DocRef>, kind: ListKind) -> ListData {
        ListData { doc, kind, cache: RefCell::new(None) }
    }

    pub fn ids(&self) -> Rc<Vec<Id>> {
        let Some(doc) = &self.doc else { return Rc::new(vec![]) };
        let d = doc.borrow();
        if let Some((v, ids)) = &*self.cache.borrow() {
            if *v == d.rev {
                return ids.clone();
            }
        }
        let ids: Vec<Id> = match &self.kind {
            ListKind::Children(p) => d.children(*p).collect(),
            ListKind::Attrs(el) => d.n(*el).attrs.clone(),
            ListKind::Empty => vec![],
            ListKind::Tag { root, ns, name } => d
                .descendants(*root)
                .into_iter()
                .filter(|&c| {
                    let n = d.n(c);
                    n.kind == ELEMENT
                        && match ns {
                            None => name == "*" || n.name == *name,
                            Some(want) => (name == "*" || n.local() == name) && (want.as_deref() == Some("*") || n.ns.as_deref() == want.as_deref()),
                        }
                })
                .collect(),
        };
        let ids = Rc::new(ids);
        *self.cache.borrow_mut() = Some((d.rev, ids.clone()));
        ids
    }
}

// ---- wrappers ----------------------------------------------------------------------

pub fn node_of(o: &Rc<Obj>) -> Option<(DocRef, Id)> {
    match &*o.try_internal()? {
        Internal::Data(d) => d.as_any().downcast_ref::<NodeData>().map(|n| (n.doc.clone(), n.id)),
        _ => None,
    }
}

pub fn this_node(it: &Interp, this: Option<&Rc<Obj>>) -> R<(DocRef, Id)> {
    let o = this.ok_or_else(|| it.error("Error", "Non-static method called statically"))?;
    node_of(o).ok_or_else(|| it.error("Error", "Couldn't fetch DOMNode. Node no longer exists"))
}

fn base_class(kind: u8) -> &'static str {
    match kind {
        ELEMENT => "DOMElement",
        ATTRIBUTE => "DOMAttr",
        TEXT => "DOMText",
        CDATA => "DOMCdataSection",
        COMMENT => "DOMComment",
        PI => "DOMProcessingInstruction",
        DOCTYPE => "DOMDocumentType",
        FRAGMENT => "DOMDocumentFragment",
        ENTITY_REF => "DOMEntityReference",
        DOCUMENT | HTML_DOCUMENT => "DOMDocument",
        _ => "DOMNode",
    }
}

/// The PHP object of node `id`, made on first use.
pub fn wrap(it: &Interp, doc: &DocRef, id: Id) -> R<Value> {
    if let Some(o) = doc.borrow().wrappers.get(&id).and_then(|w| w.upgrade()) {
        return Ok(Value::Object(o));
    }
    let (class, kind) = {
        let d = doc.borrow();
        let kind = d.n(id).kind;
        let base = base_class(kind);
        (d.classes.get(&base.to_ascii_lowercase()).cloned().unwrap_or_else(|| base.to_string()), kind)
    };
    let _ = kind;
    let cls = it.lookup_class(&class)?;
    let o = it.new_instance(&cls)?;
    o.set_internal(Internal::Data(Box::new(NodeData { doc: doc.clone(), id })));
    doc.borrow_mut().wrappers.insert(id, Rc::downgrade(&o));
    Ok(Value::Object(o))
}

pub fn wrap_opt(it: &Interp, doc: &DocRef, id: Option<Id>) -> R<Value> {
    match id {
        Some(i) => wrap(it, doc, i),
        None => Ok(Value::Null),
    }
}

/// Point an existing wrapper at another node (adoption, moves out of an
/// orphan document).
pub fn rebind(o: &Rc<Obj>, doc: &DocRef, id: Id) {
    if let Some((old, oid)) = node_of(o) {
        old.borrow_mut().wrappers.remove(&oid);
    }
    o.set_internal(Internal::Data(Box::new(NodeData { doc: doc.clone(), id })));
    doc.borrow_mut().wrappers.insert(id, Rc::downgrade(o));
}

pub fn new_list(it: &Interp, doc: Option<DocRef>, kind: ListKind, class: &str) -> R<Value> {
    let cls = it.lookup_class(class)?;
    let o = it.new_instance(&cls)?;
    o.set_internal(Internal::Data(Box::new(ListData::new(doc, kind))));
    Ok(Value::Object(o))
}

fn list_of(it: &Interp, this: Option<&Rc<Obj>>) -> R<(Option<DocRef>, ListKind, Rc<Vec<Id>>)> {
    let o = this.ok_or_else(|| it.error("Error", "Non-static method called statically"))?;
    match &*o.internal() {
        Internal::Data(d) => match d.as_any().downcast_ref::<ListData>() {
            Some(l) => Ok((l.doc.clone(), l.kind.clone(), l.ids())),
            None => Ok((None, ListKind::Empty, Rc::new(vec![]))),
        },
        _ => Ok((None, ListKind::Empty, Rc::new(vec![]))),
    }
}

// ---- errors ----------------------------------------------------------------------

pub const INDEX_SIZE_ERR: i64 = 1;
pub const HIERARCHY_REQUEST_ERR: i64 = 3;
pub const WRONG_DOCUMENT_ERR: i64 = 4;
pub const INVALID_CHARACTER_ERR: i64 = 5;
pub const NOT_FOUND_ERR: i64 = 8;
pub const NOT_SUPPORTED_ERR: i64 = 9;
pub const INUSE_ATTRIBUTE_ERR: i64 = 10;
pub const NAMESPACE_ERR: i64 = 14;

pub fn dom_error(it: &Interp, code: i64) -> Ctrl {
    let msg = match code {
        INDEX_SIZE_ERR => "Index Size Error",
        HIERARCHY_REQUEST_ERR => "Hierarchy Request Error",
        WRONG_DOCUMENT_ERR => "Wrong Document Error",
        INVALID_CHARACTER_ERR => "Invalid Character Error",
        NOT_FOUND_ERR => "Not Found Error",
        NOT_SUPPORTED_ERR => "Not Supported Error",
        INUSE_ATTRIBUTE_ERR => "Inuse Attribute Error",
        NAMESPACE_ERR => "Namespace Error",
        _ => "Unknown Error",
    };
    let c = it.error("DOMException", msg);
    if let Ctrl::Throw(o) = &c {
        o.set_prop("code", Value::Int(code));
    }
    c
}

thread_local! {
    static USE_INTERNAL: Cell<bool> = const { Cell::new(false) };
    /// (level, code, message, file, line)
    static ERRORS: RefCell<Vec<(u8, i64, String, String, u32)>> = const { RefCell::new(Vec::new()) };
}

/// libxml state of one request (server workers).
pub fn reset() {
    USE_INTERNAL.with(|u| u.set(false));
    ERRORS.with(|e| e.borrow_mut().clear());
}

const LIBXML_NOERROR: i64 = 32;
const LIBXML_NOWARNING: i64 = 64;

/// Report parser errors: kept for libxml_get_errors() with internal
/// errors on, else PHP warnings (LIBXML_NOERROR silences them).
pub fn report(it: &Interp, func: &str, file: &str, errors: Vec<html_parse::ParseError>, opts: i64) {
    for e in errors {
        if USE_INTERNAL.with(|u| u.get()) {
            ERRORS.with(|l| l.borrow_mut().push((e.level, e.code, e.message, file.to_string(), e.line)));
            continue;
        }
        let quiet = if e.level == 1 { opts & LIBXML_NOWARNING != 0 } else { opts & LIBXML_NOERROR != 0 };
        if !quiet {
            it.warn(&format!("{func}(): {} in {file}, line: {}", e.message, e.line));
        }
    }
}

fn libxml_error_object(it: &Interp, e: &(u8, i64, String, String, u32)) -> R<Value> {
    let cls = it.lookup_class("LibXMLError")?;
    let o = it.new_instance(&cls)?;
    o.set_prop("level", Value::Int(e.0 as i64));
    o.set_prop("code", Value::Int(e.1));
    o.set_prop("column", Value::Int(0));
    o.set_prop("message", Value::str(&format!("{}\n", e.2)));
    o.set_prop("file", Value::str(&e.3));
    o.set_prop("line", Value::Int(e.4 as i64));
    Ok(Value::Object(o))
}

// ---- small helpers -------------------------------------------------------------

pub fn text(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(String::from_utf8_lossy(&s(it, a, i)?).into_owned())
}

pub fn opt_text(it: &Interp, a: &[Value], i: usize) -> R<Option<String>> {
    if !has(a, i) || matches!(a[i], Value::Null) {
        return Ok(None);
    }
    text(it, a, i).map(Some)
}

/// A node argument: (document, id, object).
pub fn node_arg(it: &Interp, a: &[Value], i: usize, func: &str, param: &str, class: &str) -> R<(DocRef, Id, Rc<Obj>)> {
    if let Some(Value::Object(o)) = a.get(i).map(|v| v.deref()) {
        if let Some((d, id)) = node_of(&o) {
            return Ok((d, id, o));
        }
    }
    let given = a.get(i).map(|v| v.debug_type()).unwrap_or_else(|| "null".into());
    Err(it.error("TypeError", &format!("{func}(): Argument #{} (${param}) must be of type {class}, {given} given", i + 1)))
}

pub fn valid_name(s: &str) -> bool {
    let mut cs = s.chars();
    match cs.next() {
        Some(c) if c.is_alphabetic() || c == '_' || c == ':' || (c as u32) >= 0xC0 => {}
        _ => return false,
    }
    cs.all(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '.' | '-' | '\u{B7}') || (c as u32) >= 0xC0)
}

// ---- registration --------------------------------------------------------------

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    let mut v = vec![
        ("domnodelist", "item", list_item as NativeMethod),
        ("domnodelist", "count", list_count),
        ("domnodelist", "getiterator", list_iterator),
        ("domnodelist", "__get", list_get),
        ("domnamednodemap", "item", list_item),
        ("domnamednodemap", "count", list_count),
        ("domnamednodemap", "getiterator", list_iterator),
        ("domnamednodemap", "__get", list_get),
        ("domnamednodemap", "getnameditem", map_named_item),
        ("domnamednodemap", "getnameditemns", map_named_item_ns),
    ];
    v.extend(node::natives());
    v.extend(document::natives());
    v.extend(props::natives());
    v
}

fn list_item(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<crate::class::Class>, a: Vec<Value>) -> R<Value> {
    let (doc, _, ids) = list_of(it, this)?;
    let i = crate::builtins::int(it, &a, 0)?;
    match (doc, usize::try_from(i).ok().and_then(|i| ids.get(i))) {
        (Some(d), Some(&id)) => wrap(it, &d, id),
        _ => Ok(Value::Null),
    }
}

fn list_count(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<crate::class::Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Int(list_of(it, this)?.2.len() as i64))
}

fn list_get(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<crate::class::Class>, a: Vec<Value>) -> R<Value> {
    let name = text(it, &a, 0)?;
    if name == "length" {
        return Ok(Value::Int(list_of(it, this)?.2.len() as i64));
    }
    it.warn(&format!("Undefined property: {}::${name}", this.map(|o| o.class.name.to_string()).unwrap_or_default()));
    Ok(Value::Null)
}

fn list_iterator(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<crate::class::Class>, _: Vec<Value>) -> R<Value> {
    let (doc, kind, ids) = list_of(it, this)?;
    let mut arr = PhpArray::new();
    if let Some(d) = &doc {
        for &id in ids.iter() {
            let v = wrap(it, d, id)?;
            if matches!(kind, ListKind::Attrs(_)) {
                let name = d.borrow().n(id).name.clone();
                arr.insert(crate::value::Key::from_bytes(name.as_bytes()), v);
            } else {
                arr.push(v);
            }
        }
    }
    let cls = it.lookup_class("ArrayIterator")?;
    let f = crate::interp::Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
    Ok(Value::Object(it.instantiate(&f, &cls, vec![Value::array(arr)], vec![])?))
}

fn map_named_item(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<crate::class::Class>, a: Vec<Value>) -> R<Value> {
    let (doc, _, ids) = list_of(it, this)?;
    let name = text(it, &a, 0)?;
    let Some(d) = doc else { return Ok(Value::Null) };
    let found = ids.iter().copied().find(|&id| d.borrow().n(id).name == name);
    wrap_opt(it, &d, found)
}

fn map_named_item_ns(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<crate::class::Class>, a: Vec<Value>) -> R<Value> {
    let (doc, _, ids) = list_of(it, this)?;
    let ns = opt_text(it, &a, 0)?.filter(|s| !s.is_empty());
    let local = text(it, &a, 1)?;
    let Some(d) = doc else { return Ok(Value::Null) };
    let found = ids.iter().copied().find(|&id| {
        let d = d.borrow();
        let n = d.n(id);
        n.local() == local && n.ns == ns
    });
    wrap_opt(it, &d, found)
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("libxml_use_internal_errors", |it, _, a| {
        let prev = USE_INTERNAL.with(|u| u.get());
        if has(&a, 0) && !matches!(a[0], Value::Null) {
            let on = a[0].truthy();
            USE_INTERNAL.with(|u| u.set(on));
            if !on {
                ERRORS.with(|l| l.borrow_mut().clear());
            }
        }
        let _ = it;
        Ok(Value::Bool(prev))
    });
    t.insert("libxml_get_errors", |it, _, _| {
        let errs = ERRORS.with(|l| l.borrow().clone());
        let mut arr = PhpArray::new();
        for e in &errs {
            arr.push(libxml_error_object(it, e)?);
        }
        Ok(Value::array(arr))
    });
    t.insert("libxml_get_last_error", |it, _, _| match ERRORS.with(|l| l.borrow().last().cloned()) {
        Some(e) => libxml_error_object(it, &e),
        None => Ok(Value::Bool(false)),
    });
    t.insert("libxml_clear_errors", |_, _, _| {
        ERRORS.with(|l| l.borrow_mut().clear());
        Ok(Value::Null)
    });
    t.insert("libxml_disable_entity_loader", |it, _, _| {
        it.deprecated("Function libxml_disable_entity_loader() is deprecated since 8.0, as external entity loading is disabled by default");
        Ok(Value::Bool(true))
    });
    t.insert("libxml_set_external_entity_loader", |_, _, _| Ok(Value::Bool(true)));
    t.insert("libxml_get_external_entity_loader", |_, _, _| Ok(Value::Null));
    t.insert("libxml_set_streams_context", |_, _, _| Ok(Value::Null));
    t.insert("dom_import_simplexml", |it, _, _| Err(it.error("TypeError", "dom_import_simplexml(): SimpleXML is not supported yet")));
}
