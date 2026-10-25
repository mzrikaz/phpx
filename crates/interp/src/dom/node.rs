//! Methods of DOMNode, DOMElement, DOMAttr, DOMCharacterData, DOMText and
//! the other node classes.

use std::rc::Rc;

use super::tree::*;
use super::*;
use crate::builtins::int;
use crate::class::Class;

type Args = Vec<Value>;
type This<'a> = Option<&'a Rc<Obj>>;

const NO_MODIFICATION_ALLOWED_ERR: i64 = 7;

/// Make `id` (from `src`) usable in `doc`: same document, or moved out of
/// a constructed node's private document.
pub fn adopt(it: &Interp, doc: &DocRef, src: &DocRef, id: Id, obj: Option<&Rc<Obj>>) -> R<Id> {
    if Rc::ptr_eq(doc, src) {
        return Ok(id);
    }
    if !src.borrow().orphan {
        return Err(dom_error(it, WRONG_DOCUMENT_ERR));
    }
    let new = {
        let s = src.borrow();
        doc.borrow_mut().import(&s, id, true)
    };
    src.borrow_mut().detach(id);
    if let Some(o) = obj {
        rebind(o, doc, new);
    }
    Ok(new)
}

/// Checks before inserting `node` under `parent`.
fn check_insert(it: &Interp, d: &Doc, parent: Id, node: Id) -> R<()> {
    if d.orphan {
        // A `new DOMElement()` is read-only until it is in a document.
        return Err(dom_error(it, NO_MODIFICATION_ALLOWED_ERR));
    }
    let pk = d.n(parent).kind;
    if !matches!(pk, ELEMENT | DOCUMENT | HTML_DOCUMENT | FRAGMENT) {
        return Err(dom_error(it, HIERARCHY_REQUEST_ERR));
    }
    if d.is_ancestor(node, parent) || matches!(d.n(node).kind, DOCUMENT | HTML_DOCUMENT) {
        return Err(dom_error(it, HIERARCHY_REQUEST_ERR));
    }
    Ok(())
}

/// Insert `node` (a fragment inserts its children) before `before`.
pub fn insert_node(d: &mut Doc, parent: Id, node: Id, before: Option<Id>) {
    if d.n(node).kind == FRAGMENT {
        let kids: Vec<Id> = d.children(node).collect();
        for k in kids {
            d.insert(parent, k, before);
        }
    } else if d.n(node).kind == ATTRIBUTE {
        d.detach(node);
        let name = d.n(node).name.clone();
        if let Some(old) = d.attr(parent, &name) {
            d.detach(old);
        }
        d.m(node).parent = Some(parent);
        d.m(parent).attrs.push(node);
        d.rev += 1;
    } else {
        d.insert(parent, node, before);
    }
}

fn arg_obj(a: &[Value], i: usize) -> Option<Rc<Obj>> {
    match a.get(i).map(|v| v.deref()) {
        Some(Value::Object(o)) => Some(o),
        _ => None,
    }
}

fn append_child(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, parent) = this_node(it, this)?;
    let (src, id, obj) = node_arg(it, &a, 0, "DOMNode::appendChild", "node", "DOMNode")?;
    let id = adopt(it, &doc, &src, id, Some(&obj))?;
    let mut d = doc.borrow_mut();
    check_insert(it, &d, parent, id)?;
    insert_node(&mut d, parent, id, None);
    Ok(Value::Object(obj))
}

fn insert_before(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, parent) = this_node(it, this)?;
    let (src, id, obj) = node_arg(it, &a, 0, "DOMNode::insertBefore", "node", "DOMNode")?;
    let before = match arg_obj(&a, 1).and_then(|o| node_of(&o)) {
        Some((rd, r)) => {
            if !Rc::ptr_eq(&rd, &doc) || doc.borrow().n(r).parent != Some(parent) {
                return Err(dom_error(it, NOT_FOUND_ERR));
            }
            Some(r)
        }
        None => None,
    };
    let id = adopt(it, &doc, &src, id, Some(&obj))?;
    if Some(id) == before {
        return Ok(Value::Object(obj));
    }
    let mut d = doc.borrow_mut();
    check_insert(it, &d, parent, id)?;
    insert_node(&mut d, parent, id, before);
    Ok(Value::Object(obj))
}

fn remove_child(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, parent) = this_node(it, this)?;
    let (cd, id, obj) = node_arg(it, &a, 0, "DOMNode::removeChild", "child", "DOMNode")?;
    if !Rc::ptr_eq(&cd, &doc) || doc.borrow().n(id).parent != Some(parent) || doc.borrow().n(id).kind == ATTRIBUTE {
        return Err(dom_error(it, NOT_FOUND_ERR));
    }
    doc.borrow_mut().detach(id);
    Ok(Value::Object(obj))
}

