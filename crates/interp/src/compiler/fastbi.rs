//! Fast entry points for common built-ins, called by compiled code instead
//! of the generic built-in path (call-site lookup, signature check, panic
//! guard, argument copies). PHP does the same by compiling these calls to
//! dedicated opcodes.
//!
//! Each takes the evaluated arguments in place and handles only the common
//! argument types. It returns 0 after writing the result to `dst` and
//! consuming the arguments, or 2 without touching anything, in which case
//! the generic path runs (and raises PHP's errors and deprecations). These
//! functions run outside the built-in panic guard: they must not panic.

use std::ptr;

use crate::strings::{find, substr_range};
use crate::value::{compare, identical, loose_eq, pbytes, KeyRef, PStr, PhpArray, Value};

/// PHP built-in name -> fast entry helper name (registered in `HELPERS`).
pub fn lookup(name: &str, nargs: usize) -> Option<&'static str> {
    Some(match (name, nargs) {
        ("ord", 1) => "rt_fast_ord",
        ("chr", 1) => "rt_fast_chr",
        ("array_key_exists" | "key_exists", 2) => "rt_fast_key_exists",
        ("in_array", 2 | 3) => "rt_fast_in_array",
        ("str_contains", 2) => "rt_fast_str_contains",
        ("str_starts_with", 2) => "rt_fast_str_starts_with",
        ("str_ends_with", 2) => "rt_fast_str_ends_with",
        ("strpos", 2) => "rt_fast_strpos",
        ("substr", 2 | 3) => "rt_fast_substr",
        ("strtolower", 1) => "rt_fast_strtolower",
        ("strtoupper", 1) => "rt_fast_strtoupper",
        ("trim", 1 | 2) => "rt_fast_trim",
        ("ltrim", 1 | 2) => "rt_fast_ltrim",
        ("rtrim", 1 | 2) => "rt_fast_rtrim",
        ("str_replace", 3) => "rt_fast_str_replace",
        ("implode" | "join", 2) => "rt_fast_implode",
        ("abs", 1) => "rt_fast_abs",
        ("min", 2) => "rt_fast_min",
        ("max", 2) => "rt_fast_max",
        ("floor", 1) => "rt_fast_floor",
        ("ceil", 1) => "rt_fast_ceil",
        ("intval", 1) => "rt_fast_intval",
        ("explode", 2) => "rt_fast_explode",
        ("array_keys", 1) => "rt_fast_array_keys",
        ("array_values", 1) => "rt_fast_array_values",
        ("array_key_first", 1) => "rt_fast_array_key_first",
        ("array_key_last", 1) => "rt_fast_array_key_last",
        ("ucfirst", 1) => "rt_fast_ucfirst",
        ("lcfirst", 1) => "rt_fast_lcfirst",
        ("str_repeat", 2) => "rt_fast_str_repeat",
        ("get_class", 1) => "rt_fast_get_class",
        _ => return None,
    })
}

#[inline]
unsafe fn arg<'a>(args: *mut Value, i: usize) -> &'a Value {
    &*args.add(i)
}

/// Success: drop the arguments (leaving null slots) and store the result.
#[inline]
unsafe fn done(args: *mut Value, n: u64, dst: *mut Value, v: Value) -> u64 {
    for i in 0..n as usize {
        let p = args.add(i);
        if !matches!(*p, Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_)) {
            drop(ptr::replace(p, Value::Null));
        }
    }
    ptr::write(dst, v);
    0
}

const DECLINE: u64 = 2;

pub unsafe extern "C" fn rt_fast_ord(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    match arg(args, 0) {
        // Other lengths are deprecated in PHP 8.5: the generic path warns.
        Value::Str(s) if s.len() == 1 => {
            let c = s[0];
            done(args, n, dst, Value::Int(c as i64))
        }
        _ => DECLINE,
    }
}

thread_local! {
    /// The 256 one-byte strings (chr() returns shared ones, like PHP's
    /// interned single-character strings).
    static ONE_BYTE: Vec<PStr> = (0..=255u8).map(|b| pbytes(&[b])).collect();
}

pub unsafe extern "C" fn rt_fast_chr(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    match arg(args, 0) {
        Value::Int(i) => {
            let b = i.rem_euclid(256) as usize;
            let s = ONE_BYTE.with(|t| t[b].clone());
            done(args, n, dst, Value::Str(s))
        }
        _ => DECLINE,
    }
}

