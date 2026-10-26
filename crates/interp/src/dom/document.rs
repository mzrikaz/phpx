//! DOMDocument and DOMImplementation: construction, parsing, saving,
//! factories and lookups.

use std::rc::Rc;

use super::node::{adopt, get_elements_by_tag_name, get_elements_by_tag_name_ns};
use super::tree::*;
use super::*;
use crate::builtins::int;
use crate::class::Class;

type Args = Vec<Value>;
type This<'a> = Option<&'a Rc<Obj>>;

const LIBXML_NOXMLDECL: i64 = 2;
const LIBXML_NOEMPTYTAG: i64 = 4;

/// The document of `this`, creating an empty one for a subclass that
/// skipped the parent constructor.
fn this_doc(it: &Interp, this: This) -> R<DocRef> {
    let o = this.ok_or_else(|| it.error("Error", "Non-static method called statically"))?;
    if let Some((d, _)) = node_of(o) {
        return Ok(d);
    }
    let d = Rc::new(RefCell::new(Doc::new(false)));
    attach(o, &d);
    Ok(d)
}

fn attach(o: &Rc<Obj>, d: &DocRef) {
    o.set_internal(Internal::Data(Box::new(NodeData { doc: d.clone(), id: 0 })));
    d.borrow_mut().wrappers.insert(0, Rc::downgrade(o));
}

fn construct(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let o = this.unwrap();
    let mut d = Doc::new(false);
    if has(&a, 0) {
        d.version = Some(text(it, &a, 0)?);
    }
    if has(&a, 1) {
        let e = text(it, &a, 1)?;
        if !e.is_empty() {
            d.encoding = Some(e);
        }
    }
    attach(o, &Rc::new(RefCell::new(d)));
    Ok(Value::Null)
}