fn replace_child(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, parent) = this_node(it, this)?;
    let (src, new, nobj) = node_arg(it, &a, 0, "DOMNode::replaceChild", "node", "DOMNode")?;
    let (od, old, oobj) = node_arg(it, &a, 1, "DOMNode::replaceChild", "child", "DOMNode")?;
    if !Rc::ptr_eq(&od, &doc) || doc.borrow().n(old).parent != Some(parent) {
        return Err(dom_error(it, NOT_FOUND_ERR));
    }
    let new = adopt(it, &doc, &src, new, Some(&nobj))?;
    if new == old {
        return Ok(Value::Object(oobj));
    }
    let mut d = doc.borrow_mut();
    check_insert(it, &d, parent, new)?;
    insert_node(&mut d, parent, new, Some(old));
    d.detach(old);
    Ok(Value::Object(oobj))
}

fn clone_node(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let deep = a.first().is_some_and(|v| v.truthy());
    if id == 0 {
        let mut copy = doc.borrow().clone_tree();
        if !deep {
            let mut fresh = Doc::new(copy.html);
            fresh.encoding = copy.encoding.take();
            fresh.version = copy.version.take();
            fresh.standalone = copy.standalone;
            fresh.settings = copy.settings.clone();
            copy = fresh;
        }
        copy.classes = doc.borrow().classes.clone();
        let nd = Rc::new(RefCell::new(copy));
        let o = it.new_instance(&this.unwrap().class)?;
        o.set_internal(Internal::Data(Box::new(NodeData { doc: nd.clone(), id: 0 })));
        nd.borrow_mut().wrappers.insert(0, Rc::downgrade(&o));
        return Ok(Value::Object(o));
    }
    let new = doc.borrow_mut().copy_within(id, deep);
    wrap(it, &doc, new)
}

fn has_child_nodes(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let v = Value::Bool(doc.borrow().n(id).first.is_some());
    Ok(v)
}

fn has_attributes(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let d = doc.borrow();
    Ok(Value::Bool(d.n(id).kind == ELEMENT && (!d.n(id).attrs.is_empty() || !d.n(id).ns_defs.is_empty())))
}

fn normalize_in(d: &mut Doc, id: Id) {
    let mut cur = d.n(id).first;
    while let Some(c) = cur {
        let next = d.n(c).next;
        if d.n(c).kind == TEXT {
            if d.n(c).value.is_empty() {
                d.detach(c);
            } else if let Some(nx) = next.filter(|&n| d.n(n).kind == TEXT) {
                let v = d.n(nx).value.clone();
                d.m(c).value.push_str(&v);
                d.detach(nx);
                continue;
            }
        } else if d.n(c).kind == ELEMENT {
            normalize_in(d, c);
        }
        cur = next;
    }
}

fn normalize(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    normalize_in(&mut doc.borrow_mut(), id);
    Ok(Value::Null)
}

fn is_same_node(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    Ok(Value::Bool(arg_obj(&a, 0).and_then(|o| node_of(&o)).is_some_and(|(d, i)| Rc::ptr_eq(&d, &doc) && i == id)))
}

fn equal(a: &Doc, x: Id, b: &Doc, y: Id) -> bool {
    let (n, m) = (a.n(x), b.n(y));
    if n.kind != m.kind || n.name != m.name || n.ns != m.ns || n.value != m.value || n.attrs.len() != m.attrs.len() {
        return false;
    }
    for &at in &n.attrs {
        let an = a.n(at);
        match b.attr_ns(y, an.ns.as_deref(), an.local()) {
            Some(bt) if b.n(bt).value == an.value => {}
            _ => return false,
        }
    }
    let ka: Vec<Id> = a.children(x).collect();
    let kb: Vec<Id> = b.children(y).collect();
    ka.len() == kb.len() && ka.iter().zip(&kb).all(|(&p, &q)| equal(a, p, b, q))
}

fn is_equal_node(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let Some((od, oid)) = arg_obj(&a, 0).and_then(|o| node_of(&o)) else { return Ok(Value::Bool(false)) };
    if Rc::ptr_eq(&doc, &od) {
        let d = doc.borrow();
        return Ok(Value::Bool(equal(&d, id, &d, oid)));
    }
    let v = Value::Bool(equal(&doc.borrow(), id, &od.borrow(), oid));
    Ok(v)
}

/// The element namespace lookups start from.
fn ns_context(d: &Doc, id: Id) -> Option<Id> {
    match d.n(id).kind {
        DOCUMENT | HTML_DOCUMENT => d.document_element(),
        ATTRIBUTE => d.n(id).parent,
        ELEMENT => Some(id),
        _ => d.n(id).parent,
    }
}

fn lookup_namespace_uri(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let prefix = opt_text(it, &a, 0)?.filter(|p| !p.is_empty());
    let d = doc.borrow();
    Ok(match ns_context(&d, id).and_then(|e| d.lookup_ns(e, prefix.as_deref())) {
        Some(u) => Value::str(&u),
        None => Value::Null,
    })
}

fn lookup_prefix(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let uri = text(it, &a, 0)?;
    let d = doc.borrow();
    Ok(match ns_context(&d, id).and_then(|e| d.lookup_prefix(e, &uri)) {
        Some(Some(p)) => Value::str(&p),
        _ => Value::Null,
    })
}

