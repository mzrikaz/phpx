//! Array built-ins. By-reference ones (sort, array_push, ...) receive the
//! current value in `a[0]` and write the result back into it.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::builtins::{arg, arr, callable, has, int, random_u64};
use crate::interp::{BuiltinFn, Ctrl, Frame, Interp, R};
use crate::value::{compare, identical, loose_eq, Key, PStr, PhpArray, Value};

fn list(vals: impl IntoIterator<Item = Value>) -> R<Value> {
    Ok(Value::array(PhpArray::from_list(vals)))
}

fn by_ref_array(it: &Interp, a: &mut [Value], fname: &str) -> R<PhpArray> {
    match std::mem::take(&mut a[0]) {
        Value::Array(x) => Ok(Rc::try_unwrap(x).unwrap_or_else(|x| (*x).clone())),
        other => {
            a[0] = other.clone();
            Err(it.error(
                "TypeError",
                &format!("{fname}(): Argument #1 ($array) must be of type array, {} given", other.debug_type()),
            ))
        }
    }
}

/// Comparison used by sort() & co for a given flag.
fn sort_cmp(it: &Interp, flags: i64) -> impl Fn(&Value, &Value) -> Ordering + '_ {
    move |x, y| match flags & !8 {
        1 => it.to_float(x).unwrap_or(0.0).partial_cmp(&it.to_float(y).unwrap_or(0.0)).unwrap_or(Ordering::Equal),
        2 => {
            let (p, q) = (it.to_str(x).unwrap_or_default(), it.to_str(y).unwrap_or_default());
            if flags & 8 != 0 { p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase()) } else { p.cmp(&q) }
        }
        6 => crate::strings::natcmp(&it.to_str(x).unwrap_or_default(), &it.to_str(y).unwrap_or_default(), flags & 8 != 0),
        _ => compare(x, y).unwrap_or(Ordering::Equal),
    }
}

/// Call a user comparator; remember the first exception and stop comparing.
fn user_cmp(it: &Interp, c: &RefCell<crate::callback::Callback>, err: &RefCell<Option<Ctrl>>, x: Value, y: Value) -> Ordering {
    if err.borrow().is_some() {
        return Ordering::Equal;
    }
    match c.borrow_mut().call(&mut [x, y]) {
        Ok(v) => match v {
            Value::Bool(b) => if b { Ordering::Greater } else { Ordering::Less },
            v => it.to_int(&v).unwrap_or(0).cmp(&0),
        },
        Err(e) => {
            *err.borrow_mut() = Some(e);
            Ordering::Equal
        }
    }
}

enum SortBy {
    Value,
    Key,
}

fn sort_generic(it: &Interp, f: &Frame, a: &mut [Value], by: SortBy, reverse: bool, keep_keys: bool, user: bool) -> R<Value> {
    let fname = "sort";
    let arr = by_ref_array(it, a, fname)?;
    // Raw entries: an element that is a reference stays one after moving.
    let mut entries: Vec<(Key, Value)> = arr.into_raw_pairs();
    let err = RefCell::new(None);
    // Stable sort; descending just flips the comparator.
    let flip = |o: Ordering| if reverse { o.reverse() } else { o };
    if user {
        let c = RefCell::new(it.callback(callable(it, f, a, 1)?));
        match by {
            SortBy::Value => entries.sort_by(|x, y| flip(user_cmp(it, &c, &err, x.1.clone(), y.1.clone()))),
            SortBy::Key => entries.sort_by(|x, y| flip(user_cmp(it, &c, &err, x.0.to_value(), y.0.to_value()))),
        }
    } else {
        let flags = if has(a, 1) { it.to_int(&a[1])? } else { 0 };
        let cmp = sort_cmp(it, flags);
        match by {
            SortBy::Value => entries.sort_by(|x, y| flip(cmp(&x.1, &y.1))),
            SortBy::Key => entries.sort_by(|x, y| flip(cmp(&x.0.to_value(), &y.0.to_value()))),
        }
    }
    if let Some(e) = err.into_inner() {
        a[0] = Value::array(PhpArray::from_list(entries.into_iter().map(|e| e.1)));
        return Err(e);
    }
    let mut out = PhpArray::new();
    for (k, v) in entries {
        if keep_keys {
            out.insert(k, v);
        } else {
            out.push(v);
        }
    }
    a[0] = Value::array(out);
    Ok(Value::Bool(true))
}

