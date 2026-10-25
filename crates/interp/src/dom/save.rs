//! saveHTML() and saveXML(): libxml2's HTMLtree.c and xmlsave.c output,
//! including its escaping and output-encoding rules.

use super::html_parse::is_boolean_attr;
use super::tables::ENTITIES;
use super::tree::*;

fn named_entity(c: u32) -> Option<&'static str> {
    ENTITIES.iter().find(|e| e.0 == c).map(|e| e.1)
}

/// The final output encoding step.
pub fn encode_output(s: String, encoding: Option<&str>) -> Vec<u8> {
    let enc = encoding.map(|e| e.to_ascii_uppercase());
    match enc.as_deref() {
        None | Some("UTF-8") | Some("UTF8") => s.into_bytes(),
        Some("HTML") => {
            if s.is_ascii() {
                return s.into_bytes();
            }
            let mut out = String::with_capacity(s.len() + 16);
            for ch in s.chars() {
                if ch.is_ascii() {
                    out.push(ch);
                } else if let Some(n) = named_entity(ch as u32) {
                    out.push('&');
                    out.push_str(n);
                    out.push(';');
                } else {
                    out.push_str(&format!("&#{};", ch as u32));
                }
            }
            out.into_bytes()
        }
        Some("ISO-8859-1") | Some("LATIN1") | Some("ISO-LATIN-1") | Some("WINDOWS-1252") | Some("US-ASCII") | Some("ASCII") => {
            let max = if matches!(enc.as_deref(), Some("US-ASCII") | Some("ASCII")) { 0x7F } else { 0xFF };
            let mut out = Vec::with_capacity(s.len());
            for ch in s.chars() {
                if (ch as u32) <= max {
                    out.push(ch as u8);
                } else {
                    out.extend_from_slice(format!("&#{};", ch as u32).as_bytes());
                }
            }
            out
        }
        _ => s.into_bytes(),
    }
}

fn hex_ref(out: &mut String, c: char) {
    out.push_str(&format!("&#x{:X};", c as u32));
}

// ---- HTML ----------------------------------------------------------------------

/// xmlEncodeEntitiesInternal for an HTML document.
fn html_escape(out: &mut String, s: &str, attr: bool) {
    let b = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        let c = b[i];
        match c {
            b'<' if attr && s[i..].starts_with("<!--") => {
                if let Some(e) = s[i..].find("-->") {
                    out.push_str(&s[i..i + e + 3]);
                    i += e + 3;
                    continue;
                }
                out.push_str("&lt;");
            }
            b'<' => out.push_str("&lt;"),
            b'>' => out.push_str("&gt;"),
            b'&' if attr && b.get(i + 1) == Some(&b'{') && s[i..].contains('}') => {
                let e = s[i..].find('}').unwrap();
                out.push_str(&s[i..i + e + 1]);
                i += e + 1;
                continue;
            }
            b'&' => out.push_str("&amp;"),
            0x20..=0x7F | b'\n' | b'\t' | b'\r' => out.push(c as char),
            0x80.. => {
                let ch = s[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }
            _ => out.push_str(&format!("&#{c};")),
        }
        i += 1;
    }
}

/// xmlOutputBufferWriteQuotedString.
fn quoted(out: &mut String, v: &str) {
    if v.contains('"') {
        if v.contains('\'') {
            out.push('"');
            out.push_str(&v.replace('"', "&quot;"));
            out.push('"');
        } else {
            out.push('\'');
            out.push_str(v);
            out.push('\'');
        }
    } else {
        out.push('"');
        out.push_str(v);
        out.push('"');
    }
}

