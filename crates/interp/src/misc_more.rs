//! Assorted ext/standard functions: ini_parse_quantity, getopt, image
//! sizes, syntax highlighting, php_strip_whitespace, get_meta_tags,
//! fscanf, dir() and small odds and ends.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Frame, Interp, R};
use crate::value::{Key, PhpArray, Value};

fn text(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(crate::pstr::lossy(&s(it, a, i)?).into_owned())
}

fn key(k: &str) -> Key {
    Key::from_bytes(k.as_bytes())
}

// ---- ini_parse_quantity ----------------------------------------------------------------

/// zend_ini_parse_quantity: (value, warning).
pub fn parse_quantity(q: &str) -> (i64, Option<String>) {
    let t = q.trim_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    if t.is_empty() {
        return (0, None);
    }
    let b = t.as_bytes();
    let mut i = 0;
    let neg = match b[0] {
        b'-' => {
            i += 1;
            true
        }
        b'+' => {
            i += 1;
            false
        }
        _ => false,
    };
    let (radix, mut j) = if b.len() > i + 1 && b[i] == b'0' {
        match b[i + 1].to_ascii_lowercase() {
            b'x' => (16, i + 2),
            b'o' => (8, i + 2),
            b'b' => (2, i + 2),
            _ => (8, i + 1),
        }
    } else {
        (10, i)
    };
    let start = j;
    let mut n: i64 = 0;
    while j < b.len() && (b[j] as char).is_digit(radix) {
        n = n.wrapping_mul(radix as i64).wrapping_add((b[j] as char).to_digit(radix).unwrap() as i64);
        j += 1;
    }
    if j == start && !(radix == 8 && start > i) {
        return (0, Some(format!("Invalid quantity \"{q}\": no valid leading digits, interpreting as \"0\" for backwards compatibility")));
    }
    let n = if neg { -n } else { n };
    let rest = t[j..].trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        return (n, None);
    }
    let last = rest.as_bytes()[rest.len() - 1];
    let mult = match last.to_ascii_lowercase() {
        b'k' => 1i64 << 10,
        b'm' => 1 << 20,
        b'g' => 1 << 30,
        _ => {
            return (n, Some(format!("Invalid quantity \"{q}\": unknown multiplier \"{}\", interpreting as \"{}\" for backwards compatibility", last as char, &t[..j])));
        }
    };
    let v = n.wrapping_mul(mult);
    if rest.len() > 1 {
        return (v, Some(format!("Invalid quantity \"{q}\", interpreting as \"{}{}\" for backwards compatibility", &t[..j], last as char)));
    }
    (v, None)
}

// ---- getopt ----------------------------------------------------------------------

