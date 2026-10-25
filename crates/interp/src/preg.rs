//! preg_* functions on top of fancy-regex (backtracking engine with
//! lookaround and backreferences, close to PCRE for common patterns).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use fancy_regex::{Captures, Regex};

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Callable, Interp, R};
use crate::value::{Key, PhpArray, Value};

thread_local! {
    static CACHE: RefCell<HashMap<Vec<u8>, Rc<Compiled>>> = RefCell::new(HashMap::new());
}

struct Compiled {
    re: Regex,
    names: Vec<Option<String>>,
}

/// Parse `/pattern/flags` into a compiled regex.
fn compile(it: &Interp, pat: &[u8]) -> R<Option<Rc<Compiled>>> {
    if let Some(c) = CACHE.with(|c| c.borrow().get(pat).cloned()) {
        return Ok(Some(c));
    }
    let txt = String::from_utf8_lossy(pat).into_owned();
    let t = txt.trim_start();
    let Some(open) = t.chars().next() else {
        it.warn("preg_match(): Empty regular expression");
        return Ok(None);
    };
    if open.is_alphanumeric() || open == '\\' {
        it.warn("preg_match(): Delimiter must not be alphanumeric, backslash, or NUL");
        return Ok(None);
    }
    let close = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        c => c,
    };
    let body_start = open.len_utf8();
    let Some(end) = t.rfind(close).filter(|&e| e >= body_start) else {
        it.warn(&format!("preg_match(): No ending delimiter '{close}' found"));
        return Ok(None);
    };
    let mut body = t[body_start..end].to_string();
    let flags = &t[end + close.len_utf8()..];
    // `\/` inside `/.../` is just a slash.
    if !"\\^$.|?*+()[]{}".contains(close) {
        body = body.replace(&format!("\\{close}"), &close.to_string());
    }
    let mut inline = String::new();
    let mut anchored = false;
    for f in flags.chars() {
        match f {
            'i' | 'm' | 's' | 'x' | 'U' => inline.push(f),
            'u' | 'D' | 'S' | '\n' | '\r' | ' ' => {}
            'A' => anchored = true,
            other => {
                it.warn(&format!("preg_match(): Unknown modifier '{other}'"));
                return Ok(None);
            }
        }
    }
    let mut src = String::new();
    if !inline.is_empty() {
        src.push_str(&format!("(?{inline})"));
    }
    if anchored {
        src.push_str("\\A(?:");
    }
    src.push_str(&body);
    if anchored {
        src.push(')');
    }
    let re = match Regex::new(&src) {
        Ok(r) => r,
        Err(e) => {
            it.warn(&format!("preg_match(): Compilation failed: {e}"));
            return Ok(None);
        }
    };
    let names = re.capture_names().map(|n| n.map(str::to_string)).collect();
    let c = Rc::new(Compiled { re, names });
    CACHE.with(|cache| cache.borrow_mut().insert(pat.to_vec(), c.clone()));
    Ok(Some(c))
}

fn subject(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(String::from_utf8_lossy(&s(it, a, i)?).into_owned())
}

/// Match groups as PHP builds them: named groups appear twice (name and index);
/// trailing unmatched groups are dropped, middle ones are "".
fn groups(c: &Compiled, caps: &Captures, unmatched_null: bool, trim_trailing: bool) -> PhpArray {
    let mut out = PhpArray::new();
    let last = if trim_trailing { (0..caps.len()).rev().find(|&i| caps.get(i).is_some()).unwrap_or(0) } else { caps.len() - 1 };
    for i in 0..=last {
        let v = match caps.get(i) {
            Some(m) => Value::str(m.as_str()),
            None if unmatched_null => Value::Null,
            None => Value::str(""),
        };
        if let Some(Some(n)) = c.names.get(i) {
            out.insert(Key::Str(crate::value::pbytes(n.as_bytes())), v.clone());
        }
        out.insert(Key::Int(i as i64), v);
    }
    out
}

