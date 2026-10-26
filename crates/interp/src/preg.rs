//! preg_* functions on PCRE2 (the library PHP itself uses), with JIT.
//! Subjects are bytes, so binary strings and offsets behave as in PHP.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use pcre2::bytes::{CaptureLocations, Regex, RegexBuilder};

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Callable, Interp, R};
use crate::pstr::PStr;
use crate::value::{Key, PhpArray, Value};

const OFFSET_CAPTURE: i64 = 256;
const UNMATCHED_AS_NULL: i64 = 512;
const SET_ORDER: i64 = 2;

thread_local! {
    static CACHE: RefCell<rustc_hash::FxHashMap<Vec<u8>, Rc<Compiled>>> = RefCell::new(Default::default());
    static LAST_ERROR: Cell<i64> = const { Cell::new(0) };
}

struct Compiled {
    re: Regex,
    names: Vec<Option<String>>,
    utf: bool,
    /// A spare capture buffer (taken while in use: callbacks may reenter).
    spare: RefCell<Option<CaptureLocations>>,
}

impl Compiled {
    fn locs(&self) -> CaptureLocations {
        self.spare.borrow_mut().take().unwrap_or_else(|| self.re.capture_locations())
    }

    fn give_back(&self, l: CaptureLocations) {
        *self.spare.borrow_mut() = Some(l);
    }
}

/// Split `/body/flags`, check the delimiters, build the regex. A warning
/// and None when PHP would return false.
fn compile(it: &Interp, fname: &str, pat: &[u8]) -> R<Option<Rc<Compiled>>> {
    if let Some(c) = CACHE.with(|c| c.borrow().get(pat).cloned()) {
        return Ok(Some(c));
    }
    let start = pat.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(pat.len());
    let Some(&open) = pat.get(start) else {
        it.warn(&format!("{fname}(): Empty regular expression"));
        return Ok(None);
    };
    if open.is_ascii_alphanumeric() || open == b'\\' || open == 0 {
        it.warn(&format!("{fname}(): Delimiter must not be alphanumeric, backslash, or NUL byte"));
        return Ok(None);
    }
    let close = match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        c => c,
    };
    // Find the closing delimiter (skipping escapes; brackets nest).
    let mut i = start + 1;
    let mut depth = 1;
    let mut end = None;
    while i < pat.len() {
        let c = pat[i];
        if c == b'\\' {
            i += 2;
            continue;
        }
        if close != open && c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                end = Some(i);
                break;
            }
        }
        i += 1;
    }
    let Some(end) = end else {
        if close == open {
            it.warn(&format!("{fname}(): No ending delimiter '{}' found", close as char));
        } else {
            it.warn(&format!("{fname}(): No ending matching delimiter '{}' found", close as char));
        }
        return Ok(None);
    };
    let body = &pat[start + 1..end];
    let mut b = RegexBuilder::new();
    b.jit_if_available(true);
    let (mut prefix, mut anchored, mut utf, mut extended) = (String::new(), false, false, false);
    for &f in &pat[end + 1..] {
        match f {
            b'i' => {
                b.caseless(true);
            }
            b'm' => {
                b.multi_line(true);
            }
            b's' => {
                b.dotall(true);
            }
            b'x' => {
                b.extended(true);
                extended = true;
            }
            b'u' => {
                b.utf(true).ucp(true);
                utf = true;
            }
            b'U' => prefix.push_str("(?U)"),
            b'n' => prefix.push_str("(?n)"),
            b'J' => prefix.push_str("(?J)"),
            b'A' => anchored = true,
            b'D' | b'S' | b'X' | b'r' | b'\n' | b'\r' | b' ' => {}
            b'e' => {
                it.warn(&format!("{fname}(): Unknown modifier 'e'"));
                return Ok(None);
            }
            other => {
                it.warn(&format!("{fname}(): Unknown modifier '{}'", other as char));
                return Ok(None);
            }
        }
    }
    // Options go after leading verbs like (*UTF8), which must come first.
    let mut verbs = 0;
    while body[verbs..].starts_with(b"(*") {
        match body[verbs..].iter().position(|&c| c == b')') {
            Some(p) => verbs += p + 1,
            None => break,
        }
    }
    utf |= body[..verbs].windows(4).any(|w| w == b"(*UT");
    let mut src = Vec::with_capacity(body.len() + 16);
    src.extend_from_slice(&body[..verbs]);
    src.extend_from_slice(prefix.as_bytes());
    if anchored {
        src.extend_from_slice(b"\\G(?:");
    }
    src.extend_from_slice(&quantifiers(&body[verbs..]));
    if anchored {
        src.extend_from_slice(if extended { b"\n)" } else { b")" });
    }
    let text = crate::pstr::lossy(&src);
    let re = match b.build(&text) {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            let msg = match msg.strip_prefix("PCRE2: error compiling pattern at offset ") {
                Some(rest) => match rest.split_once(": ") {
                    Some((off, m)) => {
                        let shift = prefix.len() + if anchored { 5 } else { 0 };
                        let off = off.parse::<usize>().unwrap_or(0);
                        let off = if off >= verbs + shift { off - shift } else { off };
                        format!("{m} at offset {off}")
                    }
                    None => rest.to_string(),
                },
                None => msg,
            };
            it.warn(&format!("{fname}(): Compilation failed: {msg}"));
            set_error(1);
            return Ok(None);
        }
    };
    let names = re.capture_names().to_vec();
    let c = Rc::new(Compiled { re, names, utf, spare: RefCell::new(None) });
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert(pat.to_vec(), c.clone())
    });
    Ok(Some(c))
}