/// Entity and character references in createElement()'s value.
fn decode_refs(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest.find(';').filter(|&e| e < 12);
        let rep = end.and_then(|e| {
            let name = &rest[1..e];
            let c = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ if name.starts_with("#x") || name.starts_with("#X") => u32::from_str_radix(&name[2..], 16).ok().and_then(char::from_u32),
                _ if name.starts_with('#') => name[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, e))
        });
        match rep {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn create(it: &Interp, doc: &DocRef, n: Node) -> R<Value> {
    let id = doc.borrow_mut().add(n);
    wrap(it, doc, id)
}

fn create_element(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let name = text(it, &a, 0)?;
    if !valid_name(&name) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    let value = if has(&a, 1) { decode_refs(&text(it, &a, 1)?) } else { String::new() };
    let id = {
        let mut d = doc.borrow_mut();
        let id = d.add(Node::new(ELEMENT, &name, ""));
        if !value.is_empty() {
            let t = d.add(Node::new(TEXT, "#text", &value));
            d.append(id, t);
        }
        id
    };
    wrap(it, &doc, id)
}

/// Validate a namespaced qualified name: (prefix, local).
fn split_qname(it: &Interp, ns: &Option<String>, qname: &str) -> R<(Option<String>, String)> {
    if !valid_name(qname) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    let (prefix, local) = match qname.split_once(':') {
        Some((p, l)) => (Some(p.to_string()), l.to_string()),
        None => (None, qname.to_string()),
    };
    if local.is_empty() || local.contains(':') || prefix.as_deref() == Some("") || (prefix.is_some() && ns.is_none()) {
        return Err(dom_error(it, NAMESPACE_ERR));
    }
    if prefix.as_deref() == Some("xml") && ns.as_deref() != Some("http://www.w3.org/XML/1998/namespace") {
        return Err(dom_error(it, NAMESPACE_ERR));
    }
    Ok((prefix, local))
}

fn create_element_ns(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let ns = opt_text(it, &a, 0)?.filter(|s| !s.is_empty());
    let qname = text(it, &a, 1)?;
    let (prefix, _) = split_qname(it, &ns, &qname)?;
    let value = if has(&a, 2) { decode_refs(&text(it, &a, 2)?) } else { String::new() };
    let mut n = Node::new(ELEMENT, &qname, "");
    if let Some(u) = &ns {
        n.ns = Some(u.clone());
        n.ns_defs.push((prefix, u.clone()));
    }
    let id = {
        let mut d = doc.borrow_mut();
        let id = d.add(n);
        if !value.is_empty() {
            let t = d.add(Node::new(TEXT, "#text", &value));
            d.append(id, t);
        }
        id
    };
    wrap(it, &doc, id)
}

fn create_text_node(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    create(it, &doc, Node::new(TEXT, "#text", &text(it, &a, 0)?))
}

fn create_comment(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    create(it, &doc, Node::new(COMMENT, "#comment", &text(it, &a, 0)?))
}

fn create_cdata(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    create(it, &doc, Node::new(CDATA, "#cdata-section", &text(it, &a, 0)?))
}

fn create_pi(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let target = text(it, &a, 0)?;
    if !valid_name(&target) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    let data = if has(&a, 1) { text(it, &a, 1)? } else { String::new() };
    create(it, &doc, Node::new(PI, &target, &data))
}

fn create_attribute(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let name = text(it, &a, 0)?;
    if !valid_name(&name) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    create(it, &doc, Node::new(ATTRIBUTE, &name, ""))
}

fn create_attribute_ns(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let ns = opt_text(it, &a, 0)?.filter(|s| !s.is_empty());
    let qname = text(it, &a, 1)?;
    split_qname(it, &ns, &qname)?;
    let mut n = Node::new(ATTRIBUTE, &qname, "");
    n.ns = ns;
    create(it, &doc, n)
}

fn create_fragment(it: &Interp, this: This, _: &Rc<Class>, _: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    create(it, &doc, Node::new(FRAGMENT, "#document-fragment", ""))
}

fn create_entity_ref(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let name = text(it, &a, 0)?;
    let name = name.trim_start_matches('&').trim_end_matches(';').to_string();
    if !valid_name(&name) {
        return Err(dom_error(it, INVALID_CHARACTER_ERR));
    }
    create(it, &doc, Node::new(ENTITY_REF, &name, ""))
}

fn get_element_by_id(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let want = text(it, &a, 0)?;
    let found = {
        let d = doc.borrow();
        d.descendants(0).into_iter().find(|&e| {
            let n = d.n(e);
            n.kind == ELEMENT
                && n.attrs.iter().any(|&at| {
                    let an = d.n(at);
                    an.value == want && (an.is_id || an.name == "xml:id" || (d.html && an.name == "id"))
                })
        })
    };
    wrap_opt(it, &doc, found)
}

fn import_node(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let (src, id, _) = node_arg(it, &a, 0, "DOMDocument::importNode", "node", "DOMNode")?;
    let deep = a.get(1).is_some_and(|v| v.truthy());
    if matches!(src.borrow().n(id).kind, DOCUMENT | HTML_DOCUMENT) {
        return Err(dom_error(it, NOT_SUPPORTED_ERR));
    }
    let new = if Rc::ptr_eq(&src, &doc) {
        doc.borrow_mut().copy_within(id, deep)
    } else {
        let s = src.borrow();
        doc.borrow_mut().import(&s, id, deep)
    };
    wrap(it, &doc, new)
}

fn adopt_node(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let (src, id, obj) = node_arg(it, &a, 0, "DOMDocument::adoptNode", "node", "DOMNode")?;
    if matches!(src.borrow().n(id).kind, DOCUMENT | HTML_DOCUMENT) {
        return Err(dom_error(it, NOT_SUPPORTED_ERR));
    }
    if Rc::ptr_eq(&src, &doc) {
        doc.borrow_mut().detach(id);
        return Ok(Value::Object(obj));
    }
    let new = {
        let s = src.borrow();
        doc.borrow_mut().import(&s, id, true)
    };
    src.borrow_mut().detach(id);
    rebind(&obj, &doc, new);
    Ok(Value::Object(obj))
}

/// Replace this object's document with a parsed one.
fn load(it: &Interp, this: This, input: &[u8], opts: i64, html: bool, func: &str, file: &str) -> R<Value> {
    let o = this.ok_or_else(|| it.error("Error", "Non-static method called statically"))?;
    let old = node_of(o).map(|(d, _)| d);
    let (settings, classes) = match &old {
        Some(d) => (d.borrow().settings.clone(), d.borrow().classes.clone()),
        None => (Settings::default(), Default::default()),
    };
    let mut opts = opts;
    if !settings.preserve_white_space {
        opts |= 256;
    }
    if !html {
        if settings.substitute_entities {
            opts |= xml_parse::XML_PARSE_NOENT;
        }
        if settings.recover {
            opts |= xml_parse::XML_PARSE_RECOVER;
        }
    }
    let (parsed, errors) = if html {
        let (d, e) = html_parse::parse(input, opts);
        (Some(d), e)
    } else {
        xml_parse::parse(input, opts)
    };
    report(it, func, file, errors, opts);
    let Some(mut d) = parsed else { return Ok(Value::Bool(false)) };
    d.settings = settings;
    d.classes = classes;
    if file != "Entity" {
        d.settings.document_uri = Some(file.to_string());
    }
    attach(o, &Rc::new(RefCell::new(d)));
    Ok(Value::Bool(true))
}

fn source_arg(it: &Interp, a: &[Value], func: &str, param: &str) -> R<Vec<u8>> {
    let src = s(it, a, 0)?.to_vec();
    if src.is_empty() {
        return Err(it.error("ValueError", &format!("{func}(): Argument #1 (${param}) must not be empty")));
    }
    Ok(src)
}

fn opts(it: &Interp, a: &[Value], i: usize) -> R<i64> {
    if has(a, i) {
        int(it, a, i)
    } else {
        Ok(0)
    }
}

fn load_xml(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let src = source_arg(it, &a, "DOMDocument::loadXML", "source")?;
    load(it, this, &src, opts(it, &a, 1)?, false, "DOMDocument::loadXML", "Entity")
}

fn load_html(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let src = source_arg(it, &a, "DOMDocument::loadHTML", "source")?;
    load(it, this, &src, opts(it, &a, 1)?, true, "DOMDocument::loadHTML", "Entity")
}

fn read_file(it: &Interp, a: &[Value], func: &str) -> R<Option<(Vec<u8>, String)>> {
    let path = crate::pstr::lossy(&source_arg(it, a, func, "filename")?).into_owned();
    match std::fs::read(&path) {
        Ok(b) => {
            let full = std::fs::canonicalize(&path).map(|p| p.to_string_lossy().into_owned()).unwrap_or(path);
            Ok(Some((b, full)))
        }
        Err(_) => {
            it.warn(&format!("{func}(): I/O warning : failed to load external entity \"{path}\""));
            Ok(None)
        }
    }
}

fn load_file(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let Some((src, file)) = read_file(it, &a, "DOMDocument::load")? else { return Ok(Value::Bool(false)) };
    load(it, this, &src, opts(it, &a, 1)?, false, "DOMDocument::load", &file)
}

fn load_html_file(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let Some((src, file)) = read_file(it, &a, "DOMDocument::loadHTMLFile")? else { return Ok(Value::Bool(false)) };
    load(it, this, &src, opts(it, &a, 1)?, true, "DOMDocument::loadHTMLFile", &file)
}

/// The optional node argument of saveXML()/saveHTML().
fn save_target(it: &Interp, doc: &DocRef, a: &[Value], func: &str) -> R<Option<Id>> {
    match a.first().map(|v| v.deref()) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => {
            let (nd, id, _) = node_arg(it, a, 0, func, "node", "?DOMNode")?;
            if !Rc::ptr_eq(&nd, doc) {
                return Err(dom_error(it, WRONG_DOCUMENT_ERR));
            }
            Ok(Some(id))
        }
    }
}

