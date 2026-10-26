//! DateInterval: ISO-8601 durations, add/sub, diff() and format().

use std::rc::Rc;

use super::civil::Civil;
use super::zone::Zone;
use super::{parse, DateState};
use crate::builtins::arg;
use crate::interp::{Interp, R};
use crate::value::{pstr, Key, PhpArray, Value};

#[derive(Default, Debug, Clone)]
pub struct Iv {
    pub y: i64,
    pub m: i64,
    pub d: i64,
    pub h: i64,
    pub i: i64,
    pub s: i64,
    pub us: i64,
    pub invert: bool,
}

/// The fields of a DateInterval object.
pub fn read(it: &Interp, v: &Value) -> R<Iv> {
    let Value::Object(o) = v else {
        return Err(it.error("TypeError", "Argument #1 ($interval) must be of type DateInterval"));
    };
    let num = |k: &str| -> R<i64> {
        match o.get_prop(k) {
            Some(v) => it.to_int(&v),
            None => Ok(0),
        }
    };
    let f = match o.get_prop("f") {
        Some(v) => it.to_float(&v)?,
        None => 0.0,
    };
    Ok(Iv { y: num("y")?, m: num("m")?, d: num("d")?, h: num("h")?, i: num("i")?, s: num("s")?, us: (f * 1e6).round() as i64, invert: num("invert")? != 0 })
}

/// Add `iv` (times `sign`) to `d`: the date part on the wall clock, the
/// time part as elapsed time.
pub fn add(d: &mut DateState, iv: &Iv, sign: i64) {
    let k = if iv.invert { -sign } else { sign };
    if iv.y != 0 || iv.m != 0 || iv.d != 0 {
        let c = d.local();
        d.set_local(Civil { y: c.y + k * iv.y, m: c.m + k * iv.m, d: c.d + k * iv.d, ..c });
    }
    let total = d.us + k * (iv.us + 1_000_000 * (iv.s + 60 * iv.i + 3600 * iv.h));
    d.ts += total.div_euclid(1_000_000);
    d.us = total.rem_euclid(1_000_000);
}

fn new_interval(it: &Interp, f: &Iv, days: Option<i64>) -> R<Value> {
    let cls = it.lookup_class("DateInterval")?;
    let o = it.new_instance(&cls)?;
    for (k, v) in [("y", f.y), ("m", f.m), ("d", f.d), ("h", f.h), ("i", f.i), ("s", f.s)] {
        o.set_prop(k, Value::Int(v));
    }
    o.set_prop("f", Value::Float(f.us as f64 / 1e6));
    o.set_prop("invert", Value::Int(f.invert as i64));
    o.set_prop("days", days.map(Value::Int).unwrap_or(Value::Bool(false)));
    o.set_prop("from_string", Value::Bool(false));
    Ok(Value::Object(o))
}

/// `$a->diff($b)`.
pub fn diff(it: &Interp, a: &DateState, b: &DateState, absolute: bool) -> R<Value> {
    let invert = (b.ts, b.us) < (a.ts, a.us);
    let (one, two) = if invert { (b, a) } else { (a, b) };
    // Same named zone: compare wall clocks; otherwise both in the first's.
    let same = matches!((&one.zone, &two.zone), (Zone::Id(x), Zone::Id(y)) if x == y);
    let (o1, o2) = if same { (one.zone.offset(one.ts), two.zone.offset(two.ts)) } else { (one.zone.offset(one.ts), one.zone.offset(one.ts)) };
    let l1 = one.ts + o1 as i64;
    let l2 = two.ts + o2 as i64;
    let c1 = Civil::from_local(l1, one.us);
    let c2 = Civil::from_local(l2, two.us);
    // Whole months first (the most that still fit), then the rest as days
    // and time. PHP walks from the base object: forward when it's the
    // earlier date, backward (from the later one) when inverted.
    let mut months = (c2.y - c1.y) * 12 + (c2.m - c1.m);
    let (rest_from, rest_to) = if !invert {
        let at = |k: i64| Civil { m: c1.m + k, ..c1 }.to_local();
        while months > 0 && at(months) > (l2, two.us) {
            months -= 1;
        }
        (at(months), (l2, two.us))
    } else {
        let at = |k: i64| Civil { m: c2.m - k, ..c2 }.to_local();
        while months > 0 && at(months) < (l1, one.us) {
            months -= 1;
        }
        ((l1, one.us), at(months))
    };
    let rest = (rest_to.0 - rest_from.0) * 1_000_000 + (rest_to.1 - rest_from.1);
    let secs = rest.div_euclid(1_000_000);
    let f = Iv {
        y: months / 12,
        m: months % 12,
        d: secs / 86400,
        h: secs / 3600 % 24,
        i: secs / 60 % 60,
        s: secs % 60,
        us: rest.rem_euclid(1_000_000),
        invert: invert && !absolute,
    };
    let total_us = (l2 - l1) * 1_000_000 + (two.us - one.us);
    let days = total_us.div_euclid(86_400_000_000);
    new_interval(it, &f, Some(days))
}

