//! Standard-library breadth: serialize, pack, hash, URLs, version_compare
//! and assorted string functions.

use std::collections::HashMap;
use std::rc::Rc;

use digest::Digest;

use crate::builtins::{arg, has, int, s};
use crate::class::Vis;
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{pstr, Internal, Key, Obj, PhpArray, Value};

// ---- serialize ----------------------------------------------------------------

fn ser(it: &Interp, v: &Value, out: &mut Vec<u8>) -> R<()> {
    if it.stack_low() {
        return Err(it.stack_overflow());
    }
    match v {
        Value::Ref(r) => return ser(it, &r.borrow().clone(), out),
        Value::Null | Value::Resource(_) => out.extend_from_slice(b"N;"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"b:1;" } else { b"b:0;" }),
        Value::Int(n) => out.extend_from_slice(format!("i:{n};").as_bytes()),
        Value::Float(f) => out.extend_from_slice(format!("d:{};", ser_float(*f)).as_bytes()),
        Value::Str(b) => ser_str(b, out),
        Value::Array(a) => {
            out.extend_from_slice(format!("a:{}:{{", a.len()).as_bytes());
            for (k, v) in a.iter() {
                ser_key(&k, out);
                ser(it, &v, out)?;
            }
            out.push(b'}');
        }
        Value::Object(o) => ser_obj(it, o, out)?,
    }
    Ok(())
}

fn ser_float(f: f64) -> String {
    if f.is_nan() {
        "NAN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "INF".into() } else { "-INF".into() }
    } else {
        crate::value::fmt_float_repr(f).trim_end_matches(".0").to_string()
    }
}

fn ser_str(b: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(format!("s:{}:\"", b.len()).as_bytes());
    out.extend_from_slice(b);
    out.extend_from_slice(b"\";");
}

fn ser_key(k: &Key, out: &mut Vec<u8>) {
    match k {
        Key::Int(n) => out.extend_from_slice(format!("i:{n};").as_bytes()),
        Key::Str(s) => ser_str(s, out),
    }
}

/// Property name as serialize()/(array) cast spell it: `\0Class\0p`
/// (private), `\0*\0p` (protected).
pub fn mangled(o: &Obj, name: &str) -> Vec<u8> {
    match o.class.find_prop(name) {
        Some(d) if d.vis == Vis::Private => [b"\0", d.declaring.as_bytes(), b"\0", name.as_bytes()].concat(),
        Some(d) if d.vis == Vis::Protected => [b"\0*\0", name.as_bytes()].concat(),
        _ => name.as_bytes().to_vec(),
    }
}

fn ser_obj(it: &Interp, o: &Rc<Obj>, out: &mut Vec<u8>) -> R<()> {
    let cname = o.class.name.clone();
    if o.class.is_enum() {
        let case = match o.get_prop("name") {
            Some(Value::Str(n)) => String::from_utf8_lossy(&n).into_owned(),
            _ => String::new(),
        };
        let body = format!("{cname}:{case}");
        out.extend_from_slice(format!("E:{}:\"{body}\";", body.len()).as_bytes());
        return Ok(());
    }
    if matches!(&*o.internal.borrow(), Internal::Closure(_)) {
        return Err(it.error("Exception", "Serialization of 'Closure' is not allowed"));
    }
    if o.class.find_method("__serialize").is_some() {
        let data = it.call_method(o, "__serialize", vec![])?;
        let Value::Array(a) = data else {
            return Err(it.error("TypeError", &format!("{cname}::__serialize() must return an array")));
        };
        out.extend_from_slice(format!("O:{}:\"{cname}\":{}:{{", cname.len(), a.len()).as_bytes());
        for (k, v) in a.iter() {
            ser_key(&k, out);
            ser(it, &v, out)?;
        }
        out.push(b'}');
        return Ok(());
    }
    let props: Vec<(Rc<str>, Value)> = if o.class.find_method("__sleep").is_some() {
        let names = it.call_method(o, "__sleep", vec![])?;
        let Value::Array(names) = names else {
            it.warn("serialize(): __sleep should return an array only containing the names of instance-variables to serialize");
            out.extend_from_slice(b"N;");
            return Ok(());
        };
        let mut v = vec![];
        for n in names.values() {
            let n = it.to_string_lossy(&n)?;
            match o.get_prop(&n) {
                Some(x) => v.push((Rc::from(n.as_str()), x)),
                None => it.warn(&format!("serialize(): \"{n}\" returned as member variable from __sleep() but does not exist")),
            }
        }
        v
    } else {
        o.prop_list()
    };
    out.extend_from_slice(format!("O:{}:\"{cname}\":{}:{{", cname.len(), props.len()).as_bytes());
    for (k, v) in props {
        ser_str(&mangled(o, &k), out);
        ser(it, &v, out)?;
    }
    out.push(b'}');
    Ok(())
}

struct Unser<'a> {
    b: &'a [u8],
    i: usize,
    allowed: Option<Vec<String>>,
    depth: usize,
    max_depth: usize,
    /// Values in order of appearance, for `r:` / `R:` back-references.
    seen: Vec<Value>,
}

