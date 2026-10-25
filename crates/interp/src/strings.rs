//! String built-ins. PHP strings are bytes; `mb_*` functions treat them as UTF-8.

use std::collections::HashMap;

use crate::builtins::{arg, has, int, php_round, s};
use std::rc::Rc;

use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{fmt_float, Key, PhpArray, Value};

fn b(v: Vec<u8>) -> R<Value> {
    Ok(Value::bytes(v))
}

/// Characters for trim(): supports `a..z` ranges.
fn char_mask(spec: &[u8]) -> [bool; 256] {
    let mut m = [false; 256];
    let mut i = 0;
    while i < spec.len() {
        if i + 3 < spec.len() && &spec[i + 1..i + 3] == b".." {
            for c in spec[i]..=spec[i + 3] {
                m[c as usize] = true;
            }
            i += 4;
        } else {
            m[spec[i] as usize] = true;
            i += 1;
        }
    }
    m
}

const WS: &[u8] = b" \t\n\r\0\x0B";

fn trim_impl(it: &Interp, a: &[Value], left: bool, right: bool) -> R<Value> {
    let st = s(it, a, 0)?;
    let mask = char_mask(&if has(a, 1) { s(it, a, 1)?.to_vec() } else { WS.to_vec() });
    let mut lo = 0;
    let mut hi = st.len();
    if left {
        while lo < hi && mask[st[lo] as usize] {
            lo += 1;
        }
    }
    if right {
        while hi > lo && mask[st[hi - 1] as usize] {
            hi -= 1;
        }
    }
    b(st[lo..hi].to_vec())
}

/// PHP substr() offset/length rules.
pub fn substr_range(len: usize, start: i64, length: Option<i64>) -> (usize, usize) {
    let len = len as i64;
    let mut st = if start < 0 { (len + start).max(0) } else { start.min(len) };
    let mut end = match length {
        None => len,
        Some(l) if l < 0 => (len + l).max(st),
        Some(l) => (st + l).min(len),
    };
    if st > len {
        st = len;
    }
    if end < st {
        end = st;
    }
    (st as usize, end as usize)
}

fn find(h: &[u8], n: &[u8], from: usize) -> Option<usize> {
    if n.is_empty() {
        return (from <= h.len()).then_some(from);
    }
    h.get(from..)?.windows(n.len()).position(|w| w == n).map(|p| p + from)
}

fn rfind(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(h.len());
    }
    h.windows(n.len()).rposition(|w| w == n)
}

fn replace_all(subject: &[u8], search: &[u8], rep: &[u8], ci: bool) -> (Vec<u8>, i64) {
    if search.is_empty() {
        return (subject.to_vec(), 0);
    }
    let (hay, needle) = if ci { (subject.to_ascii_lowercase(), search.to_ascii_lowercase()) } else { (subject.to_vec(), search.to_vec()) };
    let mut out = Vec::with_capacity(subject.len());
    let mut i = 0;
    let mut n = 0;
    while let Some(p) = find(&hay, &needle, i) {
        out.extend_from_slice(&subject[i..p]);
        out.extend_from_slice(rep);
        i = p + search.len();
        n += 1;
    }
    out.extend_from_slice(&subject[i..]);
    (out, n)
}

fn str_replace_impl(it: &Interp, a: &mut [Value], ci: bool) -> R<Value> {
    let search = arg(a, 0);
    let rep = arg(a, 1);
    let mut count = 0i64;
    let mut apply = |subj: &[u8]| -> R<Vec<u8>> {
        let mut cur = subj.to_vec();
        match &search {
            Value::Array(ss) => {
                let reps: Option<Vec<Value>> = rep.as_array().map(|r| r.values().collect());
                for (i, sv) in ss.values().enumerate() {
                    let r = match &reps {
                        Some(list) => list.get(i).map(|v| it.to_str(v)).transpose()?.map(|x| x.to_vec()).unwrap_or_default(),
                        None => it.to_str(&rep)?.to_vec(),
                    };
                    let (next, n) = replace_all(&cur, &it.to_str(&sv)?, &r, ci);
                    cur = next;
                    count += n;
                }
            }
            sv => {
                let (next, n) = replace_all(&cur, &it.to_str(sv)?, &it.to_str(&rep)?, ci);
                cur = next;
                count += n;
            }
        }
        Ok(cur)
    };
    let result = match arg(a, 2) {
        Value::Array(subjects) => {
            let mut out = PhpArray::new();
            for (k, v) in subjects.iter() {
                out.insert(k.clone(), Value::bytes(apply(&it.to_str(&v)?)?));
            }
            Value::array(out)
        }
        v => Value::bytes(apply(&it.to_str(&v)?)?),
    };
    if a.len() > 3 {
        a[3] = Value::Int(count);
    }
    Ok(result)
}