fn getopt(it: &Interp, f: &mut Frame, a: &mut [Value]) -> R<Value> {
    let short = s(it, a, 0)?.to_vec();
    let mut long: Vec<String> = vec![];
    if let Value::Array(l) = arg(a, 1) {
        for v in l.values() {
            long.push(it.to_string_lossy(&v)?);
        }
    }
    let argv: Vec<String> = match it.with_globals(f, |v| v.get("argv").map(|s| s.borrow().clone())) {
        Some(Value::Array(x)) => x.values().map(|v| it.to_string_lossy(&v).unwrap_or_default()).collect(),
        _ => vec![],
    };
    // Option spec: name -> 0 none, 1 required, 2 optional.
    let spec_short = |c: u8| -> Option<u8> {
        let p = short.iter().position(|&x| x == c && x != b':')?;
        Some(match (short.get(p + 1), short.get(p + 2)) {
            (Some(b':'), Some(b':')) => 2,
            (Some(b':'), _) => 1,
            _ => 0,
        })
    };
    let spec_long = |name: &str| -> Option<u8> {
        long.iter().find_map(|l| {
            let base = l.trim_end_matches(':');
            (base == name).then(|| match l.len() - base.len() {
                0 => 0,
                1 => 1,
                _ => 2,
            })
        })
    };
    let mut out = PhpArray::new();
    let add = |out: &mut PhpArray, k: &str, v: Value| {
        let k = key(k);
        match out.get(&k) {
            Some(Value::Array(prev)) => {
                let mut p = (*prev).clone();
                p.push(v);
                out.insert(k, Value::array(p));
            }
            Some(prev) => out.insert(k, Value::array(PhpArray::from_list([prev, v]))),
            None => out.insert(k, v),
        }
    };
    let mut i = 1;
    while i < argv.len() {
        let x = &argv[i];
        if x == "--" {
            i += 1;
            break;
        }
        if let Some(body) = x.strip_prefix("--") {
            let (name, inline) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (body, None),
            };
            match spec_long(name) {
                Some(0) => add(&mut out, name, Value::Bool(false)),
                Some(1) => {
                    let v = match inline {
                        Some(v) => Some(v),
                        None if i + 1 < argv.len() => {
                            i += 1;
                            Some(argv[i].clone())
                        }
                        None => None,
                    };
                    if let Some(v) = v {
                        add(&mut out, name, Value::str(&v));
                    }
                }
                Some(_) => add(&mut out, name, inline.map(|v| Value::str(&v)).unwrap_or(Value::Bool(false))),
                None => {}
            }
            i += 1;
            continue;
        }
        if x.len() > 1 && x.starts_with('-') {
            let b = x.as_bytes();
            let mut j = 1;
            while j < b.len() {
                let c = b[j];
                let name = (c as char).to_string();
                match spec_short(c) {
                    Some(0) => add(&mut out, &name, Value::Bool(false)),
                    Some(1) => {
                        let rest = &x[j + 1..];
                        let rest = rest.strip_prefix('=').unwrap_or(rest);
                        if !rest.is_empty() {
                            add(&mut out, &name, Value::str(rest));
                        } else if i + 1 < argv.len() {
                            i += 1;
                            add(&mut out, &name, Value::str(&argv[i]));
                        }
                        break;
                    }
                    Some(_) => {
                        let rest = &x[j + 1..];
                        let rest = rest.strip_prefix('=').unwrap_or(rest);
                        add(&mut out, &name, if rest.is_empty() { Value::Bool(false) } else { Value::str(rest) });
                        break;
                    }
                    None => {}
                }
                j += 1;
            }
            i += 1;
            continue;
        }
        break;
    }
    if a.len() > 2 {
        a[2] = Value::Int(i as i64);
    }
    Ok(Value::array(out))
}

// ---- images ------------------------------------------------------------------------

const MIME: &[(i64, &str, &str)] = &[
    (1, "image/gif", ".gif"),
    (2, "image/jpeg", ".jpeg"),
    (3, "image/png", ".png"),
    (4, "application/x-shockwave-flash", ".swf"),
    (5, "image/psd", ".psd"),
    (6, "image/bmp", ".bmp"),
    (7, "image/tiff", ".tiff"),
    (8, "image/tiff", ".tiff"),
    (9, "application/octet-stream", ".jpc"),
    (10, "image/jp2", ".jp2"),
    (11, "application/octet-stream", ".jpx"),
    (12, "application/octet-stream", ".jb2"),
    (13, "application/x-shockwave-flash", ".swf"),
    (14, "image/iff", ".iff"),
    (15, "image/vnd.wap.wbmp", ".bmp"),
    (16, "image/xbm", ".xbm"),
    (17, "image/vnd.microsoft.icon", ".ico"),
    (18, "image/webp", ".webp"),
    (19, "image/avif", ".avif"),
    (20, "image/heif", ".heif"),
];

fn be16(b: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32)
}