impl Unser<'_> {
    fn expect(&mut self, c: u8) -> Option<()> {
        (self.b.get(self.i) == Some(&c)).then(|| self.i += 1)
    }

    fn until(&mut self, c: u8) -> Option<&[u8]> {
        let start = self.i;
        let n = self.b[start..].iter().position(|&x| x == c)?;
        self.i = start + n + 1;
        Some(&self.b[start..start + n])
    }

    fn num<T: std::str::FromStr>(&mut self, end: u8) -> Option<T> {
        std::str::from_utf8(self.until(end)?).ok()?.parse().ok()
    }

    fn string(&mut self) -> Option<Vec<u8>> {
        let len: usize = self.num(b':')?;
        self.expect(b'"')?;
        let s = self.b.get(self.i..self.i + len)?.to_vec();
        self.i += len;
        self.expect(b'"')?;
        Some(s)
    }

    fn key(&mut self) -> Option<Key> {
        let t = *self.b.get(self.i)?;
        self.i += 2;
        match t {
            b'i' => Some(Key::Int(self.num(b';')?)),
            b's' => {
                let s = self.string()?;
                self.expect(b';')?;
                Some(Key::Str(Rc::new(s)))
            }
            _ => None,
        }
    }

    fn value(&mut self, it: &Interp) -> R<Option<Value>> {
        if it.stack_low() {
            return Err(it.stack_overflow());
        }
        if self.depth >= self.max_depth {
            it.warn(&format!(
                "unserialize(): Maximum depth of {} exceeded. The depth limit can be changed using the max_depth unserialize() option or the unserialize_max_depth ini setting",
                self.max_depth
            ));
            return Ok(None);
        }
        self.depth += 1;
        let r = self.value_inner(it);
        self.depth -= 1;
        r
    }

    fn value_inner(&mut self, it: &Interp) -> R<Option<Value>> {
        let Some(&t) = self.b.get(self.i) else { return Ok(None) };
        if self.b.get(self.i + 1) != Some(&b':') && t != b'N' {
            return Ok(None);
        }
        self.i += 2;
        let v = match t {
            b'N' => {
                self.i -= 1;
                if self.expect(b';').is_none() {
                    return Ok(None);
                }
                Value::Null
            }
            b'b' => match self.num::<i64>(b';') {
                Some(n) => Value::Bool(n != 0),
                None => return Ok(None),
            },
            b'i' => match self.num::<i64>(b';') {
                Some(n) => Value::Int(n),
                None => return Ok(None),
            },
            b'd' => {
                let Some(raw) = self.until(b';') else { return Ok(None) };
                let raw = String::from_utf8_lossy(raw).into_owned();
                Value::Float(match raw.as_str() {
                    "NAN" => f64::NAN,
                    "INF" => f64::INFINITY,
                    "-INF" => f64::NEG_INFINITY,
                    r => match r.parse() {
                        Ok(f) => f,
                        Err(_) => return Ok(None),
                    },
                })
            }
            b's' => {
                let Some(s) = self.string() else { return Ok(None) };
                if self.expect(b';').is_none() {
                    return Ok(None);
                }
                Value::bytes(s)
            }
            b'r' | b'R' => {
                let Some(n) = self.num::<usize>(b';') else { return Ok(None) };
                let v = self.seen.get(n.wrapping_sub(1)).cloned().unwrap_or_default();
                self.seen.push(v.clone());
                return Ok(Some(v));
            }
            b'a' => {
                let Some(n) = self.num::<usize>(b':') else { return Ok(None) };
                if self.expect(b'{').is_none() {
                    return Ok(None);
                }
                let slot = self.seen.len();
                self.seen.push(Value::Null);
                let mut arr = PhpArray::new();
                for _ in 0..n {
                    let Some(k) = self.key() else { return Ok(None) };
                    let Some(v) = self.value(it)? else { return Ok(None) };
                    arr.insert(k, v);
                }
                if self.expect(b'}').is_none() {
                    return Ok(None);
                }
                let v = Value::array(arr);
                self.seen[slot] = v.clone();
                return Ok(Some(v));
            }
            b'E' => {
                let Some(s) = self.string() else { return Ok(None) };
                if self.expect(b';').is_none() {
                    return Ok(None);
                }
                let s = String::from_utf8_lossy(&s).into_owned();
                let Some((cls, case)) = s.split_once(':') else { return Ok(None) };
                let Some(c) = it.find_class(cls)? else { return Ok(None) };
                match c.consts.borrow().get(case) {
                    Some(_) => {}
                    None => return Ok(None),
                }
                let v = it.get_class_const(&c, case)?;
                self.seen.push(v.clone());
                return Ok(Some(v));
            }
            b'O' | b'C' => {
                let Some(name) = self.string() else { return Ok(None) };
                let name = String::from_utf8_lossy(&name).into_owned();
                if self.expect(b':').is_none() {
                    return Ok(None);
                }
                let Some(n) = self.num::<usize>(b':') else { return Ok(None) };
                if self.expect(b'{').is_none() {
                    return Ok(None);
                }
                let allowed = self.allowed.as_ref().is_none_or(|l| l.iter().any(|a| a.eq_ignore_ascii_case(&name)));
                let cls = if allowed { it.find_class(&name)? } else { None };
                let cls = match cls {
                    Some(c) => c,
                    None => it.find_class("__PHP_Incomplete_Class")?.expect("prelude class"),
                };
                let o = Obj::new(cls.clone(), Internal::None);
                it.init_props(&o, &cls)?;
                let slot = self.seen.len();
                self.seen.push(Value::Object(o.clone()));
                let mut data = PhpArray::new();
                for _ in 0..n {
                    let Some(k) = self.key() else { return Ok(None) };
                    let Some(v) = self.value(it)? else { return Ok(None) };
                    data.insert(k, v);
                }
                if self.expect(b'}').is_none() {
                    return Ok(None);
                }
                if cls.find_method("__unserialize").is_some() {
                    it.call_method(&o, "__unserialize", vec![Value::array(data)])?;
                } else {
                    if !allowed || cls.name.eq_ignore_ascii_case("__PHP_Incomplete_Class") {
                        o.set_prop("__PHP_Incomplete_Class_Name", Value::str(&name));
                    }
                    for (k, v) in data.iter() {
                        let k = match k {
                            Key::Str(s) => String::from_utf8_lossy(&s).into_owned(),
                            Key::Int(i) => i.to_string(),
                        };
                        // Strip visibility mangling: "\0Class\0name" / "\0*\0name".
                        let plain = match k.strip_prefix('\0') {
                            Some(rest) => rest.split_once('\0').map(|(_, p)| p.to_string()).unwrap_or(k.clone()),
                            None => k,
                        };
                        o.props.borrow_mut().insert(Rc::from(plain.as_str()), v);
                    }
                    if cls.find_method("__wakeup").is_some() {
                        it.call_method(&o, "__wakeup", vec![])?;
                    }
                }
                let v = Value::Object(o);
                self.seen[slot] = v.clone();
                return Ok(Some(v));
            }
            _ => return Ok(None),
        };
        self.seen.push(v.clone());
        Ok(Some(v))
    }
}

pub fn serialize(it: &Interp, v: &Value) -> R<Vec<u8>> {
    let mut out = vec![];
    ser(it, v, &mut out)?;
    Ok(out)
}

pub fn unserialize(it: &Interp, b: &[u8], allowed: Option<Vec<String>>, max_depth: usize) -> R<Value> {
    let mut u = Unser { b, i: 0, allowed, depth: 0, max_depth, seen: vec![] };
    match u.value(it)? {
        Some(v) => Ok(v),
        None => {
            if !b.is_empty() {
                it.notice(&format!("unserialize(): Error at offset {} of {} bytes", u.i.min(b.len()), b.len()));
            }
            Ok(Value::Bool(false))
        }
    }
}

// ---- pack / unpack --------------------------------------------------------------

fn pack_codes(fmt: &[u8]) -> Vec<(u8, Option<usize>, bool, String)> {
    // (code, count, star, name)
    let mut out = vec![];
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        let mut star = false;
        let mut count = None;
        if fmt.get(i) == Some(&b'*') {
            star = true;
            i += 1;
        } else {
            let st = i;
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                i += 1;
            }
            if i > st {
                count = std::str::from_utf8(&fmt[st..i]).ok().and_then(|x| x.parse().ok());
            }
        }
        out.push((c, count, star, String::new()));
    }
    out
}