fn str_key(it: &Interp, v: &Value) -> R<PStr> {
    it.to_str(v)
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("count", |it, _, a| count(it, &arg(a, 0), int(it, a, 1).unwrap_or(0) == 1));
    t.insert("sizeof", |it, _, a| count(it, &arg(a, 0), false));
    t.insert("array_keys", |it, _, a| {
        let x = arr(it, a, 0, "array_keys")?;
        if has(a, 1) {
            let needle = arg(a, 1);
            let strict = arg(a, 2).truthy();
            return list(x.iter().filter(|(_, v)| if strict { identical(v, &needle) } else { loose_eq(v, &needle) }).map(|(k, _)| k.to_value()));
        }
        list(x.iter().map(|(k, _)| k.to_value()))
    });
    t.insert("array_values", |it, _, a| list(arr(it, a, 0, "array_values")?.values()));
    t.insert("array_merge", |it, _, a| {
        let mut out = PhpArray::new();
        for i in 0..a.len() {
            for (k, v) in arr(it, a, i, "array_merge")?.iter() {
                match k {
                    Key::Int(_) => {
                        out.push(v.clone());
                    }
                    Key::Str(_) => out.insert(k.clone(), v.clone()),
                }
            }
        }
        Ok(Value::array(out))
    });
    t.insert("array_merge_recursive", |it, _, a| {
        fn merge(dst: &mut PhpArray, src: &PhpArray) {
            for (k, v) in src.iter() {
                match k {
                    Key::Int(_) => {
                        dst.push(v.clone());
                    }
                    Key::Str(_) => match (dst.get_mut(&k), &v) {
                        (Some(Value::Array(d)), Value::Array(s)) => merge(Rc::make_mut(d), &s),
                        (Some(existing), _) => {
                            let mut both = match std::mem::take(existing) {
                                Value::Array(d) => (*d).clone(),
                                other => PhpArray::from_list([other]),
                            };
                            match v {
                                Value::Array(s) => merge(&mut both, &s),
                                other => {
                                    both.push(other.clone());
                                }
                            }
                            *existing = Value::array(both);
                        }
                        (None, _) => dst.insert(k.clone(), v.clone()),
                    },
                }
            }
        }
        let mut out = PhpArray::new();
        for i in 0..a.len() {
            let src = arr(it, a, i, "array_merge_recursive")?;
            merge(&mut out, &src);
        }
        Ok(Value::array(out))
    });
    t.insert("array_replace", |it, _, a| {
        let mut out = (*arr(it, a, 0, "array_replace")?).clone();
        for i in 1..a.len() {
            for (k, v) in arr(it, a, i, "array_replace")?.iter() {
                out.insert(k.clone(), v.clone());
            }
        }
        Ok(Value::array(out))
    });
    t.insert("array_combine", |it, _, a| {
        let (k, v) = (arr(it, a, 0, "array_combine")?, arr(it, a, 1, "array_combine")?);
        if k.len() != v.len() {
            return Err(it.error(
                "ValueError",
                "array_combine(): Argument #1 ($keys) and argument #2 ($values) must have the same number of elements",
            ));
        }
        let mut out = PhpArray::new();
        for (kk, vv) in k.values().zip(v.values()) {
            out.insert(it.to_key(&kk)?, vv.clone());
        }
        Ok(Value::array(out))
    });
    t.insert("array_flip", |it, _, a| {
        let mut out = PhpArray::new();
        for (k, v) in arr(it, a, 0, "array_flip")?.iter() {
            match v {
                Value::Int(_) | Value::Str(_) => out.insert(it.to_key(&v)?, k.to_value()),
                _ => it.warn("array_flip(): Can only flip string and integer values, entry skipped"),
            }
        }
        Ok(Value::array(out))
    });
    t.insert("array_slice", |it, _, a| {
        let x = arr(it, a, 0, "array_slice")?;
        let len = if has(a, 2) && !arg(a, 2).is_null() { Some(int(it, a, 2)?) } else { None };
        let preserve = arg(a, 3).truthy();
        let (lo, hi) = crate::strings::substr_range(x.len(), int(it, a, 1)?, len);
        let mut out = PhpArray::new();
        for (k, v) in x.iter().skip(lo).take(hi - lo) {
            match k {
                Key::Int(_) if !preserve => {
                    out.push(v.clone());
                }
                _ => out.insert(k.clone(), v.clone()),
            }
        }
        Ok(Value::array(out))
    });
    t.insert("array_splice", |it, _, a| {
        let x = by_ref_array(it, a, "array_splice")?;
        let len = if has(a, 2) && !arg(a, 2).is_null() { Some(int(it, a, 2)?) } else { None };
        let (lo, hi) = crate::strings::substr_range(x.len(), int(it, a, 1)?, len);
        let repl: Vec<Value> = match arg(a, 3) {
            Value::Array(r) => r.values().collect(),
            Value::Null if !has(a, 3) => vec![],
            v => vec![v],
        };
        let total = x.len();
        let mut out = PhpArray::new();
        let mut removed = PhpArray::new();
        for (i, (k, v)) in x.into_pairs().into_iter().enumerate() {
            if i == lo {
                for r in &repl {
                    out.push(r.clone());
                }
            }
            if i >= lo && i < hi {
                removed.push(v);
                continue;
            }
            match k {
                Key::Int(_) => {
                    out.push(v);
                }
                _ => out.insert(k, v),
            }
        }
        if lo >= total {
            for r in &repl {
                out.push(r.clone());
            }
        }
        a[0] = Value::array(out);
        Ok(Value::array(removed))
    });
    t.insert("array_map", |it, f, a| {
        let cb = arg(a, 0);
        if a.len() == 2 {
            let x = arr(it, a, 1, "array_map")?;
            if cb.is_null() {
                return Ok(Value::Array(x));
            }
            let mut c = it.callback(it.to_callable(f, &cb)?);
            let mut out = PhpArray::new();
            for (k, v) in x.iter() {
                out.insert(k.clone(), c.call(&mut [v.clone()])?);
            }
            return Ok(Value::array(out));
        }
        let arrays: Vec<Rc<PhpArray>> = (1..a.len()).map(|i| arr(it, a, i, "array_map")).collect::<R<_>>()?;
        let n = arrays.iter().map(|x| x.len()).max().unwrap_or(0);
        let mut c = if cb.is_null() { None } else { Some(it.callback(it.to_callable(f, &cb)?)) };
        let mut out = PhpArray::new();
        for i in 0..n {
            let mut args: Vec<Value> = arrays.iter().map(|x| x.get_index(i).map(|e| e.1.clone()).unwrap_or_default()).collect();
            out.push(match &mut c {
                Some(c) => c.call(&mut args)?,
                None => Value::array(PhpArray::from_list(args)),
            });
        }
        Ok(Value::array(out))
    });
    t.insert("array_filter", |it, f, a| {
        let x = arr(it, a, 0, "array_filter")?;
        let mode = int(it, a, 2)?;
        let mut c = if has(a, 1) && !arg(a, 1).is_null() { Some(it.callback(callable(it, f, a, 1)?)) } else { None };
        let mut out = PhpArray::new();
        for (k, v) in x.iter() {
            let keep = match &mut c {
                None => v.truthy(),
                Some(c) => match mode {
                    2 => c.call(&mut [k.to_value()])?,
                    1 => c.call(&mut [v.clone(), k.to_value()])?,
                    _ => c.call(&mut [v.clone()])?,
                }
                .truthy(),
            };
            if keep {
                out.insert(k.clone(), v.clone());
            }
        }
        Ok(Value::array(out))
    });
    t.insert("array_reduce", |it, f, a| {
        let x = arr(it, a, 0, "array_reduce")?;
        let mut c = it.callback(callable(it, f, a, 1)?);
        let mut acc = arg(a, 2);
        for v in x.values() {
            acc = c.call(&mut [acc, v])?;
        }
        Ok(acc)
    });
    // Walks the array as it is at each step (a property passed in is a
    // reference: elements the callback adds are visited too, as in PHP);
    // a by-reference first parameter updates the element.
    t.insert("array_walk", |it, f, a| {
        let cb = callable(it, f, a, 1)?;
        let by_ref = crate::call::callable_params(&cb).and_then(|ps| ps.first()).is_some_and(|p| p.by_ref);
        let extra = if has(a, 2) { vec![arg(a, 2)] } else { vec![] };
        let current = |a: &[Value]| match &a[0] {
            Value::Ref(s) => s.borrow().clone(),
            v => v.clone(),
        };
        match current(a) {
            Value::Array(_) | Value::Object(_) => {}
            other => {
                return Err(it.error("TypeError", &format!("array_walk(): Argument #1 ($array) must be of type array, {} given", other.debug_type())));
            }
        }
        let mut pos = 0;
        loop {
            let Value::Array(arr) = current(a) else { break };
            let Some((k, v)) = arr.get_index(pos) else { break };
            drop(arr);
            pos += 1;
            if !by_ref {
                let mut args = vec![v, k.to_value()];
                args.extend(extra.iter().cloned());
                it.call_callable(&cb, args, vec![])?;
                continue;
            }
            let cell = crate::interp::slot(v);
            let mut args = vec![Value::Ref(cell.clone()), k.to_value()];
            args.extend(extra.iter().cloned());
            it.call_callable(&cb, args, vec![])?;
            let nv = cell.borrow().clone();
            let write = |arr: &mut Value| {
                if let Value::Array(x) = arr {
                    if x.get(&k).is_some() {
                        Rc::make_mut(x).insert(k.clone(), nv.clone());
                    }
                }
            };
            match &mut a[0] {
                Value::Ref(s) => write(&mut s.borrow_mut()),
                v => write(v),
            }
        }
        Ok(Value::Bool(true))
    });
    t.insert("array_sum", |it, _, a| {
        let mut acc = Value::Int(0);
        for v in arr(it, a, 0, "array_sum")?.values() {
            if matches!(v, Value::Array(_) | Value::Object(_)) {
                it.warn("array_sum(): Addition is not supported on type array");
                continue;
            }
            acc = it.binop(phpx_syntax::ast::BinOp::Add, &acc, &v)?;
        }
        Ok(acc)
    });
    t.insert("array_product", |it, _, a| {
        let mut acc = Value::Int(1);
        for v in arr(it, a, 0, "array_product")?.values() {
            acc = it.binop(phpx_syntax::ast::BinOp::Mul, &acc, &v)?;
        }
        Ok(acc)
    });
    t.insert("in_array", |it, _, a| {
        let (needle, strict) = (arg(a, 0), arg(a, 2).truthy());
        Ok(Value::Bool(arr(it, a, 1, "in_array")?.values().any(|v| if strict { identical(&v, &needle) } else { loose_eq(&v, &needle) })))
    });
    t.insert("array_search", |it, _, a| {
        let (needle, strict) = (arg(a, 0), arg(a, 2).truthy());
        Ok(arr(it, a, 1, "array_search")?
            .iter()
            .find(|(_, v)| if strict { identical(v, &needle) } else { loose_eq(v, &needle) })
            .map(|(k, _)| k.to_value())
            .unwrap_or(Value::Bool(false)))
    });
    t.insert("array_key_exists", |it, _, a| key_exists(it, a));
    t.insert("key_exists", |it, _, a| key_exists(it, a));
    t.insert("array_unique", |it, _, a| {
        let x = arr(it, a, 0, "array_unique")?;
        let mut seen: HashSet<PStr> = HashSet::new();
        let mut out = PhpArray::new();
        for (k, v) in x.iter() {
            if seen.insert(str_key(it, &v)?) {
                out.insert(k.clone(), v.clone());
            }
        }
        Ok(Value::array(out))
    });
    t.insert("array_count_values", |it, _, a| {
        let mut out = PhpArray::new();
        for v in arr(it, a, 0, "array_count_values")?.values() {
            let k = it.to_key(&v)?;
            let n = match out.get(&k) {
                Some(Value::Int(n)) => n + 1,
                _ => 1,
            };
            out.insert(k, Value::Int(n));
        }
        Ok(Value::array(out))
    });
    t.insert("array_reverse", |it, _, a| {
        let x = arr(it, a, 0, "array_reverse")?;
        let preserve = arg(a, 1).truthy();
        let mut out = PhpArray::new();
        for (k, v) in x.iter().rev() {
            match k {
                Key::Int(_) if !preserve => {
                    out.push(v.clone());
                }
                _ => out.insert(k.clone(), v.clone()),
            }
        }
        Ok(Value::array(out))
    });
    t.insert("array_fill", |it, _, a| {
        let (start, n) = (int(it, a, 0)?, int(it, a, 1)?);
        if n < 0 {
            return Err(it.error("ValueError", "array_fill(): Argument #2 ($count) must be greater than or equal to 0"));
        }
        crate::builtins::reserve(it, (n as usize).saturating_mul(16))?;
        let mut out = PhpArray::new();
        for i in 0..n {
            out.insert(Key::Int(start + i), arg(a, 2));
        }
        Ok(Value::array(out))
    });
    t.insert("array_fill_keys", |it, _, a| {
        let mut out = PhpArray::new();
        for k in arr(it, a, 0, "array_fill_keys")?.values() {
            out.insert(it.to_key(&k)?, arg(a, 1));
        }
        Ok(Value::array(out))
    });
    t.insert("array_pad", |it, _, a| {
        let x = arr(it, a, 0, "array_pad")?;
        let n = int(it, a, 1)?;
        let missing = (n.unsigned_abs() as usize).saturating_sub(x.len());
        let pad = std::iter::repeat_n(arg(a, 2), missing);
        let vals: Vec<Value> = if n < 0 { pad.chain(x.values()).collect() } else { x.values().chain(pad).collect() };
        list(vals)
    });
    t.insert("range", |it, _, a| range(it, a));
    t.insert("array_column", |it, _, a| {
        let x = arr(it, a, 0, "array_column")?;
        let (col, idx) = (arg(a, 1), arg(a, 2));
        let mut out = PhpArray::new();
        for row in x.values() {
            let get = |k: &Value| -> R<Option<Value>> {
                Ok(match &row {
                    Value::Array(r) => r.get(&it.to_key(k)?),
                    Value::Object(o) => o.get_prop(&it.to_string_lossy(k)?),
                    _ => None,
                })
            };
            let v = if col.is_null() { Some(row.clone()) } else { get(&col)? };
            let Some(v) = v else { continue };
            match if idx.is_null() { None } else { get(&idx)? } {
                Some(k) => out.insert(it.to_key(&k)?, v),
                None => {
                    out.push(v);
                }
            }
        }
        Ok(Value::array(out))
    });
    t.insert("array_chunk", |it, _, a| {
        let x = arr(it, a, 0, "array_chunk")?;
        let n = int(it, a, 1)?;
        if n < 1 {
            return Err(it.error("ValueError", "array_chunk(): Argument #2 ($length) must be greater than 0"));
        }
        let preserve = arg(a, 2).truthy();
        let mut out = PhpArray::new();
        let mut cur = PhpArray::new();
        for (k, v) in x.iter() {
            if preserve {
                cur.insert(k.clone(), v.clone());
            } else {
                cur.push(v.clone());
            }
            if cur.len() as i64 == n {
                out.push(Value::array(std::mem::take(&mut cur)));
            }
        }
        if !cur.is_empty() {
            out.push(Value::array(cur));
        }
        Ok(Value::array(out))
    });
    t.insert("array_diff", |it, _, a| set_op(it, a, false, false, "array_diff"));
    t.insert("array_diff_key", |it, _, a| set_op(it, a, true, false, "array_diff_key"));
    t.insert("array_intersect", |it, _, a| set_op(it, a, false, true, "array_intersect"));
    t.insert("array_intersect_key", |it, _, a| set_op(it, a, true, true, "array_intersect_key"));
    t.insert("array_diff_assoc", |it, _, a| {
        let x = arr(it, a, 0, "array_diff_assoc")?;
        let others: Vec<Rc<PhpArray>> = (1..a.len()).map(|i| arr(it, a, i, "array_diff_assoc")).collect::<R<_>>()?;
        let mut out = PhpArray::new();
        for (k, v) in x.iter() {
            let sv = it.to_str(&v)?;
            let mut found = false;
            for o in &others {
                if let Some(w) = o.get(&k) {
                    if it.to_str(&w)? == sv {
                        found = true;
                    }
                }
            }
            if !found {
                out.insert(k.clone(), v.clone());
            }
        }
        Ok(Value::array(out))
    });
    t.insert("array_first", |it, _, a| Ok(arr(it, a, 0, "array_first")?.values().next().unwrap_or_default()));
    t.insert("array_last", |it, _, a| Ok(arr(it, a, 0, "array_last")?.values().next_back().unwrap_or_default()));
    t.insert("array_key_first", |it, _, a| Ok(arr(it, a, 0, "array_key_first")?.iter().next().map(|(k, _)| k.to_value()).unwrap_or_default()));
    t.insert("array_key_last", |it, _, a| Ok(arr(it, a, 0, "array_key_last")?.iter().last().map(|(k, _)| k.to_value()).unwrap_or_default()));
    t.insert("array_is_list", |it, _, a| Ok(Value::Bool(arr(it, a, 0, "array_is_list")?.is_list())));
    t.insert("array_find", |it, f, a| {
        let mut c = it.callback(callable(it, f, a, 1)?);
        for (k, v) in arr(it, a, 0, "array_find")?.iter() {
            if c.call(&mut [v.clone(), k.to_value()])?.truthy() {
                return Ok(v.clone());
            }
        }
        Ok(Value::Null)
    });
    t.insert("array_find_key", |it, f, a| {
        let mut c = it.callback(callable(it, f, a, 1)?);
        for (k, v) in arr(it, a, 0, "array_find_key")?.iter() {
            if c.call(&mut [v.clone(), k.to_value()])?.truthy() {
                return Ok(k.to_value());
            }
        }
        Ok(Value::Null)
    });
    t.insert("array_any", |it, f, a| {
        let mut c = it.callback(callable(it, f, a, 1)?);
        for (k, v) in arr(it, a, 0, "array_any")?.iter() {
            if c.call(&mut [v.clone(), k.to_value()])?.truthy() {
                return Ok(Value::Bool(true));
            }
        }
        Ok(Value::Bool(false))
    });
    t.insert("array_all", |it, f, a| {
        let mut c = it.callback(callable(it, f, a, 1)?);
        for (k, v) in arr(it, a, 0, "array_all")?.iter() {
            if !c.call(&mut [v.clone(), k.to_value()])?.truthy() {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(true))
    });
    t.insert("array_rand", |it, _, a| {
        let x = arr(it, a, 0, "array_rand")?;
        if x.is_empty() {
            return Err(it.error("ValueError", "array_rand(): Argument #1 ($array) must not be empty"));
        }
        let i = (random_u64() % x.len() as u64) as usize;
        Ok(x.get_index(i).unwrap().0.to_value())
    });
    t.insert("shuffle", |it, _, a| {
        let x = by_ref_array(it, a, "shuffle")?;
        let mut vals: Vec<Value> = x.into_pairs().into_iter().map(|e| e.1).collect();
        for i in (1..vals.len()).rev() {
            vals.swap(i, (random_u64() % (i as u64 + 1)) as usize);
        }
        a[0] = Value::array(PhpArray::from_list(vals));
        Ok(Value::Bool(true))
    });

    // sorting (by reference)
    t.insert("sort", |it, f, a| sort_generic(it, f, a, SortBy::Value, false, false, false));
    t.insert("rsort", |it, f, a| sort_generic(it, f, a, SortBy::Value, true, false, false));
    t.insert("usort", |it, f, a| sort_generic(it, f, a, SortBy::Value, false, false, true));
    t.insert("asort", |it, f, a| sort_generic(it, f, a, SortBy::Value, false, true, false));
    t.insert("arsort", |it, f, a| sort_generic(it, f, a, SortBy::Value, true, true, false));
    t.insert("uasort", |it, f, a| sort_generic(it, f, a, SortBy::Value, false, true, true));
    t.insert("ksort", |it, f, a| sort_generic(it, f, a, SortBy::Key, false, true, false));
    t.insert("krsort", |it, f, a| sort_generic(it, f, a, SortBy::Key, true, true, false));
    t.insert("uksort", |it, f, a| sort_generic(it, f, a, SortBy::Key, false, true, true));
    t.insert("natsort", |it, f, a| {
        // natsort = asort with SORT_NATURAL (| SORT_FLAG_CASE).
        let mut v = vec![std::mem::take(&mut a[0]), Value::Int(6)];
        let r = sort_generic(it, f, &mut v, SortBy::Value, false, true, false);
        a[0] = std::mem::take(&mut v[0]);
        r
    });
    t.insert("natcasesort", |it, f, a| {
        // natsort = asort with SORT_NATURAL (| SORT_FLAG_CASE).
        let mut v = vec![std::mem::take(&mut a[0]), Value::Int(6 | 8)];
        let r = sort_generic(it, f, &mut v, SortBy::Value, false, true, false);
        a[0] = std::mem::take(&mut v[0]);
        r
    });

    // stack/queue (by reference)
    t.insert("array_push", |it, _, a| {
        let mut x = by_ref_array(it, a, "array_push")?;
        for v in a[1..].iter() {
            x.push(v.clone());
        }
        let n = x.len();
        a[0] = Value::array(x);
        Ok(Value::Int(n as i64))
    });
    t.insert("array_pop", |it, _, a| {
        let mut x = by_ref_array(it, a, "array_pop")?;
        let v = x.pop().unwrap_or_default();
        a[0] = Value::array(x);
        Ok(v)
    });
    t.insert("array_shift", |it, _, a| {
        let x = by_ref_array(it, a, "array_shift")?;
        let mut it2 = x.into_pairs().into_iter();
        let first = it2.next().map(|e| e.1).unwrap_or_default();
        let mut out = PhpArray::new();
        for (k, v) in it2 {
            match k {
                Key::Int(_) => {
                    out.push(v);
                }
                _ => out.insert(k, v),
            }
        }
        a[0] = Value::array(out);
        Ok(first)
    });
    t.insert("array_unshift", |it, _, a| {
        let x = by_ref_array(it, a, "array_unshift")?;
        let mut out = PhpArray::from_list(a[1..].iter().cloned());
        for (k, v) in x.into_pairs() {
            match k {
                Key::Int(_) => {
                    out.push(v);
                }
                _ => out.insert(k, v),
            }
        }
        let n = out.len();
        a[0] = Value::array(out);
        Ok(Value::Int(n as i64))
    });
    // Internal-pointer functions without the pointer: enough for end()/reset().
}

fn count(it: &Interp, v: &Value, recursive: bool) -> R<Value> {
    match v {
        Value::Array(x) => {
            let mut n = x.len() as i64;
            if recursive {
                let mut path = vec![std::rc::Rc::as_ptr(x) as usize];
                n += count_nested(it, x, &mut path);
            }
            Ok(Value::Int(n))
        }
        Value::Object(o) if o.class.is_a("countable") => {
            let n = it.call_method(o, "count", vec![])?;
            Ok(Value::Int(it.to_int(&n)?))
        }
        other => Err(it.error(
            "TypeError",
            &format!("count(): Argument #1 ($value) must be of type Countable|array, {} given", other.debug_type()),
        )),
    }
}

/// Elements of nested arrays for COUNT_RECURSIVE; an array met again on
/// its own path (a reference to itself) is a warning, like PHP.
fn count_nested(it: &Interp, x: &PhpArray, path: &mut Vec<usize>) -> i64 {
    let mut n = 0;
    for v in x.values() {
        if let Value::Array(inner) = v {
            let p = std::rc::Rc::as_ptr(&inner) as usize;
            if path.contains(&p) || crate::value::stack_low() {
                it.warn("count(): Recursion detected");
                continue;
            }
            path.push(p);
            n += inner.len() as i64 + count_nested(it, &inner, path);
            path.pop();
        }
    }
    n
}

fn key_exists(it: &Interp, a: &[Value]) -> R<Value> {
    match arg(a, 1) {
        Value::Array(x) if arg(a, 0).is_null() => {
            it.deprecated("Using null as the key parameter for array_key_exists() is deprecated, use an empty string instead");
            Ok(Value::Bool(x.get(&crate::value::Key::Str(crate::value::pstr(""))).is_some()))
        }
        Value::Array(x) => Ok(Value::Bool(x.get(&it.to_key(&arg(a, 0))?).is_some())),
        Value::Object(o) => Ok(Value::Bool(o.props.borrow().contains_key(&*it.to_string_lossy(&arg(a, 0))?))),
        other => Err(it.error(
            "TypeError",
            &format!("array_key_exists(): Argument #2 ($array) must be of type array, {} given", other.debug_type()),
        )),
    }
}

/// array_diff / array_intersect (by value, string comparison) and their _key variants.
fn set_op(it: &Interp, a: &[Value], by_key: bool, intersect: bool, name: &str) -> R<Value> {
    let x = arr(it, a, 0, name)?;
    let mut others: Vec<HashSet<Vec<u8>>> = vec![];
    for i in 1..a.len() {
        let o = arr(it, a, i, name)?;
        let set = if by_key {
            o.iter().map(|(k, _)| key_bytes(&k)).collect()
        } else {
            o.values().map(|v| it.to_str(&v).map(|s| s.to_vec())).collect::<R<_>>()?
        };
        others.push(set);
    }
    let mut out = PhpArray::new();
    for (k, v) in x.iter() {
        let probe = if by_key { key_bytes(&k) } else { it.to_str(&v)?.to_vec() };
        let keep = if intersect { others.iter().all(|s| s.contains(&probe)) } else { !others.iter().any(|s| s.contains(&probe)) };
        if keep {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::array(out))
}

fn key_bytes(k: &Key) -> Vec<u8> {
    match k {
        Key::Int(n) => n.to_string().into_bytes(),
        Key::Str(s) => s.to_vec(),
    }
}

fn range(it: &Interp, a: &[Value]) -> R<Value> {
    let (s, e) = (arg(a, 0), arg(a, 1));
    let step_v = if has(a, 2) { arg(a, 2) } else { Value::Int(1) };
    // Single-letter strings: character ranges.
    if let (Value::Str(x), Value::Str(y)) = (&s, &e) {
        if x.len() == 1 && y.len() == 1 && !x[0].is_ascii_digit() && !y[0].is_ascii_digit() {
            let step = it.to_int(&step_v)?.unsigned_abs().max(1) as usize;
            let (lo, hi) = (x[0], y[0]);
            let chars: Vec<u8> = if lo <= hi { (lo..=hi).step_by(step).collect() } else { (hi..=lo).rev().step_by(step).collect() };
            return list(chars.into_iter().map(|c| Value::bytes(vec![c])));
        }
    }
    let num = |v: &Value| -> R<crate::value::Num> {
        Ok(match v {
            Value::Int(n) => crate::value::Num::Int(*n),
            Value::Float(f) => crate::value::Num::Float(*f),
            Value::Str(st) => crate::value::parse_numeric(st).unwrap_or(crate::value::Num::Int(0)),
            other => crate::value::Num::Int(it.to_int(other)?),
        })
    };
    let (x, y, st) = (num(&s)?, num(&e)?, num(&step_v)?);
    use crate::value::Num;
    match (x, y, st) {
        (Num::Int(lo), Num::Int(hi), Num::Int(step)) => {
            let step = step.unsigned_abs();
            if step == 0 {
                return Err(it.error("ValueError", "range(): Argument #3 ($step) cannot be 0"));
            }
            let count = (lo.abs_diff(hi) / step) as usize + 1;
            if count > 100_000_000 {
                return Err(it.error("ValueError", "The supplied range exceeds the maximum array size"));
            }
            let vals = (0..count).map(|i| Value::Int(if lo <= hi { lo + (i as u64 * step) as i64 } else { lo - (i as u64 * step) as i64 }));
            list(vals)
        }
        _ => {
            let (lo, hi, step) = (x.as_f64(), y.as_f64(), st.as_f64().abs());
            if step == 0.0 {
                return Err(it.error("ValueError", "range(): Argument #3 ($step) cannot be 0"));
            }
            let count = ((hi - lo).abs() / step + 1e-9).floor() as usize + 1;
            list((0..count).map(|i| Value::Float(if lo <= hi { lo + i as f64 * step } else { lo - i as f64 * step })))
        }
    }
}