fn ucwords(st: &[u8], delims: &[u8]) -> Vec<u8> {
    let mut out = st.to_vec();
    let mut start = true;
    for c in out.iter_mut() {
        if start {
            *c = c.to_ascii_uppercase();
        }
        start = delims.contains(c);
    }
    out
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("strlen", |it, _, a| Ok(Value::Int(s(it, a, 0)?.len() as i64)));
    t.insert("strtolower", |it, _, a| b(s(it, a, 0)?.to_ascii_lowercase()));
    t.insert("strtoupper", |it, _, a| b(s(it, a, 0)?.to_ascii_uppercase()));
    t.insert("ucfirst", |it, _, a| {
        let mut v = s(it, a, 0)?.to_vec();
        if let Some(c) = v.first_mut() {
            *c = c.to_ascii_uppercase();
        }
        b(v)
    });
    t.insert("lcfirst", |it, _, a| {
        let mut v = s(it, a, 0)?.to_vec();
        if let Some(c) = v.first_mut() {
            *c = c.to_ascii_lowercase();
        }
        b(v)
    });
    t.insert("ucwords", |it, _, a| {
        let delims = if has(a, 1) { s(it, a, 1)?.to_vec() } else { b" \t\r\n\x0C\x0B".to_vec() };
        b(ucwords(&s(it, a, 0)?, &delims))
    });
    t.insert("trim", |it, _, a| trim_impl(it, a, true, true));
    t.insert("ltrim", |it, _, a| trim_impl(it, a, true, false));
    t.insert("rtrim", |it, _, a| trim_impl(it, a, false, true));
    t.insert("chop", |it, _, a| trim_impl(it, a, false, true));
    t.insert("str_repeat", |it, _, a| {
        let n = int(it, a, 1)?;
        if n < 0 {
            return Err(it.error("ValueError", "str_repeat(): Argument #2 ($times) must be greater than or equal to 0"));
        }
        let st = s(it, a, 0)?;
        crate::builtins::reserve(it, st.len().saturating_mul(n as usize))?;
        b(st.repeat(n as usize))
    });
    t.insert("str_pad", |it, _, a| {
        let st = s(it, a, 0)?;
        let len = int(it, a, 1)?.max(0) as usize;
        crate::builtins::reserve(it, len)?;
        let pad = if has(a, 2) { s(it, a, 2)?.to_vec() } else { b" ".to_vec() };
        let ty = if has(a, 3) { int(it, a, 3)? } else { 1 };
        if pad.is_empty() {
            return Err(it.error("ValueError", "str_pad(): Argument #3 ($pad_string) must be a non-empty string"));
        }
        if len <= st.len() {
            return b(st.to_vec());
        }
        let total = len - st.len();
        let (l, r) = match ty {
            0 => (total, 0),
            2 => (total / 2, total - total / 2),
            _ => (0, total),
        };
        let mk = |n: usize| pad.iter().cycle().take(n).copied().collect::<Vec<u8>>();
        let mut out = mk(l);
        out.extend_from_slice(&st);
        out.extend(mk(r));
        b(out)
    });
    t.insert("str_split", |it, _, a| {
        let st = s(it, a, 0)?;
        let n = if has(a, 1) { int(it, a, 1)? } else { 1 };
        if n < 1 {
            return Err(it.error("ValueError", "str_split(): Argument #2 ($length) must be greater than 0"));
        }
        if st.is_empty() {
            return Ok(Value::array(PhpArray::from_list([Value::str("")])));
        }
        Ok(Value::array(PhpArray::from_list(st.chunks(n as usize).map(|c| Value::bytes(c.to_vec())))))
    });
    t.insert("strrev", |it, _, a| b(s(it, a, 0)?.iter().rev().copied().collect()));
    t.insert("substr", |it, _, a| {
        let st = s(it, a, 0)?;
        let len = if has(a, 2) && !arg(a, 2).is_null() { Some(int(it, a, 2)?) } else { None };
        let (lo, hi) = substr_range(st.len(), int(it, a, 1)?, len);
        b(st[lo..hi].to_vec())
    });
    t.insert("strpos", |it, _, a| strpos_impl(it, a, false));
    t.insert("stripos", |it, _, a| strpos_impl(it, a, true));
    t.insert("strrpos", |it, _, a| {
        let (h, n) = (s(it, a, 0)?, s(it, a, 1)?);
        Ok(rfind(&h, &n).map(|p| Value::Int(p as i64)).unwrap_or(Value::Bool(false)))
    });
    t.insert("strstr", |it, _, a| {
        let (h, n) = (s(it, a, 0)?, s(it, a, 1)?);
        Ok(match find(&h, &n, 0) {
            Some(p) if arg(a, 2).truthy() => Value::bytes(h[..p].to_vec()),
            Some(p) => Value::bytes(h[p..].to_vec()),
            None => Value::Bool(false),
        })
    });
    t.insert("strrchr", |it, _, a| {
        let (h, n) = (s(it, a, 0)?, s(it, a, 1)?);
        let Some(&c) = n.first() else { return Ok(Value::Bool(false)) };
        Ok(h.iter().rposition(|x| *x == c).map(|p| Value::bytes(h[p..].to_vec())).unwrap_or(Value::Bool(false)))
    });
    t.insert("str_contains", |it, _, a| Ok(Value::Bool(find(&s(it, a, 0)?, &s(it, a, 1)?, 0).is_some())));
    t.insert("str_starts_with", |it, _, a| Ok(Value::Bool(s(it, a, 0)?.starts_with(&s(it, a, 1)?))));
    t.insert("str_ends_with", |it, _, a| Ok(Value::Bool(s(it, a, 0)?.ends_with(&s(it, a, 1)?))));
    t.insert("str_replace", |it, _, a| str_replace_impl(it, a, false));
    t.insert("str_ireplace", |it, _, a| str_replace_impl(it, a, true));
    t.insert("substr_count", |it, _, a| {
        let (h, n) = (s(it, a, 0)?, s(it, a, 1)?);
        if n.is_empty() {
            return Err(it.error("ValueError", "substr_count(): Argument #2 ($needle) cannot be empty"));
        }
        Ok(Value::Int(replace_all(&h, &n, b"", false).1))
    });
    t.insert("substr_replace", |it, _, a| {
        let st = s(it, a, 0)?;
        let rep = s(it, a, 1)?;
        let len = if has(a, 3) && !arg(a, 3).is_null() { Some(int(it, a, 3)?) } else { None };
        let (lo, hi) = substr_range(st.len(), int(it, a, 2)?, len);
        let mut out = st[..lo].to_vec();
        out.extend_from_slice(&rep);
        out.extend_from_slice(&st[hi..]);
        b(out)
    });
    t.insert("implode", |it, _, a| implode(it, a));
    t.insert("join", |it, _, a| implode(it, a));
    t.insert("explode", |it, _, a| {
        let sep = s(it, a, 0)?;
        let st = s(it, a, 1)?;
        if sep.is_empty() {
            return Err(it.error("ValueError", "explode(): Argument #1 ($separator) cannot be empty"));
        }
        let limit = if has(a, 2) { int(it, a, 2)? } else { i64::MAX };
        let mut parts: Vec<Vec<u8>> = vec![];
        let mut i = 0;
        while let Some(p) = find(&st, &sep, i) {
            if limit > 0 && parts.len() as i64 == limit.max(1) - 1 {
                break;
            }
            parts.push(st[i..p].to_vec());
            i = p + sep.len();
        }
        parts.push(st[i..].to_vec());
        if limit < 0 {
            let keep = parts.len() as i64 + limit;
            parts.truncate(keep.max(0) as usize);
        }
        Ok(Value::array(PhpArray::from_list(parts.into_iter().map(Value::bytes))))
    });
    t.insert("strcmp", |it, _, a| Ok(Value::Int(ord(s(it, a, 0)?.cmp(&s(it, a, 1)?)))));
    t.insert("strcasecmp", |it, _, a| {
        Ok(Value::Int(ord(s(it, a, 0)?.to_ascii_lowercase().cmp(&s(it, a, 1)?.to_ascii_lowercase()))))
    });
    t.insert("strncmp", |it, _, a| {
        let n = int(it, a, 2)?.max(0) as usize;
        let (x, y) = (s(it, a, 0)?, s(it, a, 1)?);
        Ok(Value::Int(ord(x[..n.min(x.len())].cmp(&y[..n.min(y.len())]))))
    });
    t.insert("strncasecmp", |it, _, a| {
        let n = int(it, a, 2)?.max(0) as usize;
        let (x, y) = (s(it, a, 0)?.to_ascii_lowercase(), s(it, a, 1)?.to_ascii_lowercase());
        Ok(Value::Int(ord(x[..n.min(x.len())].cmp(&y[..n.min(y.len())]))))
    });
    t.insert("strnatcmp", |it, _, a| Ok(Value::Int(ord(natcmp(&s(it, a, 0)?, &s(it, a, 1)?, false)))));
    t.insert("strnatcasecmp", |it, _, a| Ok(Value::Int(ord(natcmp(&s(it, a, 0)?, &s(it, a, 1)?, true)))));
    t.insert("chr", |it, _, a| Ok(Value::Str(one_byte(int(it, a, 0)?.rem_euclid(256) as u8))));
    t.insert("ord", |it, _, a| Ok(Value::Int(s(it, a, 0)?.first().copied().unwrap_or(0) as i64)));
    t.insert("nl2br", |it, _, a| b(replace_all(&s(it, a, 0)?, b"\n", b"<br />\n", false).0));
    t.insert("htmlspecialchars", |it, _, a| {
        let st = s(it, a, 0)?;
        let flags = if has(a, 1) { int(it, a, 1)? } else { 3 | 8 };
        let double = !has(a, 3) || arg(a, 3).truthy();
        let mut out = vec![];
        let mut i = 0;
        while i < st.len() {
            let c = st[i];
            match c {
                b'&' if !double && is_entity(&st[i..]) => out.push(b'&'),
                b'&' => out.extend_from_slice(b"&amp;"),
                b'<' => out.extend_from_slice(b"&lt;"),
                b'>' => out.extend_from_slice(b"&gt;"),
                b'"' if flags & 2 != 0 => out.extend_from_slice(b"&quot;"),
                b'\'' if flags & 3 == 3 => out.extend_from_slice(b"&#039;"),
                _ => out.push(c),
            }
            i += 1;
        }
        b(out)
    });
    t.insert("htmlspecialchars_decode", |it, _, a| {
        let mut st = s(it, a, 0)?.to_vec();
        for (from, to) in [(&b"&lt;"[..], &b"<"[..]), (b"&gt;", b">"), (b"&quot;", b"\""), (b"&#039;", b"'"), (b"&#39;", b"'"), (b"&amp;", b"&")] {
            st = replace_all(&st, from, to, false).0;
        }
        b(st)
    });
    t.insert("strip_tags", |it, _, a| {
        let st = s(it, a, 0)?;
        let mut out = vec![];
        let mut in_tag = false;
        for &c in st.iter() {
            match c {
                b'<' => in_tag = true,
                b'>' if in_tag => in_tag = false,
                _ if !in_tag => out.push(c),
                _ => {}
            }
        }
        b(out)
    });
    t.insert("addslashes", |it, _, a| {
        let mut out = vec![];
        for &c in s(it, a, 0)?.iter() {
            match c {
                b'\'' | b'"' | b'\\' => out.extend_from_slice(&[b'\\', c]),
                0 => out.extend_from_slice(b"\\0"),
                _ => out.push(c),
            }
        }
        b(out)
    });
    t.insert("stripslashes", |it, _, a| {
        let st = s(it, a, 0)?;
        let mut out = vec![];
        let mut i = 0;
        while i < st.len() {
            if st[i] == b'\\' && i + 1 < st.len() {
                out.push(if st[i + 1] == b'0' { 0 } else { st[i + 1] });
                i += 2;
            } else if st[i] != b'\\' {
                out.push(st[i]);
                i += 1;
            } else {
                i += 1;
            }
        }
        b(out)
    });
    t.insert("strtr", |it, _, a| {
        let st = s(it, a, 0)?;
        if a.len() == 2 {
            let Value::Array(pairs) = arg(a, 1) else { return b(st.to_vec()) };
            let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = pairs
                .iter()
                .map(|(k, v)| Ok((it.to_str(&k.to_value())?.to_vec(), it.to_str(&v)?.to_vec())))
                .collect::<R<_>>()?;
            pairs.retain(|(k, _)| !k.is_empty());
            // Longest match first, never re-scanning replaced text.
            pairs.sort_by(|x, y| y.0.len().cmp(&x.0.len()));
            let mut out = vec![];
            let mut i = 0;
            'outer: while i < st.len() {
                for (k, v) in &pairs {
                    if st[i..].starts_with(k) {
                        out.extend_from_slice(v);
                        i += k.len();
                        continue 'outer;
                    }
                }
                out.push(st[i]);
                i += 1;
            }
            return b(out);
        }
        let (from, to) = (s(it, a, 1)?, s(it, a, 2)?);
        let n = from.len().min(to.len());
        let mut map: [u8; 256] = std::array::from_fn(|i| i as u8);
        for i in 0..n {
            map[from[i] as usize] = to[i];
        }
        b(st.iter().map(|c| map[*c as usize]).collect())
    });
    t.insert("wordwrap", |it, _, a| {
        let st = s(it, a, 0)?;
        let width = if has(a, 1) { int(it, a, 1)? } else { 75 } as usize;
        let brk = if has(a, 2) { s(it, a, 2)?.to_vec() } else { b"\n".to_vec() };
        let cut = arg(a, 3).truthy();
        let mut out: Vec<u8> = vec![];
        let mut line_len = 0usize;
        for (wi, word) in st.split(|c| *c == b' ').enumerate() {
            let mut word = word.to_vec();
            if wi > 0 {
                if line_len + 1 + word.len() <= width {
                    out.push(b' ');
                    line_len += 1;
                } else {
                    out.extend_from_slice(&brk);
                    line_len = 0;
                }
            }
            while cut && word.len() > width && width > 0 {
                if line_len > 0 {
                    out.extend_from_slice(&brk);
                    line_len = 0;
                }
                out.extend_from_slice(&word[..width]);
                out.extend_from_slice(&brk);
                word = word[width..].to_vec();
            }
            out.extend_from_slice(&word);
            line_len += word.len();
            if let Some(p) = word.iter().rposition(|c| *c == b'\n') {
                line_len = word.len() - p - 1;
            }
        }
        b(out)
    });
    t.insert("sprintf", |it, _, a| b(sprintf(it, a)?));
    t.insert("vsprintf", |it, _, a| {
        let mut args = vec![arg(a, 0)];
        if let Value::Array(x) = arg(a, 1) {
            args.extend(x.values());
        }
        b(sprintf(it, &args)?)
    });
    t.insert("printf", |it, _, a| {
        let out = sprintf(it, a)?;
        it.echo(&out);
        Ok(Value::Int(out.len() as i64))
    });
    t.insert("number_format", |it, _, a| {
        let n = crate::builtins::float(it, a, 0)?;
        let dec = if has(a, 1) { int(it, a, 1)?.max(0) as usize } else { 0 };
        let dp = if has(a, 2) { s(it, a, 2)?.to_vec() } else { b".".to_vec() };
        let ts = if has(a, 3) { s(it, a, 3)?.to_vec() } else { b",".to_vec() };
        b(number_format(n, dec, &dp, &ts))
    });
    t.insert("str_word_count", |it, _, a| {
        let st = s(it, a, 0)?;
        let words = st.split(|c| !(c.is_ascii_alphabetic() || *c == b'\'' || *c == b'-')).filter(|w| !w.is_empty());
        if int(it, a, 1)? == 0 {
            return Ok(Value::Int(words.count() as i64));
        }
        Ok(Value::array(PhpArray::from_list(words.map(|w| Value::bytes(w.to_vec())))))
    });
    t.insert("lcg", |_, _, _| Ok(Value::Null));
    t.insert("md5", |it, _, a| Ok(Value::str(&crate::strings::md5_hex(&s(it, a, 0)?))));
    t.insert("crc32", |it, _, a| Ok(Value::Int(crc32(&s(it, a, 0)?) as i64)));
    t.insert("base64_encode", |it, _, a| Ok(Value::str(&base64_encode(&s(it, a, 0)?))));
    t.insert("base64_decode", |it, _, a| Ok(base64_decode(&s(it, a, 0)?).map(Value::bytes).unwrap_or(Value::Bool(false))));
    t.insert("urlencode", |it, _, a| Ok(Value::str(&url_encode(&s(it, a, 0)?, true))));
    t.insert("rawurlencode", |it, _, a| Ok(Value::str(&url_encode(&s(it, a, 0)?, false))));
    t.insert("urldecode", |it, _, a| b(url_decode(&s(it, a, 0)?, true)));
    t.insert("rawurldecode", |it, _, a| b(url_decode(&s(it, a, 0)?, false)));
    t.insert("http_build_query", |it, _, a| {
        let Value::Array(x) = arg(a, 0) else { return Ok(Value::str("")) };
        let mut parts = vec![];
        build_query(it, &x, None, &mut parts)?;
        Ok(Value::str(&parts.join("&")))
    });

    // mb_* (UTF-8)
    t.insert("mb_strwidth", |it, _, a| Ok(Value::Int(utf8(&s(it, a, 0)?).chars().map(char_width).sum::<usize>() as i64)));
    t.insert("mb_strimwidth", |it, _, a| {
        let st = utf8(&s(it, a, 0)?).into_owned();
        let chars: Vec<char> = st.chars().collect();
        let start = int(it, a, 1)?;
        let start = if start < 0 { (chars.len() as i64 + start).max(0) as usize } else { (start as usize).min(chars.len()) };
        let width = int(it, a, 2)?;
        let marker: Vec<char> = if has(a, 3) { utf8(&s(it, a, 3)?).chars().collect() } else { vec![] };
        let rest = &chars[start..];
        let total: usize = rest.iter().map(|c| char_width(*c)).sum();
        let width = if width < 0 { (total as i64 + width).max(0) as usize } else { width as usize };
        if total <= width {
            return Ok(Value::str(&rest.iter().collect::<String>()));
        }
        let mw: usize = marker.iter().map(|c| char_width(*c)).sum();
        let budget = width.saturating_sub(mw);
        let mut out = String::new();
        let mut w = 0;
        for c in rest {
            if w + char_width(*c) > budget {
                break;
            }
            w += char_width(*c);
            out.push(*c);
        }
        out.extend(marker);
        Ok(Value::str(&out))
    });
    t.insert("mb_ucfirst", |it, _, a| {
        let st = utf8(&s(it, a, 0)?).into_owned();
        let mut c = st.chars();
        Ok(Value::str(&match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }))
    });
    t.insert("mb_lcfirst", |it, _, a| {
        let st = utf8(&s(it, a, 0)?).into_owned();
        let mut c = st.chars();
        Ok(Value::str(&match c.next() {
            Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }))
    });
    t.insert("mb_trim", |it, _, a| mb_trim(it, a, true, true));
    t.insert("mb_ltrim", |it, _, a| mb_trim(it, a, true, false));
    t.insert("mb_rtrim", |it, _, a| mb_trim(it, a, false, true));
    t.insert("mb_strrpos", |it, _, a| {
        let (h, n) = (utf8(&s(it, a, 0)?).into_owned(), utf8(&s(it, a, 1)?).into_owned());
        Ok(h.rfind(&n).map(|p| Value::Int(h[..p].chars().count() as i64)).unwrap_or(Value::Bool(false)))
    });
    t.insert("mb_stripos", |it, _, a| {
        let (h, n) = (utf8(&s(it, a, 0)?).to_lowercase(), utf8(&s(it, a, 1)?).to_lowercase());
        Ok(h.find(&n).map(|p| Value::Int(h[..p].chars().count() as i64)).unwrap_or(Value::Bool(false)))
    });
    t.insert("mb_substr_count", |it, _, a| {
        let (h, n) = (utf8(&s(it, a, 0)?).into_owned(), utf8(&s(it, a, 1)?).into_owned());
        if n.is_empty() {
            return Err(it.error("ValueError", "mb_substr_count(): Argument #2 ($needle) must not be empty"));
        }
        Ok(Value::Int(h.matches(&n).count() as i64))
    });
    t.insert("mb_ord", |it, _, a| Ok(utf8(&s(it, a, 0)?).chars().next().map(|c| Value::Int(c as i64)).unwrap_or(Value::Bool(false))));
    t.insert("mb_chr", |it, _, a| Ok(char::from_u32(int(it, a, 0)? as u32).map(|c| Value::str(&c.to_string())).unwrap_or(Value::Bool(false))));
    t.insert("mb_check_encoding", |it, _, a| Ok(Value::Bool(std::str::from_utf8(&s(it, a, 0)?).is_ok())));
    t.insert("mb_detect_encoding", |it, _, a| {
        Ok(Value::str(if std::str::from_utf8(&s(it, a, 0)?).is_ok() { "UTF-8" } else { "ISO-8859-1" }))
    });
    t.insert("mb_convert_encoding", |it, _, a| {
        let to = String::from_utf8_lossy(&s(it, a, 1)?).to_ascii_uppercase();
        let st = s(it, a, 0)?;
        if to == "UTF-8" {
            return Ok(Value::str(&utf8(&st)));
        }
        Err(it.error("ValueError", &format!("phpx: mb_convert_encoding() to {to} is not supported yet")))
    });
    t.insert("mb_scrub", |it, _, a| Ok(Value::str(&utf8(&s(it, a, 0)?))));
    t.insert("mb_strstr", |it, _, a| {
        let (h, n) = (utf8(&s(it, a, 0)?).into_owned(), utf8(&s(it, a, 1)?).into_owned());
        Ok(match h.find(&n) {
            Some(p) if crate::builtins::arg(a, 2).truthy() => Value::str(&h[..p]),
            Some(p) => Value::str(&h[p..]),
            None => Value::Bool(false),
        })
    });
    t.insert("iconv_strlen", |it, _, a| Ok(Value::Int(utf8(&s(it, a, 0)?).chars().count() as i64)));
    t.insert("iconv", |it, _, a| Ok(Value::str(&utf8(&s(it, a, 2)?))));
    t.insert("ctype_alnum", |it, _, a| ctype(it, a, |c| c.is_ascii_alphanumeric()));
    t.insert("ctype_alpha", |it, _, a| ctype(it, a, |c| c.is_ascii_alphabetic()));
    t.insert("ctype_digit", |it, _, a| ctype(it, a, |c| c.is_ascii_digit()));
    t.insert("ctype_lower", |it, _, a| ctype(it, a, |c| c.is_ascii_lowercase()));
    t.insert("ctype_upper", |it, _, a| ctype(it, a, |c| c.is_ascii_uppercase()));
    t.insert("ctype_space", |it, _, a| ctype(it, a, |c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C)));
    t.insert("ctype_punct", |it, _, a| ctype(it, a, |c| c.is_ascii_punctuation()));
    t.insert("ctype_xdigit", |it, _, a| ctype(it, a, |c| c.is_ascii_hexdigit()));
    t.insert("ctype_cntrl", |it, _, a| ctype(it, a, |c| c.is_ascii_control()));
    t.insert("ctype_print", |it, _, a| ctype(it, a, |c| (32..127).contains(&c)));
    t.insert("ctype_graph", |it, _, a| ctype(it, a, |c| (33..127).contains(&c)));
    t.insert("mb_strlen", |it, _, a| Ok(Value::Int(utf8(&s(it, a, 0)?).chars().count() as i64)));
    t.insert("mb_strtoupper", |it, _, a| Ok(Value::str(&utf8(&s(it, a, 0)?).to_uppercase())));
    t.insert("mb_strtolower", |it, _, a| Ok(Value::str(&utf8(&s(it, a, 0)?).to_lowercase())));
    t.insert("mb_substr", |it, _, a| {
        let st = utf8(&s(it, a, 0)?).into_owned();
        let chars: Vec<char> = st.chars().collect();
        let len = if has(a, 2) && !arg(a, 2).is_null() { Some(int(it, a, 2)?) } else { None };
        let (lo, hi) = substr_range(chars.len(), int(it, a, 1)?, len);
        Ok(Value::str(&chars[lo..hi].iter().collect::<String>()))
    });
    t.insert("mb_str_split", |it, _, a| {
        let st = utf8(&s(it, a, 0)?).into_owned();
        let n = if has(a, 1) { int(it, a, 1)?.max(1) as usize } else { 1 };
        let chars: Vec<char> = st.chars().collect();
        Ok(Value::array(PhpArray::from_list(chars.chunks(n).map(|c| Value::str(&c.iter().collect::<String>())))))
    });
    t.insert("mb_strpos", |it, _, a| {
        let (h, n) = (utf8(&s(it, a, 0)?).into_owned(), utf8(&s(it, a, 1)?).into_owned());
        Ok(h.find(&n).map(|p| Value::Int(h[..p].chars().count() as i64)).unwrap_or(Value::Bool(false)))
    });
    t.insert("mb_str_pad", |it, _, a| {
        let st = utf8(&s(it, a, 0)?).into_owned();
        let len = int(it, a, 1)?.max(0) as usize;
        let pad: Vec<char> = if has(a, 2) { utf8(&s(it, a, 2)?).chars().collect() } else { vec![' '] };
        let have = st.chars().count();
        if len <= have || pad.is_empty() {
            return Ok(Value::str(&st));
        }
        let total = len - have;
        let ty = if has(a, 3) { int(it, a, 3)? } else { 1 };
        let (l, r) = match ty {
            0 => (total, 0),
            2 => (total / 2, total - total / 2),
            _ => (0, total),
        };
        let mk = |n: usize| pad.iter().cycle().take(n).collect::<String>();
        Ok(Value::str(&format!("{}{st}{}", mk(l), mk(r))))
    });
    t.insert("mb_convert_case", |it, _, a| {
        let st = utf8(&s(it, a, 0)?).into_owned();
        Ok(Value::str(&match int(it, a, 1)? {
            0 => st.to_uppercase(),
            1 => st.to_lowercase(),
            _ => st
                .split(' ')
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        }))
    });
    t.insert("mb_internal_encoding", |_, _, _| Ok(Value::Bool(true)));
}

