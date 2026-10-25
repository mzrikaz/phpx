//! The node tree: an arena per document with libxml2's links (parent,
//! first/last child, siblings). Nodes are never freed while the document
//! lives, so a PHP wrapper of a detached node stays valid.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::value::Obj;

pub type Id = u32;

// libxml2 node types, which PHP's nodeType reports.
pub const ELEMENT: u8 = 1;
pub const ATTRIBUTE: u8 = 2;
pub const TEXT: u8 = 3;
pub const CDATA: u8 = 4;
pub const ENTITY_REF: u8 = 5;
pub const PI: u8 = 7;
pub const COMMENT: u8 = 8;
pub const DOCUMENT: u8 = 9;
pub const DOCTYPE: u8 = 10;
pub const FRAGMENT: u8 = 11;
pub const HTML_DOCUMENT: u8 = 13;

#[derive(Clone, Default)]
pub struct Node {
    pub kind: u8,
    /// Qualified name (elements, attributes), PI target, doctype or
    /// entity name.
    pub name: String,
    /// Namespace URI of an element or attribute.
    pub ns: Option<String>,
    /// Text, comment, PI data or attribute value.
    pub value: String,
    pub parent: Option<Id>,
    pub first: Option<Id>,
    pub last: Option<Id>,
    pub prev: Option<Id>,
    pub next: Option<Id>,
    /// Attribute nodes of an element, in order.
    pub attrs: Vec<Id>,
    /// Namespace declarations on an element: (prefix, URI).
    pub ns_defs: Vec<(Option<String>, String)>,
    /// Doctype: public id, system id, internal subset.
    pub ids: Option<Box<(String, String, String)>>,
    /// Text that serializes without escaping (libxml2's "noenc" text);
    /// on an HTML attribute, that it has no value.
    pub raw: bool,
    /// An attribute that is an ID (getElementById).
    pub is_id: bool,
    pub line: u32,
}

impl Node {
    pub fn new(kind: u8, name: &str, value: &str) -> Node {
        Node { kind, name: name.to_string(), value: value.to_string(), ..Default::default() }
    }

    pub fn prefix(&self) -> Option<&str> {
        self.name.split_once(':').map(|(p, _)| p)
    }

    pub fn local(&self) -> &str {
        self.name.split_once(':').map_or(&self.name, |(_, l)| l)
    }

    pub fn is_text(&self) -> bool {
        matches!(self.kind, TEXT | CDATA)
    }
}

/// Document-level settings (DOMDocument properties).
#[derive(Clone)]
pub struct Settings {
    pub format_output: bool,
    pub preserve_white_space: bool,
    pub validate_on_parse: bool,
    pub resolve_externals: bool,
    pub substitute_entities: bool,
    pub recover: bool,
    pub strict_error_checking: bool,
    pub document_uri: Option<String>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            format_output: false,
            preserve_white_space: true,
            validate_on_parse: false,
            resolve_externals: false,
            substitute_entities: false,
            recover: false,
            strict_error_checking: true,
            document_uri: None,
        }
    }
}

#[derive(Clone)]
pub struct Doc {
    pub nodes: Vec<Node>,
    pub html: bool,
    pub encoding: Option<String>,
    pub version: Option<String>,
    /// -1: not declared.
    pub standalone: i8,
    pub settings: Settings,
    /// PHP wrapper objects, so the same node always gives the same object.
    pub wrappers: HashMap<Id, Weak<Obj>>,
    /// registerNodeClass(): base class (lowercase) -> user class.
    pub classes: HashMap<String, String>,
    /// Bumped on every tree change (live node lists cache on it).
    pub rev: u64,
    /// The private document of a node made with `new DOMElement(...)`:
    /// appending it elsewhere moves it.
    pub orphan: bool,
}

pub type DocRef = Rc<RefCell<Doc>>;

impl Doc {
    pub fn new(html: bool) -> Doc {
        let root = Node::new(if html { HTML_DOCUMENT } else { DOCUMENT }, "#document", "");
        Doc {
            nodes: vec![root],
            html,
            encoding: None,
            version: if html { None } else { Some("1.0".into()) },
            standalone: -1,
            settings: Settings::default(),
            wrappers: HashMap::new(),
            classes: HashMap::new(),
            rev: 0,
            orphan: false,
        }
    }

    pub fn n(&self, id: Id) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn m(&mut self, id: Id) -> &mut Node {
        &mut self.nodes[id as usize]
    }