fn bad_format(it: &Interp, s: &str) -> crate::interp::Ctrl {
    it.error("DateMalformedIntervalStringException", &format!("Unknown or bad format ({s})"))
}

/// `P1Y2M3DT4H5M6S`, `P2W` -> [y, m, d, h, i, s, f].
pub fn parse_fn(it: &Interp, a: &[Value]) -> R<Value> {
    let text = crate::pstr::lossy(&it.to_str(&arg(a, 0))?).into_owned();
    let b = text.as_bytes();
    if b.first() != Some(&b'P') || b.len() < 3 && !(b.len() == 2) {
        return Err(bad_format(it, &text));
    }
    let mut f = [0i64; 6];
    let mut time = false;
    let mut any = false;
    let mut i = 1;
    while i < b.len() {
        if b[i] == b'T' {
            if time {
                return Err(bad_format(it, &text));
            }
            time = true;
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || i >= b.len() {
            return Err(bad_format(it, &text));
        }
        let n: i64 = text[start..i].parse().map_err(|_| bad_format(it, &text))?;
        match (b[i], time) {
            (b'Y', false) => f[0] += n,
            (b'M', false) => f[1] += n,
            (b'W', false) => f[2] += n * 7,
            (b'D', false) => f[2] += n,
            (b'H', true) => f[3] += n,
            (b'M', true) => f[4] += n,
            (b'S', true) => f[5] += n,
            _ => return Err(bad_format(it, &text)),
        }
        any = true;
        i += 1;
    }
    if !any {
        return Err(bad_format(it, &text));
    }
    let mut out: Vec<Value> = f.iter().map(|&n| Value::Int(n)).collect();
    out.push(Value::Float(0.0));
    Ok(Value::array(PhpArray::from_list(out)))
}

/// DateInterval::createFromDateString(): the relative part of a date
/// string, or false.
pub fn from_string_fn(it: &Interp, a: &[Value]) -> R<Value> {
    let text = crate::pstr::lossy(&it.to_str(&arg(a, 0))?).into_owned();
    match parse::parse(&text) {
        Ok(p) => {
            let r = &p.rel;
            let vals = [r.y, r.m, r.d, r.h, r.i, r.s];
            let mut out: Vec<Value> = vals.iter().map(|&n| Value::Int(n)).collect();
            out.push(Value::Float(r.us as f64 / 1e6));
            Ok(Value::array(PhpArray::from_list(out)))
        }
        Err(e) => {
            it.warn(&format!(
                "DateInterval::createFromDateString(): Unknown or bad format ({text}) at position {} ({}): {}",
                e.pos, e.ch, e.msg
            ));
            Ok(Value::Bool(false))
        }
    }
}

/// DateInterval::format($props, $format).
pub fn format_fn(it: &Interp, a: &[Value]) -> R<Value> {
    let Value::Array(p) = arg(a, 0) else { return Ok(Value::str("")) };
    let get = |k: &str| p.get(&Key::Str(pstr(k))).unwrap_or_default();
    let n = |k: &str| -> R<i64> { it.to_int(&get(k)) };
    let fmt = it.to_str(&arg(a, 1))?;
    let mut out = String::new();
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        if c != b'%' || i + 1 >= fmt.len() {
            out.push(c as char);
            i += 1;
            continue;
        }
        let k = fmt[i + 1];
        i += 2;
        let us = (it.to_float(&get("f"))? * 1e6).round() as i64;
        match k {
            b'Y' => out.push_str(&format!("{:02}", n("y")?)),
            b'y' => out.push_str(&n("y")?.to_string()),
            b'M' => out.push_str(&format!("{:02}", n("m")?)),
            b'm' => out.push_str(&n("m")?.to_string()),
            b'D' => out.push_str(&format!("{:02}", n("d")?)),
            b'd' => out.push_str(&n("d")?.to_string()),
            b'H' => out.push_str(&format!("{:02}", n("h")?)),
            b'h' => out.push_str(&n("h")?.to_string()),
            b'I' => out.push_str(&format!("{:02}", n("i")?)),
            b'i' => out.push_str(&n("i")?.to_string()),
            b'S' => out.push_str(&format!("{:02}", n("s")?)),
            b's' => out.push_str(&n("s")?.to_string()),
            b'F' => out.push_str(&format!("{us:06}")),
            b'f' => out.push_str(&us.to_string()),
            b'a' => match get("days") {
                Value::Bool(false) | Value::Null => out.push_str("(unknown)"),
                v => out.push_str(&it.to_int(&v)?.to_string()),
            },
            b'R' => out.push(if n("invert")? != 0 { '-' } else { '+' }),
            b'r' => {
                if n("invert")? != 0 {
                    out.push('-');
                }
            }
            b'%' => out.push('%'),
            other => {
                out.push('%');
                out.push(other as char);
            }
        }
    }
    Ok(Value::str(&out))
}

#[allow(dead_code)]
fn _unused(_: Rc<()>) {}