fn expand_empty(out: Vec<u8>) -> Vec<u8> {
    // `<name .../>` -> `<name ...></name>` outside text (LIBXML_NOEMPTYTAG).
    let s = crate::pstr::lossy(&out).into_owned();
    let mut res = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(p) = rest.find("/>") {
        let head = &rest[..p];
        match head.rfind('<') {
            Some(lt) if !head[lt..].contains('>') && head[lt + 1..].starts_with(|c: char| c.is_alphabetic() || c == '_') => {
                let name: String = head[lt + 1..].chars().take_while(|c| !c.is_whitespace()).collect();
                res.push_str(head);
                res.push_str("></");
                res.push_str(&name);
                res.push('>');
            }
            _ => {
                res.push_str(head);
                res.push_str("/>");
            }
        }
        rest = &rest[p + 2..];
    }
    res.push_str(rest);
    res.into_bytes()
}

fn save_xml(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let target = save_target(it, &doc, &a, "DOMDocument::saveXML")?;
    let options = opts(it, &a, 1)?;
    let d = doc.borrow();
    let format = d.settings.format_output;
    let out = match target {
        Some(id) if id != 0 => save::save_xml_node(&d, id, format),
        _ => save::save_xml_doc(&d, format, options & LIBXML_NOXMLDECL != 0),
    };
    Ok(Value::bytes(if options & LIBXML_NOEMPTYTAG != 0 { expand_empty(out) } else { out }))
}