pub unsafe extern "C" fn rt_fast_key_exists(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Value::Array(a) = arg(args, 1) else { return DECLINE };
    let found = match arg(args, 0) {
        Value::Int(i) => a.contains_ref(KeyRef::Int(*i)),
        Value::Str(s) => a.contains_ref(KeyRef::from_bytes(s)),
        _ => return DECLINE,
    };
    done(args, n, dst, Value::Bool(found))
}

pub unsafe extern "C" fn rt_fast_in_array(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Value::Array(a) = arg(args, 1) else { return DECLINE };
    let strict = match n {
        2 => false,
        _ => match arg(args, 2) {
            Value::Bool(b) => *b,
            _ => return DECLINE,
        },
    };
    let needle = arg(args, 0);
    if matches!(needle, Value::Ref(_) | Value::Object(_)) {
        return DECLINE;
    }
    // Borrow the elements (no clones); ints and equal strings compare
    // directly, everything else with PHP's rules.
    let found = a.raw_values().any(|v| match v {
        Value::Ref(r) => {
            let v = r.borrow();
            if strict { identical(&v, needle) } else { loose_eq(&v, needle) }
        }
        v => match (v, needle) {
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Str(x), Value::Str(y)) if x.as_slice() == y.as_slice() => true,
            _ if strict => identical(v, needle),
            _ => loose_eq(v, needle),
        },
    });
    done(args, n, dst, Value::Bool(found))
}

pub unsafe extern "C" fn rt_fast_get_class(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Value::Object(o) = arg(args, 0) else { return DECLINE };
    let name = o.class.name_str.get_or_init(|| PStr::from_bytes(o.class.php_name().as_bytes())).clone();
    done(args, n, dst, Value::Str(name))
}

/// Both arguments strings.
#[inline]
unsafe fn two_strs<'a>(args: *mut Value) -> Option<(&'a [u8], &'a [u8])> {
    match (arg(args, 0), arg(args, 1)) {
        (Value::Str(h), Value::Str(n)) => Some((h, n)),
        _ => None,
    }
}

pub unsafe extern "C" fn rt_fast_str_contains(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Some((h, nd)) = two_strs(args) else { return DECLINE };
    let r = find(h, nd, 0).is_some();
    done(args, n, dst, Value::Bool(r))
}

pub unsafe extern "C" fn rt_fast_str_starts_with(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Some((h, nd)) = two_strs(args) else { return DECLINE };
    let r = h.starts_with(nd);
    done(args, n, dst, Value::Bool(r))
}

pub unsafe extern "C" fn rt_fast_str_ends_with(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Some((h, nd)) = two_strs(args) else { return DECLINE };
    let r = h.ends_with(nd);
    done(args, n, dst, Value::Bool(r))
}

pub unsafe extern "C" fn rt_fast_strpos(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Some((h, nd)) = two_strs(args) else { return DECLINE };
    let r = match find(h, nd, 0) {
        Some(p) => Value::Int(p as i64),
        None => Value::Bool(false),
    };
    done(args, n, dst, r)
}

pub unsafe extern "C" fn rt_fast_substr(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Value::Str(s) = arg(args, 0) else { return DECLINE };
    let Value::Int(start) = arg(args, 1) else { return DECLINE };
    let len = match n {
        2 => None,
        _ => match arg(args, 2) {
            Value::Int(l) => Some(*l),
            Value::Null => None,
            _ => return DECLINE,
        },
    };
    let (lo, hi) = substr_range(s.len(), *start, len);
    let r = if lo == 0 && hi == s.len() {
        // The whole string: share it.
        Value::Str(s.clone())
    } else if hi - lo == 1 {
        let b = s[lo] as usize;
        Value::Str(ONE_BYTE.with(|t| t[b].clone()))
    } else {
        Value::Str(pbytes(&s[lo..hi]))
    };
    done(args, n, dst, r)
}

/// ASCII case mapping (PHP 8: locale-independent); an unchanged string is
/// shared instead of copied.
unsafe fn map_case(args: *mut Value, n: u64, dst: *mut Value, upper: bool) -> u64 {
    let Value::Str(s) = arg(args, 0) else { return DECLINE };
    let needs = if upper { s.iter().any(u8::is_ascii_lowercase) } else { s.iter().any(u8::is_ascii_uppercase) };
    let r = if !needs {
        s.clone()
    } else if upper {
        pbytes(&s.to_ascii_uppercase())
    } else {
        pbytes(&s.to_ascii_lowercase())
    };
    done(args, n, dst, Value::Str(r))
}

