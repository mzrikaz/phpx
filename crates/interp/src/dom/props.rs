//! DOM properties (nodeName, childNodes, formatOutput, ...): read and
//! written through __get/__set, and listed for var_dump().

use std::rc::Rc;

use super::tree::*;
use super::*;
use crate::class::Class;

type Args = Vec<Value>;
type This<'a> = Option<&'a Rc<Obj>>;

/// What a property read produces before objects are wrapped.
enum P {
    V(Value),
    Node(Option<Id>),
    List(ListKind, &'static str),
    Impl,
    Missing,
}

fn s(v: &str) -> P {
    P::V(Value::str(v))
}

fn opt_s(v: Option<&str>) -> P {
    P::V(v.map_or(Value::Null, Value::str))
}

fn element_sibling(d: &Doc, mut cur: Option<Id>, next: bool) -> Option<Id> {
    while let Some(c) = cur {
        if d.n(c).kind == ELEMENT {
            return Some(c);
        }
        cur = if next { d.n(c).next } else { d.n(c).prev };
    }
    None
}

pub fn node_name(n: &Node) -> String {
    match n.kind {
        TEXT => "#text".into(),
        CDATA => "#cdata-section".into(),
        COMMENT => "#comment".into(),
        DOCUMENT | HTML_DOCUMENT => "#document".into(),
        FRAGMENT => "#document-fragment".into(),
        _ => n.name.clone(),
    }
}

fn read(d: &Doc, id: Id, name: &str) -> P {
    let n = d.n(id);
    let k = n.kind;
    let is_doc = matches!(k, DOCUMENT | HTML_DOCUMENT);
    let parent_node = matches!(k, ELEMENT | DOCUMENT | HTML_DOCUMENT | FRAGMENT);
    let chardata = matches!(k, TEXT | CDATA | COMMENT);
    match name {
        // DOMNode
        "nodeName" => s(&node_name(n)),
        "nodeValue" => match k {
            ELEMENT => s(&d.text_content(id)),
            TEXT | CDATA | COMMENT | PI | ATTRIBUTE => s(&n.value),
            _ => P::V(Value::Null),
        },
        "nodeType" => P::V(Value::Int(k as i64)),
        "parentNode" => P::Node(if k == ATTRIBUTE { None } else { n.parent }),
        "parentElement" => P::Node(n.parent.filter(|&p| k != ATTRIBUTE && d.n(p).kind == ELEMENT)),
        "childNodes" => P::List(if k == ATTRIBUTE { ListKind::Empty } else { ListKind::Children(id) }, "DOMNodeList"),
        "firstChild" => P::Node(n.first),
        "lastChild" => P::Node(n.last),
        "previousSibling" => P::Node(n.prev),
        "nextSibling" => P::Node(n.next),
        "attributes" => {
            if k == ELEMENT {
                P::List(ListKind::Attrs(id), "DOMNamedNodeMap")
            } else {
                P::V(Value::Null)
            }
        }
        "isConnected" => P::V(Value::Bool(!d.orphan && d.root(id) == 0)),
        "ownerDocument" => {
            if is_doc || d.orphan {
                P::V(Value::Null)
            } else {
                P::Node(Some(0))
            }
        }
        "namespaceURI" => match k {
            ELEMENT | ATTRIBUTE => opt_s(n.ns.as_deref()),
            _ => P::V(Value::Null),
        },
        "prefix" => match k {
            ELEMENT | ATTRIBUTE if n.ns.is_some() => s(n.prefix().unwrap_or("")),
            _ => s(""),
        },
        "localName" => match k {
            ELEMENT | ATTRIBUTE => s(n.local()),
            _ => P::V(Value::Null),
        },
        "baseURI" => opt_s(d.settings.document_uri.as_deref()),
        "textContent" => match k {
            DOCUMENT | HTML_DOCUMENT | DOCTYPE => P::V(Value::Null),
            _ => s(&d.text_content(id)),
        },
        // ParentNode / ChildNode
        "firstElementChild" if parent_node => P::Node(element_sibling(d, n.first, true)),
        "lastElementChild" if parent_node => P::Node(element_sibling(d, n.last, false)),
        "childElementCount" if parent_node => P::V(Value::Int(d.children(id).filter(|&c| d.n(c).kind == ELEMENT).count() as i64)),
        "previousElementSibling" if k == ELEMENT || chardata => P::Node(element_sibling(d, n.prev, false)),
        "nextElementSibling" if k == ELEMENT || chardata => P::Node(element_sibling(d, n.next, true)),
        // DOMElement
        "tagName" if k == ELEMENT => s(&n.name),
        "className" if k == ELEMENT => s(d.get_attr(id, "class").unwrap_or("")),
        "id" if k == ELEMENT => s(d.get_attr(id, "id").unwrap_or("")),
        "schemaTypeInfo" if k == ELEMENT || k == ATTRIBUTE => P::V(Value::Null),
        // DOMAttr
        "name" if k == ATTRIBUTE => s(&n.name),
        "value" if k == ATTRIBUTE => s(&n.value),
        "specified" if k == ATTRIBUTE => P::V(Value::Bool(true)),
        "ownerElement" if k == ATTRIBUTE => P::Node(n.parent),
        // DOMCharacterData, DOMText, DOMProcessingInstruction
        "data" if chardata || k == PI => s(&n.value),
        "length" if chardata => P::V(Value::Int(n.value.chars().count() as i64)),
        "wholeText" if k == TEXT || k == CDATA => {
            let mut first = id;
            while let Some(p) = d.n(first).prev.filter(|&p| d.n(p).is_text()) {
                first = p;
            }
            let mut out = String::new();
            let mut cur = Some(first);
            while let Some(c) = cur.filter(|&c| d.n(c).is_text()) {
                out.push_str(&d.n(c).value);
                cur = d.n(c).next;
            }
            s(&out)
        }
        "target" if k == PI => s(&n.name),
        // DOMDocumentType
        "name" if k == DOCTYPE => s(&n.name),
        "publicId" if k == DOCTYPE => s(&n.ids.as_ref().unwrap().0),
        "systemId" if k == DOCTYPE => s(&n.ids.as_ref().unwrap().1),
        "internalSubset" if k == DOCTYPE => {
            let sub = &n.ids.as_ref().unwrap().2;
            if sub.is_empty() {
                P::V(Value::Null)
            } else {
                s(sub)
            }
        }
        "entities" | "notations" if k == DOCTYPE => P::List(ListKind::Empty, "DOMNamedNodeMap"),
        // DOMDocument
        "doctype" if is_doc => P::Node(d.doctype()),
        "implementation" if is_doc => P::Impl,
        "documentElement" if is_doc => P::Node(d.document_element()),
        "actualEncoding" | "encoding" | "xmlEncoding" if is_doc => opt_s(d.encoding.as_deref()),
        "standalone" | "xmlStandalone" if is_doc => P::V(Value::Bool(d.standalone == 1)),
        "version" | "xmlVersion" if is_doc => opt_s(d.version.as_deref()),
        "strictErrorChecking" if is_doc => P::V(Value::Bool(d.settings.strict_error_checking)),
        "documentURI" if is_doc => opt_s(d.settings.document_uri.as_deref()),
        "config" if is_doc => P::V(Value::Null),
        "formatOutput" if is_doc => P::V(Value::Bool(d.settings.format_output)),
        "validateOnParse" if is_doc => P::V(Value::Bool(d.settings.validate_on_parse)),
        "resolveExternals" if is_doc => P::V(Value::Bool(d.settings.resolve_externals)),
        "preserveWhiteSpace" if is_doc => P::V(Value::Bool(d.settings.preserve_white_space)),
        "recover" if is_doc => P::V(Value::Bool(d.settings.recover)),
        "substituteEntities" if is_doc => P::V(Value::Bool(d.settings.substitute_entities)),
        _ => P::Missing,
    }
}

/// Property names per node kind, in PHP's var_dump() order.
fn names(kind: u8) -> Vec<&'static str> {
    let node = [
        "nodeName", "nodeValue", "nodeType", "parentNode", "parentElement", "childNodes", "firstChild", "lastChild", "previousSibling", "nextSibling", "attributes", "isConnected", "ownerDocument",
        "namespaceURI", "prefix", "localName", "baseURI", "textContent",
    ];
    let head: &[&str] = match kind {
        ELEMENT => &["tagName", "className", "id", "schemaTypeInfo", "firstElementChild", "lastElementChild", "childElementCount", "previousElementSibling", "nextElementSibling"],
        ATTRIBUTE => &["name", "specified", "value", "ownerElement", "schemaTypeInfo"],
        TEXT | CDATA => &["wholeText", "data", "length", "previousElementSibling", "nextElementSibling"],
        COMMENT => &["data", "length", "previousElementSibling", "nextElementSibling"],
        PI => &["target", "data"],
        DOCTYPE => &["name", "entities", "notations", "publicId", "systemId", "internalSubset"],
        FRAGMENT => &["firstElementChild", "lastElementChild", "childElementCount"],
        DOCUMENT | HTML_DOCUMENT => &[
            "doctype", "implementation", "documentElement", "actualEncoding", "encoding", "xmlEncoding", "standalone", "xmlStandalone", "version", "xmlVersion", "strictErrorChecking", "documentURI",
            "config", "formatOutput", "validateOnParse", "resolveExternals", "preserveWhiteSpace", "recover", "substituteEntities", "firstElementChild", "lastElementChild", "childElementCount",
        ],
        _ => &[],
    };
    head.iter().copied().chain(node).collect()
}