fn save_html(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let target = save_target(it, &doc, &a, "DOMDocument::saveHTML")?;
    let d = doc.borrow();
    let format = d.settings.format_output;
    let out = match target {
        Some(id) if id != 0 => {
            let mut s = String::new();
            if d.n(id).kind == FRAGMENT {
                for c in d.children(id) {
                    save::html_node(&d, &mut s, c, format);
                }
            } else {
                save::html_node(&d, &mut s, id, format);
            }
            s.into_bytes()
        }
        _ => save::save_html_doc(&d, format),
    };
    Ok(Value::bytes(out))
}

fn write_file(it: &Interp, a: &[Value], data: Vec<u8>, func: &str) -> R<Value> {
    let path = crate::pstr::lossy(&source_arg(it, a, func, "filename")?).into_owned();
    match std::fs::write(&path, &data) {
        Ok(()) => Ok(Value::Int(data.len() as i64)),
        Err(e) => {
            it.warn(&format!("{func}(): Failed to open stream \"{path}\": {e}"));
            Ok(Value::Bool(false))
        }
    }
}

fn save_file(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let options = opts(it, &a, 1)?;
    let out = {
        let d = doc.borrow();
        save::save_xml_doc(&d, d.settings.format_output, options & LIBXML_NOXMLDECL != 0)
    };
    write_file(it, &a, if options & LIBXML_NOEMPTYTAG != 0 { expand_empty(out) } else { out }, "DOMDocument::save")
}

fn save_html_file(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let out = {
        let d = doc.borrow();
        save::save_html_doc(&d, d.settings.format_output)
    };
    write_file(it, &a, out, "DOMDocument::saveHTMLFile")
}

fn register_node_class(it: &Interp, this: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let base = text(it, &a, 0)?;
    let base_cls = it.lookup_class(&base)?;
    let lbase = base_cls.lname.to_string();
    if !lbase.starts_with("dom") {
        return Err(it.error("ValueError", "DOMDocument::registerNodeClass(): Argument #1 ($baseClass) must be a class name derived from DOMNode or DOMDocument"));
    }
    match opt_text(it, &a, 1)? {
        None => {
            doc.borrow_mut().classes.remove(&lbase);
        }
        Some(ext) => {
            let cls = it.lookup_class(&ext)?;
            if cls.lname != base_cls.lname && !cls.ancestors.iter().any(|x| **x == *lbase) {
                return Err(it.error("ValueError", &format!("DOMDocument::registerNodeClass(): Argument #2 ($extendedClass) must be a class name derived from {base} or null, {ext} given")));
            }
            doc.borrow_mut().classes.insert(lbase, cls.name.to_string());
        }
    }
    Ok(Value::Bool(true))
}

fn normalize_document(it: &Interp, this: This, c: &Rc<Class>, a: Args) -> R<Value> {
    let doc = this_doc(it, this)?;
    let _ = (doc, c, a);
    Ok(Value::Null)
}

