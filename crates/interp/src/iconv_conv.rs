//! ext/iconv on the C library's iconv(3): iconv(), the iconv_str*
//! functions, MIME header encoding/decoding and the encoding settings.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{Key, PhpArray, Value};

pub enum Failure {
    /// The conversion can't be opened (unknown charset).
    Open,
    /// Illegal input.
    Illegal,
    /// Truncated multibyte sequence at the end.
    Incomplete,
}

/// Convert `data` from `from` to `to` with iconv(3).
pub fn convert(from: &str, to: &str, data: &[u8]) -> Result<Vec<u8>, Failure> {
    let (Ok(cf), Ok(ct)) = (CString::new(from), CString::new(to)) else { return Err(Failure::Open) };
    let cd = unsafe { libc::iconv_open(ct.as_ptr(), cf.as_ptr()) };
    if cd as isize == -1 {
        return Err(Failure::Open);
    }
    let mut out = Vec::with_capacity(data.len() * 2 + 16);
    let mut inbuf = data.as_ptr() as *mut libc::c_char;
    let mut inleft = data.len();
    let mut chunk = vec![0u8; 4096.max(data.len() * 4)];
    let result = loop {
        let mut outbuf = chunk.as_mut_ptr() as *mut libc::c_char;
        let mut outleft = chunk.len();
        let r = unsafe { libc::iconv(cd, &mut inbuf, &mut inleft, &mut outbuf, &mut outleft) };
        out.extend_from_slice(&chunk[..chunk.len() - outleft]);
        if r != usize::MAX {
            // Flush any shift state.
            let mut outbuf = chunk.as_mut_ptr() as *mut libc::c_char;
            let mut outleft = chunk.len();
            unsafe { libc::iconv(cd, std::ptr::null_mut(), std::ptr::null_mut(), &mut outbuf, &mut outleft) };
            out.extend_from_slice(&chunk[..chunk.len() - outleft]);
            break Ok(out);
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::E2BIG) => continue,
            Some(libc::EINVAL) => break Err(Failure::Incomplete),
            _ => break Err(Failure::Illegal),
        }
    };
    unsafe { libc::iconv_close(cd) };
    result
}

thread_local! {
    static ENC: RefCell<(String, String, String)> = RefCell::new(("UTF-8".into(), "UTF-8".into(), "UTF-8".into()));
}

fn internal() -> String {
    ENC.with(|e| e.borrow().2.clone())
}

fn enc_arg(it: &Interp, a: &[Value], i: usize) -> R<String> {
    match a.get(i) {
        Some(v) if !v.is_null() => it.to_string_lossy(v),
        _ => Ok(internal()),
    }
}

/// Decode to chars (via UTF-8); None with PHP's notice on bad input.
fn chars(it: &Interp, data: &[u8], enc: &str, func: &str) -> R<Option<Vec<char>>> {
    match convert(enc, "UTF-8", data) {
        Ok(u) => Ok(Some(crate::pstr::lossy(&u).chars().collect())),
        Err(Failure::Open) => {
            it.warn(&format!("{func}(): Wrong encoding, conversion from \"{enc}\" to \"UCS-4LE\" is not allowed"));
            Ok(None)
        }
        Err(Failure::Illegal) => {
            it.notice(&format!("{func}(): Detected an illegal character in input string"));
            Ok(None)
        }
        Err(Failure::Incomplete) => {
            it.notice(&format!("{func}(): Detected an incomplete multibyte character in input string"));
            Ok(None)
        }
    }
}

fn encode(chars: &[char], enc: &str) -> Vec<u8> {
    let s: String = chars.iter().collect();
    convert("UTF-8", enc, s.as_bytes()).unwrap_or_else(|_| s.into_bytes())
}

fn find(h: &[char], n: &[char], from: usize) -> Option<usize> {
    if n.is_empty() || n.len() > h.len() {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| h[i..i + n.len()] == *n)
}

// ---- MIME headers -------------------------------------------------------------------

fn b64(data: &[u8]) -> String {
    crate::strings::base64_encode(data)
}