pub unsafe extern "C" fn rt_fast_strtolower(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    map_case(args, n, dst, false)
}

pub unsafe extern "C" fn rt_fast_strtoupper(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    map_case(args, n, dst, true)
}

/// trim()/ltrim()/rtrim() with the default characters or a plain list
/// (a list with a `..` range takes the generic path).
#[inline]
unsafe fn trim_sides(args: *mut Value, n: u64, dst: *mut Value, left: bool, right: bool) -> u64 {
    let Value::Str(s) = arg(args, 0) else { return DECLINE };
    let list: &[u8] = match n {
        1 => b" \t\n\r\0\x0B",
        _ => match arg(args, 1) {
            Value::Str(l) if !l.windows(2).any(|w| w == b"..") => l,
            _ => return DECLINE,
        },
    };
    let strip = |c: &u8| list.contains(c);
    let lo = if left { s.iter().position(|c| !strip(c)).unwrap_or(s.len()) } else { 0 };
    let hi = if right { s.iter().rposition(|c| !strip(c)).map_or(lo, |p| (p + 1).max(lo)) } else { s.len() };
    let r = if lo == 0 && hi == s.len() { s.clone() } else { pbytes(&s[lo..hi]) };
    done(args, n, dst, Value::Str(r))
}

pub unsafe extern "C" fn rt_fast_trim(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    trim_sides(args, n, dst, true, true)
}

pub unsafe extern "C" fn rt_fast_ltrim(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    trim_sides(args, n, dst, true, false)
}

pub unsafe extern "C" fn rt_fast_rtrim(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    trim_sides(args, n, dst, false, true)
}

pub unsafe extern "C" fn rt_fast_str_replace(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let (Value::Str(from), Value::Str(to), Value::Str(subject)) = (arg(args, 0), arg(args, 1), arg(args, 2)) else { return DECLINE };
    if from.is_empty() {
        return DECLINE;
    }
    let Some(first) = find(subject, from, 0) else {
        // Nothing to replace: the subject itself.
        let r = Value::Str(subject.clone());
        return done(args, n, dst, r);
    };
    let mut out = Vec::with_capacity(subject.len());
    let mut at = 0;
    let mut hit = Some(first);
    while let Some(p) = hit {
        out.extend_from_slice(&subject[at..p]);
        out.extend_from_slice(to);
        at = p + from.len();
        hit = find(subject, from, at);
    }
    out.extend_from_slice(&subject[at..]);
    done(args, n, dst, Value::Str(pbytes(&out)))
}

pub unsafe extern "C" fn rt_fast_implode(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let (Value::Str(sep), Value::Array(a)) = (arg(args, 0), arg(args, 1)) else { return DECLINE };
    let mut out = Vec::new();
    let mut digits = [0u8; 20];
    for (i, v) in a.raw_values().enumerate() {
        if i > 0 {
            out.extend_from_slice(sep);
        }
        match v {
            Value::Str(s) => out.extend_from_slice(s),
            Value::Int(k) => out.extend_from_slice(crate::pstr::fmt_int(*k, &mut digits)),
            // Floats, bools, null, arrays, objects: PHP's conversions.
            _ => return DECLINE,
        }
    }
    done(args, n, dst, Value::Str(pbytes(&out)))
}

pub unsafe extern "C" fn rt_fast_abs(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let r = match arg(args, 0) {
        Value::Int(i) => match i.checked_abs() {
            Some(a) => Value::Int(a),
            None => Value::Float(-(*i as f64)),
        },
        Value::Float(f) => Value::Float(f.abs()),
        _ => return DECLINE,
    };
    done(args, n, dst, r)
}

/// Direct two-argument min()/max() of numbers, with PHP 8.4+'s
/// "frameless" rules: min is `a < b ? a : b`, max is `a < b ? b : a` (so
/// on a tie min returns the second argument, max the first).
unsafe fn pick(args: *mut Value, n: u64, dst: *mut Value, max: bool) -> u64 {
    let (a, b) = (arg(args, 0), arg(args, 1));
    if matches!(a, Value::Ref(_) | Value::Object(_)) || matches!(b, Value::Ref(_) | Value::Object(_)) {
        return DECLINE;
    }
    let r = frameless_pick(a, b, max);
    done(args, n, dst, r)
}