/// xmlURIEscapeStr keeping printable ASCII except space.
fn uri_escape(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        if b > 0x20 && b < 0x7F {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn html_attr(doc: &Doc, out: &mut String, a: Id) {
    let n = doc.n(a);
    out.push(' ');
    out.push_str(&n.name);
    if n.raw || is_boolean_attr(&n.name) {
        return;
    }
    let mut v = String::new();
    html_escape(&mut v, &n.value, true);
    out.push('=');
    let parent = n.parent.map(|p| doc.n(p));
    let urlish = n.ns.is_none()
        && parent.is_some_and(|p| p.ns.is_none())
        && (["href", "action", "src"].iter().any(|k| n.name.eq_ignore_ascii_case(k)) || (n.name.eq_ignore_ascii_case("name") && parent.is_some_and(|p| p.name.eq_ignore_ascii_case("a"))));
    if urlish {
        quoted(out, &uri_escape(v.trim_start_matches([' ', '\t', '\n', '\r'])));
    } else {
        quoted(out, &v);
    }
}

fn html_doctype(doc: &Doc, out: &mut String, dt: Id) {
    let n = doc.n(dt);
    let (public, system, _) = &**n.ids.as_ref().unwrap();
    out.push_str("<!DOCTYPE ");
    out.push_str(&n.name);
    if !public.is_empty() {
        out.push_str(" PUBLIC ");
        quoted(out, public);
        if !system.is_empty() {
            out.push(' ');
            quoted(out, system);
        }
    } else if !system.is_empty() && system != "about:legacy-compat" {
        out.push_str(" SYSTEM ");
        quoted(out, system);
    }
    out.push_str(">\n");
}

fn inline_info(doc: &Doc, id: Id) -> Option<(u8, u8, bool, bool)> {
    let n = doc.n(id);
    if n.ns.is_some() {
        None
    } else {
        html_info(&n.name)
    }
}

fn not_text(doc: &Doc, id: Option<Id>) -> bool {
    id.is_some_and(|i| !matches!(doc.n(i).kind, TEXT | ENTITY_REF))
}

/// htmlNodeDumpFormatOutput.
pub fn html_node(doc: &Doc, out: &mut String, root: Id, format: bool) {
    let mut cur = root;
    loop {
        let n = doc.n(cur);
        let mut descend = false;
        match n.kind {
            DOCUMENT | HTML_DOCUMENT => {
                if let Some(dt) = doc.doctype() {
                    html_doctype(doc, out, dt);
                }
                if n.first.is_some() {
                    descend = true;
                } else {
                    out.push('\n');
                }
            }
            FRAGMENT => descend = n.first.is_some(),
            ELEMENT => {
                let info = inline_info(doc, cur);
                out.push('<');
                out.push_str(&n.name);
                ns_defs(doc, out, cur);
                for &a in &n.attrs {
                    html_attr(doc, out, a);
                }
                if info.is_some_and(|i| i.2) {
                    out.push('>');
                } else if n.first.is_none() {
                    if info.is_some_and(|i| i.1 != 0) && n.name != "html" && n.name != "body" {
                        out.push('>');
                    } else {
                        out.push_str("></");
                        out.push_str(&n.name);
                        out.push('>');
                    }
                } else {
                    out.push('>');
                    if format && info.is_some_and(|i| !i.3) && not_text(doc, n.first) && n.first != n.last && !n.name.starts_with('p') {
                        out.push('\n');
                    }
                    descend = true;
                }
                if !descend && format && n.next.is_some() && info.is_some_and(|i| !i.3) {
                    let parent_p = n.parent.map(|p| doc.n(p).name.starts_with('p'));
                    if not_text(doc, n.next) && parent_p == Some(false) {
                        out.push('\n');
                    }
                }
            }
            ATTRIBUTE => html_attr(doc, out, cur),
            TEXT | CDATA => {
                let parent = n.parent.map(|p| doc.n(p).name.to_ascii_lowercase());
                let raw_parent = matches!(parent.as_deref(), Some("script") | Some("style"));
                if n.raw || raw_parent {
                    out.push_str(&n.value);
                } else {
                    html_escape(out, &n.value, false);
                }
            }
            COMMENT => {
                out.push_str("<!--");
                out.push_str(&n.value);
                out.push_str("-->");
            }
            PI => {
                out.push_str("<?");
                out.push_str(&n.name);
                if !n.value.is_empty() {
                    out.push(' ');
                    out.push_str(&n.value);
                }
                out.push('>');
            }
            ENTITY_REF => {
                out.push('&');
                out.push_str(&n.name);
                out.push(';');
            }
            _ => {}
        }
        if descend {
            cur = n.first.unwrap();
            continue;
        }
        // Climb to the next node, closing elements.
        loop {
            if cur == root {
                return;
            }
            let n = doc.n(cur);
            if let Some(nx) = n.next {
                cur = nx;
                break;
            }
            let Some(p) = n.parent else { return };
            cur = p;
            let pn = doc.n(cur);
            if matches!(pn.kind, DOCUMENT | HTML_DOCUMENT) {
                out.push('\n');
            } else if pn.kind == ELEMENT {
                let info = if format { inline_info(doc, cur) } else { None };
                if format && info.is_some_and(|i| !i.3) && not_text(doc, pn.last) && pn.first != pn.last && !pn.name.starts_with('p') {
                    out.push('\n');
                }
                out.push_str("</");
                out.push_str(&pn.name);
                out.push('>');
                if format && info.is_some_and(|i| !i.3) && pn.next.is_some() {
                    let parent_p = pn.parent.map(|p| doc.n(p).name.starts_with('p'));
                    if not_text(doc, pn.next) && parent_p == Some(false) {
                        out.push('\n');
                    }
                }
            }
            if cur == root {
                return;
            }
        }
    }
}

/// htmlGetMetaEncoding: charset of `<meta http-equiv="Content-Type">`.
fn meta_encoding(doc: &Doc) -> Option<String> {
    let mut stack = vec![0];
    while let Some(id) = stack.pop() {
        for c in doc.children(id) {
            let n = doc.n(c);
            if n.kind != ELEMENT {
                continue;
            }
            match n.name.as_str() {
                "html" | "head" => stack.push(c),
                "meta" => {
                    let http = doc.get_attr(c, "http-equiv").is_some_and(|v| v.eq_ignore_ascii_case("Content-Type"));
                    if let (true, Some(content)) = (http, doc.get_attr(c, "content")) {
                        let lower = content.to_ascii_lowercase();
                        if let Some(p) = lower.find("charset=").map(|p| p + 8).or_else(|| lower.find("charset =").map(|p| p + 9)) {
                            return Some(content[p..].trim_start_matches([' ', '\t']).to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

pub fn save_html_doc(doc: &Doc, format: bool) -> Vec<u8> {
    let mut out = String::new();
    html_node(doc, &mut out, 0, format);
    let enc = meta_encoding(doc).unwrap_or_else(|| "HTML".into());
    encode_output(out, Some(&enc))
}

// ---- XML -----------------------------------------------------------------------

/// How non-ASCII text is written.
#[derive(Clone, Copy, PartialEq)]
pub enum Escape {
    /// Character references (no output encoding).
    Refs,
    /// As is.
    Raw,
}

fn xml_text(out: &mut String, s: &str, esc: Escape) {
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '\r' => out.push_str(if esc == Escape::Refs { "&#xD;" } else { "&#13;" }),
            '\n' | '\t' | ' '..='\x7F' => out.push(ch),
            _ if (ch as u32) < 0x80 => hex_ref(out, '\u{FFFD}'),
            _ if esc == Escape::Refs => hex_ref(out, ch),
            _ => out.push(ch),
        }
    }
}

fn xml_attr_text(out: &mut String, s: &str, refs: bool) {
    for ch in s.chars() {
        match ch {
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ if refs && (ch as u32) >= 0x80 => hex_ref(out, ch),
            _ => out.push(ch),
        }
    }
}

fn ns_defs(doc: &Doc, out: &mut String, el: Id) {
    for (p, uri) in &doc.n(el).ns_defs {
        if p.as_deref() == Some("xml") {
            continue;
        }
        out.push_str(" xmlns");
        if let Some(p) = p {
            out.push(':');
            out.push_str(p);
        }
        out.push_str("=\"");
        xml_attr_text(out, uri, doc.encoding.is_none());
        out.push('"');
    }
}

fn xml_attr(doc: &Doc, out: &mut String, a: Id) {
    let n = doc.n(a);
    out.push(' ');
    out.push_str(&n.name);
    out.push_str("=\"");
    xml_attr_text(out, &n.value, doc.encoding.is_none());
    out.push('"');
}

fn xml_doctype(doc: &Doc, out: &mut String, dt: Id) {
    let n = doc.n(dt);
    let (public, system, subset) = &**n.ids.as_ref().unwrap();
    out.push_str("<!DOCTYPE ");
    out.push_str(&n.name);
    if !public.is_empty() {
        out.push_str(" PUBLIC ");
        quoted(out, public);
        out.push(' ');
        quoted(out, system);
    } else if !system.is_empty() {
        out.push_str(" SYSTEM ");
        quoted(out, system);
    }
    if subset.is_empty() {
        out.push('>');
    } else {
        out.push_str(" [\n");
        out.push_str(subset);
        out.push_str("]>");
    }
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level.min(60 / 2) {
        out.push_str("  ");
    }
}

/// xmlNodeDumpOutputInternal.
pub fn xml_node(doc: &Doc, out: &mut String, root: Id, format: bool, esc: Escape) {
    let mut format = format;
    let mut level = 0usize;
    let mut unformatted: Option<Id> = None;
    let mut cur = root;
    loop {
        let n = doc.n(cur);
        let mut descend = false;
        match n.kind {
            DOCUMENT | HTML_DOCUMENT => {
                xml_doc_content(doc, out, format, esc);
            }
            DOCTYPE => xml_doctype(doc, out, cur),
            FRAGMENT => descend = n.first.is_some(),
            ELEMENT => {
                if cur != root && format {
                    indent(out, level);
                }
                out.push('<');
                out.push_str(&n.name);
                ns_defs(doc, out, cur);
                for &a in &n.attrs {
                    xml_attr(doc, out, a);
                }
                if n.first.is_none() {
                    out.push_str("/>");
                } else {
                    if format && doc.children(cur).any(|c| matches!(doc.n(c).kind, TEXT | CDATA | ENTITY_REF)) {
                        format = false;
                        unformatted = Some(cur);
                    }
                    out.push('>');
                    if format {
                        out.push('\n');
                    }
                    level += 1;
                    descend = true;
                }
            }
            TEXT => {
                if n.raw {
                    out.push_str(&n.value);
                } else {
                    xml_text(out, &n.value, esc);
                }
            }
            PI => {
                if cur != root && format {
                    indent(out, level);
                }
                out.push_str("<?");
                out.push_str(&n.name);
                if !n.value.is_empty() {
                    out.push(' ');
                    out.push_str(&n.value);
                }
                out.push_str("?>");
            }
            COMMENT => {
                if cur != root && format {
                    indent(out, level);
                }
                out.push_str("<!--");
                out.push_str(&n.value);
                out.push_str("-->");
            }
            ENTITY_REF => {
                out.push('&');
                out.push_str(&n.name);
                out.push(';');
            }
            CDATA => {
                if n.value.is_empty() {
                    out.push_str("<![CDATA[]]>");
                } else {
                    let mut rest = n.value.as_str();
                    while let Some(p) = rest.find("]]>") {
                        out.push_str("<![CDATA[");
                        out.push_str(&rest[..p + 2]);
                        out.push_str("]]>");
                        rest = &rest[p + 2..];
                    }
                    if !rest.is_empty() {
                        out.push_str("<![CDATA[");
                        out.push_str(rest);
                        out.push_str("]]>");
                    }
                }
            }
            ATTRIBUTE => xml_attr(doc, out, cur),
            _ => {}
        }
        if descend {
            cur = n.first.unwrap();
            continue;
        }
        loop {
            if cur == root {
                return;
            }
            if format {
                out.push('\n');
            }
            let n = doc.n(cur);
            if let Some(nx) = n.next {
                cur = nx;
                break;
            }
            let Some(p) = n.parent else { return };
            cur = p;
            let pn = doc.n(cur);
            if pn.kind == ELEMENT {
                level = level.saturating_sub(1);
                if format {
                    indent(out, level);
                }
                out.push_str("</");
                out.push_str(&pn.name);
                out.push('>');
                if unformatted == Some(cur) {
                    format = true;
                    unformatted = None;
                }
            }
        }
    }
}

/// xmlDocContentDumpOutput as XML: declaration, then each top-level node
/// on its own line.
fn xml_doc_content(doc: &Doc, out: &mut String, format: bool, esc: Escape) {
    out.push_str("<?xml version=");
    quoted(out, doc.version.as_deref().unwrap_or("1.0"));
    if let Some(e) = &doc.encoding {
        out.push_str(" encoding=");
        quoted(out, e);
    }
    match doc.standalone {
        0 => out.push_str(" standalone=\"no\""),
        1 => out.push_str(" standalone=\"yes\""),
        _ => {}
    }
    out.push_str("?>\n");
    for c in doc.children(0) {
        xml_node(doc, out, c, format, esc);
        out.push('\n');
    }
}

pub fn save_xml_doc(doc: &Doc, format: bool, no_decl: bool) -> Vec<u8> {
    let mut out = String::new();
    let esc = if doc.encoding.is_some() { Escape::Raw } else { Escape::Refs };
    if no_decl {
        for c in doc.children(0) {
            xml_node(doc, &mut out, c, format, esc);
            out.push('\n');
        }
    } else {
        xml_doc_content(doc, &mut out, format, esc);
    }
    encode_output(out, doc.encoding.as_deref())
}

pub fn save_xml_node(doc: &Doc, id: Id, format: bool) -> Vec<u8> {
    let mut out = String::new();
    xml_node(doc, &mut out, id, format, Escape::Raw);
    out.into_bytes()
}