/// ctype_*: true for a non-empty string whose every byte matches.
fn ctype(it: &Interp, a: &[Value], pred: fn(u8) -> bool) -> R<Value> {
    Ok(Value::Bool(match arg(a, 0) {
        Value::Str(st) => !st.is_empty() && st.iter().all(|c| pred(*c)),
        // Legacy: ints in -128..=255 are treated as a character code.
        Value::Int(n) if (-128..=255).contains(&n) => pred(n.rem_euclid(256) as u8),
        Value::Int(n) => !it.to_str(&Value::Int(n))?.is_empty() && it.to_str(&Value::Int(n))?.iter().all(|c| pred(*c)),
        _ => false,
    }))
}

fn mb_trim(it: &Interp, a: &[Value], left: bool, right: bool) -> R<Value> {
    let st = utf8(&s(it, a, 0)?).into_owned();
    let chars: Vec<char> = if has(a, 1) && !arg(a, 1).is_null() { utf8(&s(it, a, 1)?).chars().collect() } else { vec![] };
    let pred = |c: char| if chars.is_empty() { c.is_whitespace() || c == '\0' } else { chars.contains(&c) };
    let r = match (left, right) {
        (true, true) => st.trim_matches(pred),
        (true, false) => st.trim_start_matches(pred),
        _ => st.trim_end_matches(pred),
    };
    Ok(Value::str(r))
}