fn le16(b: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32)
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn le32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// (type, width, height, bits, channels)
fn image_info(b: &[u8]) -> Option<(i64, u32, u32, Option<u32>, Option<u32>)> {
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        let flags = *b.get(10)?;
        return Some((1, le16(b, 6)?, le16(b, 8)?, Some((flags & 7) as u32 + 1), Some(3)));
    }
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((3, be32(b, 16)?, be32(b, 20)?, Some(*b.get(24)? as u32), None));
    }
    if b.starts_with(b"\xff\xd8") {
        let mut i = 2;
        while i + 4 <= b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let m = b[i + 1];
            if m == 0xFF {
                i += 1;
                continue;
            }
            if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
                i += 2;
                continue;
            }
            let len = be16(b, i + 2)? as usize;
            if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                return Some((2, be16(b, i + 7)?, be16(b, i + 5)?, Some(*b.get(i + 4)? as u32), Some(*b.get(i + 9)? as u32)));
            }
            if m == 0xD9 || m == 0xDA {
                return None;
            }
            i += 2 + len;
        }
        return None;
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        let chunk = b.get(12..16)?;
        return match chunk {
            b"VP8 " => Some((18, le16(b, 26)? & 0x3fff, le16(b, 28)? & 0x3fff, Some(8), None)),
            b"VP8L" => {
                let v = le32(b, 21)?;
                Some((18, (v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1, Some(8), None))
            }
            b"VP8X" => {
                let w = u32::from_le_bytes([*b.get(24)?, *b.get(25)?, *b.get(26)?, 0]) + 1;
                let h = u32::from_le_bytes([*b.get(27)?, *b.get(28)?, *b.get(29)?, 0]) + 1;
                Some((18, w, h, Some(8), None))
            }
            _ => None,
        };
    }
    if b.starts_with(b"BM") {
        let hsize = le32(b, 14)?;
        if hsize == 12 {
            return Some((6, le16(b, 18)?, le16(b, 20)?, Some(le16(b, 24)?), None));
        }
        let h = le32(b, 22)? as i32;
        return Some((6, le32(b, 18)?, h.unsigned_abs(), Some(le16(b, 28)?), None));
    }
    if b.starts_with(b"\x00\x00\x01\x00") && le16(b, 4)? > 0 {
        let w = *b.get(6)? as u32;
        let h = *b.get(7)? as u32;
        return Some((17, if w == 0 { 256 } else { w }, if h == 0 { 256 } else { h }, Some(le16(b, 12)?), None));
    }
    None
}

fn image_size(data: &[u8]) -> Value {
    let Some((ty, w, h, bits, channels)) = image_info(data) else { return Value::Bool(false) };
    let mut out = PhpArray::new();
    out.push(Value::Int(w as i64));
    out.push(Value::Int(h as i64));
    out.push(Value::Int(ty));
    out.push(Value::str(&format!("width=\"{w}\" height=\"{h}\"")));
    if let Some(b) = bits {
        out.insert(key("bits"), Value::Int(b as i64));
    }
    if let Some(c) = channels {
        out.insert(key("channels"), Value::Int(c as i64));
    }
    out.insert(key("mime"), Value::str(MIME.iter().find(|m| m.0 == ty).map_or("application/octet-stream", |m| m.1)));
    out.insert(key("width_unit"), Value::str("px"));
    out.insert(key("height_unit"), Value::str("px"));
    Value::array(out)
}

fn iptcparse(b: &[u8]) -> Value {
    let mut out = PhpArray::new();
    let mut i = 0;
    let mut found = false;
    while i + 5 <= b.len() {
        if b[i] != 0x1C {
            if found {
                break;
            }
            i += 1;
            continue;
        }
        found = true;
        let (rec, ds) = (b[i + 1], b[i + 2]);
        let len = u16::from_be_bytes([b[i + 3], b[i + 4]]) as usize;
        i += 5;
        if len & 0x8000 != 0 || i + len > b.len() {
            break;
        }
        let k = key(&format!("{rec}#{ds:03}"));
        let v = Value::bytes(b[i..i + len].to_vec());
        let mut list = match out.get(&k) {
            Some(Value::Array(x)) => (*x).clone(),
            _ => PhpArray::new(),
        };
        list.push(v);
        out.insert(k, Value::array(list));
        i += len;
    }
    if found { Value::array(out) } else { Value::Bool(false) }
}

// ---- highlighting ------------------------------------------------------------------

const COLOR_COMMENT: &str = "#FF8000";
const COLOR_DEFAULT: &str = "#0000BB";
const COLOR_HTML: &str = "#000000";
const COLOR_KEYWORD: &str = "#007700";
const COLOR_STRING: &str = "#DD0000";

fn html_puts(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '\t' => out.push_str("    "),
            c => out.push(c),
        }
    }
}

