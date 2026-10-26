//! ext/intl's grapheme_* functions on extended grapheme clusters. Offsets
//! and results count graphemes (grapheme_extract() takes bytes), and the
//! case-insensitive variants compare case-folded clusters.

use std::collections::HashMap;

use unicode_segmentation::UnicodeSegmentation;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{PhpArray, Value};

fn text(it: &Interp, a: &[Value], i: usize) -> R<Option<String>> {
    Ok(String::from_utf8(s(it, a, i)?.to_vec()).ok())
}

fn clusters(s: &str) -> Vec<&str> {
    s.graphemes(true).collect()
}

fn fold(c: &str) -> String {
    crate::mbcase::convert(c, 3)
}

/// The grapheme index of `needle` in `hay` at or after `from` (or the last
/// one starting at or before `last` when searching backwards).
fn find(hay: &[&str], needle: &[&str], ci: bool, range: std::ops::RangeInclusive<usize>, rev: bool) -> Option<usize> {
    let eq = |a: &str, b: &str| if ci { fold(a) == fold(b) } else { a == b };
    let matches = |i: usize| i + needle.len() <= hay.len() && needle.iter().enumerate().all(|(k, n)| eq(hay[i + k], n));
    if rev {
        range.rev().find(|&i| matches(i))
    } else {
        range.into_iter().find(|&i| matches(i))
    }
}

fn offset_error(it: &Interp, func: &str) -> crate::interp::Ctrl {
    it.error("ValueError", &format!("{func}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"))
}

/// grapheme_strpos & co: Some(position) / None (not found).
fn position(it: &Interp, a: &[Value], func: &str, ci: bool, rev: bool) -> R<Value> {
    let (Some(hay), Some(needle)) = (text(it, a, 0)?, text(it, a, 1)?) else { return Ok(Value::Bool(false)) };
    let offset = if has(a, 2) { int(it, a, 2)? } else { 0 };
    let h = clusters(&hay);
    let n = h.len() as i64;
    if offset > n || offset < -n {
        return Err(offset_error(it, func));
    }
    if needle.is_empty() {
        // ICU reports these in UTF-16 units.
        return Ok(Value::Int(if rev {
            let len16 = hay.encode_utf16().count() as i64;
            if offset < 0 { len16 + offset } else { len16 }
        } else if offset < 0 {
            n + offset
        } else {
            offset
        }));
    }
    let nd = clusters(&needle);
    let found = if rev {
        let (lo, hi) = if offset >= 0 { (offset as usize, h.len()) } else { (0, (n + offset) as usize) };
        if lo > hi { None } else { find(&h, &nd, ci, lo..=hi, true) }
    } else {
        let start = if offset < 0 { (n + offset) as usize } else { offset as usize };
        if start > h.len() { None } else { find(&h, &nd, ci, start..=h.len(), false) }
    };
    Ok(found.map_or(Value::Bool(false), |i| Value::Int(i as i64)))
}

fn strstr(it: &Interp, a: &[Value], ci: bool) -> R<Value> {
    let (Some(hay), Some(needle)) = (text(it, a, 0)?, text(it, a, 1)?) else { return Ok(Value::Bool(false)) };
    let before = arg(a, 2).truthy();
    let h = clusters(&hay);
    let i = if needle.is_empty() { Some(0) } else { find(&h, &clusters(&needle), ci, 0..=h.len(), false) };
    Ok(match i {
        Some(i) => {
            let (head, tail) = (h[..i].concat(), h[i..].concat());
            Value::str(if before { &head } else { &tail })
        }
        None => Value::Bool(false),
    })
}