/// PHP 8.4+'s frameless two-argument min/max (uncomparable values count
/// as "not less").
pub fn frameless_pick(a: &Value, b: &Value, max: bool) -> Value {
    let less = compare(a, b) == Some(std::cmp::Ordering::Less);
    if less != max { a.clone() } else { b.clone() }
}

pub unsafe extern "C" fn rt_fast_min(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    pick(args, n, dst, false)
}

pub unsafe extern "C" fn rt_fast_max(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    pick(args, n, dst, true)
}

pub unsafe extern "C" fn rt_fast_floor(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let r = match arg(args, 0) {
        Value::Int(i) => Value::Float(*i as f64),
        Value::Float(f) => Value::Float(f.floor()),
        _ => return DECLINE,
    };
    done(args, n, dst, r)
}

pub unsafe extern "C" fn rt_fast_ceil(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let r = match arg(args, 0) {
        Value::Int(i) => Value::Float(*i as f64),
        Value::Float(f) => Value::Float(f.ceil()),
        _ => return DECLINE,
    };
    done(args, n, dst, r)
}

pub unsafe extern "C" fn rt_fast_intval(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let r = match arg(args, 0) {
        Value::Int(i) => *i,
        Value::Bool(b) => *b as i64,
        Value::Null => 0,
        Value::Float(f) => crate::ops::float_to_int(*f),
        _ => return DECLINE,
    };
    done(args, n, dst, Value::Int(r))
}

pub unsafe extern "C" fn rt_fast_explode(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Some((sep, s)) = two_strs(args) else { return DECLINE };
    // An empty separator is a ValueError: the generic path raises it.
    if sep.is_empty() {
        return DECLINE;
    }
    let mut out = PhpArray::new();
    let mut at = 0;
    while let Some(p) = find(s, sep, at) {
        out.push(Value::Str(pbytes(&s[at..p])));
        at = p + sep.len();
    }
    out.push(Value::Str(pbytes(&s[at..])));
    done(args, n, dst, Value::array(out))
}

pub unsafe extern "C" fn rt_fast_array_keys(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Value::Array(a) = arg(args, 0) else { return DECLINE };
    let out = PhpArray::from_list(a.iter_raw().map(|(k, _)| k.to_value()).collect::<Vec<_>>());
    done(args, n, dst, Value::array(out))
}

pub unsafe extern "C" fn rt_fast_array_values(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Value::Array(a) = arg(args, 0) else { return DECLINE };
    let out = PhpArray::from_list(a.values().collect::<Vec<_>>());
    done(args, n, dst, Value::array(out))
}

pub unsafe extern "C" fn rt_fast_array_key_first(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Value::Array(a) = arg(args, 0) else { return DECLINE };
    let r = a.iter_raw().next().map(|(k, _)| k.to_value()).unwrap_or(Value::Null);
    done(args, n, dst, r)
}

pub unsafe extern "C" fn rt_fast_array_key_last(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let Value::Array(a) = arg(args, 0) else { return DECLINE };
    let r = a.iter_raw().next_back().map(|(k, _)| k.to_value()).unwrap_or(Value::Null);
    done(args, n, dst, r)
}

unsafe fn first_case(args: *mut Value, n: u64, dst: *mut Value, upper: bool) -> u64 {
    let Value::Str(s) = arg(args, 0) else { return DECLINE };
    let r = match s.first() {
        Some(c) if upper && c.is_ascii_lowercase() || !upper && c.is_ascii_uppercase() => {
            let mut b = s.to_vec();
            b[0] = if upper { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
            pbytes(&b)
        }
        _ => s.clone(),
    };
    done(args, n, dst, Value::Str(r))
}

pub unsafe extern "C" fn rt_fast_ucfirst(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    first_case(args, n, dst, true)
}

pub unsafe extern "C" fn rt_fast_lcfirst(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    first_case(args, n, dst, false)
}

pub unsafe extern "C" fn rt_fast_str_repeat(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let (Value::Str(s), Value::Int(times)) = (arg(args, 0), arg(args, 1)) else { return DECLINE };
    // Negative counts are a ValueError, big results need the memory_limit
    // pre-check: the generic path does both.
    let Some(total) = (*times as usize).checked_mul(s.len()) else { return DECLINE };
    if *times < 0 || total > 1 << 20 {
        return DECLINE;
    }
    let r = Value::Str(pbytes(&s.repeat(*times as usize)));
    done(args, n, dst, r)
}