fn ini_color(it: &Interp, name: &str, default: &'static str) -> String {
    it.ini_get(name).filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

/// zend_highlight().
pub fn highlight(it: &Interp, src: &[u8]) -> String {
    let (html, comment, default, keyword, string) = (
        ini_color(it, "highlight.html", COLOR_HTML),
        ini_color(it, "highlight.comment", COLOR_COMMENT),
        ini_color(it, "highlight.default", COLOR_DEFAULT),
        ini_color(it, "highlight.keyword", COLOR_KEYWORD),
        ini_color(it, "highlight.string", COLOR_STRING),
    );
    // Colors are compared by role (the C code compares pointers).
    let mut out = format!("<pre><code style=\"color: {html}\">");
    let mut last = 0u8;
    for t in crate::tokenizer::tokenize(src) {
        let txt = crate::pstr::lossy(&src[t.start..t.end]);
        let role = match t.id {
            267 => 0,
            392 | 393 => 1,
            394..=396 | 346..=354 => 2,
            34 | 268 | 269 => 4,
            397 => {
                html_puts(&mut out, &txt);
                continue;
            }
            260..=266 | 270 | 271 => 2,
            _ => 3,
        };
        if role != last {
            if last != 0 {
                out.push_str("</span>");
            }
            last = role;
            if last != 0 {
                let c = [&html, &comment, &default, &keyword, &string][role as usize];
                out.push_str(&format!("<span style=\"color: {c}\">"));
            }
        }
        html_puts(&mut out, &txt);
    }
    if last != 0 {
        out.push_str("</span>");
    }
    out.push_str("</code></pre>");
    out
}

/// zend_strip(): php_strip_whitespace().
fn strip(src: &[u8]) -> Vec<u8> {
    let toks = crate::tokenizer::tokenize(src);
    let mut out = vec![];
    let mut prev_space = false;
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        let txt = &src[t.start..t.end];
        match t.id {
            397 => {
                if !prev_space {
                    out.push(b' ');
                    prev_space = true;
                }
            }
            392 | 393 => {}
            399 => {
                out.extend_from_slice(txt);
                if let Some(n) = toks.get(i + 1) {
                    if n.id != 397 {
                        out.extend_from_slice(&src[n.start..n.end]);
                    }
                    i += 1;
                }
                out.push(b'\n');
                prev_space = true;
            }
            _ => {
                out.extend_from_slice(txt);
                prev_space = false;
            }
        }
        i += 1;
    }
    out
}

fn read_source(it: &Interp, path: &str, func: &str) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(b) => Some(b),
        Err(e) => {
            it.warn(&format!("{func}({path}): Failed to open stream: {}", crate::streams::io_msg(&e)));
            None
        }
    }
}

fn highlight_out(it: &Interp, html: String, ret: bool) -> Value {
    if ret {
        Value::str(&html)
    } else {
        it.echo(html.as_bytes());
        Value::Bool(true)
    }
}

// ---- get_meta_tags -----------------------------------------------------------------

fn meta_tags(src: &str) -> PhpArray {
    let mut out = PhpArray::new();
    let lower = src.to_ascii_lowercase();
    let end = lower.find("</head>").unwrap_or(lower.len());
    let mut from = 0;
    while let Some(p) = lower[from..end].find("<meta") {
        let st = from + p + 5;
        let close = lower[st..].find('>').map_or(lower.len(), |e| st + e);
        let tag = &src[st..close];
        let mut name = None;
        let mut content = None;
        let mut rest = tag;
        loop {
            rest = rest.trim_start();
            let Some(eq) = rest.find('=') else { break };
            let attr = rest[..eq].trim().to_ascii_lowercase();
            let after = rest[eq + 1..].trim_start();
            let (val, remain) = match after.chars().next() {
                Some(q @ ('"' | '\'')) => match after[1..].find(q) {
                    Some(e) => (&after[1..1 + e], &after[e + 2..]),
                    None => (&after[1..], ""),
                },
                _ => {
                    let e = after.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(after.len());
                    (&after[..e], &after[e..])
                }
            };
            match attr.as_str() {
                "name" => name = Some(val.to_string()),
                "content" => content = Some(val.to_string()),
                _ => {}
            }
            rest = remain;
        }
        if let (Some(n), Some(c)) = (name, content) {
            let k: String = n.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
            out.insert(key(&k), Value::str(&c));
        }
        from = close;
        if from >= end {
            break;
        }
    }
    out
}

// ---- debug_zval_dump -----------------------------------------------------------------