fn is_default_namespace(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let uri = text(it, &a, 0)?;
    let d = doc.borrow();
    Ok(Value::Bool(ns_context(&d, id).and_then(|e| d.lookup_ns(e, None)).is_some_and(|u| u == uri)))
}

/// xmlGetNodePath.
pub fn node_path(d: &Doc, id: Id) -> String {
    let mut parts: Vec<String> = vec![];
    let mut cur = id;
    loop {
        let n = d.n(cur);
        let same = |pred: &dyn Fn(&Node) -> bool| -> (usize, usize) {
            let Some(p) = n.parent else { return (1, 1) };
            let mut idx = 0;
            let mut total = 0;
            for c in d.children(p) {
                if pred(d.n(c)) {
                    total += 1;
                    if c == cur {
                        idx = total;
                    }
                }
            }
            (idx, total)
        };
        let step = match n.kind {
            DOCUMENT | HTML_DOCUMENT => break,
            ELEMENT => {
                let (i, t) = same(&|m: &Node| m.kind == ELEMENT && m.name == n.name);
                let name = if n.ns.is_some() && n.prefix().is_none() { "*".to_string() } else { n.name.clone() };
                if n.ns.is_some() && n.prefix().is_none() {
                    let (i, t) = same(&|m: &Node| m.kind == ELEMENT);
                    if t > 1 { format!("*[{i}]") } else { name }
                } else if t > 1 {
                    format!("{name}[{i}]")
                } else {
                    name
                }
            }
            TEXT | CDATA => {
                let (i, t) = same(&|m: &Node| m.is_text());
                if t > 1 { format!("text()[{i}]") } else { "text()".into() }
            }
            COMMENT => {
                let (i, t) = same(&|m: &Node| m.kind == COMMENT);
                if t > 1 { format!("comment()[{i}]") } else { "comment()".into() }
            }
            PI => {
                let (i, t) = same(&|m: &Node| m.kind == PI);
                if t > 1 { format!("processing-instruction('{}')[{i}]", n.name) } else { format!("processing-instruction('{}')", n.name) }
            }
            ATTRIBUTE => format!("@{}", n.name),
            _ => break,
        };
        parts.push(step);
        match n.parent {
            Some(p) => cur = p,
            None => break,
        }
    }
    parts.reverse();
    format!("/{}", parts.join("/"))
}

fn get_node_path(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let v = Value::str(&node_path(&doc.borrow(), id));
    Ok(v)
}

fn get_line_no(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let v = Value::Int(doc.borrow().n(id).line as i64);
    Ok(v)
}

fn contains(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    Ok(Value::Bool(arg_obj(&a, 0).and_then(|o| node_of(&o)).is_some_and(|(d, o)| Rc::ptr_eq(&d, &doc) && doc.borrow().is_ancestor(id, o))))
}

fn get_root_node(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let r = doc.borrow().root(id);
    wrap(it, &doc, r)
}

fn compare_document_position(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let (od, other, _) = node_arg(it, &a, 0, "DOMNode::compareDocumentPosition", "other", "DOMNode")?;
    if Rc::ptr_eq(&doc, &od) && id == other {
        return Ok(Value::Int(0));
    }
    let d = doc.borrow();
    if !Rc::ptr_eq(&doc, &od) || d.root(id) != d.root(other) {
        let order = if Rc::as_ptr(&doc) < Rc::as_ptr(&od) { 4 } else { 2 };
        return Ok(Value::Int(1 | 32 | order));
    }
    if d.is_ancestor(other, id) {
        return Ok(Value::Int(8 | 2));
    }
    if d.is_ancestor(id, other) {
        return Ok(Value::Int(16 | 4));
    }
    let all = d.descendants(d.root(id));
    let (pi, po) = (all.iter().position(|&x| x == id), all.iter().position(|&x| x == other));
    Ok(Value::Int(if po < pi { 2 } else { 4 }))
}

// ---- ParentNode / ChildNode ------------------------------------------------------

/// Nodes and strings (as text nodes) for append(), before(), ...
fn node_list_args(it: &Interp, doc: &DocRef, a: &[Value], func: &str) -> R<Vec<Id>> {
    let mut out = vec![];
    for (i, v) in a.iter().enumerate() {
        match v.deref() {
            Value::Object(o) => {
                let Some((src, id)) = node_of(&o) else {
                    return Err(it.error("TypeError", &format!("{func}(): Argument #{} must be of type DOMNode|string, {} given", i + 1, o.class.name)));
                };
                out.push(adopt(it, doc, &src, id, Some(&o))?);
            }
            v => {
                let s = String::from_utf8_lossy(&it.to_str(&v)?).into_owned();
                let t = doc.borrow_mut().add(Node::new(TEXT, "#text", &s));
                out.push(t);
            }
        }
    }
    Ok(out)
}

fn insert_all(it: &Interp, doc: &DocRef, parent: Id, nodes: &[Id], before: Option<Id>) -> R<()> {
    let mut d = doc.borrow_mut();
    for &n in nodes {
        check_insert(it, &d, parent, n)?;
    }
    for &n in nodes {
        insert_node(&mut d, parent, n, before);
    }
    Ok(())
}

