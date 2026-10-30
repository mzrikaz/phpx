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
        let mut out = Vec::new();
        match encode(it, &arg(a, 0), flags, 0, &mut out) {
            Ok(()) => {
                LAST_ERROR.set((0, "No error"));
                Ok(Value::Str(crate::pstr::PStr::from_bytes(&out)))
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
        let mut p = Parser::new(it, &src, if has(a, 2) { int(it, a, 2)? as usize } else { 512 }, assoc);
        let r = p.ws_value().and_then(|v| {
            p.ws();
            if p.i == p.b.len() { Ok(v) } else { Err((4, "Syntax error")) }
        });
        match r {
            Ok(v) => {
                LAST_ERROR.set((0, "No error"));
                Ok(v)
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
        let mut p = Parser::new(it, &src, 512, true);
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

fn nl(out: &mut Vec<u8>, flags: i64, depth: usize) {
    if flags & PRETTY != 0 {
        out.push(b'\n');
        for _ in 0..depth {
            out.extend_from_slice(b"    ");
        }
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

fn encode(it: &Interp, v: &Value, flags: i64, depth: usize, out: &mut Vec<u8>) -> Result<(), EncErr> {
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
        Value::Null => out.extend_from_slice(b"null"),
        Value::Resource(_) => return Err(EncErr::Json(8, "Type is not supported")),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Int(n) => out.extend_from_slice(crate::pstr::fmt_int(*n, &mut [0; 20])),
        Value::Float(f) => {
            if !f.is_finite() {
                return Err(EncErr::Json(7, "Inf and NaN cannot be JSON encoded"));
            }
            let s = if flags & PRESERVE_ZERO_FRACTION != 0 { fmt_float_repr(*f) } else { crate::value::fmt_float(*f) };
            // JSON's exponent is lowercase (`1.0e+25`), unlike echo's.
            out.extend(s.bytes().map(|c| if c == b'E' { b'e' } else { c }));
        }
        Value::Str(b) => encode_str(b, flags, out)?,
        Value::Array(a) => {
            // (Held: a jsonSerialize() that changes the array copies it.)
            let a = a.clone();
            if a.is_list() && flags & FORCE_OBJECT == 0 {
                if a.is_empty() {
                    out.extend_from_slice(b"[]");
                    return Ok(());
                }
                out.push(b'[');
                for (i, (_, v)) in a.iter_raw().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    nl(out, flags, depth + 1);
                    encode(it, v, flags, depth + 1, out)?;
                }
                nl(out, flags, depth);
                out.push(b']');
            } else {
                encode_pairs(it, a.iter_raw(), a.is_empty(), flags, depth, out)?;
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
                return encode_pairs(it, a.iter_raw(), a.is_empty(), flags, depth, out);
            }
            if o.class.is_enum() {
                return match o.get_prop("value") {
                    Some(v) => encode(it, &v, flags, depth, out),
                    None => Err(EncErr::Json(11, "Non-backed enums have no default serialization")),
                };
            }
            // (Collected first: encoding may run PHP code that changes them.)
            let mut pairs: Vec<(Rc<str>, Value)> = o
                .props
                .borrow()
                .iter()
                .filter(|(n, _)| o.class.find_prop(n).is_none_or(|d| d.vis == crate::class::Vis::Public))
                .map(|(n, v)| (n.clone(), v.clone()))
                .collect();
            pairs.extend(o.view_props());
            let empty = pairs.is_empty();
            encode_pairs(it, pairs.iter().map(|(k, v)| (k, v)), empty, flags, depth, out)?;
        }
    }
    Ok(())
}

/// An array key or property name, written as a JSON string.
trait JsonKey {
    fn write(&self, flags: i64, out: &mut Vec<u8>) -> Result<(), EncErr>;
}

impl JsonKey for Key {
    fn write(&self, flags: i64, out: &mut Vec<u8>) -> Result<(), EncErr> {
        match self {
            Key::Int(n) => {
                out.push(b'"');
                out.extend_from_slice(crate::pstr::fmt_int(*n, &mut [0; 20]));
                out.push(b'"');
                Ok(())
            }
            Key::Str(s) => encode_str(s, flags, out),
        }
    }
}

impl JsonKey for &Rc<str> {
    fn write(&self, flags: i64, out: &mut Vec<u8>) -> Result<(), EncErr> {
        encode_str(self.as_bytes(), flags, out)
    }
}

/// `{"key":value,...}` (`{}` when `empty`).
fn encode_pairs<'v, K: JsonKey>(it: &Interp, pairs: impl Iterator<Item = (K, &'v Value)>, empty: bool, flags: i64, depth: usize, out: &mut Vec<u8>) -> Result<(), EncErr> {
    if empty {
        out.extend_from_slice(b"{}");
        return Ok(());
    }
    out.push(b'{');
    for (i, (k, v)) in pairs.enumerate() {
        if i > 0 {
            out.push(b',');
        }
        nl(out, flags, depth + 1);
        k.write(flags, out)?;
        out.extend_from_slice(if flags & PRETTY != 0 { b": " } else { b":" });
        encode(it, v, flags, depth + 1, out)?;
    }
    nl(out, flags, depth);
    out.push(b'}');
    Ok(())
}

/// Bytes an ASCII string can copy as they are (under `flags`).
fn plain_ascii(flags: i64) -> [bool; 128] {
    let mut t = [true; 128];
    for (c, ok) in t.iter_mut().enumerate() {
        *ok = match c as u8 {
            0..0x20 | b'"' | b'\\' => false,
            b'/' => flags & UNESCAPED_SLASHES != 0,
            b'<' | b'>' => flags & HEX_TAG == 0,
            b'&' => flags & HEX_AMP == 0,
            b'\'' => flags & HEX_APOS == 0,
            _ => true,
        };
    }
    t
}

fn encode_str(b: &[u8], flags: i64, out: &mut Vec<u8>) -> Result<(), EncErr> {
    if b.is_ascii() {
        // Runs of plain bytes copied at once; the rest escaped below.
        thread_local! {
            static PLAIN: std::cell::Cell<(i64, [bool; 128])> = const { std::cell::Cell::new((-1, [false; 128])) };
        }
        let (f, plain) = PLAIN.get();
        let plain = if f == flags { plain } else {
            let t = plain_ascii(flags);
            PLAIN.set((flags, t));
            t
        };
        out.reserve(b.len() + 2);
        out.push(b'"');
        let mut run = 0;
        for (i, &c) in b.iter().enumerate() {
            if plain[c as usize] {
                continue;
            }
            out.extend_from_slice(&b[run..i]);
            run = i + 1;
            escape_char(c as char, flags, out);
        }
        out.extend_from_slice(&b[run..]);
        out.push(b'"');
        return Ok(());
    }
    let s = std::str::from_utf8(b).map_err(|_| EncErr::Json(5, "Malformed UTF-8 characters, possibly incorrectly encoded"))?;
    out.push(b'"');
    for c in s.chars() {
        escape_char(c, flags, out);
    }
    out.push(b'"');
    Ok(())
}

fn escape_char(c: char, flags: i64, out: &mut Vec<u8>) {
    let push = |out: &mut Vec<u8>, s: &str| out.extend_from_slice(s.as_bytes());
    match c {
        '"' if flags & HEX_QUOT != 0 => push(out, "\\u0022"),
        '"' => push(out, "\\\""),
        '\\' => push(out, "\\\\"),
        '/' if flags & UNESCAPED_SLASHES == 0 => push(out, "\\/"),
        '\u{8}' => push(out, "\\b"),
        '\u{c}' => push(out, "\\f"),
        '\n' => push(out, "\\n"),
        '\r' => push(out, "\\r"),
        '\t' => push(out, "\\t"),
        '<' if flags & HEX_TAG != 0 => push(out, "\\u003C"),
        '>' if flags & HEX_TAG != 0 => push(out, "\\u003E"),
        '&' if flags & HEX_AMP != 0 => push(out, "\\u0026"),
        '\'' if flags & HEX_APOS != 0 => push(out, "\\u0027"),
        c if (c as u32) < 0x20 => push(out, &format!("\\u{:04x}", c as u32)),
        c if (c as u32) > 0x7F && flags & UNESCAPED_UNICODE == 0 => {
            let mut buf = [0u16; 2];
            for u in c.encode_utf16(&mut buf) {
                push(out, &format!("\\u{u:04x}"));
            }
        }
        c => {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
}

// ---- decoding ----------------------------------------------------------------

struct Parser<'a> {
    it: &'a Interp,
    b: &'a [u8],
    i: usize,
    depth: usize,
    max_depth: usize,
    /// Objects as arrays; otherwise stdClass (looked up once).
    assoc: bool,
    std: Option<Rc<crate::class::Class>>,
}

type PR<T> = Result<T, (i64, &'static str)>;
const SYNTAX: (i64, &str) = (4, "Syntax error");
const CONTROL: (i64, &str) = (3, "Control character error, possibly incorrectly encoded");

impl<'a> Parser<'a> {
    fn new(it: &'a Interp, b: &'a [u8], max_depth: usize, assoc: bool) -> Self {
        Parser { it, b, i: 0, depth: 0, max_depth, assoc, std: None }
    }

    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn ws_value(&mut self) -> PR<Value> {
        self.ws();
        self.value()
    }

    fn lit(&mut self, s: &[u8], v: Value) -> PR<Value> {
        if self.b[self.i..].starts_with(s) {
            self.i += s.len();
            Ok(v)
        } else {
            Err(SYNTAX)
        }
    }

    fn enter(&mut self) -> PR<()> {
        // (As in PHP, values inside the innermost array count as a level:
        // `[1]` needs a depth of 2.)
        self.depth += 1;
        if self.depth >= self.max_depth {
            return Err((1, "Maximum stack depth exceeded"));
        }
        self.i += 1;
        self.ws();
        Ok(())
    }

    /// After an element: true at the closing `close`, false after a comma.
    fn next(&mut self, close: u8) -> PR<bool> {
        self.ws();
        match self.b.get(self.i) {
            Some(b',') => {
                self.i += 1;
                Ok(false)
            }
            Some(&c) if c == close => {
                self.i += 1;
                self.depth -= 1;
                Ok(true)
            }
            _ => Err(SYNTAX),
        }
    }

    fn value(&mut self) -> PR<Value> {
        if phpx_runtime::stack_remaining().is_some_and(|r| r < 256 * 1024) {
            return Err((1, "Maximum stack depth exceeded"));
        }
        match self.b.get(self.i) {
            Some(b'n') => self.lit(b"null", Value::Null),
            Some(b't') => self.lit(b"true", Value::Bool(true)),
            Some(b'f') => self.lit(b"false", Value::Bool(false)),
            Some(b'"') => self.string(|s| Value::Str(crate::pstr::PStr::from_bytes(s))),
            Some(b'[') => {
                self.enter()?;
                let mut items = vec![];
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    self.depth -= 1;
                } else {
                    loop {
                        items.push(self.ws_value()?);
                        if self.next(b']')? {
                            break;
                        }
                    }
                }
                Ok(Value::array(PhpArray::from_list(items.into_iter())))
            }
            Some(b'{') => {
                self.enter()?;
                let mut arr = self.assoc.then(PhpArray::new);
                // (PHP creates the object once its first member is parsed,
                // after any objects inside that member.)
                let mut obj = None;
                let new_obj = |p: &mut Self| {
                    let cls = p.std.get_or_insert_with(|| p.it.classes.borrow().get("stdclass").cloned().unwrap()).clone();
                    Obj::new(cls, crate::value::Internal::None)
                };
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    self.depth -= 1;
                } else {
                    loop {
                        self.ws();
                        if self.b.get(self.i) != Some(&b'"') {
                            return Err(SYNTAX);
                        }
                        let k = match &arr {
                            Some(_) => Ok(self.string(Key::from_bytes)?),
                            None => Err(self.string(|s| Rc::<str>::from(crate::pstr::lossy(s).as_ref()))?),
                        };
                        self.ws();
                        if self.b.get(self.i) != Some(&b':') {
                            return Err(SYNTAX);
                        }
                        self.i += 1;
                        let v = self.ws_value()?;
                        match (k, &mut arr) {
                            (Ok(k), Some(a)) => {
                                a.insert(k, v);
                            }
                            (Err(n), _) => {
                                obj.get_or_insert_with(|| new_obj(self)).props.borrow_mut().insert(n, v);
                            }
                            _ => unreachable!(),
                        }
                        if self.next(b'}')? {
                            break;
                        }
                    }
                }
                Ok(match arr {
                    Some(a) => Value::array(a),
                    None => Value::Object(obj.unwrap_or_else(|| new_obj(self))),
                })
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            _ => Err(SYNTAX),
        }
    }

    fn number(&mut self) -> PR<Value> {
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
        // SAFETY: only ASCII digits, signs, dots and exponents were accepted.
        let txt = unsafe { std::str::from_utf8_unchecked(&self.b[start..self.i]) };
        if !float {
            if let Ok(n) = txt.parse::<i64>() {
                return Ok(Value::Int(n));
            }
        }
        Ok(Value::Float(txt.parse().map_err(|_| SYNTAX)?))
    }

    /// The string at the opening quote, unescaped, handed to `make`.
    fn string<T>(&mut self, make: impl FnOnce(&[u8]) -> T) -> PR<T> {
        self.i += 1;
        // Without escapes: the source bytes themselves.
        let start = self.i;
        let mut j = start;
        while let Some(&c) = self.b.get(j) {
            match c {
                b'"' => {
                    self.i = j + 1;
                    return Ok(make(&self.b[start..j]));
                }
                b'\\' => break,
                c if c < 0x20 => return Err(CONTROL),
                _ => j += 1,
            }
        }
        let mut out = self.b[start..j].to_vec();
        self.i = j;
        loop {
            let Some(&c) = self.b.get(self.i) else { return Err(CONTROL) };
            self.i += 1;
            match c {
                b'"' => return Ok(make(&out)),
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
                c if c < 0x20 => return Err(CONTROL),
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