fn int_bytes(code: u8, v: i64) -> Option<Vec<u8>> {
    Some(match code {
        b'c' | b'C' => vec![v as u8],
        b's' | b'S' | b'v' => (v as u16).to_le_bytes().to_vec(),
        b'n' => (v as u16).to_be_bytes().to_vec(),
        b'l' | b'L' | b'V' | b'i' | b'I' => (v as u32).to_le_bytes().to_vec(),
        b'N' => (v as u32).to_be_bytes().to_vec(),
        b'q' | b'Q' | b'P' => (v as u64).to_le_bytes().to_vec(),
        b'J' => (v as u64).to_be_bytes().to_vec(),
        _ => return None,
    })
}

fn pack(it: &Interp, a: &[Value]) -> R<Value> {
    let fmt = s(it, a, 0)?;
    let mut out: Vec<u8> = vec![];
    let mut ai = 1;
    for (c, count, star, _) in pack_codes(&fmt) {
        match c {
            b'a' | b'A' | b'Z' | b'h' | b'H' => {
                let data = s(it, a, ai)?;
                ai += 1;
                match c {
                    b'h' | b'H' => {
                        let n = if star { data.len() } else { count.unwrap_or(1).min(data.len()) };
                        let digits: Vec<u8> = data[..n]
                            .iter()
                            .map(|&d| (d as char).to_digit(16).unwrap_or(0) as u8)
                            .collect();
                        for ch in digits.chunks(2) {
                            let (x, y) = (ch[0], *ch.get(1).unwrap_or(&0));
                            out.push(if c == b'H' { x << 4 | y } else { y << 4 | x });
                        }
                    }
                    _ => {
                        let n = if star { data.len() + (c == b'Z') as usize } else { count.unwrap_or(1) };
                        let pad = if c == b'A' { b' ' } else { 0 };
                        let mut field: Vec<u8> = data.iter().copied().take(n).collect();
                        if c == b'Z' && !star && n > 0 && field.len() >= n {
                            field.truncate(n - 1);
                        }
                        field.resize(n, pad);
                        out.extend_from_slice(&field);
                    }
                }
            }
            b'x' => out.extend(std::iter::repeat_n(0u8, count.unwrap_or(1))),
            b'X' => {
                let n = count.unwrap_or(1).min(out.len());
                out.truncate(out.len() - n);
            }
            b'@' => out.resize(count.unwrap_or(0), 0),
            b'f' | b'g' | b'G' | b'd' | b'e' | b'E' => {
                let n = if star { a.len().saturating_sub(ai) } else { count.unwrap_or(1) };
                for _ in 0..n {
                    let f = it.to_float(&arg(a, ai))?;
                    ai += 1;
                    match c {
                        b'f' | b'g' => out.extend_from_slice(&(f as f32).to_le_bytes()),
                        b'G' => out.extend_from_slice(&(f as f32).to_be_bytes()),
                        b'd' | b'e' => out.extend_from_slice(&f.to_le_bytes()),
                        _ => out.extend_from_slice(&f.to_be_bytes()),
                    }
                }
            }
            _ => {
                let n = if star { a.len().saturating_sub(ai) } else { count.unwrap_or(1) };
                for _ in 0..n {
                    if ai >= a.len() {
                        return Err(it.error("ValueError", &format!("Type {}: too few arguments", c as char)));
                    }
                    let v = it.to_int(&arg(a, ai))?;
                    ai += 1;
                    match int_bytes(c, v) {
                        Some(b) => out.extend_from_slice(&b),
                        None => return Err(it.error("ValueError", &format!("Type {}: unknown format code", c as char))),
                    }
                }
            }
        }
    }
    Ok(Value::bytes(out))
}

fn unpack(it: &Interp, a: &[Value]) -> R<Value> {
    let fmt = s(it, a, 0)?;
    let data = s(it, a, 1)?;
    let mut pos = if has(a, 2) { int(it, a, 2)?.max(0) as usize } else { 0 };
    let mut out = PhpArray::new();
    for part in fmt.split(|&c| c == b'/') {
        if part.is_empty() {
            continue;
        }
        let c = part[0];
        let mut i = 1;
        let mut star = false;
        let mut count = None;
        if part.get(1) == Some(&b'*') {
            star = true;
            i = 2;
        } else {
            let st = i;
            while i < part.len() && part[i].is_ascii_digit() {
                i += 1;
            }
            if i > st {
                count = std::str::from_utf8(&part[st..i]).ok().and_then(|x| x.parse::<usize>().ok());
            }
        }
        let name = String::from_utf8_lossy(&part[i..]).into_owned();
        let key = |idx: usize, n: usize| -> Key {
            if name.is_empty() {
                Key::Int(idx as i64 + 1)
            } else if n > 1 || star {
                Key::Str(pstr(&format!("{name}{}", idx + 1)))
            } else {
                match name.parse::<i64>() {
                    Ok(k) => Key::Int(k),
                    Err(_) => Key::Str(pstr(&name)),
                }
            }
        };
        match c {
            b'a' | b'A' | b'Z' | b'h' | b'H' => {
                let rest = data.len().saturating_sub(pos);
                let n = match c {
                    b'h' | b'H' => {
                        let digits = if star { rest * 2 } else { count.unwrap_or(1) };
                        let bytes = digits.div_ceil(2).min(rest);
                        let mut hex = String::new();
                        for &b in &data[pos..pos + bytes] {
                            let (hi, lo) = (b >> 4, b & 15);
                            let (x, y) = if c == b'H' { (hi, lo) } else { (lo, hi) };
                            hex.push(std::char::from_digit(x as u32, 16).unwrap());
                            hex.push(std::char::from_digit(y as u32, 16).unwrap());
                        }
                        hex.truncate(digits);
                        pos += bytes;
                        out.insert(key(0, 1), Value::str(&hex));
                        continue;
                    }
                    _ => if star { rest } else { count.unwrap_or(1) },
                };
                if pos + n > data.len() {
                    it.warn(&format!("unpack(): Type {}: not enough input, need {n}, have {rest}", c as char));
                    return Ok(Value::Bool(false));
                }
                let mut field = data[pos..pos + n].to_vec();
                pos += n;
                match c {
                    b'A' => {
                        while field.last().is_some_and(|&x| matches!(x, b' ' | 0 | b'\t' | b'\r' | b'\n')) {
                            field.pop();
                        }
                    }
                    b'Z' => {
                        if let Some(z) = field.iter().position(|&x| x == 0) {
                            field.truncate(z);
                        }
                    }
                    _ => {}
                }
                out.insert(key(0, 1), Value::bytes(field));
            }
            b'x' => pos += count.unwrap_or(1),
            b'X' => pos = pos.saturating_sub(count.unwrap_or(1)),
            b'@' => pos = count.unwrap_or(0),
            _ => {
                let size = match c {
                    b'c' | b'C' => 1,
                    b's' | b'S' | b'n' | b'v' => 2,
                    b'l' | b'L' | b'N' | b'V' | b'i' | b'I' | b'f' | b'g' | b'G' => 4,
                    b'q' | b'Q' | b'J' | b'P' | b'd' | b'e' | b'E' => 8,
                    _ => return Err(it.error("ValueError", &format!("Type {}: unknown format code", c as char))),
                };
                let n = if star { (data.len().saturating_sub(pos)) / size } else { count.unwrap_or(1) };
                for idx in 0..n {
                    if pos + size > data.len() {
                        it.warn(&format!("unpack(): Type {}: not enough input, need {size}, have {}", c as char, data.len().saturating_sub(pos)));
                        return Ok(Value::Bool(false));
                    }
                    let b = &data[pos..pos + size];
                    pos += size;
                    let le = |b: &[u8]| b.iter().rev().fold(0u64, |acc, &x| acc << 8 | x as u64);
                    let be = |b: &[u8]| b.iter().fold(0u64, |acc, &x| acc << 8 | x as u64);
                    let v = match c {
                        b'c' => Value::Int(b[0] as i8 as i64),
                        b'C' => Value::Int(b[0] as i64),
                        b's' => Value::Int(le(b) as u16 as i16 as i64),
                        b'S' | b'v' => Value::Int(le(b) as i64),
                        b'n' => Value::Int(be(b) as i64),
                        b'l' | b'i' => Value::Int(le(b) as u32 as i32 as i64),
                        b'L' | b'V' | b'I' => Value::Int(le(b) as i64),
                        b'N' => Value::Int(be(b) as i64),
                        b'q' | b'Q' | b'P' => Value::Int(le(b) as i64),
                        b'J' => Value::Int(be(b) as i64),
                        b'f' | b'g' => Value::Float(f32::from_bits(le(b) as u32) as f64),
                        b'G' => Value::Float(f32::from_bits(be(b) as u32) as f64),
                        b'd' | b'e' => Value::Float(f64::from_bits(le(b))),
                        _ => Value::Float(f64::from_bits(be(b))),
                    };
                    out.insert(key(idx, n), v);
                }
            }
        }
    }
    Ok(Value::array(out))
}