fn append(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let nodes = node_list_args(it, &doc, &a, "DOMElement::append")?;
    insert_all(it, &doc, id, &nodes, None)?;
    Ok(Value::Null)
}

fn prepend(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let nodes = node_list_args(it, &doc, &a, "DOMElement::prepend")?;
    let first = doc.borrow().n(id).first;
    insert_all(it, &doc, id, &nodes, first)?;
    Ok(Value::Null)
}

fn replace_children(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let nodes = node_list_args(it, &doc, &a, "DOMElement::replaceChildren")?;
    {
        let mut d = doc.borrow_mut();
        let kids: Vec<Id> = d.children(id).filter(|k| !nodes.contains(k)).collect();
        for k in kids {
            d.detach(k);
        }
    }
    insert_all(it, &doc, id, &nodes, None)?;
    Ok(Value::Null)
}

fn before(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let nodes = node_list_args(it, &doc, &a, "DOMElement::before")?;
    let Some(parent) = doc.borrow().n(id).parent else { return Ok(Value::Null) };
    insert_all(it, &doc, parent, &nodes, Some(id))?;
    Ok(Value::Null)
}

fn after(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let nodes = node_list_args(it, &doc, &a, "DOMElement::after")?;
    let (parent, next) = {
        let d = doc.borrow();
        (d.n(id).parent, d.n(id).next)
    };
    let Some(parent) = parent else { return Ok(Value::Null) };
    let next = next.filter(|n| !nodes.contains(n));
    insert_all(it, &doc, parent, &nodes, next)?;
    Ok(Value::Null)
}

fn replace_with(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let nodes = node_list_args(it, &doc, &a, "DOMElement::replaceWith")?;
    let Some(parent) = doc.borrow().n(id).parent else { return Ok(Value::Null) };
    if !nodes.contains(&id) {
        insert_all(it, &doc, parent, &nodes, Some(id))?;
        doc.borrow_mut().detach(id);
    } else {
        insert_all(it, &doc, parent, &nodes, None)?;
    }
    Ok(Value::Null)
}

fn remove(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    doc.borrow_mut().detach(id);
    Ok(Value::Null)
}

// ---- DOMElement -------------------------------------------------------------------

fn ns_decl_value(d: &Doc, el: Id, name: &str) -> Option<String> {
    let prefix = if name == "xmlns" { None } else { Some(name.strip_prefix("xmlns:")?) };
    d.n(el).ns_defs.iter().find(|(p, _)| p.as_deref() == prefix).map(|(_, u)| u.clone())
}

fn get_attribute(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    let d = doc.borrow();
    if let Some(v) = d.get_attr(id, &name) {
        return Ok(Value::str(v));
    }
    Ok(Value::str(&ns_decl_value(&d, id, &name).unwrap_or_default()))
}

fn has_attribute(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    let d = doc.borrow();
    Ok(Value::Bool(d.attr(id, &name).is_some() || ns_decl_value(&d, id, &name).is_some()))
}

fn set_attribute(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    let value = text(it, &a, 1)?;
    if !valid_name(&name) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    if name == "xmlns" || name.starts_with("xmlns:") {
        let prefix = name.strip_prefix("xmlns:").map(str::to_string);
        let mut d = doc.borrow_mut();
        let defs = &mut d.m(id).ns_defs;
        match defs.iter_mut().find(|(p, _)| *p == prefix) {
            Some(e) => e.1 = value,
            None => defs.push((prefix, value)),
        }
        return Ok(Value::Bool(true));
    }
    let attr = {
        let mut d = doc.borrow_mut();
        let a = d.set_attr(id, &name, &value);
        d.m(a).raw = false;
        a
    };
    wrap(it, &doc, attr)
}

fn remove_attribute(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    let mut d = doc.borrow_mut();
    if let Some(at) = d.attr(id, &name) {
        d.detach(at);
        return Ok(Value::Bool(true));
    }
    if name == "xmlns" || name.starts_with("xmlns:") {
        let prefix = name.strip_prefix("xmlns:").map(str::to_string);
        let before = d.n(id).ns_defs.len();
        d.m(id).ns_defs.retain(|(p, _)| *p != prefix);
        return Ok(Value::Bool(d.n(id).ns_defs.len() != before));
    }
    Ok(Value::Bool(false))
}

fn toggle_attribute(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    if !valid_name(&name) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    let force = a.get(1).filter(|v| !matches!(v, Value::Null)).map(|v| v.truthy());
    let mut d = doc.borrow_mut();
    match (d.attr(id, &name), force) {
        (Some(at), None) | (Some(at), Some(false)) => {
            d.detach(at);
            Ok(Value::Bool(false))
        }
        (Some(_), Some(true)) => Ok(Value::Bool(true)),
        (None, Some(false)) => Ok(Value::Bool(false)),
        (None, _) => {
            d.set_attr(id, &name, "");
            Ok(Value::Bool(true))
        }
    }
}

fn get_attribute_node(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    let found = doc.borrow().attr(id, &name);
    match found {
        Some(at) => wrap(it, &doc, at),
        None => Ok(Value::Bool(false)),
    }
}

