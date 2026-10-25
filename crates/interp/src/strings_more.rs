//! More of ext/standard's string functions: strrpos()/strripos() with
//! offsets, count_chars, str_increment/str_decrement, metaphone, uuencode,
//! quoted-printable decoding and friends.

use std::collections::HashMap;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{Key, PhpArray, Value};

fn rfind(h: &[u8], n: &[u8], lo: usize, hi_start: usize) -> Option<usize> {
    // Last match starting in lo..=hi_start.
    if n.len() > h.len() {
        return None;
    }
    let last = hi_start.min(h.len() - n.len());
    (lo..=last).rev().find(|&i| i <= last && &h[i..i + n.len()] == n)
}

/// strrpos() / strripos() with PHP's offset rules.
fn strrpos(it: &Interp, a: &[Value], name: &str, ci: bool) -> R<Value> {
    let (mut h, mut n) = (s(it, a, 0)?.to_vec(), s(it, a, 1)?.to_vec());
    if ci {
        h.make_ascii_lowercase();
        n.make_ascii_lowercase();
    }
    let offset = if has(a, 2) { int(it, a, 2)? } else { 0 };
    let len = h.len() as i64;
    let bad = || it.error("ValueError", &format!("{name}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"));
    let r = if offset >= 0 {
        if offset > len {
            return Err(bad());
        }
        if h.len() < n.len() {
            None
        } else {
            rfind(&h, &n, offset as usize, h.len())
        }
    } else {
        if -offset > len {
            return Err(bad());
        }
        let e = if (-offset) < n.len() as i64 { len - n.len() as i64 } else { len + offset };
        if e < 0 {
            None
        } else {
            rfind(&h, &n, 0, e as usize)
        }
    };
    Ok(r.map(|p| Value::Int(p as i64)).unwrap_or(Value::Bool(false)))
}

fn count_chars(it: &Interp, a: &[Value]) -> R<Value> {
    let st = s(it, a, 0)?;
    let mode = if has(a, 1) { int(it, a, 1)? } else { 0 };
    if !(0..=4).contains(&mode) {
        return Err(it.error("ValueError", "count_chars(): Argument #2 ($mode) must be between 0 and 4 (inclusive)"));
    }
    let mut counts = [0i64; 256];
    for &c in st.iter() {
        counts[c as usize] += 1;
    }
    Ok(match mode {
        0..=2 => {
            let mut out = PhpArray::new();
            for (c, &n) in counts.iter().enumerate() {
                if mode == 0 || (mode == 1 && n > 0) || (mode == 2 && n == 0) {
                    out.insert(Key::Int(c as i64), Value::Int(n));
                }
            }
            Value::array(out)
        }
        _ => Value::bytes((0..=255u8).filter(|&c| (counts[c as usize] > 0) == (mode == 3)).collect()),
    })
}

fn str_increment(it: &Interp, a: &[Value]) -> R<Value> {
    let st = s(it, a, 0)?;
    if st.is_empty() {
        return Err(it.error("ValueError", "str_increment(): Argument #1 ($string) must not be empty"));
    }
    if !st.iter().all(u8::is_ascii_alphanumeric) {
        return Err(it.error("ValueError", "str_increment(): Argument #1 ($string) must be composed only of alphanumeric ASCII characters"));
    }
    let mut b = st.to_vec();
    let mut i = b.len();
    loop {
        if i == 0 {
            let first = match b[0] {
                b'a' => b'a',
                b'A' => b'A',
                _ => b'1',
            };
            b.insert(0, first);
            break;
        }
        i -= 1;
        match b[i] {
            b'z' => b[i] = b'a',
            b'Z' => b[i] = b'A',
            b'9' => b[i] = b'0',
            c => {
                b[i] = c + 1;
                break;
            }
        }
    }
    Ok(Value::bytes(b))
}

