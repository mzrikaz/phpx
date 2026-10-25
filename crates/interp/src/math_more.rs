//! The rest of ext/standard's math: inverse and hyperbolic trig, hypot,
//! fdiv(), fpow(), angle conversion; gettimeofday().

use std::collections::HashMap;

use crate::builtins::{float, has};
use crate::interp::{BuiltinFn, R};
use crate::value::{Key, PhpArray, Value};

fn f1(t: &mut HashMap<&'static str, BuiltinFn>, name: &'static str, g: BuiltinFn) {
    t.insert(name, g);
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    f1(t, "asin", |it, _, a| Ok(Value::Float(float(it, a, 0)?.asin())));
    f1(t, "acos", |it, _, a| Ok(Value::Float(float(it, a, 0)?.acos())));
    f1(t, "sinh", |it, _, a| Ok(Value::Float(float(it, a, 0)?.sinh())));
    f1(t, "cosh", |it, _, a| Ok(Value::Float(float(it, a, 0)?.cosh())));
    f1(t, "tanh", |it, _, a| Ok(Value::Float(float(it, a, 0)?.tanh())));
    f1(t, "asinh", |it, _, a| Ok(Value::Float(float(it, a, 0)?.asinh())));
    f1(t, "acosh", |it, _, a| Ok(Value::Float(float(it, a, 0)?.acosh())));
    f1(t, "atanh", |it, _, a| Ok(Value::Float(float(it, a, 0)?.atanh())));
    f1(t, "expm1", |it, _, a| Ok(Value::Float(float(it, a, 0)?.exp_m1())));
    f1(t, "log1p", |it, _, a| Ok(Value::Float(float(it, a, 0)?.ln_1p())));
    f1(t, "hypot", |it, _, a| Ok(Value::Float(float(it, a, 0)?.hypot(float(it, a, 1)?))));
    f1(t, "deg2rad", |it, _, a| Ok(Value::Float(float(it, a, 0)? / 180.0 * std::f64::consts::PI)));
    f1(t, "rad2deg", |it, _, a| Ok(Value::Float(float(it, a, 0)? / std::f64::consts::PI * 180.0)));
    f1(t, "fdiv", |it, _, a| Ok(Value::Float(float(it, a, 0)? / float(it, a, 1)?)));
    f1(t, "fpow", |it, _, a| Ok(Value::Float(float(it, a, 0)?.powf(float(it, a, 1)?))));
    f1(t, "gettimeofday", |it, _, a| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        if has(a, 0) && a[0].truthy() {
            return Ok(Value::Float(now.as_secs() as f64 + now.subsec_micros() as f64 / 1e6));
        }
        let (offset, dst, _) = crate::date::default_zone(it).at(now.as_secs() as i64);
        let mut out = PhpArray::new();
        out.insert(Key::from_bytes(b"sec"), Value::Int(now.as_secs() as i64));
        out.insert(Key::from_bytes(b"usec"), Value::Int(now.subsec_micros() as i64));
        out.insert(Key::from_bytes(b"minuteswest"), Value::Int(-(offset as i64) / 60));
        out.insert(Key::from_bytes(b"dsttime"), Value::Int(dst as i64));
        let r: R<Value> = Ok(Value::array(out));
        r
    });
}