fn set_attribute_node(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let (src, at, obj) = node_arg(it, &a, 0, "DOMElement::setAttributeNode", "attr", "DOMAttr")?;
    let at = adopt(it, &doc, &src, at, Some(&obj))?;
    let mut d = doc.borrow_mut();
    if let Some(owner) = d.n(at).parent {
        if owner != id {
            return Err(dom_error(it, INUSE_ATTRIBUTE_ERR));
        }
        return Ok(Value::Null);
    }
    let (name, ns) = (d.n(at).name.clone(), d.n(at).ns.clone());
    let old = match &ns {
        Some(u) => {
            let local = d.n(at).local().to_string();
            d.attr_ns(id, Some(u), &local)
        }
        None => d.attr(id, &name),
    };
    if let Some(o) = old {
        d.detach(o);
    }
    d.m(at).parent = Some(id);
    d.m(id).attrs.push(at);
    d.rev += 1;
    drop(d);
    match old {
        Some(o) => wrap(it, &doc, o),
        None => Ok(Value::Null),
    }
}

fn remove_attribute_node(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let (_, at, obj) = node_arg(it, &a, 0, "DOMElement::removeAttributeNode", "attr", "DOMAttr")?;
    if doc.borrow().n(at).parent != Some(id) {
        return Err(dom_error(it, NOT_FOUND_ERR));
    }
    doc.borrow_mut().detach(at);
    Ok(Value::Object(obj))
}

fn ns_arg(it: &Interp, a: &[Value], i: usize) -> R<Option<String>> {
    Ok(opt_text(it, a, i)?.filter(|s| !s.is_empty()))
}

fn get_attribute_ns(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let ns = ns_arg(it, &a, 0)?;
    let local = text(it, &a, 1)?;
    let d = doc.borrow();
    if ns.as_deref() == Some("http://www.w3.org/2000/xmlns/") {
        let prefix = if local == "xmlns" { None } else { Some(local.clone()) };
        return Ok(Value::str(&d.n(id).ns_defs.iter().find(|(p, _)| *p == prefix).map(|(_, u)| u.clone()).unwrap_or_default()));
    }
    Ok(Value::str(d.attr_ns(id, ns.as_deref(), &local).map(|at| d.n(at).value.as_str()).unwrap_or("")))
}

fn has_attribute_ns(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let ns = ns_arg(it, &a, 0)?;
    let local = text(it, &a, 1)?;
    let v = Value::Bool(doc.borrow().attr_ns(id, ns.as_deref(), &local).is_some());
    Ok(v)
}

fn get_attribute_node_ns(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let ns = ns_arg(it, &a, 0)?;
    let local = text(it, &a, 1)?;
    let found = doc.borrow().attr_ns(id, ns.as_deref(), &local);
    wrap_opt(it, &doc, found)
}

/// Declare `prefix` -> `uri` on `el` unless it is already in scope.
fn ensure_ns(d: &mut Doc, el: Id, prefix: Option<&str>, uri: &str) {
    if d.lookup_ns(el, prefix).as_deref() == Some(uri) {
        return;
    }
    if prefix == Some("xml") {
        return;
    }
    let defs = &mut d.m(el).ns_defs;
    match defs.iter_mut().find(|(p, _)| p.as_deref() == prefix) {
        Some(e) => e.1 = uri.to_string(),
        None => defs.push((prefix.map(str::to_string), uri.to_string())),
    }
}

fn set_attribute_ns(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let ns = ns_arg(it, &a, 0)?;
    let qname = text(it, &a, 1)?;
    let value = text(it, &a, 2)?;
    if !valid_name(&qname) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    let (prefix, local) = match qname.split_once(':') {
        Some((p, l)) => (Some(p.to_string()), l.to_string()),
        None => (None, qname.clone()),
    };
    let mut d = doc.borrow_mut();
    if ns.as_deref() == Some("http://www.w3.org/2000/xmlns/") {
        let p = if prefix.is_none() { None } else { Some(local.clone()) };
        let defs = &mut d.m(id).ns_defs;
        match defs.iter_mut().find(|(q, _)| *q == p) {
            Some(e) => e.1 = value,
            None => defs.push((p, value)),
        }
        return Ok(Value::Null);
    }
    if prefix.is_some() && ns.is_none() {
        return Err(dom_error(it, NAMESPACE_ERR));
    }
    if let (Some(u), Some(p)) = (&ns, &prefix) {
        ensure_ns(&mut d, id, Some(p), u);
    }
    let existing = d.attr_ns(id, ns.as_deref(), &local);
    let at = match existing {
        Some(at) => {
            d.m(at).value = value;
            d.m(at).name = qname;
            at
        }
        None => {
            let mut n = Node::new(ATTRIBUTE, &qname, &value);
            n.ns = ns;
            n.parent = Some(id);
            let at = d.add(n);
            d.m(id).attrs.push(at);
            at
        }
    };
    d.rev += 1;
    let _ = at;
    Ok(Value::Null)
}

fn remove_attribute_ns(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let ns = ns_arg(it, &a, 0)?;
    let local = text(it, &a, 1)?;
    let mut d = doc.borrow_mut();
    if let Some(at) = d.attr_ns(id, ns.as_deref(), &local) {
        d.detach(at);
    }
    Ok(Value::Null)
}