/// East Asian wide characters count as 2 (same ranges as mbstring's polyfill).
fn char_width(c: char) -> usize {
    let u = c as u32;
    let wide = matches!(u, 0x1100..=0x115F | 0x2329 | 0x232A | 0x2E80..=0x303E | 0x3040..=0xA4CF | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF | 0xFE10..=0xFE19 | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6
        | 0x20000..=0x2FFFD | 0x30000..=0x3FFFD);
    if wide { 2 } else { 1 }
}

thread_local! {
    /// Interned one-byte strings (like PHP's ZSTR_CHAR): chr() and string
    /// offsets don't allocate.
    static ONE_BYTE: Vec<crate::value::PStr> = (0..=255u8).map(|c| Rc::new(vec![c])).collect();
}

pub fn one_byte(c: u8) -> crate::value::PStr {
    ONE_BYTE.with(|t| t[c as usize].clone())
}

fn utf8(b: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(b)
}

fn ord(o: std::cmp::Ordering) -> i64 {
    o as i64
}

fn is_entity(s: &[u8]) -> bool {
    let end = s.iter().take(12).position(|c| *c == b';');
    match end {
        Some(e) if e > 1 => s[1..e].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'#'),
        _ => false,
    }
}

fn strpos_impl(it: &Interp, a: &[Value], ci: bool) -> R<Value> {
    let (mut h, mut n) = (s(it, a, 0)?.to_vec(), s(it, a, 1)?.to_vec());
    if ci {
        h.make_ascii_lowercase();
        n.make_ascii_lowercase();
    }
    let off = if has(a, 2) { int(it, a, 2)? } else { 0 };
    let start = if off < 0 { h.len() as i64 + off } else { off };
    if start < 0 || start > h.len() as i64 {
        return Err(it.error("ValueError", "strpos(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"));
    }
    Ok(find(&h, &n, start as usize).map(|p| Value::Int(p as i64)).unwrap_or(Value::Bool(false)))
}