fn yes(_: &Interp, _: This, _: &Rc<Class>, _: Args) -> R<Value> {
    Ok(Value::Bool(true))
}

fn xinclude(_: &Interp, _: This, _: &Rc<Class>, _: Args) -> R<Value> {
    Ok(Value::Int(0))
}

// ---- DOMImplementation ---------------------------------------------------------------

fn create_document_type(it: &Interp, _: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let qname = text(it, &a, 0)?;
    if qname.is_empty() {
        return Err(it.error("ValueError", "DOMImplementation::createDocumentType(): Argument #1 ($qualifiedName) cannot be empty"));
    }
    let public = if has(&a, 1) { text(it, &a, 1)? } else { String::new() };
    let system = if has(&a, 2) { text(it, &a, 2)? } else { String::new() };
    let mut d = Doc::new(false);
    d.orphan = true;
    let mut n = Node::new(DOCTYPE, &qname, "");
    n.ids = Some(Box::new((public, system, String::new())));
    let id = d.add(n);
    let doc = Rc::new(RefCell::new(d));
    wrap(it, &doc, id)
}

fn create_document(it: &Interp, _: This, _: &Rc<Class>, a: Args) -> R<Value> {
    let ns = opt_text(it, &a, 0)?.filter(|s| !s.is_empty());
    let qname = if has(&a, 1) { text(it, &a, 1)? } else { String::new() };
    let doc = Rc::new(RefCell::new(Doc::new(false)));
    if let Some(Value::Object(dt)) = a.get(2).map(|v| v.deref()) {
        if let Some((src, id)) = node_of(&dt) {
            let id = adopt(it, &doc, &src, id, Some(&dt))?;
            doc.borrow_mut().append(0, id);
        }
    }
    if !qname.is_empty() {
        let (prefix, _) = split_qname(it, &ns, &qname)?;
        let mut n = Node::new(ELEMENT, &qname, "");
        if let Some(u) = &ns {
            n.ns = Some(u.clone());
            n.ns_defs.push((prefix, u.clone()));
        }
        let mut d = doc.borrow_mut();
        let id = d.add(n);
        d.append(0, id);
    }
    wrap(it, &doc, 0)
}

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![
        ("domdocument", "__construct", construct as NativeMethod),
        ("domdocument", "createelement", create_element),
        ("domdocument", "createelementns", create_element_ns),
        ("domdocument", "createtextnode", create_text_node),
        ("domdocument", "createcomment", create_comment),
        ("domdocument", "createcdatasection", create_cdata),
        ("domdocument", "createprocessinginstruction", create_pi),
        ("domdocument", "createattribute", create_attribute),
        ("domdocument", "createattributens", create_attribute_ns),
        ("domdocument", "createdocumentfragment", create_fragment),
        ("domdocument", "createentityreference", create_entity_ref),
        ("domdocument", "getelementsbytagname", get_elements_by_tag_name),
        ("domdocument", "getelementsbytagnamens", get_elements_by_tag_name_ns),
        ("domdocument", "getelementbyid", get_element_by_id),
        ("domdocument", "importnode", import_node),
        ("domdocument", "adoptnode", adopt_node),
        ("domdocument", "load", load_file),
        ("domdocument", "loadxml", load_xml),
        ("domdocument", "loadhtml", load_html),
        ("domdocument", "loadhtmlfile", load_html_file),
        ("domdocument", "save", save_file),
        ("domdocument", "savexml", save_xml),
        ("domdocument", "savehtml", save_html),
        ("domdocument", "savehtmlfile", save_html_file),
        ("domdocument", "registernodeclass", register_node_class),
        ("domdocument", "normalizedocument", normalize_document),
        ("domdocument", "validate", yes),
        ("domdocument", "schemavalidate", yes),
        ("domdocument", "schemavalidatesource", yes),
        ("domdocument", "relaxngvalidate", yes),
        ("domdocument", "relaxngvalidatesource", yes),
        ("domdocument", "xinclude", xinclude),
        ("domimplementation", "createdocument", create_document),
        ("domimplementation", "createdocumenttype", create_document_type),
        ("domimplementation", "hasfeature", yes),
    ]
}