fn get_attribute_names(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let d = doc.borrow();
    let mut out = PhpArray::new();
    for (p, _) in &d.n(id).ns_defs {
        out.push(Value::str(&match p {
            Some(p) => format!("xmlns:{p}"),
            None => "xmlns".into(),
        }));
    }
    for &at in &d.n(id).attrs {
        out.push(Value::str(&d.n(at).name));
    }
    Ok(Value::array(out))
}

fn tag_list(it: &Interp, doc: &DocRef, root: Id, ns: Option<Option<String>>, name: String) -> R<Value> {
    new_list(it, Some(doc.clone()), ListKind::Tag { root, ns, name }, "DOMNodeList")
}

pub fn get_elements_by_tag_name(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    tag_list(it, &doc, id, None, name)
}

pub fn get_elements_by_tag_name_ns(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let ns = opt_text(it, &a, 0)?;
    let local = text(it, &a, 1)?;
    let ns = match ns {
        Some(s) if s == "*" => Some(Some("*".to_string())),
        Some(s) if s.is_empty() => Some(None),
        Some(s) => Some(Some(s)),
        None => Some(None),
    };
    tag_list(it, &doc, id, ns, local)
}

fn set_id_attribute(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let name = text(it, &a, 0)?;
    let is_id = a.get(1).is_some_and(|v| v.truthy());
    let mut d = doc.borrow_mut();
    match d.attr(id, &name) {
        Some(at) => {
            d.m(at).is_id = is_id;
            Ok(Value::Null)
        }
        None => Err(dom_error(it, NOT_FOUND_ERR)),
    }
}

fn set_id_attribute_ns(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let ns = ns_arg(it, &a, 0)?;
    let local = text(it, &a, 1)?;
    let is_id = a.get(2).is_some_and(|v| v.truthy());
    let mut d = doc.borrow_mut();
    match d.attr_ns(id, ns.as_deref(), &local) {
        Some(at) => {
            d.m(at).is_id = is_id;
            Ok(Value::Null)
        }
        None => Err(dom_error(it, NOT_FOUND_ERR)),
    }
}

fn set_id_attribute_node(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let (_, at, _) = node_arg(it, &a, 0, "DOMElement::setIdAttributeNode", "attr", "DOMAttr")?;
    let is_id = a.get(1).is_some_and(|v| v.truthy());
    let mut d = doc.borrow_mut();
    if d.n(at).parent != Some(id) {
        return Err(dom_error(it, NOT_FOUND_ERR));
    }
    d.m(at).is_id = is_id;
    Ok(Value::Null)
}

fn insert_adjacent(it: &Interp, doc: &DocRef, id: Id, where_: &str, node: Id) -> R<bool> {
    let (parent, first, next) = {
        let d = doc.borrow();
        (d.n(id).parent, d.n(id).first, d.n(id).next)
    };
    match where_.to_ascii_lowercase().as_str() {
        "beforebegin" => match parent {
            Some(p) => insert_all(it, doc, p, &[node], Some(id))?,
            None => return Ok(false),
        },
        "afterbegin" => insert_all(it, doc, id, &[node], first)?,
        "beforeend" => insert_all(it, doc, id, &[node], None)?,
        "afterend" => match parent {
            Some(p) => insert_all(it, doc, p, &[node], next)?,
            None => return Ok(false),
        },
        _ => return Err(it.error("DOMException", "Syntax Error")),
    }
    Ok(true)
}

fn insert_adjacent_element(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let where_ = text(it, &a, 0)?;
    let (src, el, obj) = node_arg(it, &a, 1, "DOMElement::insertAdjacentElement", "element", "DOMElement")?;
    let el = adopt(it, &doc, &src, el, Some(&obj))?;
    Ok(if insert_adjacent(it, &doc, id, &where_, el)? { Value::Object(obj) } else { Value::Null })
}

fn insert_adjacent_text(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let where_ = text(it, &a, 0)?;
    let data = text(it, &a, 1)?;
    let t = doc.borrow_mut().add(Node::new(TEXT, "#text", &data));
    insert_adjacent(it, &doc, id, &where_, t)?;
    Ok(Value::Null)
}

// ---- constructors -------------------------------------------------------------------

/// A constructed node lives in a private document until inserted.
fn construct(this: This, node: Node) -> Id {
    let mut d = Doc::new(false);
    d.orphan = true;
    let id = d.add(node);
    let doc = Rc::new(RefCell::new(d));
    let o = this.unwrap();
    o.set_internal(Internal::Data(Box::new(NodeData { doc: doc.clone(), id })));
    doc.borrow_mut().wrappers.insert(id, Rc::downgrade(o));
    id
}