fn implode(it: &Interp, a: &[Value]) -> R<Value> {
    let (sep, list) = match (arg(a, 0), arg(a, 1)) {
        (Value::Array(x), Value::Null) if a.len() == 1 => (crate::value::pbytes(b""), x),
        (sep, Value::Array(x)) => (it.to_str(&sep)?, x),
        (Value::Array(_), _) => return Err(it.error("TypeError", "implode(): Argument #2 ($array) must be of type ?array, string given")),
        (_, other) => {
            return Err(it.error(
                "TypeError",
                &format!("implode(): Argument #2 ($array) must be of type ?array, {} given", other.type_name()),
            ))
        }
    };
    let mut out = vec![];
    for (i, v) in list.values().enumerate() {
        if i > 0 {
            out.extend_from_slice(&sep);
        }
        out.extend_from_slice(&it.to_str(&v)?);
    }
    Ok(Value::bytes(out))
}

/// Natural-order comparison (strnatcmp): digit runs compare numerically.
pub fn natcmp(a: &[u8], b: &[u8], ci: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let si = i;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            let sj = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let x = std::str::from_utf8(&a[si..i]).unwrap().trim_start_matches('0');
            let y = std::str::from_utf8(&b[sj..j]).unwrap().trim_start_matches('0');
            match x.len().cmp(&y.len()).then(x.cmp(y)) {
                Equal => continue,
                o => return o,
            }
        }
        let (x, y) = if ci { (a[i].to_ascii_lowercase(), b[j].to_ascii_lowercase()) } else { (a[i], b[j]) };
        match x.cmp(&y) {
            Equal => {
                i += 1;
                j += 1;
            }
            o => return o,
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

pub fn number_format(n: f64, dec: usize, dp: &[u8], ts: &[u8]) -> Vec<u8> {
    let r = php_round(n, dec as i64);
    let txt = format!("{:.*}", dec, r.abs());
    let (int_part, frac) = match txt.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (txt, None),
    };
    let mut out = vec![];
    if r < 0.0 && (r * 10f64.powi(dec as i32)).round() != 0.0 {
        out.push(b'-');
    }
    let digits = int_part.as_bytes();
    for (i, d) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.extend_from_slice(ts);
        }
        out.push(*d);
    }
    if let Some(f) = frac {
        out.extend_from_slice(dp);
        out.extend_from_slice(f.as_bytes());
    }
    out
}