// ---- hashing ------------------------------------------------------------------

#[derive(Clone)]
enum Hasher {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha224(sha2::Sha224),
    Sha256(sha2::Sha256),
    Sha384(sha2::Sha384),
    Sha512(sha2::Sha512),
    Crc32b(Vec<u8>),
}

const ALGOS: &[&str] = &["md5", "sha1", "sha224", "sha256", "sha384", "sha512", "crc32b"];

impl Hasher {
    fn new(algo: &str) -> Option<Hasher> {
        Some(match algo.to_ascii_lowercase().as_str() {
            "md5" => Hasher::Md5(md5::Md5::new()),
            "sha1" => Hasher::Sha1(sha1::Sha1::new()),
            "sha224" => Hasher::Sha224(sha2::Sha224::new()),
            "sha256" => Hasher::Sha256(sha2::Sha256::new()),
            "sha384" => Hasher::Sha384(sha2::Sha384::new()),
            "sha512" => Hasher::Sha512(sha2::Sha512::new()),
            "crc32b" => Hasher::Crc32b(vec![]),
            _ => return None,
        })
    }

    fn update(&mut self, d: &[u8]) {
        match self {
            Hasher::Md5(h) => h.update(d),
            Hasher::Sha1(h) => h.update(d),
            Hasher::Sha224(h) => h.update(d),
            Hasher::Sha256(h) => h.update(d),
            Hasher::Sha384(h) => h.update(d),
            Hasher::Sha512(h) => h.update(d),
            Hasher::Crc32b(b) => b.extend_from_slice(d),
        }
    }

    fn finish(self) -> Vec<u8> {
        match self {
            Hasher::Md5(h) => h.finalize().to_vec(),
            Hasher::Sha1(h) => h.finalize().to_vec(),
            Hasher::Sha224(h) => h.finalize().to_vec(),
            Hasher::Sha256(h) => h.finalize().to_vec(),
            Hasher::Sha384(h) => h.finalize().to_vec(),
            Hasher::Sha512(h) => h.finalize().to_vec(),
            Hasher::Crc32b(b) => crate::strings::crc32(&b).to_be_bytes().to_vec(),
        }
    }