/// `x{,3}` (PCRE2 10.43+, which PHP bundles) as `x{0,3}`.
fn quantifiers(body: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if !body.windows(2).any(|w| w == b"{,") {
        return std::borrow::Cow::Borrowed(body);
    }
    let mut out = Vec::with_capacity(body.len() + 4);
    let (mut i, mut class) = (0, false);
    while i < body.len() {
        let c = body[i];
        if c == b'\\' && i + 1 < body.len() {
            out.extend_from_slice(&body[i..i + 2]);
            i += 2;
            continue;
        }
        match c {
            b'[' if !class => class = true,
            b']' if class => class = false,
            b'{' if !class && body.get(i + 1) == Some(&b',') => {
                let digits = body[i + 2..].iter().take_while(|b| b.is_ascii_digit()).count();
                if digits > 0 && body.get(i + 2 + digits) == Some(&b'}') {
                    out.extend_from_slice(b"{0");
                    i += 1;
                    continue;
                }
            }
            _ => {}
        }
        out.push(c);
        i += 1;
    }
    std::borrow::Cow::Owned(out)
}

/// A match-time failure as PHP's preg_last_error() code.
fn error_code(e: &pcre2::Error) -> i64 {
    let m = e.to_string().to_ascii_lowercase();
    if m.contains("utf") {
        if m.contains("offset") { 5 } else { 4 }
    } else if m.contains("jit") {
        6
    } else if m.contains("depth") || m.contains("recursion") {
        3
    } else if m.contains("limit") {
        2
    } else {
        1
    }
}

fn set_error(code: i64) {
    LAST_ERROR.with(|e| e.set(code));
}

/// One match at or after `at`: the capture locations, or None. Errors set
/// the last error and return Err(()).
fn find(c: &Compiled, locs: &mut CaptureLocations, subj: &[u8], at: usize) -> Result<bool, ()> {
    // UTF mode: the subject must be valid UTF-8 and the offset on a
    // character boundary (checked once, at the first search).
    if c.utf && at == 0 && std::str::from_utf8(subj).is_err() {
        set_error(4);
        return Err(());
    }
    if c.utf && at > 0 && at < subj.len() && (subj[at] & 0xC0) == 0x80 {
        set_error(5);
        return Err(());
    }
    match c.re.captures_read_at(locs, subj, at) {
        Ok(m) => Ok(m.is_some()),
        Err(e) => {
            set_error(error_code(&e));
            Err(())
        }
    }
}