fn regex_err(it: &Interp, e: fancy_regex::Error) -> crate::interp::Ctrl {
    it.error("Error", &format!("preg: regex execution failed: {e}"))
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("preg_match", |it, _, a| {
        let Some(c) = compile(it, &s(it, a, 0)?)? else { return Ok(Value::Bool(false)) };
        let subj = subject(it, a, 1)?;
        let flags = int(it, a, 3)?;
        let off = if has(a, 4) { int(it, a, 4)?.max(0) as usize } else { 0 };
        let caps = c.re.captures_from_pos(&subj, off.min(subj.len())).map_err(|e| regex_err(it, e))?;
        let (found, arr) = match caps {
            Some(caps) => (1, groups(&c, &caps, flags & 512 != 0, true)),
            None => (0, PhpArray::new()),
        };
        if a.len() > 2 {
            a[2] = Value::array(arr);
        }
        Ok(Value::Int(found))
    });
    t.insert("preg_match_all", |it, _, a| {
        let Some(c) = compile(it, &s(it, a, 0)?)? else { return Ok(Value::Bool(false)) };
        let subj = subject(it, a, 1)?;
        let flags = if has(a, 3) && !arg(a, 3).is_null() { int(it, a, 3)? } else { 1 };
        let mut sets = vec![];
        for caps in c.re.captures_iter(&subj) {
            sets.push(groups(&c, &caps.map_err(|e| regex_err(it, e))?, flags & 512 != 0, false));
        }
        let n = sets.len() as i64;
        if a.len() > 2 {
            let result = if flags & 2 != 0 {
                PhpArray::from_list(sets.into_iter().map(Value::array))
            } else {
                // PREG_PATTERN_ORDER: one list per group.
                let mut out = PhpArray::new();
                for (i, name) in c.names.iter().enumerate() {
                    let col = PhpArray::from_list(sets.iter().map(|s| s.get(&Key::Int(i as i64)).unwrap_or(Value::str(""))));
                    if let Some(n) = name {
                        out.insert(Key::Str(crate::value::pbytes(n.as_bytes())), Value::array(col.clone()));
                    }
                    out.insert(Key::Int(i as i64), Value::array(col));
                }
                out
            };
            a[2] = Value::array(result);
        }
        Ok(Value::Int(n))
    });
    t.insert("preg_replace", |it, f, a| replace(it, f, a, false));
    t.insert("preg_replace_callback", |it, f, a| replace(it, f, a, true));
    t.insert("preg_split", |it, _, a| {
        let Some(c) = compile(it, &s(it, a, 0)?)? else { return Ok(Value::Bool(false)) };
        let subj = subject(it, a, 1)?;
        let limit = if has(a, 2) && !arg(a, 2).is_null() { int(it, a, 2)? } else { -1 };
        let flags = int(it, a, 3)?;
        let (no_empty, delim) = (flags & 1 != 0, flags & 2 != 0);
        let mut out = PhpArray::new();
        let mut last = 0;
        let push = |out: &mut PhpArray, piece: &str| {
            if !(no_empty && piece.is_empty()) {
                out.push(Value::str(piece));
            }
        };
        for caps in c.re.captures_iter(&subj) {
            let caps = caps.map_err(|e| regex_err(it, e))?;
            if limit > 0 && out.len() as i64 >= limit - 1 {
                break;
            }
            let m = caps.get(0).unwrap();
            if m.end() == 0 && m.start() == 0 {
                continue;
            }
            if m.start() == m.end() && m.start() == subj.len() {
                break;
            }
            push(&mut out, &subj[last..m.start()]);
            if delim {
                for i in 1..caps.len() {
                    if let Some(g) = caps.get(i) {
                        push(&mut out, g.as_str());
                    }
                }
            }
            last = m.end();
        }
        push(&mut out, &subj[last..]);
        Ok(Value::array(out))
    });
    t.insert("preg_quote", |it, _, a| {
        let st = s(it, a, 0)?;
        let delim = if has(a, 1) && !arg(a, 1).is_null() { s(it, a, 1)?.first().copied() } else { None };
        let mut out = vec![];
        for &c in st.iter() {
            if b".\\+*?[^]$(){}=!<>|:-#/".contains(&c) && (c != b'/' || delim == Some(b'/')) || Some(c) == delim {
                out.push(b'\\');
            }
            if c == 0 {
                out.extend_from_slice(b"\\000");
                continue;
            }
            out.push(c);
        }
        Ok(Value::bytes(out))
    });
    t.insert("preg_grep", |it, _, a| {
        let Some(c) = compile(it, &s(it, a, 0)?)? else { return Ok(Value::Bool(false)) };
        let invert = int(it, a, 2)? & 1 != 0;
        let mut out = PhpArray::new();
        if let Value::Array(x) = arg(a, 1) {
            for (k, v) in x.iter() {
                let txt = it.to_string_lossy(&v)?;
                if c.re.is_match(&txt).map_err(|e| regex_err(it, e))? != invert {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        Ok(Value::array(out))
    });
    t.insert("preg_last_error", |_, _, _| Ok(Value::Int(0)));
    t.insert("preg_last_error_msg", |_, _, _| Ok(Value::str("No error")));
}

/// Expand `$1`, `\1`, `${1}` in a replacement string.
fn expand(rep: &str, caps: &Captures) -> String {
    let b = rep.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if (c == b'$' || c == b'\\') && i + 1 < b.len() {
            let (braced, start) = if c == b'$' && b[i + 1] == b'{' { (true, i + 2) } else { (false, i + 1) };
            let mut j = start;
            while j < b.len() && j - start < 2 && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > start && (!braced || b.get(j) == Some(&b'}')) {
                let n: usize = rep[start..j].parse().unwrap();
                if let Some(m) = caps.get(n) {
                    out.push_str(m.as_str());
                }
                i = if braced { j + 1 } else { j };
                continue;
            }
        }
        let ch = rep[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn replace(it: &Interp, f: &mut crate::interp::Frame, a: &mut Vec<Value>, callback: bool) -> R<Value> {
    let patterns: Vec<Value> = match arg(a, 0) {
        Value::Array(x) => x.values().collect(),
        v => vec![v],
    };
    let reps: Vec<Value> = match arg(a, 1) {
        Value::Array(x) if !callback => x.values().collect(),
        v => vec![v; patterns.len()],
    };
    let cb: Option<Callable> = if callback { Some(it.to_callable(f, &arg(a, 1))?) } else { None };
    let limit = if has(a, 3) && !arg(a, 3).is_null() { int(it, a, 3)? } else { -1 };
    let total = std::cell::Cell::new(0i64);
    let one = |subj: String| -> R<Option<String>> {
        let mut cur = subj;
        for (pi, p) in patterns.iter().enumerate() {
            let Some(c) = compile(it, &it.to_str(p)?)? else { return Ok(None) };
            let rep = match &cb {
                Some(_) => String::new(),
                None => reps.get(pi).map(|r| it.to_string_lossy(r)).transpose()?.unwrap_or_default(),
            };
            let mut out = String::new();
            let mut last = 0;
            let mut n = 0;
            for caps in c.re.captures_iter(&cur) {
                let caps = caps.map_err(|e| regex_err(it, e))?;
                if limit >= 0 && n >= limit {
                    break;
                }
                let m = caps.get(0).unwrap();
                out.push_str(&cur[last..m.start()]);
                match &cb {
                    Some(cb) => {
                        let arr = groups(&c, &caps, false, true);
                        let r = it.call_callable(cb, vec![Value::array(arr)], vec![])?;
                        out.push_str(&it.to_string_lossy(&r)?);
                    }
                    None => out.push_str(&expand(&rep, &caps)),
                }
                last = m.end();
                n += 1;
            }
            total.set(total.get() + n);
            out.push_str(&cur[last..]);
            cur = out;
        }
        Ok(Some(cur))
    };
    let result = match arg(a, 2) {
        Value::Array(subjects) => {
            let mut out = PhpArray::new();
            for (k, v) in subjects.iter() {
                match one(it.to_string_lossy(&v)?)? {
                    Some(r) => out.insert(k.clone(), Value::str(&r)),
                    None => return Ok(Value::Null),
                }
            }
            Value::array(out)
        }
        v => one(it.to_string_lossy(&v)?)?.map(|r| Value::str(&r)).unwrap_or(Value::Null),
    };
    if a.len() > 4 {
        a[4] = Value::Int(total.get());
    }
    Ok(result)
}