/// Decode RFC 2047 encoded words in `text` into `to`.
fn mime_decode(it: &Interp, text: &[u8], to: &str, mode: i64) -> R<Option<Vec<u8>>> {
    // Unfold header continuations.
    let mut t = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if (text[i] == b'\r' && text.get(i + 1) == Some(&b'\n') && matches!(text.get(i + 2), Some(b' ' | b'\t'))) || (text[i] == b'\n' && matches!(text.get(i + 1), Some(b' ' | b'\t'))) {
            i += if text[i] == b'\r' { 2 } else { 1 };
            continue;
        }
        t.push(text[i]);
        i += 1;
    }
    let mut out = vec![];
    let mut i = 0;
    let mut last_was_word = false;
    let mut pending_ws: Vec<u8> = vec![];
    while i < t.len() {
        if t[i..].starts_with(b"=?") {
            if let Some(word) = encoded_word(&t[i..]) {
                let (cs, enc, payload, len) = word;
                let raw = match enc {
                    b'B' | b'b' => crate::strings::base64_decode(payload),
                    _ => Some(q_decode(payload)),
                };
                if let Some(raw) = raw {
                    match convert(&cs, to, &raw) {
                        Ok(conv) => {
                            if !last_was_word {
                                out.extend_from_slice(&pending_ws);
                            }
                            pending_ws.clear();
                            out.extend_from_slice(&conv);
                            last_was_word = true;
                            i += len;
                            continue;
                        }
                        Err(_) if mode & 2 == 0 => {
                            it.notice("iconv_mime_decode(): Detected an illegal character in input string");
                            return Ok(None);
                        }
                        Err(_) => {}
                    }
                }
            }
        }
        let c = t[i];
        if c == b' ' || c == b'\t' {
            pending_ws.push(c);
        } else {
            out.extend_from_slice(&pending_ws);
            pending_ws.clear();
            out.push(c);
            last_was_word = false;
        }
        i += 1;
    }
    out.extend_from_slice(&pending_ws);
    Ok(Some(out))
}

/// `=?charset?E?payload?=`: (charset, encoding, payload, length).
fn encoded_word(t: &[u8]) -> Option<(String, u8, &[u8], usize)> {
    let rest = &t[2..];
    let q1 = rest.iter().position(|&c| c == b'?')?;
    let cs = crate::pstr::lossy(&rest[..q1]).into_owned();
    let enc = *rest.get(q1 + 1)?;
    if rest.get(q1 + 2) != Some(&b'?') || !matches!(enc, b'B' | b'b' | b'Q' | b'q') {
        return None;
    }
    let body = &rest[q1 + 3..];
    let end = body.windows(2).position(|w| w == b"?=")?;
    let cs = cs.split('*').next().unwrap_or("").to_string();
    Some((cs, enc, &body[..end], 2 + q1 + 3 + end + 2))
}

fn q_decode(p: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            b'_' => out.push(b' '),
            b'=' if i + 2 < p.len() => {
                if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&p[i + 1..(i + 3).min(p.len())]).unwrap_or("x"), 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'=');
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