fn zval_dump(it: &Interp, out: &mut String, v: &Value, indent: usize) {
    let pad = " ".repeat(indent);
    match v.deref() {
        Value::Null => out.push_str(&format!("{pad}NULL\n")),
        Value::Bool(b) => out.push_str(&format!("{pad}bool({b})\n")),
        Value::Int(n) => out.push_str(&format!("{pad}int({n})\n")),
        Value::Float(f) => out.push_str(&format!("{pad}float({})\n", it.to_string_lossy(&Value::Float(f)).unwrap_or_default())),
        Value::Str(s) => out.push_str(&format!("{pad}string({}) \"{}\" refcount(1)\n", s.len(), crate::pstr::lossy(&s))),
        Value::Array(a) => {
            out.push_str(&format!("{pad}array({}) {}refcount(2){{\n", a.len(), if a.is_list() { "packed " } else { "" }));
            for (k, x) in a.iter() {
                match k {
                    Key::Int(n) => out.push_str(&format!("{pad}  [{n}]=>\n")),
                    Key::Str(s) => out.push_str(&format!("{pad}  [\"{}\"]=>\n", crate::pstr::lossy(&s))),
                }
                zval_dump(it, out, &x, indent + 2);
            }
            out.push_str(&format!("{pad}}}\n"));
        }
        Value::Object(o) => {
            let props: Vec<_> = o.props.borrow().iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
            out.push_str(&format!("{pad}object({})#{} ({}) refcount(2){{\n", o.class.name, o.id, props.len()));
            for (k, x) in props {
                out.push_str(&format!("{pad}  [\"{k}\"]=>\n"));
                zval_dump(it, out, &x, indent + 2);
            }
            out.push_str(&format!("{pad}}}\n"));
        }
        Value::Resource(r) => out.push_str(&format!("{pad}resource({}) of type ({}) refcount(2)\n", r.id, r.kind.get())),
        Value::Ref(_) => {}
    }
}

