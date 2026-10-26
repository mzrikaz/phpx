//! json_encode / json_decode with PHP's defaults (escaped slashes and
//! unicode unless flags say otherwise).

use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp};
use crate::value::{fmt_float_repr, Key, Obj, PhpArray, Value};

const FORCE_OBJECT: i64 = 16;
const UNESCAPED_SLASHES: i64 = 64;
const PRETTY: i64 = 128;
const UNESCAPED_UNICODE: i64 = 256;
const PRESERVE_ZERO_FRACTION: i64 = 1024;
const THROW: i64 = 4_194_304;
const HEX_TAG: i64 = 1;
const HEX_AMP: i64 = 2;
const HEX_APOS: i64 = 4;
const HEX_QUOT: i64 = 8;

thread_local! {
    static LAST_ERROR: std::cell::Cell<(i64, &'static str)> = const { std::cell::Cell::new((0, "No error")) };
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("json_encode", |it, _, a| {
        let flags = int(it, a, 1)?;
        let max = if crate::builtins::has(a, 2) { int(it, a, 2)? } else { 512 };
        MAX_DEPTH.set(max.max(0) as usize);
        let mut out = String::new();
        match encode(it, &arg(a, 0), flags, 0, &mut out) {
            Ok(()) => {
                LAST_ERROR.set((0, "No error"));
                Ok(Value::str(&out))
            }
            Err(EncErr::Php(c)) => Err(c),
            Err(EncErr::Json(code, msg)) => {
                LAST_ERROR.set((code, msg));
                if flags & THROW != 0 {
                    return Err(json_exception(it, msg, code));
                }
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("json_decode", |it, _, a| {
        let src = s(it, a, 0)?;
        let assoc = arg(a, 1).truthy() || (int(it, a, 3)? & 1 != 0);
        let flags = int(it, a, 3)?;
        let mut p = Parser { b: &src, i: 0, depth: 0, max_depth: if has(a, 2) { int(it, a, 2)? as usize } else { 512 } };
        let r = p.ws_value().and_then(|v| {
            p.ws();
            if p.i == p.b.len() { Ok(v) } else { Err((4, "Syntax error")) }
        });
        match r {
            Ok(v) => {
                LAST_ERROR.set((0, "No error"));
                Ok(to_php(it, v, assoc))
            }
            Err((code, msg)) => {
                LAST_ERROR.set((code, msg));
                if flags & THROW != 0 {
                    return Err(json_exception(it, msg, code));
                }
                Ok(Value::Null)
            }
        }
    });
    t.insert("json_last_error", |_, _, _| Ok(Value::Int(LAST_ERROR.get().0)));
    t.insert("json_last_error_msg", |_, _, _| Ok(Value::str(LAST_ERROR.get().1)));
    t.insert("json_validate", |it, _, a| {
        let src = s(it, a, 0)?;
        let mut p = Parser { b: &src, i: 0, depth: 0, max_depth: 512 };
        let ok = p.ws_value().is_ok() && {
            p.ws();
            p.i == p.b.len()
        };
        Ok(Value::Bool(ok))
    });
}

fn json_exception(it: &Interp, msg: &str, code: i64) -> crate::interp::Ctrl {
    let c = it.error("JsonException", msg);
    if let crate::interp::Ctrl::Throw(o) = &c {
        o.set_prop("code", Value::Int(code));
    }
    c
}

enum EncErr {
    Php(crate::interp::Ctrl),
    Json(i64, &'static str),
}

impl From<crate::interp::Ctrl> for EncErr {
    fn from(c: crate::interp::Ctrl) -> Self {
        EncErr::Php(c)
    }
}

fn nl(out: &mut String, flags: i64, depth: usize) {
    if flags & PRETTY != 0 {
        out.push('\n');
        out.push_str(&"    ".repeat(depth));
    }
}

thread_local! {
    /// json_encode()'s `$depth`: the most arrays/objects nested.
    static MAX_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(512) };
    /// Arrays and objects being encoded (outer to inner), to report a
    /// structure that contains itself the way PHP does.
    static PATH: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Pops the path entry when an array/object is done.
struct OnPath;

impl Drop for OnPath {
    fn drop(&mut self) {
        PATH.with(|p| p.borrow_mut().pop());
    }
}

fn encode(it: &Interp, v: &Value, flags: i64, depth: usize, out: &mut String) -> Result<(), EncErr> {
    let container = match v {
        Value::Array(a) => Some(Rc::as_ptr(a) as usize),
        Value::Object(o) => Some(Rc::as_ptr(o) as usize),
        _ => None,
    };
    let _on_path = match container {
        Some(p) => {
            if PATH.with(|path| path.borrow().contains(&p)) {
                return Err(EncErr::Json(6, "Recursion detected"));
            }
            PATH.with(|path| path.borrow_mut().push(p));
            Some(OnPath)
        }
        None => None,
    };
    if it.stack_low() {
        return Err(EncErr::Json(1, "Maximum stack depth exceeded"));
    }
    if container.is_some() && depth + 1 > MAX_DEPTH.get() {
        return Err(EncErr::Json(1, "Maximum stack depth exceeded"));
    }
    match v {
        Value::Ref(s) => return encode(it, &s.borrow().clone(), flags, depth, out),
        Value::Null => out.push_str("null"),
        Value::Resource(_) => return Err(EncErr::Json(8, "Type is not supported")),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Float(f) => {
            if !f.is_finite() {
                return Err(EncErr::Json(7, "Inf and NaN cannot be JSON encoded"));
            }
            let s = if flags & PRESERVE_ZERO_FRACTION != 0 { fmt_float_repr(*f) } else { crate::value::fmt_float(*f) };
            out.push_str(&s);
        }
        Value::Str(b) => encode_str(b, flags, out)?,
        Value::Array(a) => {
            if a.is_list() && flags & FORCE_OBJECT == 0 {
                if a.is_empty() {
                    out.push_str("[]");
                    return Ok(());
                }
                out.push('[');
                for (i, v) in a.values().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    nl(out, flags, depth + 1);
                    encode(it, &v, flags, depth + 1, out)?;
                }
                nl(out, flags, depth);
                out.push(']');
            } else {
                let pairs: Vec<(Vec<u8>, Value)> = a
                    .iter()
                    .map(|(k, v)| {
                        let k = match k {
                            Key::Int(n) => n.to_string().into_bytes(),
                            Key::Str(s) => s.to_vec(),
                        };
                        (k, v.clone())
                    })
                    .collect();
                encode_object(it, &pairs, flags, depth, out)?;
            }
        }
        Value::Object(o) => {
            if o.class.is_a("jsonserializable") {
                let inner = it.call_method(o, "jsonSerialize", vec![])?;
                return encode(it, &inner, flags, depth, out);
            }
            if o.class.find_method("__phpx_cast").is_some() {
                // ArrayObject / ArrayIterator: their storage, always as an object.
                let Value::Array(a) = it.call_method(o, "__phpx_cast", vec![])? else { unreachable!("__phpx_cast returns an array") };
                let pairs: Vec<(Vec<u8>, Value)> = a
                    .iter()
                    .map(|(k, v)| {
                        let k = match k {
                            Key::Int(n) => n.to_string().into_bytes(),
                            Key::Str(s) => s.to_vec(),
                        };
                        (k, v.clone())
                    })
                    .collect();
                return encode_object(it, &pairs, flags, depth, out);
            }
            if o.class.is_enum() {
                return match o.get_prop("value") {
                    Some(v) => encode(it, &v, flags, depth, out),
                    None => Err(EncErr::Json(11, "Non-backed enums have no default serialization")),
                };
            }
            let mut pairs: Vec<(Vec<u8>, Value)> = o
                .props
                .borrow()
                .iter()
                .filter(|(n, _)| o.class.find_prop(n).is_none_or(|d| d.vis == crate::class::Vis::Public))
                .map(|(n, v)| (n.as_bytes().to_vec(), v.clone()))
                .collect();
            pairs.extend(o.view_props().into_iter().map(|(n, v)| (n.as_bytes().to_vec(), v)));
            encode_object(it, &pairs, flags, depth, out)?;
        }
    }
    Ok(())
}

fn encode_object(it: &Interp, pairs: &[(Vec<u8>, Value)], flags: i64, depth: usize, out: &mut String) -> Result<(), EncErr> {
    if pairs.is_empty() {
        out.push_str("{}");
        return Ok(());
    }
    out.push('{');
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        nl(out, flags, depth + 1);
        encode_str(k, flags, out)?;
        out.push_str(if flags & PRETTY != 0 { ": " } else { ":" });
        encode(it, v, flags, depth + 1, out)?;
    }
    nl(out, flags, depth);
    out.push('}');
    Ok(())
}

fn encode_str(b: &[u8], flags: i64, out: &mut String) -> Result<(), EncErr> {
    let s = std::str::from_utf8(b).map_err(|_| EncErr::Json(5, "Malformed UTF-8 characters, possibly incorrectly encoded"))?;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' if flags & HEX_QUOT != 0 => out.push_str("\\u0022"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' if flags & UNESCAPED_SLASHES == 0 => out.push_str("\\/"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' if flags & HEX_TAG != 0 => out.push_str("\\u003C"),
            '>' if flags & HEX_TAG != 0 => out.push_str("\\u003E"),
            '&' if flags & HEX_AMP != 0 => out.push_str("\\u0026"),
            '\'' if flags & HEX_APOS != 0 => out.push_str("\\u0027"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) > 0x7F && flags & UNESCAPED_UNICODE == 0 => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(())
}

// ---- decoding ----------------------------------------------------------------

enum J {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Vec<u8>),
    Arr(Vec<J>),
    Obj(Vec<(Vec<u8>, J)>),
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
    max_depth: usize,
}

type PR<T> = Result<T, (i64, &'static str)>;
const SYNTAX: (i64, &str) = (4, "Syntax error");

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn ws_value(&mut self) -> PR<J> {
        self.ws();
        self.value()
    }

    fn lit(&mut self, s: &[u8], v: J) -> PR<J> {
        if self.b[self.i..].starts_with(s) {
            self.i += s.len();
            Ok(v)
        } else {
            Err(SYNTAX)
        }
    }

    fn value(&mut self) -> PR<J> {
        if phpx_runtime::stack_remaining().is_some_and(|r| r < 256 * 1024) {
            return Err((1, "Maximum stack depth exceeded"));
        }
        match self.b.get(self.i) {
            Some(b'n') => self.lit(b"null", J::Null),
            Some(b't') => self.lit(b"true", J::Bool(true)),
            Some(b'f') => self.lit(b"false", J::Bool(false)),
            Some(b'"') => self.string().map(J::Str),
            Some(b'[') => {
                self.depth += 1;
                if self.depth > self.max_depth {
                    return Err((1, "Maximum stack depth exceeded"));
                }
                self.i += 1;
                let mut items = vec![];
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    self.depth -= 1;
                    return Ok(J::Arr(items));
                }
                loop {
                    items.push(self.ws_value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            self.depth -= 1;
                            return Ok(J::Arr(items));
                        }
                        _ => return Err(SYNTAX),
                    }
                }
            }
            Some(b'{') => {
                self.depth += 1;
                if self.depth > self.max_depth {
                    return Err((1, "Maximum stack depth exceeded"));
                }
                self.i += 1;
                let mut items = vec![];
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    self.depth -= 1;
                    return Ok(J::Obj(items));
                }
                loop {
                    self.ws();
                    if self.b.get(self.i) != Some(&b'"') {
                        return Err(SYNTAX);
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') {
                        return Err(SYNTAX);
                    }
                    self.i += 1;
                    let v = self.ws_value()?;
                    items.push((k, v));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            self.depth -= 1;
                            return Ok(J::Obj(items));
                        }
                        _ => return Err(SYNTAX),
                    }
                }
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            _ => Err(SYNTAX),
        }
    }

    fn number(&mut self) -> PR<J> {
        let start = self.i;
        if self.b[self.i] == b'-' {
            self.i += 1;
        }
        let digits = |p: &mut Self| {
            let s = p.i;
            while p.i < p.b.len() && p.b[p.i].is_ascii_digit() {
                p.i += 1;
            }
            p.i - s
        };
        let int_start = self.i;
        if digits(self) == 0 || (self.b[int_start] == b'0' && self.i - int_start > 1) {
            return Err(SYNTAX);
        }
        let mut float = false;
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            if digits(self) == 0 {
                return Err(SYNTAX);
            }
            float = true;
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if digits(self) == 0 {
                return Err(SYNTAX);
            }
            float = true;
        }
        let txt = std::str::from_utf8(&self.b[start..self.i]).unwrap();
        if !float {
            if let Ok(n) = txt.parse::<i64>() {
                return Ok(J::Int(n));
            }
        }
        Ok(J::Float(txt.parse().map_err(|_| SYNTAX)?))
    }

    fn string(&mut self) -> PR<Vec<u8>> {
        self.i += 1;
        let mut out = vec![];
        loop {
            let Some(&c) = self.b.get(self.i) else { return Err((3, "Control character error, possibly incorrectly encoded")) };
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(&e) = self.b.get(self.i) else { return Err(SYNTAX) };
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.b[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let ch = char::from_u32(cp).ok_or((10, "Single unpaired UTF-16 surrogate in unicode escape"))?;
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(SYNTAX),
                    }
                }
                c if c < 0x20 => return Err((3, "Control character error, possibly incorrectly encoded")),
                c => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> PR<u32> {
        let h = self.b.get(self.i..self.i + 4).ok_or(SYNTAX)?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(h).map_err(|_| SYNTAX)?, 16).map_err(|_| SYNTAX)
    }
}

fn to_php(it: &Interp, j: J, assoc: bool) -> Value {
    match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(b),
        J::Int(n) => Value::Int(n),
        J::Float(f) => Value::Float(f),
        J::Str(s) => Value::bytes(s),
        J::Arr(items) => Value::array(PhpArray::from_list(items.into_iter().map(|x| to_php(it, x, assoc)))),
        J::Obj(items) if assoc => {
            let mut a = PhpArray::new();
            for (k, v) in items {
                a.insert(Key::from_bytes(&k), to_php(it, v, assoc));
            }
            Value::array(a)
        }
        J::Obj(items) => {
            let cls = it.classes.borrow().get("stdclass").cloned().unwrap();
            let o = Obj::new(cls, crate::value::Internal::None);
            for (k, v) in items {
                let v = to_php(it, v, assoc);
                o.props.borrow_mut().insert(Rc::from(crate::pstr::lossy(&k).as_ref()), v);
            }
            Value::Object(o)
        }
    }
}