fn element_construct(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let name = text(it, &a, 0)?;
    if !valid_name(&name) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    let value = opt_text(it, &a, 1)?;
    let ns = ns_arg(it, &a, 2)?;
    let mut n = Node::new(ELEMENT, &name, "");
    n.ns = ns.clone();
    if let Some(u) = &ns {
        n.ns_defs.push((name.split_once(':').map(|(p, _)| p.to_string()), u.clone()));
    }
    let id = construct(this, n);
    if let Some(v) = value.filter(|v| !v.is_empty()) {
        let (doc, _) = this_node(it, this)?;
        let mut d = doc.borrow_mut();
        let t = d.add(Node::new(TEXT, "#text", &v));
        d.append(id, t);
    }
    Ok(Value::Null)
}

fn text_construct(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let v = if a.is_empty() { String::new() } else { text(it, &a, 0)? };
    construct(this, Node::new(TEXT, "#text", &v));
    Ok(Value::Null)
}

fn comment_construct(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let v = if a.is_empty() { String::new() } else { text(it, &a, 0)? };
    construct(this, Node::new(COMMENT, "#comment", &v));
    Ok(Value::Null)
}

fn cdata_construct(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let v = text(it, &a, 0)?;
    construct(this, Node::new(CDATA, "#cdata-section", &v));
    Ok(Value::Null)
}

fn pi_construct(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let name = text(it, &a, 0)?;
    if !valid_name(&name) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    let v = if has(&a, 1) { text(it, &a, 1)? } else { String::new() };
    construct(this, Node::new(PI, &name, &v));
    Ok(Value::Null)
}

fn attr_construct(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let name = text(it, &a, 0)?;
    if !valid_name(&name) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    let v = if has(&a, 1) { text(it, &a, 1)? } else { String::new() };
    construct(this, Node::new(ATTRIBUTE, &name, &v));
    Ok(Value::Null)
}

fn entity_ref_construct(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let name = text(it, &a, 0)?;
    construct(this, Node::new(ENTITY_REF, &name, ""));
    Ok(Value::Null)
}

fn fragment_construct(_: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    construct(this, Node::new(FRAGMENT, "#document-fragment", ""));
    Ok(Value::Null)
}

fn fragment_append_xml(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let src = text(it, &a, 0)?;
    if src.is_empty() {
        return Err(it.error("ValueError", "DOMDocumentFragment::appendXML(): Argument #1 ($data) must not be empty"));
    }
    let wrapped = format!("<phpx-fragment>{src}</phpx-fragment>");
    let (parsed, errors) = super::xml_parse::parse(wrapped.as_bytes(), 0);
    report(it, "DOMDocumentFragment::appendXML", "Entity", errors, 0);
    let Some(p) = parsed else { return Ok(Value::Bool(false)) };
    let root = p.document_element().unwrap();
    let kids: Vec<Id> = p.children(root).collect();
    let mut d = doc.borrow_mut();
    for k in kids {
        let n = d.import(&p, k, true);
        d.append(id, n);
    }
    Ok(Value::Bool(true))
}

// ---- character data -------------------------------------------------------------------

fn char_range(it: &Interp, s: &str, a: &[Value], with_count: bool) -> R<(usize, usize)> {
    let len = s.chars().count() as i64;
    let off = int(it, a, 0)?;
    let count = if with_count { int(it, a, 1)? } else { 0 };
    if off < 0 || off > len || count < 0 {
        return Err(dom_error(it, INDEX_SIZE_ERR));
    }
    let start = s.char_indices().nth(off as usize).map_or(s.len(), |(i, _)| i);
    let end = s.char_indices().nth((off + count).min(len) as usize).map_or(s.len(), |(i, _)| i);
    Ok((start, end))
}

fn substring_data(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let v = doc.borrow().n(id).value.clone();
    let (s, e) = char_range(it, &v, &a, true)?;
    Ok(Value::str(&v[s..e]))
}

fn append_data(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let data = text(it, &a, 0)?;
    doc.borrow_mut().m(id).value.push_str(&data);
    Ok(Value::Bool(true))
}

fn insert_data(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let v = doc.borrow().n(id).value.clone();
    let (s, _) = char_range(it, &v, &a, false)?;
    let data = text(it, &a, 1)?;
    doc.borrow_mut().m(id).value.insert_str(s, &data);
    Ok(Value::Bool(true))
}

fn delete_data(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let v = doc.borrow().n(id).value.clone();
    let (s, e) = char_range(it, &v, &a, true)?;
    doc.borrow_mut().m(id).value.replace_range(s..e, "");
    Ok(Value::Bool(true))
}

fn replace_data(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let v = doc.borrow().n(id).value.clone();
    let (s, e) = char_range(it, &v, &a, true)?;
    let data = text(it, &a, 2)?;
    doc.borrow_mut().m(id).value.replace_range(s..e, &data);
    Ok(Value::Bool(true))
}

fn split_text(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let v = doc.borrow().n(id).value.clone();
    let (s, _) = char_range(it, &v, &a, false)?;
    let new = {
        let mut d = doc.borrow_mut();
        let kind = d.n(id).kind;
        let name = d.n(id).name.clone();
        d.m(id).value.truncate(s);
        let t = d.add(Node::new(kind, &name, &v[s..]));
        if let Some(p) = d.n(id).parent {
            let next = d.n(id).next;
            d.insert(p, t, next);
        }
        t
    };
    wrap(it, &doc, new)
}

