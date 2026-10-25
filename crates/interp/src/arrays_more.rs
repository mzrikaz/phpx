//! The rest of ext/standard's array functions: key case, the assoc and
//! user-callback variants of diff/intersect, and array_multisort().

use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::{arr, callable, int};
use crate::interp::{BuiltinFn, Frame, Interp, R};
use crate::value::{Key, PhpArray, Value};

fn change_key_case(it: &Interp, a: &mut [Value]) -> R<Value> {
    let x = arr(it, a, 0, "array_change_key_case")?;
    let upper = a.len() > 1 && int(it, a, 1)? != 0;
    let mut out = PhpArray::new();
    for (k, v) in x.iter() {
        let k = match k {
            Key::Str(s) => Key::from_bytes(&if upper { s.to_ascii_uppercase() } else { s.to_ascii_lowercase() }),
            k => k,
        };
        out.insert(k, v);
    }
    Ok(Value::array(out))
}

/// How diff/intersect compare values and keys: built-in (string equality,
/// key identity) or a user callback returning 0 for "equal".
enum Cmp<'a> {
    Builtin,
    User(crate::callback::Callback<'a>),
}

impl Cmp<'_> {
    fn values(&mut self, it: &Interp, x: &Value, y: &Value) -> R<bool> {
        match self {
            Cmp::Builtin => Ok(it.to_str(x)? == it.to_str(y)?),
            Cmp::User(c) => Ok(it.to_int(&c.call(&mut [x.clone(), y.clone()])?)? == 0),
        }
    }

    fn keys(&mut self, it: &Interp, x: &Key, y: &Key) -> R<bool> {
        match self {
            Cmp::Builtin => Ok(x == y),
            Cmp::User(c) => Ok(it.to_int(&c.call(&mut [x.to_value(), y.to_value()])?)? == 0),
        }
    }
}

/// diff/intersect over (key, value) pairs. `vcb`/`kcb`: whether the last
/// arguments are value / key callbacks (value callback first).
fn assoc_op(it: &Interp, f: &Frame, a: &[Value], name: &str, intersect: bool, vcb: bool, kcb: bool) -> R<Value> {
    let ncb = vcb as usize + kcb as usize;
    if a.len() < ncb + 1 {
        return Err(it.error("ArgumentCountError", &format!("{name}() expects at least {} arguments, {} given", ncb + 1, a.len())));
    }
    let narr = a.len() - ncb;
    let mut vc = if vcb { Cmp::User(it.callback(callable(it, f, a, narr)?)) } else { Cmp::Builtin };
    let mut kc = if kcb { Cmp::User(it.callback(callable(it, f, a, a.len() - 1)?)) } else { Cmp::Builtin };
    let x = arr(it, a, 0, name)?;
    let others: Vec<Rc<PhpArray>> = (1..narr).map(|i| arr(it, a, i, name)).collect::<R<_>>()?;
    let mut out = PhpArray::new();
    for (k, v) in x.iter() {
        let mut in_all = true;
        let mut in_any = false;
        for o in &others {
            let mut found = false;
            if !kcb {
                if let Some(w) = o.get(&k) {
                    found = vc.values(it, &v, &w)?;
                }
            } else {
                for (ok, ov) in o.iter() {
                    if kc.keys(it, &k, &ok)? && vc.values(it, &v, &ov)? {
                        found = true;
                        break;
                    }
                }
            }
            in_all &= found;
            in_any |= found;
        }
        if if intersect { in_all } else { !in_any } {
            out.insert(k, v);
        }
    }
    Ok(Value::array(out))
}

const SORT_ASC: i64 = 4;
const SORT_DESC: i64 = 3;

/// array_multisort(&$array, $order, $flags, &...$rest).
fn multisort(it: &Interp, a: &mut [Value]) -> R<Value> {
    // Group the arguments: each array, then its optional order and flags.
    let mut groups: Vec<(usize, i64, i64)> = vec![];
    let (mut order_set, mut flags_set) = (false, false);
    for i in 0..a.len() {
        match a[i].deref() {
            Value::Array(_) => {
                groups.push((i, SORT_ASC, 0));
                order_set = false;
                flags_set = false;
            }
            Value::Int(n) => {
                let Some(g) = groups.last_mut() else {
                    return Err(it.error("TypeError", &format!("array_multisort(): Argument #{} ($array) must be an array or a sort flag", i + 1)));
                };
                if (n == SORT_ASC || n == SORT_DESC) && !order_set {
                    g.1 = n;
                    order_set = true;
                } else if n & !8 <= 6 && !flags_set {
                    g.2 = n;
                    flags_set = true;
                } else {
                    return Err(it.error("ValueError", &format!("array_multisort(): Argument #{} must be a valid sort flag", i + 1)));
                }
            }
            _ => {
                return Err(it.error("TypeError", &format!("array_multisort(): Argument #{} must be an array or a sort flag", i + 1)));
            }
        }
    }
    let arrays: Vec<Vec<(Key, Value)>> = groups
        .iter()
        .map(|g| match a[g.0].deref() {
            Value::Array(x) => (*x).clone().into_raw_pairs(),
            _ => vec![],
        })
        .collect();
    let n = arrays.first().map_or(0, Vec::len);
    if arrays.iter().any(|x| x.len() != n) {
        return Err(it.error("ValueError", "Array sizes are inconsistent"));
    }
    let cmps: Vec<_> = groups.iter().map(|g| (crate::arrays::sort_cmp(it, g.2), g.1 == SORT_DESC)).collect();
    let mut rows: Vec<usize> = (0..n).collect();
    crate::zsort::sort(&mut rows, |&x, &y| {
        for (j, (cmp, desc)) in cmps.iter().enumerate() {
            let o = cmp(&arrays[j][x].1.deref(), &arrays[j][y].1.deref());
            let o = if *desc { o.reverse() } else { o };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
    for (j, g) in groups.iter().enumerate() {
        let mut out = PhpArray::new();
        for &r in &rows {
            let (k, v) = &arrays[j][r];
            match k {
                Key::Str(_) => out.insert(k.clone(), v.clone()),
                Key::Int(_) => {
                    out.push(v.clone());
                }
            }
        }
        match &mut a[g.0] {
            Value::Ref(cell) => *cell.borrow_mut() = Value::array(out),
            t => *t = Value::array(out),
        }
    }
    Ok(Value::Bool(true))
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("array_change_key_case", |it, _, a| change_key_case(it, a));
    t.insert("array_intersect_assoc", |it, f, a| assoc_op(it, f, a, "array_intersect_assoc", true, false, false));
    t.insert("array_uintersect_assoc", |it, f, a| assoc_op(it, f, a, "array_uintersect_assoc", true, true, false));
    t.insert("array_intersect_uassoc", |it, f, a| assoc_op(it, f, a, "array_intersect_uassoc", true, false, true));
    t.insert("array_uintersect_uassoc", |it, f, a| assoc_op(it, f, a, "array_uintersect_uassoc", true, true, true));
    t.insert("array_diff_uassoc", |it, f, a| assoc_op(it, f, a, "array_diff_uassoc", false, false, true));
    t.insert("array_udiff_assoc", |it, f, a| assoc_op(it, f, a, "array_udiff_assoc", false, true, false));
    t.insert("array_udiff_uassoc", |it, f, a| assoc_op(it, f, a, "array_udiff_uassoc", false, true, true));
    t.insert("array_multisort", |it, _, a| multisort(it, a));
}