pub fn dump(d: &Doc, id: Id) -> Vec<(Rc<str>, Value)> {
    let omitted = || Value::str("(object value omitted)");
    names(d.n(id).kind)
        .into_iter()
        .map(|name| {
            let v = match read(d, id, name) {
                P::V(v) => v,
                P::Node(None) => Value::Null,
                P::Node(Some(_)) | P::List(..) | P::Impl => omitted(),
                P::Missing => Value::Null,
            };
            (Rc::from(name), v)
        })
        .collect()
}

fn get(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    let p = read(&doc.borrow(), id, &name);
    match p {
        P::V(v) => Ok(v),
        P::Node(n) => wrap_opt(it, &doc, n),
        P::List(kind, class) => new_list(it, Some(doc.clone()), kind, class),
        P::Impl => {
            let cls = it.lookup_class("DOMImplementation")?;
            Ok(Value::Object(it.new_instance(&cls)?))
        }
        P::Missing => {
            let o = this.unwrap();
            if let Some(v) = o.get_prop(&name) {
                return Ok(v);
            }
            it.warn(&format!("Undefined property: {}::${name}", o.class.name));
            Ok(Value::Null)
        }
    }
}

fn isset(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    let p = read(&doc.borrow(), id, &name);
    Ok(Value::Bool(match p {
        P::V(v) => !matches!(v, Value::Null),
        P::Node(n) => n.is_some(),
        P::List(..) | P::Impl => true,
        P::Missing => this.unwrap().get_prop(&name).is_some_and(|v| !matches!(v, Value::Null)),
    }))
}