fn levenshtein(a: &[&str], b: &[&str], ins: i64, rep: i64, del: i64) -> i64 {
    let mut prev: Vec<i64> = (0..=b.len() as i64).map(|j| j * ins).collect();
    for (i, x) in a.iter().enumerate() {
        let mut cur = vec![(i as i64 + 1) * del];
        for (j, y) in b.iter().enumerate() {
            let c = if x == y { prev[j] } else { prev[j] + rep };
            cur.push(c.min(prev[j + 1] + del).min(cur[j] + ins));
        }
        prev = cur;
    }
    prev[b.len()]
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("grapheme_strlen", |it, _, a| Ok(text(it, a, 0)?.map_or(Value::Null, |s| Value::Int(clusters(&s).len() as i64))));
    t.insert("grapheme_substr", |it, _, a| {
        let Some(st) = text(it, a, 0)? else { return Ok(Value::Bool(false)) };
        let h = clusters(&st);
        let n = h.len() as i64;
        let mut start = int(it, a, 1)?;
        if start < 0 {
            start = (n + start).max(0);
        }
        let start = start.min(n);
        let end = match arg(a, 2) {
            Value::Null => n,
            v => {
                let l = it.to_int(&v)?;
                if l < 0 { (n + l).max(start) } else { (start + l).min(n) }
            }
        };
        Ok(Value::str(&h[start as usize..end.max(start) as usize].concat()))
    });
    t.insert("grapheme_strpos", |it, _, a| position(it, a, "grapheme_strpos", false, false));
    t.insert("grapheme_stripos", |it, _, a| position(it, a, "grapheme_stripos", true, false));
    t.insert("grapheme_strrpos", |it, _, a| position(it, a, "grapheme_strrpos", false, true));
    t.insert("grapheme_strripos", |it, _, a| position(it, a, "grapheme_strripos", true, true));
    t.insert("grapheme_strstr", |it, _, a| strstr(it, a, false));
    t.insert("grapheme_stristr", |it, _, a| strstr(it, a, true));
    t.insert("grapheme_str_split", |it, _, a| {
        let len = if has(a, 1) { int(it, a, 1)? } else { 1 };
        if !(1..=1073741823).contains(&len) {
            return Err(it.error("ValueError", "grapheme_str_split(): Argument #2 ($length) must be greater than 0 and less than or equal to 1073741823"));
        }
        let Some(st) = text(it, a, 0)? else { return Ok(Value::Bool(false)) };
        let parts: Vec<Value> = clusters(&st).chunks(len as usize).map(|c| Value::str(&c.concat())).collect();
        Ok(Value::array(PhpArray::from_list(parts)))
    });
    t.insert("grapheme_levenshtein", |it, _, a| {
        let (Some(x), Some(y)) = (text(it, a, 0)?, text(it, a, 1)?) else { return Ok(Value::Bool(false)) };
        let cost = |i: usize| -> R<i64> { if has(a, i) { int(it, a, i) } else { Ok(1) } };
        Ok(Value::Int(levenshtein(&clusters(&x), &clusters(&y), cost(2)?, cost(3)?, cost(4)?)))
    });
    t.insert("grapheme_extract", |it, _, a| {
        let bytes = s(it, a, 0)?.to_vec();
        let size = int(it, a, 1)?;
        let ty = if has(a, 2) { int(it, a, 2)? } else { 0 };
        let mut offset = if has(a, 3) { int(it, a, 3)? } else { 0 };
        if !(0..=2).contains(&ty) {
            return Err(it.error("ValueError", "grapheme_extract(): Argument #3 ($type) must be one of GRAPHEME_EXTR_COUNT, GRAPHEME_EXTR_MAXBYTES, or GRAPHEME_EXTR_MAXCHARS"));
        }
        if offset < 0 {
            offset += bytes.len() as i64;
        }
        let set_next = |a: &mut [Value], n: i64| {
            if a.len() > 4 {
                a[4] = Value::Int(n);
            }
        };
        if offset < 0 || offset as usize > bytes.len() {
            set_next(a, offset.max(0));
            return Ok(Value::Bool(false));
        }
        // An offset inside a character starts at the next one.
        while (offset as usize) < bytes.len() && bytes[offset as usize] & 0xC0 == 0x80 {
            offset += 1;
        }
        let rest = String::from_utf8_lossy(&bytes[offset as usize..]).into_owned();
        let mut out = String::new();
        let (mut count, mut chars) = (0i64, 0i64);
        for g in rest.graphemes(true) {
            let fits = match ty {
                0 => count < size,
                1 => (out.len() + g.len()) as i64 <= size,
                _ => chars + g.chars().count() as i64 <= size,
            };
            if !fits {
                break;
            }
            out.push_str(g);
            count += 1;
            chars += g.chars().count() as i64;
        }
        set_next(a, offset + out.len() as i64);
        Ok(Value::str(&out))
    });
}