fn str_decrement(it: &Interp, a: &[Value]) -> R<Value> {
    let st = s(it, a, 0)?;
    if st.is_empty() {
        return Err(it.error("ValueError", "str_decrement(): Argument #1 ($string) must not be empty"));
    }
    if !st.iter().all(u8::is_ascii_alphanumeric) {
        return Err(it.error("ValueError", "str_decrement(): Argument #1 ($string) must be composed only of alphanumeric ASCII characters"));
    }
    let out_of_range = || it.error("ValueError", &format!("str_decrement(): Argument #1 ($string) \"{}\" is out of decrement range", String::from_utf8_lossy(&st)));
    if st[0] == b'0' {
        return Err(out_of_range());
    }
    let mut b = st.to_vec();
    let mut i = b.len();
    let mut carry = true;
    while carry && i > 0 {
        i -= 1;
        match b[i] {
            b'a' => b[i] = b'z',
            b'A' => b[i] = b'Z',
            b'0' => b[i] = b'9',
            c => {
                b[i] = c - 1;
                carry = false;
            }
        }
    }
    if carry || (b[0] == b'0' && b.len() > 1) {
        if b.len() == 1 {
            return Err(out_of_range());
        }
        b.remove(0);
    }
    Ok(Value::bytes(b))
}

// ---- metaphone (ext/standard/metaphone.c) -----------------------------------------

const CODES: [u8; 26] = [1, 16, 4, 16, 9, 2, 4, 16, 9, 2, 0, 2, 2, 2, 1, 4, 0, 2, 4, 4, 1, 0, 0, 0, 8, 0];

fn enc(c: u8) -> u8 {
    if c.is_ascii_uppercase() {
        CODES[(c - b'A') as usize]
    } else {
        0
    }
}

fn metaphone(word: &[u8], max: usize) -> Vec<u8> {
    // The C code reads a NUL-terminated string.
    let word = &word[..word.iter().position(|&c| c == 0).unwrap_or(word.len())];
    let at = |i: usize| -> u8 { word.get(i).copied().unwrap_or(0) };
    let up = |c: u8| c.to_ascii_uppercase();
    let isvowel = |c: u8| enc(c) & 1 != 0;
    let affecth = |c: u8| enc(c) & 4 != 0;
    let makesoft = |c: u8| enc(c) & 8 != 0;
    let noghtof = |c: u8| enc(c) & 16 != 0;
    let mut out: Vec<u8> = vec![];
    let mut w = 0usize;
    // First letter.
    loop {
        let c = at(w);
        if c == 0 {
            return out;
        }
        if c.is_ascii_alphabetic() {
            break;
        }
        w += 1;
    }
    let next = |w: usize| up(at(w + 1));
    let after_next = |w: usize| if at(w + 1) != 0 { up(at(w + 2)) } else { 0 };
    let back = |w: usize, n: usize| if w >= n { up(at(w - n)) } else { 0 };
    let ahead = |w: usize, n: usize| {
        let mut i = 0;
        while at(w + i) != 0 && i < n {
            i += 1;
        }
        up(at(w + i))
    };
    match up(at(w)) {
        b'A' => {
            if next(w) == b'E' {
                out.push(b'E');
                w += 2;
            } else {
                out.push(b'A');
                w += 1;
            }
        }
        b'G' | b'K' | b'P' => {
            if next(w) == b'N' {
                out.push(b'N');
                w += 2;
            }
        }
        b'W' => {
            let n = next(w);
            if n == b'R' {
                out.push(b'R');
                w += 2;
            } else if n == b'H' || isvowel(n) {
                out.push(b'W');
                w += 2;
            }
        }
        b'X' => {
            out.push(b'S');
            w += 1;
        }
        c @ (b'E' | b'I' | b'O' | b'U') => {
            out.push(c);
            w += 1;
        }
        _ => {}
    }
    while at(w) != 0 && (max == 0 || out.len() < max) {
        let raw = at(w);
        let mut skip = 0;
        if !raw.is_ascii_alphabetic() {
            w += 1;
            continue;
        }
        let c = up(raw);
        let prev = back(w, 1);
        if c == prev && c != b'C' {
            w += 1;
            continue;
        }
        match c {
            b'B' => {
                if prev != b'M' {
                    out.push(b'B');
                }
            }
            b'C' => {
                let n = next(w);
                if makesoft(n) {
                    if n == b'I' && after_next(w) == b'A' {
                        out.push(b'X');
                    } else if prev == b'S' {
                    } else {
                        out.push(b'S');
                    }
                } else if n == b'H' {
                    out.push(b'X');
                    skip += 1;
                } else {
                    out.push(b'K');
                }
            }
            b'D' => {
                if next(w) == b'G' && makesoft(after_next(w)) {
                    out.push(b'J');
                    skip += 1;
                } else {
                    out.push(b'T');
                }
            }
            b'G' => {
                let n = next(w);
                if n == b'H' {
                    if !(noghtof(back(w, 3)) || back(w, 4) == b'H') {
                        out.push(b'F');
                        skip += 1;
                    }
                } else if n == b'N' {
                    let an = after_next(w);
                    if !an.is_ascii_alphabetic() || (an == b'E' && ahead(w, 3) == b'D') {
                    } else {
                        out.push(b'K');
                    }
                } else if makesoft(n) && prev != b'G' {
                    out.push(b'J');
                } else {
                    out.push(b'K');
                }
            }
            b'H' => {
                if isvowel(next(w)) && !affecth(prev) {
                    out.push(b'H');
                }
            }
            b'K' => {
                if prev != b'C' {
                    out.push(b'K');
                }
            }
            b'P' => out.push(if next(w) == b'H' { b'F' } else { b'P' }),
            b'Q' => out.push(b'K'),
            b'S' => {
                let n = next(w);
                if n == b'I' && matches!(after_next(w), b'O' | b'A') {
                    out.push(b'X');
                } else if n == b'H' {
                    out.push(b'X');
                    skip += 1;
                } else {
                    out.push(b'S');
                }
            }
            b'T' => {
                let n = next(w);
                if n == b'I' && matches!(after_next(w), b'O' | b'A') {
                    out.push(b'X');
                } else if n == b'H' {
                    out.push(b'0');
                    skip += 1;
                } else if !(n == b'C' && after_next(w) == b'H') {
                    out.push(b'T');
                }
            }
            b'V' => out.push(b'F'),
            b'W' => {
                if isvowel(next(w)) {
                    out.push(b'W');
                }
            }
            b'X' => {
                out.push(b'K');
                out.push(b'S');
            }
            b'Y' => {
                if isvowel(next(w)) {
                    out.push(b'Y');
                }
            }
            b'Z' => out.push(b'S'),
            b'F' | b'J' | b'L' | b'M' | b'N' | b'R' => out.push(c),
            _ => {}
        }
        w += skip + 1;
    }
    out
}