    fn block_size(&self) -> usize {
        match self {
            Hasher::Sha384(_) | Hasher::Sha512(_) => 128,
            Hasher::Crc32b(_) => 4,
            _ => 64,
        }
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn digest_out(raw: Vec<u8>, binary: bool) -> Value {
    if binary {
        Value::bytes(raw)
    } else {
        Value::str(&hex(&raw))
    }
}

fn hmac(algo: &str, key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let proto = Hasher::new(algo)?;
    let bs = proto.block_size();
    let mut k = if key.len() > bs {
        let mut h = proto.clone();
        h.update(key);
        h.finish()
    } else {
        key.to_vec()
    };
    k.resize(bs, 0);
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let mut inner = proto.clone();
    inner.update(&ipad);
    inner.update(data);
    let ih = inner.finish();
    let mut outer = proto;
    outer.update(&opad);
    outer.update(&ih);
    Some(outer.finish())
}

fn unknown_algo(it: &Interp, fname: &str, algo: &str) -> crate::interp::Ctrl {
    it.error("ValueError", &format!("{fname}(): Argument #1 ($algo) must be a valid hashing algorithm, \"{algo}\" given"))
}

/// State kept in a HashContext object.
#[derive(Clone)]
struct HashState {
    h: Hasher,
    hmac_key: Option<(String, Vec<u8>)>,
}

// ---- URLs ---------------------------------------------------------------------------

/// PHP's parse_url() components; None = malformed.
pub fn parse_url(u: &str) -> Option<Vec<(&'static str, Value)>> {
    let mut out: Vec<(&'static str, Value)> = vec![];
    let mut rest = u;
    // fragment / query split off first
    let mut fragment = None;
    if let Some(i) = rest.find('#') {
        fragment = Some(&rest[i + 1..]);
        rest = &rest[..i];
    }
    let mut query = None;
    if let Some(i) = rest.find('?') {
        query = Some(&rest[i + 1..]);
        rest = &rest[..i];
    }
    // scheme
    let mut scheme = None;
    if let Some(i) = rest.find(':') {
        let cand = &rest[..i];
        let valid = !cand.is_empty()
            && cand.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
            && cand.bytes().next().is_some_and(|c| c.is_ascii_alphabetic());
        let after = &rest[i + 1..];
        // "host:port" without scheme: digits after the colon.
        let port_like = !after.is_empty() && after.bytes().take_while(|c| *c != b'/').all(|c| c.is_ascii_digit());
        if valid && !(port_like && !after.starts_with("//")) {
            scheme = Some(cand);
            rest = after;
        }
    }
    let mut host = None;
    let (mut user, mut pass, mut port) = (None, None, None);
    let authority;
    if let Some(r) = rest.strip_prefix("//") {
        let end = r.find('/').unwrap_or(r.len());
        authority = Some(&r[..end]);
        rest = &r[end..];
    } else if scheme.is_none() && !rest.starts_with('/') && rest.contains(':') {
        // "host:port/path"
        let end = rest.find('/').unwrap_or(rest.len());
        authority = Some(&rest[..end]);
        rest = &rest[end..];
    } else {
        authority = None;
    }
    if let Some(mut auth) = authority {
        if let Some(at) = auth.rfind('@') {
            let ui = &auth[..at];
            auth = &auth[at + 1..];
            match ui.split_once(':') {
                Some((u2, p)) => {
                    user = Some(u2);
                    pass = Some(p);
                }
                None => user = Some(ui),
            }
        }
        let (h, p) = if auth.starts_with('[') {
            match auth.find(']') {
                Some(e) => (&auth[..=e], auth[e + 1..].strip_prefix(':')),
                None => return None,
            }
        } else {
            match auth.rsplit_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (auth, None),
            }
        };
        if let Some(p) = p {
            if !p.is_empty() {
                let n: i64 = p.parse().ok()?;
                if !(0..=65535).contains(&n) {
                    return None;
                }
                port = Some(n);
            }
        }
        if h.is_empty() && scheme.is_some_and(|s| s != "file") {
            return None;
        }
        if !h.is_empty() {
            host = Some(h);
        }
    }
    if let Some(s) = scheme {
        out.push(("scheme", Value::str(s)));
    }
    if let Some(h) = host {
        out.push(("host", Value::str(h)));
    }
    if let Some(p) = port {
        out.push(("port", Value::Int(p)));
    }
    if let Some(u2) = user {
        out.push(("user", Value::str(u2)));
    }
    if let Some(p) = pass {
        out.push(("pass", Value::str(p)));
    }
    if !rest.is_empty() {
        out.push(("path", Value::str(rest)));
    }
    if let Some(q) = query {
        out.push(("query", Value::str(q)));
    }
    if let Some(f) = fragment {
        out.push(("fragment", Value::str(f)));
    }
    Some(out)
}

pub fn urldecode(b: &[u8], plus: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' if plus => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let h = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|x| u8::from_str_radix(x, 16).ok());
                match h {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// parse_str() / $_GET-style query parsing with `a[b][]=` nesting.
pub fn parse_query(q: &[u8]) -> PhpArray {
    let mut out = PhpArray::new();
    for pair in q.split(|&c| c == b'&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.iter().position(|&c| c == b'=') {
            Some(i) => (&pair[..i], &pair[i + 1..]),
            None => (pair, &b""[..]),
        };
        let key = urldecode(k, true);
        let val = Value::bytes(urldecode(v, true));
        // Split "a[b][c]" into base + segments.
        let (base, segs) = match key.iter().position(|&c| c == b'[') {
            Some(i) if i > 0 => {
                let mut segs = vec![];
                let mut j = i;
                while j < key.len() && key[j] == b'[' {
                    match key[j..].iter().position(|&c| c == b']') {
                        Some(e) => {
                            segs.push(key[j + 1..j + e].to_vec());
                            j += e + 1;
                        }
                        None => break,
                    }
                }
                (key[..i].to_vec(), segs)
            }
            _ => (key.clone(), vec![]),
        };
        // PHP turns spaces and dots in the base name into underscores.
        let base: Vec<u8> = base.iter().map(|&c| if c == b' ' || c == b'.' { b'_' } else { c }).collect();
        fn to_key(b: &[u8]) -> Key {
            match std::str::from_utf8(b).ok().and_then(|s| {
                let n: i64 = s.parse().ok()?;
                (n.to_string() == s).then_some(n)
            }) {
                Some(n) => Key::Int(n),
                None => Key::Str(Rc::new(b.to_vec())),
            }
        }
        fn put(arr: &mut PhpArray, segs: &[Vec<u8>], val: Value) {
            match segs.split_first() {
                None => {}
                Some((s, rest)) => {
                    if rest.is_empty() {
                        if s.is_empty() {
                            arr.push(val);
                        } else {
                            arr.insert(to_key(s), val);
                        }
                        return;
                    }
                    let k = if s.is_empty() { None } else { Some(to_key(s)) };
                    let mut child = match k.as_ref().and_then(|k| arr.get(k)) {
                        Some(Value::Array(a)) => (*a).clone(),
                        _ => PhpArray::new(),
                    };
                    put(&mut child, rest, val);
                    match k {
                        Some(k) => arr.insert(k, Value::array(child)),
                        None => {
                            arr.push(Value::array(child));
                        }
                    }
                }
            }
        }
        let mut segs_all = vec![base];
        segs_all.extend(segs);
        put(&mut out, &segs_all, val);
    }
    out
}

// ---- version_compare ------------------------------------------------------------------

fn canon_version(v: &str) -> Vec<String> {
    let mut parts = vec![];
    let mut cur = String::new();
    let mut kind = 0; // 1 digit, 2 alpha
    for c in v.chars() {
        let k = if c.is_ascii_digit() {
            1
        } else if c.is_ascii_alphabetic() {
            2
        } else {
            0
        };
        if k == 0 {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            kind = 0;
            continue;
        }
        if kind != 0 && k != kind && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
        }
        cur.push(c);
        kind = k;
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

fn special_order(s: &str) -> i32 {
    let l = s.to_ascii_lowercase();
    if l.starts_with("dev") {
        0
    } else if l.starts_with("alpha") || l == "a" {
        1
    } else if l.starts_with("beta") || l == "b" {
        2
    } else if l.starts_with("rc") || l == "c" {
        3
    } else if l == "#" {
        4
    } else if l.starts_with("pl") || l == "p" {
        5
    } else {
        -1
    }
}

pub fn version_compare(a: &str, b: &str) -> i32 {
    let (pa, pb) = (canon_version(a), canon_version(b));
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let (x, y) = (pa.get(i), pb.get(i));
        let c = match (x, y) {
            (Some(x), Some(y)) => {
                let (xd, yd) = (x.chars().all(|c| c.is_ascii_digit()), y.chars().all(|c| c.is_ascii_digit()));
                match (xd, yd) {
                    (true, true) => x.parse::<u64>().unwrap_or(0).cmp(&y.parse::<u64>().unwrap_or(0)) as i32,
                    (false, false) => special_order(x).cmp(&special_order(y)) as i32,
                    (true, false) => special_order("#").cmp(&special_order(y)) as i32,
                    (false, true) => special_order(x).cmp(&special_order("#")) as i32,
                }
            }
            (Some(x), None) => {
                if x.chars().all(|c| c.is_ascii_digit()) {
                    1
                } else {
                    special_order(x).cmp(&special_order("#")) as i32
                }
            }
            (None, Some(y)) => {
                if y.chars().all(|c| c.is_ascii_digit()) {
                    -1
                } else {
                    special_order("#").cmp(&special_order(y)) as i32
                }
            }
            (None, None) => 0,
        };
        if c != 0 {
            return c.signum();
        }
    }
    0
}

// ---- helpers ---------------------------------------------------------------------------

fn bstr(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(String::from_utf8_lossy(&s(it, a, i)?).into_owned())
}

fn span_len(sub: &[u8], mask: &[u8], accept: bool) -> usize {
    sub.iter().take_while(|c| mask.contains(c) == accept).count()
}

fn span(it: &Interp, a: &[Value], accept: bool) -> R<Value> {
    let st = s(it, a, 0)?;
    let mask = s(it, a, 1)?;
    let len = st.len() as i64;
    let mut off = if has(a, 2) { int(it, a, 2)? } else { 0 };
    if off < 0 {
        off = (len + off).max(0);
    }
    let off = off.min(len) as usize;
    let mut end = st.len();
    if has(a, 3) && !arg(a, 3).is_null() {
        let l = int(it, a, 3)?;
        end = if l < 0 { (st.len() as i64 + l).max(off as i64) as usize } else { (off + l as usize).min(st.len()) };
    }
    Ok(Value::Int(span_len(&st[off..end.max(off)], &mask, accept) as i64))
}

/// Character list with `a..z` ranges (addcslashes, trim masks).
pub fn char_mask(m: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut i = 0;
    while i < m.len() {
        if i + 3 < m.len() && m[i + 1] == b'.' && m[i + 2] == b'.' && m[i] <= m[i + 3] {
            out.extend(m[i]..=m[i + 3]);
            i += 4;
            continue;
        }
        out.push(m[i]);
        i += 1;
    }
    out
}

pub fn constants(add: &mut dyn FnMut(&str, Value)) {
    add("HASH_HMAC", Value::Int(1));
    for (i, n) in ["SCHEME", "HOST", "PORT", "USER", "PASS", "PATH", "QUERY", "FRAGMENT"].iter().enumerate() {
        add(&format!("PHP_URL_{n}"), Value::Int(i as i64));
    }
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("serialize", |it, _, a| Ok(Value::bytes(serialize(it, &arg(a, 0))?)));
    t.insert("unserialize", |it, _, a| {
        let data = s(it, a, 0)?;
        let mut allowed = None;
        let mut max_depth = 4096;
        if let Value::Array(o) = arg(a, 1) {
            if let Some(d) = o.get(&Key::Str(pstr("max_depth"))) {
                let d = it.to_int(&d)?;
                if d > 0 {
                    max_depth = d as usize;
                }
            }
            match o.get(&Key::Str(pstr("allowed_classes"))) {
                Some(Value::Bool(false)) => allowed = Some(vec![]),
                Some(Value::Array(l)) => allowed = Some(l.values().map(|v| it.to_string_lossy(&v)).collect::<R<Vec<_>>>()?),
                _ => {}
            }
        }
        unserialize(it, &data, allowed, max_depth)
    });
    t.insert("pack", |it, _, a| pack(it, a));
    t.insert("unpack", |it, _, a| unpack(it, a));

    // hashing
    t.insert("hash", |it, _, a| {
        let algo = bstr(it, a, 0)?;
        let Some(mut h) = Hasher::new(&algo) else { return Err(unknown_algo(it, "hash", &algo)) };
        h.update(&s(it, a, 1)?);
        Ok(digest_out(h.finish(), arg(a, 2).truthy()))
    });
    t.insert("hash_file", |it, _, a| {
        let algo = bstr(it, a, 0)?;
        let Some(mut h) = Hasher::new(&algo) else { return Err(unknown_algo(it, "hash_file", &algo)) };
        match std::fs::read(bstr(it, a, 1)?) {
            Ok(d) => {
                h.update(&d);
                Ok(digest_out(h.finish(), arg(a, 2).truthy()))
            }
            Err(_) => Ok(Value::Bool(false)),
        }
    });
    t.insert("hash_hmac", |it, _, a| {
        let algo = bstr(it, a, 0)?;
        match hmac(&algo, &s(it, a, 2)?, &s(it, a, 1)?) {
            Some(r) => Ok(digest_out(r, arg(a, 3).truthy())),
            None => Err(it.error("ValueError", &format!("hash_hmac(): Argument #1 ($algo) must be a valid cryptographic hashing algorithm, \"{algo}\" given"))),
        }
    });
    t.insert("hash_algos", |_, _, _| Ok(Value::array(PhpArray::from_list(ALGOS.iter().map(|a| Value::str(a))))));
    t.insert("hash_hmac_algos", |_, _, _| Ok(Value::array(PhpArray::from_list(ALGOS[..6].iter().map(|a| Value::str(a))))));
    t.insert("hash_equals", |it, _, a| {
        let (x, y) = (arg(a, 0), arg(a, 1));
        for (i, v) in [(1, &x), (2, &y)] {
            if !matches!(v, Value::Str(_)) {
                let name = if i == 1 { "known_string" } else { "user_string" };
                return Err(it.error("TypeError", &format!("hash_equals(): Argument #{i} (${name}) must be of type string, {} given", v.debug_type())));
            }
        }
        let (x, y) = (it.to_str(&x)?, it.to_str(&y)?);
        Ok(Value::Bool(x.len() == y.len() && x.iter().zip(y.iter()).fold(0u8, |acc, (p, q)| acc | (p ^ q)) == 0))
    });
    t.insert("hash_init", |it, _, a| {
        let algo = bstr(it, a, 0)?;
        let Some(h) = Hasher::new(&algo) else { return Err(unknown_algo(it, "hash_init", &algo)) };
        let flags = if has(a, 1) { int(it, a, 1)? } else { 0 };
        let hmac_key = if flags & 1 != 0 { Some((algo.clone(), s(it, a, 2)?.to_vec())) } else { None };
        let mut h = h;
        if let Some((_, k)) = &hmac_key {
            // Inner hash of HMAC: key ^ ipad first.
            let bs = h.block_size();
            let mut key = if k.len() > bs {
                let mut kh = Hasher::new(&algo).unwrap();
                kh.update(k);
                kh.finish()
            } else {
                k.clone()
            };
            key.resize(bs, 0);
            h.update(&key.iter().map(|b| b ^ 0x36).collect::<Vec<_>>());
        }
        let cls = it.find_class("HashContext")?.expect("prelude HashContext");
        let o = Obj::new(cls, Internal::Data(Box::new(HashState { h, hmac_key })));
        Ok(Value::Object(o))
    });
    t.insert("hash_copy", |it, _, a| match arg(a, 0) {
        Value::Object(o) => Ok(Value::Object(it.clone_object(&o)?)),
        _ => Err(it.error("TypeError", "hash_copy(): Argument #1 ($context) must be of type HashContext")),
    });
    t.insert("hash_update", |it, _, a| {
        let data = s(it, a, 1)?;
        with_hash(it, &arg(a, 0), |st| st.h.update(&data))?;
        Ok(Value::Bool(true))
    });
    t.insert("hash_final", |it, _, a| {
        let binary = arg(a, 1).truthy();
        let (h, key) = with_hash(it, &arg(a, 0), |st| (st.h.clone(), st.hmac_key.clone()))?;
        let mut raw = h.finish();
        if let Some((algo, k)) = key {
            let mut outer = Hasher::new(&algo).unwrap();
            let bs = outer.block_size();
            let mut key = if k.len() > bs {
                let mut kh = Hasher::new(&algo).unwrap();
                kh.update(&k);
                kh.finish()
            } else {
                k
            };
            key.resize(bs, 0);
            outer.update(&key.iter().map(|b| b ^ 0x5c).collect::<Vec<_>>());
            outer.update(&raw);
            raw = outer.finish();
        }
        Ok(digest_out(raw, binary))
    });
    t.insert("sha1", |it, _, a| {
        let mut h = Hasher::new("sha1").unwrap();
        h.update(&s(it, a, 0)?);
        Ok(digest_out(h.finish(), arg(a, 1).truthy()))
    });
    t.insert("sha1_file", |it, _, a| match std::fs::read(bstr(it, a, 0)?) {
        Ok(d) => {
            let mut h = Hasher::new("sha1").unwrap();
            h.update(&d);
            Ok(digest_out(h.finish(), arg(a, 1).truthy()))
        }
        Err(_) => Ok(Value::Bool(false)),
    });
    t.insert("md5_file", |it, _, a| match std::fs::read(bstr(it, a, 0)?) {
        Ok(d) => {
            let mut h = Hasher::new("md5").unwrap();
            h.update(&d);
            Ok(digest_out(h.finish(), arg(a, 1).truthy()))
        }
        Err(_) => Ok(Value::Bool(false)),
    });

    // URLs
    t.insert("parse_url", |it, _, a| {
        let u = bstr(it, a, 0)?;
        let comp = if has(a, 1) { int(it, a, 1)? } else { -1 };
        let Some(parts) = parse_url(&u) else { return Ok(Value::Bool(false)) };
        if comp >= 0 {
            let name = ["scheme", "host", "port", "user", "pass", "path", "query", "fragment"].get(comp as usize).copied().unwrap_or("");
            return Ok(parts.into_iter().find(|(k, _)| *k == name).map(|(_, v)| v).unwrap_or(Value::Null));
        }
        let mut arr = PhpArray::new();
        for (k, v) in parts {
            arr.insert(Key::Str(pstr(k)), v);
        }
        Ok(Value::array(arr))
    });
    t.insert("parse_str", |it, _, a| {
        let q = s(it, a, 0)?;
        if a.len() > 1 {
            a[1] = Value::array(parse_query(&q));
        }
        Ok(Value::Null)
    });

    t.insert("version_compare", |it, _, a| {
        let c = version_compare(&bstr(it, a, 0)?, &bstr(it, a, 1)?);
        if !has(a, 2) || arg(a, 2).is_null() {
            return Ok(Value::Int(c as i64));
        }
        let op = bstr(it, a, 2)?;
        Ok(Value::Bool(match op.as_str() {
            "<" | "lt" => c < 0,
            "<=" | "le" => c <= 0,
            ">" | "gt" => c > 0,
            ">=" | "ge" => c >= 0,
            "==" | "eq" => c == 0,
            "!=" | "<>" | "ne" => c != 0,
            _ => return Err(it.error("ValueError", "version_compare(): Argument #3 ($operator) must be a valid comparison operator")),
        }))
    });

    // strings
    t.insert("strspn", |it, _, a| span(it, a, true));
    t.insert("strcspn", |it, _, a| span(it, a, false));
    t.insert("strpbrk", |it, _, a| {
        let st = s(it, a, 0)?;
        let cl = s(it, a, 1)?;
        Ok(match st.iter().position(|c| cl.contains(c)) {
            Some(i) => Value::bytes(st[i..].to_vec()),
            None => Value::Bool(false),
        })
    });
    t.insert("substr_compare", |it, _, a| {
        let (hay, needle) = (s(it, a, 0)?, s(it, a, 1)?);
        let mut off = int(it, a, 2)?;
        if off < 0 {
            off = (hay.len() as i64 + off).max(0);
        }
        if off as usize > hay.len() {
            return Err(it.error("ValueError", "substr_compare(): Argument #3 ($offset) must be contained in argument #1 ($main_str)"));
        }
        let len = if has(a, 3) && !arg(a, 3).is_null() { Some(int(it, a, 3)?.max(0) as usize) } else { None };
        let ci = arg(a, 4).truthy();
        let mut x = hay[off as usize..].to_vec();
        let mut y = needle.to_vec();
        if let Some(l) = len {
            x.truncate(l);
            y.truncate(l);
        }
        if ci {
            x.make_ascii_lowercase();
            y.make_ascii_lowercase();
        }
        Ok(Value::Int(match x.cmp(&y) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }))
    });
    t.insert("stristr", |it, _, a| {
        let (hay, needle) = (s(it, a, 0)?, s(it, a, 1)?);
        let (hl, nl) = (hay.to_ascii_lowercase(), needle.to_ascii_lowercase());
        let pos = if nl.is_empty() { Some(0) } else { hl.windows(nl.len()).position(|w| w == &nl[..]) };
        Ok(match pos {
            Some(i) if arg(a, 2).truthy() => Value::bytes(hay[..i].to_vec()),
            Some(i) => Value::bytes(hay[i..].to_vec()),
            None => Value::Bool(false),
        })
    });
    t.insert("strtok", |it, _, a| {
        thread_local! {
            static TOK: std::cell::RefCell<(Vec<u8>, usize)> = const { std::cell::RefCell::new((Vec::new(), 0)) };
        }
        let delims = if has(a, 1) {
            TOK.with(|t| *t.borrow_mut() = (s(it, a, 0).map(|x| x.to_vec()).unwrap_or_default(), 0));
            s(it, a, 1)?
        } else {
            s(it, a, 0)?
        };
        Ok(TOK.with(|t| {
            let mut t = t.borrow_mut();
            let (buf, pos) = &mut *t;
            while *pos < buf.len() && delims.contains(&buf[*pos]) {
                *pos += 1;
            }
            if *pos >= buf.len() {
                return Value::Bool(false);
            }
            let start = *pos;
            while *pos < buf.len() && !delims.contains(&buf[*pos]) {
                *pos += 1;
            }
            let tok = buf[start..*pos].to_vec();
            if *pos < buf.len() {
                *pos += 1;
            }
            Value::bytes(tok)
        }))
    });
    t.insert("addcslashes", |it, _, a| {
        let st = s(it, a, 0)?;
        let raw = s(it, a, 1)?;
        if raw.windows(4).any(|w| w[1] == b'.' && w[2] == b'.' && w[0] > w[3]) {
            it.warn("addcslashes(): Invalid '..'-range, '..'-range needs to be incrementing");
        }
        let mask = char_mask(&raw);
        let mut out = vec![];
        for &c in st.iter() {
            if mask.contains(&c) {
                out.push(b'\\');
                match c {
                    b'\n' => out.push(b'n'),
                    b'\t' => out.push(b't'),
                    b'\r' => out.push(b'r'),
                    0x07 => out.push(b'a'),
                    0x0b => out.push(b'v'),
                    0x08 => out.push(b'b'),
                    0x0c => out.push(b'f'),
                    c if !(32..127).contains(&c) => out.extend_from_slice(format!("{c:03o}").as_bytes()),
                    c => out.push(c),
                }
            } else {
                out.push(c);
            }
        }
        Ok(Value::bytes(out))
    });
    t.insert("stripcslashes", |it, _, a| {
        let st = s(it, a, 0)?;
        let mut out = vec![];
        let mut i = 0;
        while i < st.len() {
            if st[i] == b'\\' && i + 1 < st.len() {
                i += 1;
                match st[i] {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'a' => out.push(7),
                    b'v' => out.push(11),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'x' if i + 1 < st.len() && st[i + 1].is_ascii_hexdigit() => {
                        let mut v = 0u32;
                        let mut k = 0;
                        while k < 2 && i + 1 < st.len() && st[i + 1].is_ascii_hexdigit() {
                            i += 1;
                            v = v * 16 + (st[i] as char).to_digit(16).unwrap();
                            k += 1;
                        }
                        out.push(v as u8);
                    }
                    c @ b'0'..=b'7' => {
                        let mut v = (c - b'0') as u32;
                        let mut k = 1;
                        while k < 3 && i + 1 < st.len() && (b'0'..=b'7').contains(&st[i + 1]) {
                            i += 1;
                            v = v * 8 + (st[i] - b'0') as u32;
                            k += 1;
                        }
                        out.push(v as u8);
                    }
                    c => out.push(c),
                }
            } else {
                out.push(st[i]);
            }
            i += 1;
        }
        Ok(Value::bytes(out))
    });
    t.insert("chunk_split", |it, _, a| {
        let st = s(it, a, 0)?;
        let n = if has(a, 1) { int(it, a, 1)? } else { 76 };
        if n < 1 {
            return Err(it.error("ValueError", "chunk_split(): Argument #2 ($length) must be greater than 0"));
        }
        let end = if has(a, 2) { s(it, a, 2)?.to_vec() } else { b"\r\n".to_vec() };
        let mut out = vec![];
        for c in st.chunks(n as usize) {
            out.extend_from_slice(c);
            out.extend_from_slice(&end);
        }
        Ok(Value::bytes(out))
    });
    t.insert("utf8_encode", |it, _, a| {
        it.deprecated("Function utf8_encode() is deprecated since 8.2, visit the php.net documentation for various alternatives");
        Ok(Value::str(&s(it, a, 0)?.iter().map(|&b| b as char).collect::<String>()))
    });
    t.insert("utf8_decode", |it, _, a| {
        it.deprecated("Function utf8_decode() is deprecated since 8.2, visit the php.net documentation for various alternatives");
        let st = s(it, a, 0)?;
        Ok(Value::bytes(String::from_utf8_lossy(&st).chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()))
    });
    t.insert("quoted_printable_encode", |it, _, a| {
        let st = s(it, a, 0)?;
        let mut out = String::new();
        let mut line = 0;
        for &c in st.iter() {
            let enc = if c == b'\r' || c == b'\n' {
                line = 0;
                out.push(c as char);
                continue;
            } else if (33..=126).contains(&c) && c != b'=' || c == b' ' {
                (c as char).to_string()
            } else {
                format!("={c:02X}")
            };
            if line + enc.len() > 75 {
                out.push_str("=\r\n");
                line = 0;
            }
            line += enc.len();
            out.push_str(&enc);
        }
        Ok(Value::str(&out))
    });
    t.insert("ip2long", |it, _, a| {
        let ip = bstr(it, a, 0)?;
        Ok(match ip.parse::<std::net::Ipv4Addr>() {
            Ok(v) => Value::Int(u32::from(v) as i64),
            Err(_) => Value::Bool(false),
        })
    });
    t.insert("long2ip", |it, _, a| Ok(Value::str(&std::net::Ipv4Addr::from(int(it, a, 0)? as u32).to_string())));
    t.insert("inet_pton", |it, _, a| {
        let ip = bstr(it, a, 0)?;
        Ok(match ip.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(v)) => Value::bytes(v.octets().to_vec()),
            Ok(std::net::IpAddr::V6(v)) => Value::bytes(v.octets().to_vec()),
            Err(_) => Value::Bool(false),
        })
    });
    t.insert("inet_ntop", |it, _, a| {
        let b = s(it, a, 0)?;
        Ok(match b.len() {
            4 => Value::str(&std::net::Ipv4Addr::new(b[0], b[1], b[2], b[3]).to_string()),
            16 => {
                let mut o = [0u8; 16];
                o.copy_from_slice(&b);
                Value::str(&std::net::Ipv6Addr::from(o).to_string())
            }
            _ => Value::Bool(false),
        })
    });
}

fn with_hash<T>(it: &Interp, v: &Value, g: impl FnOnce(&mut HashState) -> T) -> R<T> {
    if let Value::Object(o) = v {
        if let Internal::Data(d) = &mut *o.internal.borrow_mut() {
            if let Some(st) = d.downcast_mut::<HashState>() {
                return Ok(g(st));
            }
        }
    }
    Err(it.error("TypeError", "hash_update(): Argument #1 ($context) must be of type HashContext"))
}

/// clone of a built-in object's native state.
pub fn clone_data(d: &dyn std::any::Any) -> Option<Box<dyn std::any::Any>> {
    d.downcast_ref::<HashState>().map(|h| Box::new(h.clone()) as Box<dyn std::any::Any>)
}