/// Replace `id`'s children with one text node (none for "").
fn set_text_children(d: &mut Doc, id: Id, v: &str) {
    let kids: Vec<Id> = d.children(id).collect();
    for k in kids {
        d.detach(k);
    }
    if !v.is_empty() {
        let t = d.add(Node::new(TEXT, "#text", v));
        d.append(id, t);
    }
}

fn set(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    let val = a.get(1).cloned().unwrap_or_default();
    let sval = || -> R<String> { Ok(crate::pstr::lossy(&it.to_str(&val)?).into_owned()) };
    let o = this.unwrap();
    let mut d = doc.borrow_mut();
    let k = d.n(id).kind;
    let is_doc = matches!(k, DOCUMENT | HTML_DOCUMENT);
    let chardata = matches!(k, TEXT | CDATA | COMMENT);
    match name.as_str() {
        "nodeValue" | "textContent" => match k {
            ELEMENT | FRAGMENT => set_text_children(&mut d, id, &sval()?),
            TEXT | CDATA | COMMENT | PI => d.m(id).value = sval()?,
            ATTRIBUTE => d.m(id).value = sval()?,
            _ => {}
        },
        "data" if chardata || k == PI => d.m(id).value = sval()?,
        "value" if k == ATTRIBUTE => {
            d.m(id).value = sval()?;
            d.m(id).raw = false;
        }
        "className" if k == ELEMENT => {
            let v = sval()?;
            d.set_attr(id, "class", &v);
        }
        "id" if k == ELEMENT => {
            let v = sval()?;
            d.set_attr(id, "id", &v);
        }
        "prefix" if matches!(k, ELEMENT | ATTRIBUTE) => {
            let p = sval()?;
            if d.n(id).ns.is_none() {
                if !p.is_empty() {
                    return Err(dom_error(it, NAMESPACE_ERR));
                }
            } else {
                let local = d.n(id).local().to_string();
                let uri = d.n(id).ns.clone().unwrap();
                d.m(id).name = if p.is_empty() { local } else { format!("{p}:{local}") };
                if k == ELEMENT {
                    let pref = if p.is_empty() { None } else { Some(p) };
                    if d.lookup_ns(id, pref.as_deref()).as_deref() != Some(uri.as_str()) {
                        d.m(id).ns_defs.push((pref, uri));
                    }
                }
            }
        }
        "formatOutput" if is_doc => d.settings.format_output = val.truthy(),
        "validateOnParse" if is_doc => d.settings.validate_on_parse = val.truthy(),
        "resolveExternals" if is_doc => d.settings.resolve_externals = val.truthy(),
        "preserveWhiteSpace" if is_doc => d.settings.preserve_white_space = val.truthy(),
        "recover" if is_doc => d.settings.recover = val.truthy(),
        "substituteEntities" if is_doc => d.settings.substitute_entities = val.truthy(),
        "strictErrorChecking" if is_doc => d.settings.strict_error_checking = val.truthy(),
        "encoding" | "xmlEncoding" if is_doc => d.encoding = if matches!(val, Value::Null) { None } else { Some(sval()?) },
        "version" | "xmlVersion" if is_doc => d.version = if matches!(val, Value::Null) { None } else { Some(sval()?) },
        "standalone" | "xmlStandalone" if is_doc => d.standalone = if val.truthy() { 1 } else { 0 },
        "documentURI" if is_doc => d.settings.document_uri = if matches!(val, Value::Null) { None } else { Some(sval()?) },
        _ => {
            let known = !matches!(read(&d, id, &name), P::Missing);
            drop(d);
            if known {
                return Err(it.error("Error", &format!("Cannot modify readonly property {}::${name}", o.class.name)));
            }
            o.set_prop(&name, val);
        }
    }
    Ok(Value::Null)
}

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![("domnode", "__get", get as NativeMethod), ("domnode", "__set", set), ("domnode", "__isset", isset)]
}