/// PHP sprintf: %[argnum$][flags][width][.precision]specifier
pub fn sprintf(it: &Interp, a: &[Value]) -> R<Vec<u8>> {
    let fmt = s(it, a, 0)?;
    let mut out = vec![];
    let mut next_arg = 1;
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        if c != b'%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if fmt.get(i) == Some(&b'%') {
            out.push(b'%');
            i += 1;
            continue;
        }
        // argnum$
        let mut argnum = None;
        let save = i;
        let mut n = 0usize;
        while i < fmt.len() && fmt[i].is_ascii_digit() {
            n = n * 10 + (fmt[i] - b'0') as usize;
            i += 1;
        }
        if fmt.get(i) == Some(&b'$') && i > save {
            argnum = Some(n);
            i += 1;
        } else {
            i = save;
        }
        let (mut left, mut plus, mut pad) = (false, false, b' ');
        loop {
            match fmt.get(i) {
                Some(b'-') => left = true,
                Some(b'+') => plus = true,
                Some(b'0') => pad = b'0',
                Some(b' ') => pad = b' ',
                Some(b'\'') => {
                    i += 1;
                    pad = *fmt.get(i).unwrap_or(&b' ');
                }
                _ => break,
            }
            i += 1;
        }
        let mut width = 0usize;
        while i < fmt.len() && fmt[i].is_ascii_digit() {
            width = width * 10 + (fmt[i] - b'0') as usize;
            i += 1;
        }
        let mut prec = None;
        if fmt.get(i) == Some(&b'.') {
            i += 1;
            let mut p = 0usize;
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                p = p * 10 + (fmt[i] - b'0') as usize;
                i += 1;
            }
            prec = Some(p);
        }
        let Some(&spec) = fmt.get(i) else {
            return Err(it.error("ValueError", "Missing format specifier at end of string"));
        };
        i += 1;
        let idx = match argnum {
            Some(n) => n,
            None => {
                next_arg += 1;
                next_arg - 1
            }
        };
        if idx >= a.len() {
            return Err(it.error("ArgumentCountError", &format!("{} arguments are required, {} given", idx + 1, a.len())));
        }
        let v = &a[idx];
        let (body, numeric): (Vec<u8>, bool) = match spec {
            b'd' | b'i' => {
                let n = it.to_int(v)?;
                let mut s = n.abs().to_string().into_bytes();
                if n < 0 {
                    s.insert(0, b'-');
                } else if plus {
                    s.insert(0, b'+');
                }
                (s, true)
            }
            b'u' => ((it.to_int(v)? as u64).to_string().into_bytes(), true),
            b'f' | b'F' => {
                let x = it.to_float(v)?;
                let p = prec.unwrap_or(6);
                let mut s = format!("{:.*}", p, x.abs()).into_bytes();
                if x.is_sign_negative() && format!("{:.*}", p, x.abs()).bytes().any(|c| c.is_ascii_digit() && c != b'0') {
                    s.insert(0, b'-');
                } else if plus {
                    s.insert(0, b'+');
                }
                (s, true)
            }
            b'e' | b'E' => {
                let x = it.to_float(v)?;
                let p = prec.unwrap_or(6);
                let t = format!("{:.*e}", p, x);
                let (m, e) = t.split_once('e').unwrap();
                let e: i32 = e.parse().unwrap();
                let mut s = format!("{m}e{}{}", if e < 0 { "-" } else { "+" }, e.abs());
                if spec == b'E' {
                    s = s.to_uppercase();
                }
                if plus && x >= 0.0 {
                    s.insert(0, '+');
                }
                (s.into_bytes(), true)
            }
            b'g' | b'G' => {
                let x = it.to_float(v)?;
                (fmt_float(x).into_bytes(), true)
            }
            b's' => {
                let st = it.to_str(v)?;
                let st = match prec {
                    Some(p) => st[..p.min(st.len())].to_vec(),
                    None => st.to_vec(),
                };
                (st, false)
            }
            b'x' => (format!("{:x}", it.to_int(v)? as u64).into_bytes(), false),
            b'X' => (format!("{:X}", it.to_int(v)? as u64).into_bytes(), false),
            b'o' => (format!("{:o}", it.to_int(v)? as u64).into_bytes(), false),
            b'b' => (format!("{:b}", it.to_int(v)? as u64).into_bytes(), false),
            b'c' => (vec![it.to_int(v)? as u8], false),
            other => {
                return Err(it.error("ValueError", &format!("Unknown format specifier \"{}\"", other as char)));
            }
        };
        if body.len() >= width {
            out.extend_from_slice(&body);
        } else if left {
            out.extend_from_slice(&body);
            // Zero padding never goes on the right.
            let p = if pad == b'0' { b' ' } else { pad };
            out.extend(std::iter::repeat_n(p, width - body.len()));
        } else if pad == b'0' && numeric && matches!(body.first(), Some(b'-' | b'+')) {
            out.push(body[0]);
            out.extend(std::iter::repeat_n(b'0', width - body.len()));
            out.extend_from_slice(&body[1..]);
        } else {
            out.extend(std::iter::repeat_n(pad, width - body.len()));
            out.extend_from_slice(&body);
        }
    }
    Ok(out)
}