// ---- uuencode, quoted-printable ----------------------------------------------------------

fn uu_enc(c: u8) -> u8 {
    if c == 0 {
        b'`'
    } else {
        (c & 0o77) + b' '
    }
}

fn uuencode(src: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    for line in src.chunks(45) {
        out.push(uu_enc(line.len() as u8));
        for g in line.chunks(3) {
            let b = [g[0], *g.get(1).unwrap_or(&0), *g.get(2).unwrap_or(&0)];
            out.push(uu_enc(b[0] >> 2));
            out.push(uu_enc(((b[0] << 4) & 0o60) | ((b[1] >> 4) & 0o17)));
            out.push(uu_enc(((b[1] << 2) & 0o74) | ((b[2] >> 6) & 0o3)));
            out.push(uu_enc(b[2] & 0o77));
        }
        out.push(b'\n');
    }
    out.extend_from_slice(b"`\n");
    out
}

fn uu_dec(c: u8) -> u8 {
    (c.wrapping_sub(b' ')) & 0o77
}

fn uudecode(src: &[u8]) -> Option<Vec<u8>> {
    let mut out = vec![];
    let mut lines = src.split(|&c| c == b'\n');
    for line in &mut lines {
        if line.is_empty() {
            return None;
        }
        let n = uu_dec(line[0]) as usize;
        if n == 0 {
            return Some(out);
        }
        let body = &line[1..];
        if body.len() < n.div_ceil(3) * 4 {
            return None;
        }
        let mut got = vec![];
        for g in body.chunks(4) {
            if g.len() < 4 {
                break;
            }
            let d: Vec<u8> = g.iter().map(|&c| uu_dec(c)).collect();
            got.push((d[0] << 2) | (d[1] >> 4));
            got.push((d[1] << 4) | (d[2] >> 2));
            got.push((d[2] << 6) | d[3]);
        }
        got.truncate(n);
        out.extend_from_slice(&got);
    }
    None
}