fn mime_encode(it: &Interp, a: &[Value]) -> R<Value> {
    let name = crate::pstr::lossy(&s(it, a, 0)?).into_owned();
    let value = s(it, a, 1)?.to_vec();
    let mut scheme = "B".to_string();
    let (mut in_cs, mut out_cs) = (internal(), internal());
    let mut line_len = 76usize;
    let mut brk = "\r\n".to_string();
    if let Value::Array(p) = arg(a, 2) {
        for (k, v) in p.iter() {
            let k = crate::pstr::lossy(&k.to_value().scalar_bytes().unwrap_or_default()).into_owned();
            match k.as_str() {
                "scheme" => scheme = it.to_string_lossy(&v)?,
                "input-charset" => in_cs = it.to_string_lossy(&v)?,
                "output-charset" => out_cs = it.to_string_lossy(&v)?,
                "line-length" => line_len = it.to_int(&v)?.max(1) as usize,
                "line-break-chars" => brk = it.to_string_lossy(&v)?,
                _ => {}
            }
        }
    }
    let Some(chs) = chars(it, &value, &in_cs, "iconv_mime_encode")? else { return Ok(Value::Bool(false)) };
    let q = scheme.eq_ignore_ascii_case("Q");
    let prefix = format!("=?{out_cs}?{}?", if q { "Q" } else { "B" });
    let mut out = format!("{name}: ");
    let mut avail = line_len.saturating_sub(name.len() + 2);
    let mut i = 0;
    let enc_len = |n: usize, q: bool, part: &[u8]| -> usize {
        if q {
            part.iter().map(|&c| if c.is_ascii_alphanumeric() { 1 } else { 3 }).sum()
        } else {
            n.div_ceil(3) * 4
        }
    };
    while i < chs.len() {
        let mut j = i;
        let mut best = i;
        while j < chs.len() {
            j += 1;
            let bytes = encode(&chs[i..j], &out_cs);
            if prefix.len() + enc_len(bytes.len(), q, &bytes) + 2 > avail {
                break;
            }
            best = j;
        }
        if best == i {
            if avail < line_len.saturating_sub(1) && i > 0 {
                out.push_str(&brk);
                out.push(' ');
                avail = line_len - 1;
                continue;
            }
            best = i + 1;
        }
        let bytes = encode(&chs[i..best], &out_cs);
        out.push_str(&prefix);
        if q {
            for &c in &bytes {
                if c.is_ascii_alphanumeric() {
                    out.push(c as char);
                } else {
                    out.push_str(&format!("={c:02X}"));
                }
            }
        } else {
            out.push_str(&b64(&bytes));
        }
        out.push_str("?=");
        i = best;
        if i < chs.len() {
            out.push_str(&brk);
            out.push(' ');
            avail = line_len - 1;
        }
    }
    Ok(Value::str(&out))
}