fn url_encode(s: &[u8], plus: bool) -> String {
    let mut out = String::new();
    for &c in s {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(c as char),
            b'~' if !plus => out.push('~'),
            b' ' if plus => out.push('+'),
            _ => out.push_str(&format!("%{c:02X}")),
        }
    }
    out
}

fn url_decode(s: &[u8], plus: bool) -> Vec<u8> {
    let mut out = vec![];
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'+' if plus => out.push(b' '),
            b'%' if i + 2 < s.len() => {
                match std::str::from_utf8(&s[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                        continue;
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

fn build_query(it: &Interp, a: &PhpArray, prefix: Option<&str>, out: &mut Vec<String>) -> R<()> {
    for (k, v) in a.iter() {
        let ks = match k {
            Key::Int(n) => n.to_string(),
            Key::Str(s) => String::from_utf8_lossy(&s).into_owned(),
        };
        let name = match prefix {
            Some(p) => format!("{p}[{ks}]"),
            None => ks,
        };
        match v {
            Value::Array(inner) => build_query(it, &inner, Some(&name), out)?,
            Value::Null => {}
            Value::Bool(b) => out.push(format!("{}={}", url_encode(name.as_bytes(), true), b as i32)),
            other => out.push(format!("{}={}", url_encode(name.as_bytes(), true), url_encode(&it.to_str(&other)?, true))),
        }
    }
    Ok(())
}

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = vec![];
    let mut buf = 0u32;
    let mut bits = 0;
    for &c in data {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => continue,
        } as u32;
        buf = buf << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// MD5 (RFC 1321). For md5() compatibility only: not for security.
pub fn md5_hex(input: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4,
        11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    [a0, b0, c0, d0].iter().flat_map(|x| x.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_encodings() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"hello world"), "5eb63bbbe01eeed093cb22bb8f5acdc3");
        assert_eq!(crc32(b"The quick brown fox jumped over the lazy dog."), 2191738434);
        assert_eq!(base64_encode(b"hello!?"), "aGVsbG8hPw==");
        assert_eq!(base64_decode(b"aGVsbG8hPw==").unwrap(), b"hello!?");
        assert_eq!(number_format(1234567.891, 2, b".", b","), b"1,234,567.89");
        assert_eq!(number_format(-0.4, 0, b".", b","), b"0");
        assert_eq!(natcmp(b"img12", b"img10", false), std::cmp::Ordering::Greater);
        assert_eq!(natcmp(b"img2", b"img10", false), std::cmp::Ordering::Less);
    }
}