fn qp_decode(st: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(st.len());
    let at = |i: usize| st.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while i < st.len() {
        if st[i] == b'=' {
            if at(i + 1).is_ascii_hexdigit() && at(i + 2).is_ascii_hexdigit() {
                out.push(u8::from_str_radix(std::str::from_utf8(&st[i + 1..i + 3]).unwrap(), 16).unwrap());
                i += 3;
            } else {
                let mut k = 1;
                while matches!(at(i + k), b' ' | b'\t') {
                    k += 1;
                }
                if i + k >= st.len() {
                    i += k;
                } else if at(i + k) == 13 && at(i + k + 1) == 10 {
                    i += k + 2;
                } else if matches!(at(i + k), 13 | 10) {
                    i += k + 1;
                } else {
                    out.push(st[i]);
                    i += 1;
                }
            }
        } else {
            out.push(st[i]);
            i += 1;
        }
    }
    out
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("strrpos", |it, _, a| strrpos(it, a, "strrpos", false));
    t.insert("strripos", |it, _, a| strrpos(it, a, "strripos", true));
    t.insert("strchr", |it, f, a| (it.builtins["strstr"])(it, f, a));
    t.insert("quotemeta", |it, _, a| {
        let st = s(it, a, 0)?;
        let mut out = Vec::with_capacity(st.len());
        for &c in st.iter() {
            if b".\\+*?[^]$()".contains(&c) {
                out.push(b'\\');
            }
            out.push(c);
        }
        Ok(Value::bytes(out))
    });
    t.insert("count_chars", |it, _, a| count_chars(it, a));
    t.insert("str_rot13", |it, _, a| {
        let st = s(it, a, 0)?;
        Ok(Value::bytes(
            st.iter()
                .map(|&c| match c {
                    b'a'..=b'z' => (c - b'a' + 13) % 26 + b'a',
                    b'A'..=b'Z' => (c - b'A' + 13) % 26 + b'A',
                    _ => c,
                })
                .collect(),
        ))
    });
    t.insert("str_shuffle", |it, _, a| {
        let mut b = s(it, a, 0)?.to_vec();
        let mut n = b.len();
        while n > 1 {
            n -= 1;
            let j = (crate::builtins::random_u64() % (n as u64 + 1)) as usize;
            b.swap(n, j);
        }
        Ok(Value::bytes(b))
    });
    t.insert("str_increment", |it, _, a| str_increment(it, a));
    t.insert("str_decrement", |it, _, a| str_decrement(it, a));
    t.insert("metaphone", |it, _, a| {
        let st = s(it, a, 0)?;
        let max = if has(a, 1) { int(it, a, 1)? } else { 0 };
        if max < 0 {
            return Err(it.error("ValueError", "metaphone(): Argument #2 ($max_phonemes) must be greater than or equal to 0"));
        }
        Ok(Value::bytes(metaphone(&st, max as usize)))
    });
    t.insert("strcoll", |it, _, a| {
        let (x, y) = (s(it, a, 0)?, s(it, a, 1)?);
        let cx = std::ffi::CString::new(x.iter().copied().take_while(|&c| c != 0).collect::<Vec<u8>>()).unwrap();
        let cy = std::ffi::CString::new(y.iter().copied().take_while(|&c| c != 0).collect::<Vec<u8>>()).unwrap();
        Ok(Value::Int(unsafe { libc::strcoll(cx.as_ptr(), cy.as_ptr()) } as i64))
    });
    t.insert("hebrev", |it, _, a| Ok(Value::Str(s(it, a, 0)?)));
    t.insert("convert_uuencode", |it, _, a| Ok(Value::bytes(uuencode(&s(it, a, 0)?))));
    t.insert("convert_uudecode", |it, _, a| {
        let st = s(it, a, 0)?;
        match uudecode(&st) {
            Some(b) => Ok(Value::bytes(b)),
            None => {
                it.warn("convert_uudecode(): Argument #1 ($data) is not a valid uuencoded string");
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("quoted_printable_decode", |it, _, a| Ok(Value::bytes(qp_decode(&s(it, a, 0)?))));
    t.insert("nl_langinfo", |it, _, a| {
        let item = int(it, a, 0)?;
        let p = unsafe { libc::nl_langinfo(item as libc::nl_item) };
        if p.is_null() {
            it.warn(&format!("nl_langinfo(): Item '{item}' is not valid"));
            return Ok(Value::Bool(false));
        }
        Ok(Value::bytes(unsafe { std::ffi::CStr::from_ptr(p) }.to_bytes().to_vec()))
    });
    let _ = arg;
}