    pub fn add(&mut self, node: Node) -> Id {
        self.nodes.push(node);
        (self.nodes.len() - 1) as Id
    }

    pub fn children(&self, id: Id) -> Children<'_> {
        Children { doc: self, cur: self.n(id).first }
    }

    /// Unlink `id` from its parent (or owner element, for attributes).
    pub fn detach(&mut self, id: Id) {
        self.rev += 1;
        let n = self.n(id);
        let (parent, prev, next, kind) = (n.parent, n.prev, n.next, n.kind);
        if kind == ATTRIBUTE {
            if let Some(p) = parent {
                self.m(p).attrs.retain(|&a| a != id);
            }
            self.m(id).parent = None;
            return;
        }
        if let Some(p) = prev {
            self.m(p).next = next;
        } else if let Some(par) = parent {
            self.m(par).first = next;
        }
        if let Some(nx) = next {
            self.m(nx).prev = prev;
        } else if let Some(par) = parent {
            self.m(par).last = prev;
        }
        let n = self.m(id);
        n.parent = None;
        n.prev = None;
        n.next = None;
    }

    /// Insert `id` under `parent` before `before` (None: at the end).
    pub fn insert(&mut self, parent: Id, id: Id, before: Option<Id>) {
        self.detach(id);
        self.rev += 1;
        let prev = match before {
            Some(b) => self.n(b).prev,
            None => self.n(parent).last,
        };
        {
            let n = self.m(id);
            n.parent = Some(parent);
            n.prev = prev;
            n.next = before;
        }
        match prev {
            Some(p) => self.m(p).next = Some(id),
            None => self.m(parent).first = Some(id),
        }
        match before {
            Some(b) => self.m(b).prev = Some(id),
            None => self.m(parent).last = Some(id),
        }
    }

    pub fn append(&mut self, parent: Id, id: Id) {
        self.insert(parent, id, None);
    }

    /// Append text, merging into a preceding text node (what the parsers
    /// do; DOM methods never merge).
    pub fn append_text(&mut self, parent: Id, kind: u8, s: &str) {
        if let Some(l) = self.n(parent).last {
            if self.n(l).kind == kind {
                self.m(l).value.push_str(s);
                return;
            }
        }
        let t = self.add(Node::new(kind, if kind == CDATA { "#cdata-section" } else { "#text" }, s));
        self.append(parent, t);
    }

    pub fn is_ancestor(&self, anc: Id, mut id: Id) -> bool {
        loop {
            if id == anc {
                return true;
            }
            match self.n(id).parent {
                Some(p) => id = p,
                None => return false,
            }
        }
    }

    /// The root of `id`'s tree (the document node when connected).
    pub fn root(&self, mut id: Id) -> Id {
        while let Some(p) = self.n(id).parent {
            id = p;
        }
        id
    }

    pub fn document_element(&self) -> Option<Id> {
        self.children(0).find(|&c| self.n(c).kind == ELEMENT)
    }

    pub fn doctype(&self) -> Option<Id> {
        self.children(0).find(|&c| self.n(c).kind == DOCTYPE)
    }

    /// Descendants in document order (not including `id`).
    pub fn descendants(&self, id: Id) -> Vec<Id> {
        let mut out = vec![];
        let mut stack: Vec<Id> = vec![];
        let mut cur = self.n(id).first;
        loop {
            match cur {
                Some(c) => {
                    out.push(c);
                    if self.n(c).first.is_some() {
                        stack.push(c);
                        cur = self.n(c).first;
                    } else {
                        cur = self.n(c).next;
                    }
                }
                None => match stack.pop() {
                    Some(p) => cur = self.n(p).next,
                    None => return out,
                },
            }
        }
    }

    /// textContent: concatenated text of descendants.
    pub fn text_content(&self, id: Id) -> String {
        let n = self.n(id);
        match n.kind {
            TEXT | CDATA | COMMENT | PI | ATTRIBUTE => n.value.clone(),
            DOCUMENT | HTML_DOCUMENT | DOCTYPE => String::new(),
            _ => {
                let mut s = String::new();
                for d in self.descendants(id) {
                    let dn = self.n(d);
                    if dn.is_text() {
                        s.push_str(&dn.value);
                    }
                }
                s
            }
        }
    }

    /// Attribute node of element `el` by qualified name.
    pub fn attr(&self, el: Id, name: &str) -> Option<Id> {
        self.n(el).attrs.iter().copied().find(|&a| self.n(a).name == name)
    }

    pub fn attr_ns(&self, el: Id, ns: Option<&str>, local: &str) -> Option<Id> {
        self.n(el).attrs.iter().copied().find(|&a| self.n(a).ns.as_deref() == ns && self.n(a).local() == local)
    }

    pub fn get_attr(&self, el: Id, name: &str) -> Option<&str> {
        self.attr(el, name).map(|a| self.n(a).value.as_str())
    }

    pub fn set_attr(&mut self, el: Id, name: &str, value: &str) -> Id {
        self.rev += 1;
        if let Some(a) = self.attr(el, name) {
            self.m(a).value = value.to_string();
            return a;
        }
        let mut n = Node::new(ATTRIBUTE, name, value);
        n.parent = Some(el);
        let a = self.add(n);
        self.m(el).attrs.push(a);
        a
    }

    /// Namespace URI bound to `prefix` in scope at `id`.
    pub fn lookup_ns(&self, mut id: Id, prefix: Option<&str>) -> Option<String> {
        match prefix {
            Some("xml") => return Some("http://www.w3.org/XML/1998/namespace".into()),
            Some("xmlns") => return Some("http://www.w3.org/2000/xmlns/".into()),
            _ => {}
        }
        loop {
            let n = self.n(id);
            if n.kind == ELEMENT {
                if let Some((_, uri)) = n.ns_defs.iter().find(|(p, _)| p.as_deref() == prefix) {
                    return if uri.is_empty() { None } else { Some(uri.clone()) };
                }
            }
            id = n.parent?;
        }
    }

    /// Prefix bound to `uri` in scope at `id`.
    pub fn lookup_prefix(&self, mut id: Id, uri: &str) -> Option<Option<String>> {
        loop {
            let n = self.n(id);
            if n.kind == ELEMENT {
                if let Some((p, _)) = n.ns_defs.iter().find(|(_, u)| u == uri) {
                    return Some(p.clone());
                }
            }
            id = n.parent?;
        }
    }

    /// Copy of `id` within this document (cloneNode).
    pub fn copy_within(&mut self, id: Id, deep: bool) -> Id {
        let mut n = self.n(id).clone();
        n.parent = None;
        n.prev = None;
        n.next = None;
        n.first = None;
        n.last = None;
        n.attrs = vec![];
        let new = self.add(n);
        let attrs = self.n(id).attrs.clone();
        for a in attrs {
            let mut an = self.n(a).clone();
            an.parent = Some(new);
            let na = self.add(an);
            self.m(new).attrs.push(na);
        }
        if deep {
            let kids: Vec<Id> = self.children(id).collect();
            for c in kids {
                let nc = self.copy_within(c, true);
                self.append(new, nc);
            }
        }
        new
    }

    /// Deep (or shallow) copy of `id` from `src` into this document.
    pub fn import(&mut self, src: &Doc, id: Id, deep: bool) -> Id {
        let mut n = src.n(id).clone();
        n.parent = None;
        n.prev = None;
        n.next = None;
        n.first = None;
        n.last = None;
        n.attrs = vec![];
        if n.kind == DOCUMENT || n.kind == HTML_DOCUMENT {
            n.kind = FRAGMENT;
        }
        let new = self.add(n);
        if src.n(id).kind == ELEMENT {
            for &a in &src.n(id).attrs {
                let mut an = src.n(a).clone();
                an.parent = Some(new);
                let na = self.add(an);
                self.m(new).attrs.push(na);
            }
        }
        if deep || src.n(id).kind == ATTRIBUTE {
            let kids: Vec<Id> = src.children(id).collect();
            for c in kids {
                let nc = self.import(src, c, true);
                self.append(new, nc);
            }
        }
        new
    }
}

pub struct Children<'a> {
    doc: &'a Doc,
    cur: Option<Id>,
}

impl Iterator for Children<'_> {
    type Item = Id;
    fn next(&mut self) -> Option<Id> {
        let c = self.cur?;
        self.cur = self.doc.n(c).next;
        Some(c)
    }
}

/// HTML element info from libxml2's table: (end tag, save end tag, empty,
/// inline).
pub fn html_info(name: &str) -> Option<(u8, u8, bool, bool)> {
    let t = super::tables::ELEMENTS;
    let lower;
    let name = if name.bytes().any(|c| c.is_ascii_uppercase()) {
        lower = name.to_ascii_lowercase();
        &lower
    } else {
        name
    };
    t.binary_search_by(|e| e.0.cmp(name)).ok().map(|i| (t[i].1, t[i].2, t[i].3, t[i].4))
}