/// Byte length of the character at `i` (1 outside UTF mode).
fn char_len(c: &Compiled, subj: &[u8], i: usize) -> usize {
    if !c.utf {
        return 1;
    }
    match subj.get(i) {
        Some(b) if *b >= 0xF0 => 4,
        Some(b) if *b >= 0xE0 => 3,
        Some(b) if *b >= 0xC0 => 2,
        _ => 1,
    }
}

/// Every match, Perl /g style: after an empty match the search resumes
/// one character later.
fn each_match(c: &Compiled, subj: &[u8], limit: i64, mut g: impl FnMut(&CaptureLocations) -> R<()>) -> R<Result<usize, ()>> {
    let mut locs = c.locs();
    let mut at = 0;
    let mut n = 0;
    while at <= subj.len() && (limit < 0 || (n as i64) < limit) {
        match find(c, &mut locs, subj, at) {
            Ok(true) => {}
            Ok(false) => break,
            Err(()) => return Ok(Err(())),
        }
        let (s, e) = locs.get(0).unwrap();
        g(&locs)?;
        n += 1;
        at = if e == s { e + char_len(c, subj, e) } else { e };
    }
    c.give_back(locs);
    Ok(Ok(n))
}

fn piece(subj: &[u8], span: Option<(usize, usize)>, offsets: bool, null_unmatched: bool) -> Value {
    let v = match span {
        Some((s, e)) => Value::bytes(subj[s..e].to_vec()),
        None if null_unmatched => Value::Null,
        None => Value::str(""),
    };
    if offsets {
        let off = span.map_or(-1, |(s, _)| s as i64);
        Value::array(PhpArray::from_list([v, Value::Int(off)]))
    } else {
        v
    }
}

/// The match array PHP builds: named groups appear twice; trailing
/// unmatched groups are dropped unless `all` (or unmatched-as-null).
fn groups(c: &Compiled, locs: &CaptureLocations, subj: &[u8], flags: i64, all: bool) -> PhpArray {
    let (offsets, null_unmatched) = (flags & OFFSET_CAPTURE != 0, flags & UNMATCHED_AS_NULL != 0);
    let n = locs.len();
    let last = if all || null_unmatched { n - 1 } else { (0..n).rev().find(|&i| locs.get(i).is_some()).unwrap_or(0) };
    let mut out = PhpArray::new();
    for i in 0..=last {
        let v = piece(subj, locs.get(i), offsets, null_unmatched);
        if let Some(Some(name)) = c.names.get(i) {
            out.insert(Key::Str(crate::value::pbytes(name.as_bytes())), v.clone());
        }
        out.insert(Key::Int(i as i64), v);
    }
    out
}

fn flags_arg(it: &Interp, a: &[Value], i: usize, dflt: i64) -> R<i64> {
    if has(a, i) && !arg(a, i).is_null() {
        int(it, a, i)
    } else {
        Ok(dflt)
    }
}