fn is_whitespace(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let v = Value::Bool(doc.borrow().n(id).value.bytes().all(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r')));
    Ok(v)
}

fn attr_is_id(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let d = doc.borrow();
    let n = d.n(id);
    Ok(Value::Bool(n.is_id || n.name == "xml:id" || (d.html && n.name == "id")))
}

fn not_serializable(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let name = this.map(|o| o.class.name.to_string()).unwrap_or_default();
    Err(it.error("Exception", &format!("Serialization of '{name}' is not allowed, unless serialization methods are implemented in a subclass")))
}

fn c14n(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let (doc, id) = this_node(it, this)?;
    let d = doc.borrow();
    let mut out = String::new();
    super::save::xml_node(&d, &mut out, id, false, super::save::Escape::Raw);
    Ok(Value::str(&out))
}

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![
        ("domnode", "appendchild", append_child as NativeMethod),
        ("domnode", "insertbefore", insert_before),
        ("domnode", "removechild", remove_child),
        ("domnode", "replacechild", replace_child),
        ("domnode", "clonenode", clone_node),
        ("domnode", "haschildnodes", has_child_nodes),
        ("domnode", "hasattributes", has_attributes),
        ("domnode", "normalize", normalize),
        ("domnode", "issamenode", is_same_node),
        ("domnode", "isequalnode", is_equal_node),
        ("domnode", "lookupnamespaceuri", lookup_namespace_uri),
        ("domnode", "lookupprefix", lookup_prefix),
        ("domnode", "isdefaultnamespace", is_default_namespace),
        ("domnode", "getnodepath", get_node_path),
        ("domnode", "getlineno", get_line_no),
        ("domnode", "contains", contains),
        ("domnode", "getrootnode", get_root_node),
        ("domnode", "comparedocumentposition", compare_document_position),
        ("domnode", "c14n", c14n),
        ("domnode", "__sleep", not_serializable),
        ("domnode", "__wakeup", not_serializable),
        ("domelement", "__construct", element_construct),
        ("domelement", "getattribute", get_attribute),
        ("domelement", "hasattribute", has_attribute),
        ("domelement", "setattribute", set_attribute),
        ("domelement", "removeattribute", remove_attribute),
        ("domelement", "toggleattribute", toggle_attribute),
        ("domelement", "getattributenode", get_attribute_node),
        ("domelement", "setattributenode", set_attribute_node),
        ("domelement", "setattributenodens", set_attribute_node),
        ("domelement", "removeattributenode", remove_attribute_node),
        ("domelement", "getattributens", get_attribute_ns),
        ("domelement", "hasattributens", has_attribute_ns),
        ("domelement", "getattributenodens", get_attribute_node_ns),
        ("domelement", "setattributens", set_attribute_ns),
        ("domelement", "removeattributens", remove_attribute_ns),
        ("domelement", "getattributenames", get_attribute_names),
        ("domelement", "getelementsbytagname", get_elements_by_tag_name),
        ("domelement", "getelementsbytagnamens", get_elements_by_tag_name_ns),
        ("domelement", "setidattribute", set_id_attribute),
        ("domelement", "setidattributens", set_id_attribute_ns),
        ("domelement", "setidattributenode", set_id_attribute_node),
        ("domelement", "insertadjacentelement", insert_adjacent_element),
        ("domelement", "insertadjacenttext", insert_adjacent_text),
        ("domelement", "append", append),
        ("domelement", "prepend", prepend),
        ("domelement", "replacechildren", replace_children),
        ("domelement", "before", before),
        ("domelement", "after", after),
        ("domelement", "replacewith", replace_with),
        ("domelement", "remove", remove),
        ("domdocument", "append", append),
        ("domdocument", "prepend", prepend),
        ("domdocument", "replacechildren", replace_children),
        ("domdocumentfragment", "__construct", fragment_construct),
        ("domdocumentfragment", "appendxml", fragment_append_xml),
        ("domdocumentfragment", "append", append),
        ("domdocumentfragment", "prepend", prepend),
        ("domdocumentfragment", "replacechildren", replace_children),
        ("domcharacterdata", "substringdata", substring_data),
        ("domcharacterdata", "appenddata", append_data),
        ("domcharacterdata", "insertdata", insert_data),
        ("domcharacterdata", "deletedata", delete_data),
        ("domcharacterdata", "replacedata", replace_data),
        ("domcharacterdata", "before", before),
        ("domcharacterdata", "after", after),
        ("domcharacterdata", "replacewith", replace_with),
        ("domcharacterdata", "remove", remove),
        ("domtext", "__construct", text_construct),
        ("domtext", "splittext", split_text),
        ("domtext", "iswhitespaceinelementcontent", is_whitespace),
        ("domtext", "iselementcontentwhitespace", is_whitespace),
        ("domcomment", "__construct", comment_construct),
        ("domcdatasection", "__construct", cdata_construct),
        ("domprocessinginstruction", "__construct", pi_construct),
        ("domattr", "__construct", attr_construct),
        ("domattr", "isid", attr_is_id),
        ("domentityreference", "__construct", entity_ref_construct),
    ]
}