thread_local! {
    static PROCESS_TITLE: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("password_algos", |_, _, _| Ok(Value::array(PhpArray::from_list([Value::str("2y"), Value::str("argon2i"), Value::str("argon2id")]))));
    t.insert("ini_parse_quantity", |it, _, a| {
        let q = text(it, a, 0)?;
        let (v, warn) = parse_quantity(&q);
        if let Some(w) = warn {
            it.warn(&w);
        }
        Ok(Value::Int(v))
    });
    t.insert("getopt", getopt);
    t.insert("time_sleep_until", |it, _, a| {
        let target = crate::builtins::float(it, a, 0)?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        if target < now {
            it.warn("time_sleep_until(): Argument #1 ($timestamp) must be greater than or equal to the current time");
            return Ok(Value::Bool(false));
        }
        let _ = phpx_runtime::sleep(std::time::Duration::from_secs_f64(target - now));
        Ok(Value::Bool(true))
    });
    t.insert("debug_zval_dump", |it, _, a| {
        let mut out = String::new();
        for v in a.iter() {
            zval_dump(it, &mut out, v, 0);
        }
        it.echo(out.as_bytes());
        Ok(Value::Null)
    });
    t.insert("highlight_string", |it, _, a| {
        let src = s(it, a, 0)?;
        let html = highlight(it, &src);
        Ok(highlight_out(it, html, has(a, 1) && a[1].truthy()))
    });
    let hl_file: BuiltinFn = |it, _, a| {
        let path = text(it, a, 0)?;
        let Some(src) = read_source(it, &path, "highlight_file") else {
            it.warn(&format!("highlight_file(): Failed opening '{path}' for highlighting"));
            return Ok(Value::Bool(false));
        };
        let html = highlight(it, &src);
        Ok(highlight_out(it, html, has(a, 1) && a[1].truthy()))
    };
    t.insert("highlight_file", hl_file);
    t.insert("show_source", hl_file);
    t.insert("php_strip_whitespace", |it, _, a| {
        let path = text(it, a, 0)?;
        Ok(match read_source(it, &path, "php_strip_whitespace") {
            Some(src) => Value::bytes(strip(&src)),
            None => Value::str(""),
        })
    });
    t.insert("get_meta_tags", |it, _, a| {
        let path = text(it, a, 0)?;
        let Some(src) = read_source(it, &path, "get_meta_tags") else { return Ok(Value::Bool(false)) };
        Ok(Value::array(meta_tags(&crate::pstr::lossy(&src))))
    });
    t.insert("fscanf", |it, f, a| {
        let line = (it.builtins["fgets"])(it, f, &mut [a[0].clone()])?;
        if matches!(line, Value::Bool(false)) {
            return Ok(Value::Bool(false));
        }
        let mut args: Vec<Value> = vec![line];
        args.extend(a[1..].iter().cloned());
        let r = (it.builtins["sscanf"])(it, f, &mut args)?;
        for (i, v) in args.into_iter().enumerate().skip(2) {
            a[i] = v;
        }
        Ok(r)
    });
    t.insert("fsync", |it, f, a| (it.builtins["fflush"])(it, f, a));
    t.insert("fdatasync", |it, f, a| (it.builtins["fflush"])(it, f, a));
    t.insert("diskfreespace", |it, f, a| (it.builtins["disk_free_space"])(it, f, a));
    t.insert("dir", |it, f, a| {
        let path = text(it, a, 0)?;
        let mut args = a.to_vec();
        let h = (it.builtins["opendir"])(it, f, &mut args)?;
        if matches!(h, Value::Bool(false)) {
            return Ok(h);
        }
        let cls = it.lookup_class("Directory")?;
        let o = it.new_instance(&cls)?;
        o.set_prop("path", Value::str(&path));
        o.set_prop("handle", h);
        Ok(Value::Object(o))
    });
    t.insert("getlastmod", |it, _, _| {
        let main = it.included.borrow().first().cloned();
        Ok(match main.and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok()) {
            Some(t) => Value::Int(t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)),
            None => Value::Bool(false),
        })
    });
    t.insert("image_type_to_mime_type", |it, _, a| {
        let ty = int(it, a, 0)?;
        Ok(Value::str(MIME.iter().find(|m| m.0 == ty).map_or("application/octet-stream", |m| m.1)))
    });
    t.insert("image_type_to_extension", |it, _, a| {
        let ty = int(it, a, 0)?;
        let dot = !has(a, 1) || a[1].truthy();
        Ok(match MIME.iter().find(|m| m.0 == ty) {
            Some(m) => Value::str(if dot { m.2 } else { &m.2[1..] }),
            None => Value::Bool(false),
        })
    });
    t.insert("getimagesize", |it, _, a| {
        let path = text(it, a, 0)?;
        match std::fs::read(&path) {
            Ok(b) => Ok(image_size(&b)),
            Err(e) => {
                it.warn(&format!("getimagesize({path}): Failed to open stream: {}", crate::streams::io_msg(&e)));
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("getimagesizefromstring", |it, _, a| {
        let b = s(it, a, 0)?;
        let r = image_size(&b);
        if matches!(r, Value::Bool(false)) {
            it.notice("getimagesizefromstring(): Error reading from string");
        }
        Ok(r)
    });
    t.insert("iptcparse", |it, _, a| Ok(iptcparse(&s(it, a, 0)?)));
    t.insert("iptcembed", |it, _, _| {
        it.warn("iptcembed(): Unable to embed IPTC data");
        Ok(Value::Bool(false))
    });
    t.insert("linkinfo", |it, _, a| {
        let path = text(it, a, 0)?;
        match std::fs::symlink_metadata(&path) {
            Ok(m) => Ok(Value::Int(std::os::unix::fs::MetadataExt::dev(&m) as i64)),
            Err(e) => {
                it.warn(&format!("linkinfo(): {}", crate::streams::io_msg(&e)));
                Ok(Value::Int(-1))
            }
        }
    });
    t.insert("strptime", |it, _, a| {
        it.deprecated("Function strptime() is deprecated since 8.1, use date_parse_from_format() (for locale-independent parsing), or IntlDateFormatter::parse() (for locale-dependent parsing) instead");
        let (ts, fmt) = (s(it, a, 0)?.to_vec(), s(it, a, 1)?.to_vec());
        let cts = std::ffi::CString::new(ts.into_iter().take_while(|&c| c != 0).collect::<Vec<u8>>()).unwrap();
        let cfmt = std::ffi::CString::new(fmt.into_iter().take_while(|&c| c != 0).collect::<Vec<u8>>()).unwrap();
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let end = unsafe { libc::strptime(cts.as_ptr(), cfmt.as_ptr(), &mut tm) };
        if end.is_null() {
            return Ok(Value::Bool(false));
        }
        let rest = unsafe { std::ffi::CStr::from_ptr(end) }.to_bytes().to_vec();
        let mut out = PhpArray::new();
        for (k, v) in [("tm_sec", tm.tm_sec), ("tm_min", tm.tm_min), ("tm_hour", tm.tm_hour), ("tm_mday", tm.tm_mday), ("tm_mon", tm.tm_mon), ("tm_year", tm.tm_year), ("tm_wday", tm.tm_wday), ("tm_yday", tm.tm_yday)] {
            out.insert(key(k), Value::Int(v as i64));
        }
        out.insert(key("unparsed"), Value::bytes(rest));
        Ok(Value::array(out))
    });
    t.insert("assert_options", |it, _, a| {
        it.deprecated("Function assert_options() is deprecated since 8.3");
        let what = int(it, a, 0)?;
        Ok(match what {
            1 => Value::Int(1),
            2 => Value::Int(1),
            3 => Value::Int(0),
            5 => Value::Int(1),
            4 => Value::Null,
            _ => return Err(it.error("ValueError", "assert_options(): Argument #1 ($option) must be an ASSERT_* constant")),
        })
    });
    t.insert("register_tick_function", |_, _, _| Ok(Value::Bool(true)));
    t.insert("unregister_tick_function", |_, _, _| Ok(Value::Null));
    t.insert("get_browser", |it, _, _| {
        it.warn("get_browser(): browscap ini directive not set");
        Ok(Value::Bool(false))
    });
    t.insert("realpath_cache_get", |_, _, _| Ok(Value::array(PhpArray::new())));
    t.insert("http_get_last_response_headers", |_, _, _| Ok(Value::Null));
    t.insert("http_clear_last_response_headers", |_, _, _| Ok(Value::Null));
    t.insert("request_parse_body", |_, _, _| Ok(Value::array(PhpArray::from_list([Value::array(PhpArray::new()), Value::array(PhpArray::new())]))));
    t.insert("cli_set_process_title", |it, _, a| {
        let title = text(it, a, 0)?;
        if let Ok(c) = std::ffi::CString::new(title.clone()) {
            unsafe { libc::prctl(libc::PR_SET_NAME, c.as_ptr() as libc::c_ulong, 0, 0, 0) };
        }
        PROCESS_TITLE.with(|t| *t.borrow_mut() = title);
        Ok(Value::Bool(true))
    });
    t.insert("cli_get_process_title", |_, _, _| Ok(PROCESS_TITLE.with(|t| Value::str(&t.borrow()))));
    t.insert("phpinfo", |it, _, _| {
        let mut out = String::from("phpinfo()\nPHP Version => 8.5.10\n\nSystem => ");
        let mut u: libc::utsname = unsafe { std::mem::zeroed() };
        unsafe { libc::uname(&mut u) };
        let f = |b: &[libc::c_char]| unsafe { std::ffi::CStr::from_ptr(b.as_ptr()) }.to_string_lossy().into_owned();
        out.push_str(&format!("{} {} {} {} {}\n", f(&u.sysname), f(&u.nodename), f(&u.release), f(&u.version), f(&u.machine)));
        out.push_str("Server API => Command Line Interface\nVirtual Directory Support => disabled\nZend Extension Build => API420250925,NTS\nPHP Extension Build => API20250925,NTS\nThread Safety => disabled\nRuntime => phpx\n");
        it.echo(out.as_bytes());
        Ok(Value::Bool(true))
    });
    t.insert("phpcredits", |it, _, _| {
        it.echo(b"PHP Credits\n\nPHP Group\nThies C. Arntzen, Stig Bakken, Shane Caraveo, Andi Gutmans, Rasmus Lerdorf, Sam Ruby, Sascha Schumann, Zeev Suraski, Jim Winstead, Andrei Zmievski\n");
        Ok(Value::Bool(true))
    });
    t.insert("dl", |it, _, _| {
        it.warn("dl(): Dynamically loaded extensions aren't enabled");
        Ok(Value::Bool(false))
    });
}