/// Expand `$1`, `\1`, `${1}` in a replacement.
fn expand(rep: &[u8], locs: &CaptureLocations, subj: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < rep.len() {
        let c = rep[i];
        if (c == b'$' || c == b'\\') && i + 1 < rep.len() {
            let (braced, start) = if c == b'$' && rep[i + 1] == b'{' { (true, i + 2) } else { (false, i + 1) };
            let mut j = start;
            while j < rep.len() && j - start < 2 && rep[j].is_ascii_digit() {
                j += 1;
            }
            if j > start && (!braced || rep.get(j) == Some(&b'}')) {
                let n: usize = std::str::from_utf8(&rep[start..j]).unwrap().parse().unwrap();
                if n < locs.len() {
                    if let Some((s, e)) = locs.get(n) {
                        out.extend_from_slice(&subj[s..e]);
                    }
                }
                i = if braced { j + 1 } else { j };
                continue;
            }
            if c == b'\\' && rep[i + 1] == b'\\' {
                out.push(b'\\');
                i += 2;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
}

enum Rep<'a> {
    Str(Vec<u8>),
    Call(&'a Callable),
}

/// One pattern over one subject. None: error (the caller returns null).
fn replace_one(it: &Interp, c: &Compiled, rep: &Rep, subj: &[u8], limit: i64, flags: i64, count: &mut i64) -> R<Option<Vec<u8>>> {
    let mut out = Vec::with_capacity(subj.len());
    let mut last = 0;
    let r = each_match(c, subj, limit, |locs| {
        let (s, e) = locs.get(0).unwrap();
        out.extend_from_slice(&subj[last..s]);
        match rep {
            Rep::Str(r) => expand(r, locs, subj, &mut out),
            Rep::Call(cb) => {
                let arr = groups(c, locs, subj, flags, false);
                let v = it.call_callable(cb, vec![Value::array(arr)], vec![])?;
                out.extend_from_slice(&it.to_str(&v)?);
            }
        }
        last = e;
        Ok(())
    })?;
    match r {
        Ok(n) => {
            *count += n as i64;
            out.extend_from_slice(&subj[last..]);
            Ok(Some(out))
        }
        Err(()) => Ok(None),
    }
}

/// preg_replace / preg_replace_callback / preg_replace_callback_array.
fn replace_all(it: &Interp, fname: &str, pairs: &[(Value, Rep)], subject: Value, limit: i64, flags: i64, count: &mut i64) -> R<Value> {
    let one = |subj: PStr, count: &mut i64| -> R<Option<Vec<u8>>> {
        let mut cur = subj.to_vec();
        for (p, rep) in pairs {
            let Some(c) = compile(it, fname, &it.to_str(p)?)? else { return Ok(None) };
            match replace_one(it, &c, rep, &cur, limit, flags, count)? {
                Some(r) => cur = r,
                None => return Ok(None),
            }
        }
        Ok(Some(cur))
    };
    set_error(0);
    Ok(match subject {
        Value::Array(subjects) => {
            let mut out = PhpArray::new();
            for (k, v) in subjects.iter() {
                if let Some(r) = one(it.to_str(&v)?, count)? {
                    out.insert(k.clone(), Value::bytes(r));
                }
            }
            Value::array(out)
        }
        v => one(it.to_str(&v)?, count)?.map(Value::bytes).unwrap_or(Value::Null),
    })
}

fn last_error_msg(code: i64) -> &'static str {
    match code {
        0 => "No error",
        1 => "Internal error",
        2 => "Backtrack limit exhausted",
        3 => "Recursion limit exhausted",
        4 => "Malformed UTF-8 characters, possibly incorrectly encoded",
        5 => "The offset did not correspond to the beginning of a valid UTF-8 code point",
        6 => "JIT stack limit exhausted",
        _ => "Unknown error",
    }
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("preg_match", |it, _, a| {
        let Some(c) = compile(it, "preg_match", &s(it, a, 0)?)? else { return Ok(Value::Bool(false)) };
        let subj = s(it, a, 1)?;
        let flags = flags_arg(it, a, 3, 0)?;
        let mut off = flags_arg(it, a, 4, 0)?;
        if off < 0 {
            off += subj.len() as i64;
        }
        set_error(0);
        if off < 0 || off as usize > subj.len() {
            set_error(1);
            if a.len() > 2 {
                a[2] = Value::array(PhpArray::new());
            }
            return Ok(Value::Bool(false));
        }
        let mut locs = c.locs();
        let want = a.len() > 2;
        let (r, arr) = match find(&c, &mut locs, &subj, off as usize) {
            Ok(true) => (Value::Int(1), if want { groups(&c, &locs, &subj, flags, false) } else { PhpArray::new() }),
            Ok(false) => (Value::Int(0), PhpArray::new()),
            Err(()) => (Value::Bool(false), PhpArray::new()),
        };
        c.give_back(locs);
        if a.len() > 2 {
            a[2] = Value::array(arr);
        }
        Ok(r)
    });
    t.insert("preg_match_all", |it, _, a| {
        let Some(c) = compile(it, "preg_match_all", &s(it, a, 0)?)? else { return Ok(Value::Bool(false)) };
        let subj = s(it, a, 1)?;
        let flags = flags_arg(it, a, 3, 1)?;
        let off = flags_arg(it, a, 4, 0)?.max(0) as usize;
        set_error(0);
        let mut sets = vec![];
        let tail = &subj[..];
        let r = {
            let mut locs = c.re.capture_locations();
            let mut at = off.min(tail.len());
            let mut n = 0;
            let mut failed = false;
            while at <= tail.len() {
                match find(&c, &mut locs, tail, at) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(()) => {
                        failed = true;
                        break;
                    }
                }
                let (s0, e0) = locs.get(0).unwrap();
                if flags & SET_ORDER != 0 {
                    sets.push(Value::array(groups(&c, &locs, tail, flags, false)));
                } else {
                    sets.push(Value::array(groups(&c, &locs, tail, flags, true)));
                }
                n += 1;
                at = if e0 == s0 { e0 + char_len(&c, tail, e0) } else { e0 };
            }
            if failed { None } else { Some(n) }
        };
        if a.len() > 2 {
            let result = if flags & SET_ORDER != 0 {
                PhpArray::from_list(sets)
            } else {
                // PREG_PATTERN_ORDER: one list per group.
                let mut out = PhpArray::new();
                for (i, name) in c.names.iter().enumerate() {
                    let col = PhpArray::from_list(sets.iter().map(|s| match s {
                        Value::Array(s) => s.get(&Key::Int(i as i64)).unwrap_or(Value::str("")),
                        _ => Value::str(""),
                    }));
                    if let Some(n) = name {
                        out.insert(Key::Str(crate::value::pbytes(n.as_bytes())), Value::array(col.clone()));
                    }
                    out.insert(Key::Int(i as i64), Value::array(col));
                }
                out
            };
            a[2] = Value::array(result);
        }
        Ok(r.map(|n| Value::Int(n)).unwrap_or(Value::Bool(false)))
    });
    t.insert("preg_replace", |it, _, a| {
        let patterns: Vec<Value> = match arg(a, 0) {
            Value::Array(x) => x.values().collect(),
            v => vec![v],
        };
        let reps: Vec<Vec<u8>> = match arg(a, 1) {
            Value::Array(x) => x.values().map(|v| it.to_str(&v).map(|p| p.to_vec())).collect::<R<_>>()?,
            v => vec![it.to_str(&v)?.to_vec(); patterns.len()],
        };
        let pairs: Vec<(Value, Rep)> = patterns.into_iter().enumerate().map(|(i, p)| (p, Rep::Str(reps.get(i).cloned().unwrap_or_default()))).collect();
        let limit = flags_arg(it, a, 3, -1)?;
        let mut count = 0;
        let r = replace_all(it, "preg_replace", &pairs, arg(a, 2), limit, 0, &mut count)?;
        if a.len() > 4 {
            a[4] = Value::Int(count);
        }
        Ok(r)
    });
    t.insert("preg_replace_callback", |it, f, a| {
        let cb = it.to_callable(f, &arg(a, 1))?;
        let patterns: Vec<Value> = match arg(a, 0) {
            Value::Array(x) => x.values().collect(),
            v => vec![v],
        };
        let pairs: Vec<(Value, Rep)> = patterns.into_iter().map(|p| (p, Rep::Call(&cb))).collect();
        let limit = flags_arg(it, a, 3, -1)?;
        let flags = flags_arg(it, a, 5, 0)?;
        let mut count = 0;
        let r = replace_all(it, "preg_replace_callback", &pairs, arg(a, 2), limit, flags, &mut count)?;
        if a.len() > 4 {
            a[4] = Value::Int(count);
        }
        Ok(r)
    });
    t.insert("preg_replace_callback_array", |it, f, a| {
        let Value::Array(map) = arg(a, 0) else { return Ok(Value::Null) };
        let mut cbs = vec![];
        for (k, v) in map.iter() {
            let p = match k {
                Key::Int(n) => Value::Int(n),
                Key::Str(s) => Value::Str(s.clone()),
            };
            cbs.push((p, it.to_callable(f, &v)?));
        }
        let pairs: Vec<(Value, Rep)> = cbs.iter().map(|(p, c)| (p.clone(), Rep::Call(c))).collect();
        let limit = flags_arg(it, a, 2, -1)?;
        let flags = flags_arg(it, a, 4, 0)?;
        let mut count = 0;
        let r = replace_all(it, "preg_replace_callback_array", &pairs, arg(a, 1), limit, flags, &mut count)?;
        if a.len() > 3 {
            a[3] = Value::Int(count);
        }
        Ok(r)
    });
    t.insert("preg_split", |it, _, a| {
        let Some(c) = compile(it, "preg_split", &s(it, a, 0)?)? else { return Ok(Value::Bool(false)) };
        let subj = s(it, a, 1)?;
        let mut limit = flags_arg(it, a, 2, -1)?;
        if limit == 0 {
            limit = -1;
        }
        let flags = flags_arg(it, a, 3, 0)?;
        let (no_empty, delim, offsets) = (flags & 1 != 0, flags & 2 != 0, flags & 4 != 0);
        set_error(0);
        let mut out = PhpArray::new();
        let add = |out: &mut PhpArray, s: usize, e: usize| {
            let v = Value::bytes(subj[s..e].to_vec());
            out.push(if offsets { Value::array(PhpArray::from_list([v, Value::Int(s as i64)])) } else { v });
        };
        let mut locs = c.re.capture_locations();
        let mut last = 0;
        let mut at = 0;
        while (limit == -1 || limit > 1) && at <= subj.len() {
            match find(&c, &mut locs, &subj, at) {
                Ok(true) => {}
                Ok(false) => break,
                Err(()) => return Ok(Value::Bool(false)),
            }
            let (s0, e0) = locs.get(0).unwrap();
            if !no_empty || s0 != last {
                add(&mut out, last, s0);
                if limit != -1 {
                    limit -= 1;
                }
            }
            if delim {
                for i in 1..locs.len() {
                    if let Some((gs, ge)) = locs.get(i) {
                        if !no_empty || ge > gs {
                            add(&mut out, gs, ge);
                        }
                    }
                }
            }
            last = e0;
            at = if e0 == s0 { e0 + char_len(&c, &subj, e0) } else { e0 };
        }
        if !no_empty || last < subj.len() {
            add(&mut out, last, subj.len());
        }
        Ok(Value::array(out))
    });
    t.insert("preg_quote", |it, _, a| {
        let st = s(it, a, 0)?;
        let delim = if has(a, 1) && !arg(a, 1).is_null() { s(it, a, 1)?.first().copied() } else { None };
        let special = |c: u8| {
            matches!(c, b'.' | b'\\' | b'+' | b'*' | b'?' | b'[' | b'^' | b']' | b'$' | b'(' | b')' | b'{' | b'}' | b'=' | b'!' | b'<' | b'>' | b'|' | b':' | b'-' | b'#' | 0)
                || Some(c) == delim
        };
        let Some(first) = st.iter().position(|&c| special(c)) else { return Ok(Value::Str(st)) };
        let mut out = Vec::with_capacity(st.len() + 8);
        out.extend_from_slice(&st[..first]);
        for &c in &st[first..] {
            if c == 0 {
                out.extend_from_slice(b"\\000");
                continue;
            }
            if special(c) {
                out.push(b'\\');
            }
            out.push(c);
        }
        Ok(Value::bytes(out))
    });
    t.insert("preg_grep", |it, _, a| {
        let Some(c) = compile(it, "preg_grep", &s(it, a, 0)?)? else { return Ok(Value::Bool(false)) };
        let invert = flags_arg(it, a, 2, 0)? & 1 != 0;
        set_error(0);
        let mut out = PhpArray::new();
        let mut locs = c.re.capture_locations();
        if let Value::Array(x) = arg(a, 1) {
            for (k, v) in x.iter() {
                let txt = it.to_str(&v)?;
                match find(&c, &mut locs, &txt, 0) {
                    Ok(m) if m != invert => out.insert(k.clone(), v.clone()),
                    Ok(_) => {}
                    Err(()) => return Ok(Value::Bool(false)),
                }
            }
        }
        Ok(Value::array(out))
    });
    t.insert("preg_last_error", |_, _, _| Ok(Value::Int(LAST_ERROR.with(|e| e.get()))));
    t.insert("preg_last_error_msg", |_, _, _| Ok(Value::str(last_error_msg(LAST_ERROR.with(|e| e.get())))));
}