fn decode_headers(it: &Interp, a: &[Value]) -> R<Value> {
    let text = s(it, a, 0)?.to_vec();
    let mode = if has(a, 1) { int(it, a, 1)? } else { 0 };
    let enc = enc_arg(it, a, 2)?;
    let Some(decoded) = mime_decode(it, &text, &enc, mode)? else { return Ok(Value::Bool(false)) };
    let mut out = PhpArray::new();
    for line in decoded.split(|&c| c == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(colon) = line.iter().position(|&c| c == b':') else { continue };
        let name = Key::from_bytes(&line[..colon]);
        let mut v = &line[colon + 1..];
        while v.first().is_some_and(|c| *c == b' ' || *c == b'\t') {
            v = &v[1..];
        }
        let v = Value::bytes(v.to_vec());
        let cur = out.get(&name);
        match cur {
            Some(Value::Array(prev)) => {
                let mut p = (*prev).clone();
                p.push(v);
                out.insert(name, Value::array(p));
            }
            Some(prev) => out.insert(name, Value::array(PhpArray::from_list([prev, v]))),
            None => out.insert(name, v),
        }
    }
    Ok(Value::array(out))
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("iconv", |it, _, a| {
        let (from, to) = (it.to_string_lossy(&arg(a, 0))?, it.to_string_lossy(&arg(a, 1))?);
        let data = s(it, a, 2)?;
        match convert(&from, &to, &data) {
            Ok(b) => Ok(Value::bytes(b)),
            Err(Failure::Open) => {
                it.warn(&format!("iconv(): Wrong encoding, conversion from \"{from}\" to \"{to}\" is not allowed"));
                Ok(Value::Bool(false))
            }
            Err(Failure::Illegal) => {
                it.notice("iconv(): Detected an illegal character in input string");
                Ok(Value::Bool(false))
            }
            Err(Failure::Incomplete) => {
                it.notice("iconv(): Detected an incomplete multibyte character in input string");
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("iconv_strlen", |it, _, a| {
        let enc = enc_arg(it, a, 1)?;
        Ok(match chars(it, &s(it, a, 0)?, &enc, "iconv_strlen")? {
            Some(c) => Value::Int(c.len() as i64),
            None => Value::Bool(false),
        })
    });
    t.insert("iconv_substr", |it, _, a| {
        let enc = enc_arg(it, a, 3)?;
        let Some(c) = chars(it, &s(it, a, 0)?, &enc, "iconv_substr")? else { return Ok(Value::Bool(false)) };
        let len = c.len() as i64;
        let mut off = int(it, a, 1)?;
        if off < 0 {
            off = (len + off).max(0);
        }
        if off > len {
            return Ok(Value::str(""));
        }
        let mut n = match a.get(2) {
            Some(v) if !v.is_null() => it.to_int(v)?,
            _ => len,
        };
        if n < 0 {
            n = (len - off + n).max(0);
        }
        let end = (off + n).min(len);
        Ok(Value::bytes(encode(&c[off as usize..end.max(off) as usize], &enc)))
    });
    t.insert("iconv_strpos", |it, _, a| {
        let enc = enc_arg(it, a, 3)?;
        let Some(h) = chars(it, &s(it, a, 0)?, &enc, "iconv_strpos")? else { return Ok(Value::Bool(false)) };
        let Some(n) = chars(it, &s(it, a, 1)?, &enc, "iconv_strpos")? else { return Ok(Value::Bool(false)) };
        let mut off = if has(a, 2) { int(it, a, 2)? } else { 0 };
        if off < 0 {
            off += h.len() as i64;
        }
        if off < 0 || off > h.len() as i64 {
            return Err(it.error("ValueError", "iconv_strpos(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"));
        }
        Ok(find(&h, &n, off as usize).map(|p| Value::Int(p as i64)).unwrap_or(Value::Bool(false)))
    });
    t.insert("iconv_strrpos", |it, _, a| {
        let enc = enc_arg(it, a, 2)?;
        let Some(h) = chars(it, &s(it, a, 0)?, &enc, "iconv_strrpos")? else { return Ok(Value::Bool(false)) };
        let Some(n) = chars(it, &s(it, a, 1)?, &enc, "iconv_strrpos")? else { return Ok(Value::Bool(false)) };
        if n.is_empty() || n.len() > h.len() {
            return Ok(Value::Bool(false));
        }
        Ok((0..=h.len() - n.len()).rev().find(|&i| h[i..i + n.len()] == *n).map(|p| Value::Int(p as i64)).unwrap_or(Value::Bool(false)))
    });
    t.insert("iconv_mime_encode", |it, _, a| mime_encode(it, a));
    t.insert("iconv_mime_decode", |it, _, a| {
        let text = s(it, a, 0)?.to_vec();
        let mode = if has(a, 1) { int(it, a, 1)? } else { 0 };
        let enc = enc_arg(it, a, 2)?;
        Ok(match mime_decode(it, &text, &enc, mode)? {
            Some(b) => Value::bytes(b),
            None => Value::Bool(false),
        })
    });
    t.insert("iconv_mime_decode_headers", |it, _, a| decode_headers(it, a));
    t.insert("iconv_get_encoding", |it, _, a| {
        let ty = if has(a, 0) { it.to_string_lossy(&a[0])? } else { "all".into() };
        let (i, o, n) = ENC.with(|e| e.borrow().clone());
        Ok(match ty.as_str() {
            "input_encoding" => Value::str(&i),
            "output_encoding" => Value::str(&o),
            "internal_encoding" => Value::str(&n),
            _ => {
                let mut arr = PhpArray::new();
                arr.insert(Key::from_bytes(b"input_encoding"), Value::str(&i));
                arr.insert(Key::from_bytes(b"output_encoding"), Value::str(&o));
                arr.insert(Key::from_bytes(b"internal_encoding"), Value::str(&n));
                Value::array(arr)
            }
        })
    });
    t.insert("iconv_set_encoding", |it, _, a| {
        let ty = it.to_string_lossy(&arg(a, 0))?;
        let enc = it.to_string_lossy(&arg(a, 1))?;
        if enc.len() >= 64 {
            return Ok(Value::Bool(false));
        }
        it.deprecated(&format!("iconv_set_encoding(): Use of iconv.{ty} is deprecated"));
        ENC.with(|e| {
            let mut e = e.borrow_mut();
            match ty.as_str() {
                "input_encoding" => e.0 = enc,
                "output_encoding" => e.1 = enc,
                "internal_encoding" => e.2 = enc,
                _ => {}
            }
        });
        Ok(Value::Bool(true))
    });
    t.insert("ob_iconv_handler", |it, _, a| Ok(Value::Str(it.to_str(&arg(a, 0))?)));
}
