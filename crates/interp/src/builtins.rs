//! Built-in function table, argument helpers, and the core built-ins:
//! output, types, math, classes, output buffering, time, async.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::interp::{BuiltinFn, Callable, Ctrl, Frame, Interp, R};
use crate::value::{self, Key, Out, PStr, PhpArray, Value};

// ---- argument helpers (shared by the other builtin modules) -------------------

pub fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).cloned().unwrap_or_default()
}

pub fn has(a: &[Value], i: usize) -> bool {
    i < a.len()
}

pub fn s(it: &Interp, a: &[Value], i: usize) -> R<PStr> {
    it.to_str(&arg(a, i))
}

pub fn int(it: &Interp, a: &[Value], i: usize) -> R<i64> {
    it.to_int(&arg(a, i))
}

pub fn float(it: &Interp, a: &[Value], i: usize) -> R<f64> {
    it.to_float(&arg(a, i))
}

pub fn arr(it: &Interp, a: &[Value], i: usize, fname: &str) -> R<Rc<PhpArray>> {
    match arg(a, i) {
        Value::Array(x) => Ok(x),
        other => Err(it.error(
            "TypeError",
            &format!("{fname}(): Argument #{} must be of type array, {} given", i + 1, other.debug_type()),
        )),
    }
}

pub fn callable(it: &Interp, f: &Frame, a: &[Value], i: usize) -> R<Callable> {
    it.to_callable(f, &arg(a, i))
}

/// Run a built-in, turning a Rust panic (a phpx bug) into a PHP `Error`
/// instead of aborting the process. Unwinding must not cross generated code.
pub fn guarded(it: &Interp, name: &str, f: impl FnOnce() -> R<Value>) -> R<Value> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            Err(it.error("Error", &format!("phpx internal error in {name}(): {msg}")))
        }
    }
}

/// Extensions phpx implements (enough for feature checks to pass).
pub const EXTENSIONS: &[&str] = &["Core", "date", "pcre", "hash", "json", "SPL", "standard", "filter", "zlib", "ctype", "random"];

/// All built-in functions.
pub fn table() -> HashMap<&'static str, BuiltinFn> {
    let mut t: HashMap<&'static str, BuiltinFn> = HashMap::new();
    register(&mut t);
    crate::strings::register(&mut t);
    crate::arrays::register(&mut t);
    crate::json::register(&mut t);
    crate::preg::register(&mut t);
    crate::fs::register(&mut t);
    crate::streams::register(&mut t);
    crate::ext::register(&mut t);
    crate::misc::register(&mut t);
    t
}

pub fn constants() -> HashMap<String, Value> {
    let mut c = HashMap::new();
    let mut add = |k: &str, v: Value| {
        c.insert(k.to_string(), v);
    };
    add("PHP_EOL", Value::str("\n"));
    add("PHP_INT_MAX", Value::Int(i64::MAX));
    add("PHP_INT_MIN", Value::Int(i64::MIN));
    add("PHP_INT_SIZE", Value::Int(8));
    add("PHP_FLOAT_EPSILON", Value::Float(f64::EPSILON));
    add("PHP_FLOAT_MAX", Value::Float(f64::MAX));
    add("PHP_FLOAT_MIN", Value::Float(f64::MIN_POSITIVE));
    add("PHP_FLOAT_DIG", Value::Int(15));
    add("PHP_VERSION", Value::str("8.5.10"));
    add("PHP_VERSION_ID", Value::Int(80510));
    add("PHP_RELEASE_VERSION", Value::Int(10));
    add("PHP_EXTRA_VERSION", Value::str(""));
    add("PHP_MAXPATHLEN", Value::Int(4096));
    add("PHP_DEBUG", Value::Int(0));
    add("PHP_ZTS", Value::Int(0));
    add("PHP_SAPI", Value::str("cli"));
    add("PHP_BINARY", Value::str(""));
    add("PATH_SEPARATOR", Value::str(":"));
    add("PHP_MAJOR_VERSION", Value::Int(8));
    add("PHP_MINOR_VERSION", Value::Int(5));
    add("PHP_OS", Value::str(if cfg!(target_os = "macos") { "Darwin" } else { "Linux" }));
    add("PHP_OS_FAMILY", Value::str(if cfg!(target_os = "macos") { "Darwin" } else { "Linux" }));
    add("DIRECTORY_SEPARATOR", Value::str("/"));
    add("NAN", Value::Float(f64::NAN));
    add("INF", Value::Float(f64::INFINITY));
    add("M_PI", Value::Float(std::f64::consts::PI));
    add("M_E", Value::Float(std::f64::consts::E));
    add("M_SQRT2", Value::Float(std::f64::consts::SQRT_2));
    add("E_ALL", Value::Int(32767));
    crate::streams::std_constants(&mut add);
    crate::ext::constants(&mut add);
    for k in ["PASSWORD_DEFAULT", "PASSWORD_BCRYPT"] {
        add(k, Value::str("2y"));
    }
    for (k, v) in crate::misc::FILTER_CONSTS {
        add(k, Value::Int(*v));
    }
    for (n, v) in [("EXTR_OVERWRITE", 0), ("EXTR_SKIP", 1), ("EXTR_PREFIX_SAME", 2), ("EXTR_PREFIX_ALL", 3), ("EXTR_REFS", 256)] {
        add(n, Value::Int(v));
    }
    add("E_ERROR", Value::Int(1));
    add("E_WARNING", Value::Int(2));
    add("E_NOTICE", Value::Int(8));
    add("E_DEPRECATED", Value::Int(8192));
    add("E_STRICT", Value::Int(2048));
    add("E_USER_ERROR", Value::Int(256));
    add("E_USER_WARNING", Value::Int(512));
    add("E_USER_NOTICE", Value::Int(1024));
    add("E_USER_DEPRECATED", Value::Int(16384));
    for (k, v) in [
        ("SORT_REGULAR", 0),
        ("SORT_NUMERIC", 1),
        ("SORT_STRING", 2),
        ("SORT_FLAG_CASE", 8),
        ("SORT_NATURAL", 6),
        ("COUNT_RECURSIVE", 1),
        ("ARRAY_FILTER_USE_BOTH", 1),
        ("ARRAY_FILTER_USE_KEY", 2),
        ("STR_PAD_LEFT", 0),
        ("STR_PAD_RIGHT", 1),
        ("STR_PAD_BOTH", 2),
        ("JSON_HEX_TAG", 1),
        ("JSON_HEX_AMP", 2),
        ("JSON_HEX_APOS", 4),
        ("JSON_HEX_QUOT", 8),
        ("JSON_FORCE_OBJECT", 16),
        ("JSON_NUMERIC_CHECK", 32),
        ("JSON_UNESCAPED_SLASHES", 64),
        ("JSON_PRETTY_PRINT", 128),
        ("JSON_UNESCAPED_UNICODE", 256),
        ("JSON_PARTIAL_OUTPUT_ON_ERROR", 512),
        ("JSON_PRESERVE_ZERO_FRACTION", 1024),
        ("JSON_OBJECT_AS_ARRAY", 1),
        ("JSON_BIGINT_AS_STRING", 2),
        ("JSON_INVALID_UTF8_IGNORE", 1048576),
        ("JSON_INVALID_UTF8_SUBSTITUTE", 2097152),
        ("JSON_THROW_ON_ERROR", 4194304),
        ("JSON_ERROR_NONE", 0),
        ("JSON_ERROR_SYNTAX", 4),
        ("PREG_PATTERN_ORDER", 1),
        ("PREG_SET_ORDER", 2),
        ("PREG_OFFSET_CAPTURE", 256),
        ("PREG_UNMATCHED_AS_NULL", 512),
        ("PREG_SPLIT_NO_EMPTY", 1),
        ("PREG_SPLIT_DELIM_CAPTURE", 2),
        ("PREG_SPLIT_OFFSET_CAPTURE", 4),
        ("ENT_QUOTES", 3),
        ("ENT_COMPAT", 2),
        ("ENT_NOQUOTES", 0),
        ("ENT_HTML401", 0),
        ("ENT_HTML5", 48),
        ("ENT_SUBSTITUTE", 8),
        ("PHP_ROUND_HALF_UP", 1),
        ("PHP_ROUND_HALF_DOWN", 2),
        ("PHP_ROUND_HALF_EVEN", 3),
        ("PHP_ROUND_HALF_ODD", 4),
        ("LC_ALL", 6),
        ("MB_CASE_UPPER", 0),
        ("MB_CASE_LOWER", 1),
        ("MB_CASE_TITLE", 2),
    ] {
        add(k, Value::Int(v));
    }
    c
}

fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    // output
    t.insert("var_dump", |it, _, a| {
        for v in a.iter() {
            let mut o = Out(vec![]);
            value::var_dump(v, 0, &mut o, &mut vec![]);
            it.echo(&o.0);
        }
        Ok(Value::Null)
    });
    t.insert("print_r", |it, _, a| {
        let mut o = Out(vec![]);
        value::print_r(&arg(a, 0), 0, &mut o, &mut vec![]);
        if arg(a, 1).truthy() {
            return Ok(Value::bytes(o.0));
        }
        it.echo(&o.0);
        Ok(Value::Bool(true))
    });
    t.insert("var_export", |it, _, a| {
        let mut o = Out(vec![]);
        value::var_export(&arg(a, 0), 0, &mut o);
        if arg(a, 1).truthy() {
            return Ok(Value::bytes(o.0));
        }
        it.echo(&o.0);
        Ok(Value::Null)
    });
    t.insert("error_log", |it, _, a| {
        eprintln!("{}", String::from_utf8_lossy(&s(it, a, 0)?));
        Ok(Value::Bool(true))
    });
    t.insert("trigger_error", |it, _, a| {
        let level = if has(a, 1) { int(it, a, 1)? } else { 1024 };
        let msg = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        if level == 256 {
            return Err(it.error("Error", &msg));
        }
        it.warn(&msg);
        Ok(Value::Bool(true))
    });

    // types
    t.insert("gettype", |_, _, a| Ok(Value::str(arg(a, 0).gettype())));
    t.insert("get_debug_type", |_, _, a| Ok(Value::str(&arg(a, 0).debug_type())));
    t.insert("is_int", |_, _, a| Ok(Value::Bool(matches!(arg(a, 0), Value::Int(_)))));
    t.insert("is_integer", |_, _, a| Ok(Value::Bool(matches!(arg(a, 0), Value::Int(_)))));
    t.insert("is_long", |_, _, a| Ok(Value::Bool(matches!(arg(a, 0), Value::Int(_)))));
    t.insert("is_float", |_, _, a| Ok(Value::Bool(matches!(arg(a, 0), Value::Float(_)))));
    t.insert("is_double", |_, _, a| Ok(Value::Bool(matches!(arg(a, 0), Value::Float(_)))));
    t.insert("is_string", |_, _, a| Ok(Value::Bool(matches!(arg(a, 0), Value::Str(_)))));
    t.insert("is_bool", |_, _, a| Ok(Value::Bool(matches!(arg(a, 0), Value::Bool(_)))));
    t.insert("is_array", |_, _, a| Ok(Value::Bool(matches!(arg(a, 0), Value::Array(_)))));
    t.insert("is_object", |_, _, a| Ok(Value::Bool(matches!(arg(a, 0), Value::Object(_)))));
    t.insert("is_null", |_, _, a| Ok(Value::Bool(arg(a, 0).is_null())));
    t.insert("is_scalar", |_, _, a| {
        Ok(Value::Bool(matches!(arg(a, 0), Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_))))
    });
    t.insert("is_numeric", |_, _, a| {
        Ok(Value::Bool(match arg(a, 0) {
            Value::Int(_) | Value::Float(_) => true,
            Value::Str(s) => value::parse_numeric(&s).is_some() && !s.last().is_some_and(|c| c.is_ascii_whitespace() && false),
            _ => false,
        }))
    });
    t.insert("is_iterable", |_, _, a| {
        Ok(Value::Bool(match arg(a, 0) {
            Value::Array(_) => true,
            Value::Object(o) => o.class.is_a("traversable"),
            _ => false,
        }))
    });
    t.insert("is_countable", |_, _, a| {
        Ok(Value::Bool(match arg(a, 0) {
            Value::Array(_) => true,
            Value::Object(o) => o.class.is_a("countable"),
            _ => false,
        }))
    });
    t.insert("is_callable", |it, f, a| Ok(Value::Bool(it.to_callable(f, &arg(a, 0)).is_ok())));
    t.insert("intval", |it, _, a| {
        if has(a, 1) {
            let base = int(it, a, 1)?;
            if let Value::Str(st) = arg(a, 0) {
                let txt = String::from_utf8_lossy(&st).trim().to_ascii_lowercase();
                let (neg, txt) = match txt.strip_prefix('-') {
                    Some(r) => (true, r.to_string()),
                    None => (false, txt.trim_start_matches('+').to_string()),
                };
                let (base, txt) = match base {
                    16 => (16, txt.trim_start_matches("0x").to_string()),
                    8 => (8, txt.trim_start_matches("0o").to_string()),
                    2 => (2, txt.trim_start_matches("0b").to_string()),
                    0 if txt.starts_with("0x") => (16, txt[2..].to_string()),
                    0 if txt.starts_with("0b") => (2, txt[2..].to_string()),
                    0 if txt.starts_with('0') && txt.len() > 1 => (8, txt[1..].to_string()),
                    0 => (10, txt),
                    b => (b as u32, txt),
                };
                let digits: String = txt.chars().take_while(|c| c.is_digit(base)).collect();
                let n = i64::from_str_radix(&digits, base).unwrap_or(0);
                return Ok(Value::Int(if neg { -n } else { n }));
            }
        }
        Ok(Value::Int(it.to_int(&arg(a, 0))?))
    });
    t.insert("floatval", |it, _, a| Ok(Value::Float(it.to_float(&arg(a, 0))?)));
    t.insert("doubleval", |it, _, a| Ok(Value::Float(it.to_float(&arg(a, 0))?)));
    t.insert("strval", |it, _, a| Ok(Value::Str(s(it, a, 0)?)));
    t.insert("boolval", |_, _, a| Ok(Value::Bool(arg(a, 0).truthy())));
    t.insert("settype", |it, _, a| {
        let ty = String::from_utf8_lossy(&s(it, a, 1)?).to_ascii_lowercase();
        let ty = match ty.as_str() {
            "integer" => "int",
            "boolean" => "bool",
            "double" => "float",
            "null" => {
                a[0] = Value::Null;
                return Ok(Value::Bool(true));
            }
            other => other,
        }
        .to_string();
        let v = std::mem::take(&mut a[0]);
        a[0] = it.cast(Box::leak(ty.into_boxed_str()), v)?;
        Ok(Value::Bool(true))
    });

    // math
    t.insert("abs", |it, _, a| {
        Ok(match arg(a, 0) {
            Value::Int(n) => n.checked_abs().map(Value::Int).unwrap_or(Value::Float((n as f64).abs())),
            v => Value::Float(it.to_float(&v)?.abs()),
        })
    });
    t.insert("floor", |it, _, a| Ok(Value::Float(float(it, a, 0)?.floor())));
    t.insert("ceil", |it, _, a| Ok(Value::Float(float(it, a, 0)?.ceil())));
    t.insert("round", |it, _, a| {
        let p = if has(a, 1) { int(it, a, 1)? } else { 0 };
        Ok(Value::Float(php_round(float(it, a, 0)?, p)))
    });
    t.insert("sqrt", |it, _, a| Ok(Value::Float(float(it, a, 0)?.sqrt())));
    t.insert("exp", |it, _, a| Ok(Value::Float(float(it, a, 0)?.exp())));
    t.insert("log", |it, _, a| {
        let x = float(it, a, 0)?;
        Ok(Value::Float(if has(a, 1) { x.ln() / float(it, a, 1)?.ln() } else { x.ln() }))
    });
    t.insert("log10", |it, _, a| Ok(Value::Float(float(it, a, 0)?.log10())));
    t.insert("log2", |it, _, a| Ok(Value::Float(float(it, a, 0)?.log2())));
    t.insert("sin", |it, _, a| Ok(Value::Float(float(it, a, 0)?.sin())));
    t.insert("cos", |it, _, a| Ok(Value::Float(float(it, a, 0)?.cos())));
    t.insert("tan", |it, _, a| Ok(Value::Float(float(it, a, 0)?.tan())));
    t.insert("atan", |it, _, a| Ok(Value::Float(float(it, a, 0)?.atan())));
    t.insert("atan2", |it, _, a| Ok(Value::Float(float(it, a, 0)?.atan2(float(it, a, 1)?))));
    t.insert("pi", |_, _, _| Ok(Value::Float(std::f64::consts::PI)));
    t.insert("fmod", |it, _, a| Ok(Value::Float(float(it, a, 0)? % float(it, a, 1)?)));
    t.insert("pow", |it, _, a| it.binop(phpx_syntax::ast::BinOp::Pow, &arg(a, 0), &arg(a, 1)));
    t.insert("intdiv", |it, _, a| {
        let (x, y) = (int(it, a, 0)?, int(it, a, 1)?);
        if y == 0 {
            return Err(it.error("DivisionByZeroError", "Division by zero"));
        }
        x.checked_div(y)
            .map(Value::Int)
            .ok_or_else(|| it.error("ArithmeticError", "Division of PHP_INT_MIN by -1 is not an integer"))
    });
    t.insert("is_nan", |it, _, a| Ok(Value::Bool(float(it, a, 0)?.is_nan())));
    t.insert("is_infinite", |it, _, a| Ok(Value::Bool(float(it, a, 0)?.is_infinite())));
    t.insert("is_finite", |it, _, a| Ok(Value::Bool(float(it, a, 0)?.is_finite())));
    t.insert("max", |it, _, a| min_max(it, a, std::cmp::Ordering::Greater, "max"));
    t.insert("min", |it, _, a| min_max(it, a, std::cmp::Ordering::Less, "min"));
    t.insert("rand", |it, _, a| rand_between(it, a));
    t.insert("mt_rand", |it, _, a| rand_between(it, a));
    t.insert("random_int", |it, _, a| {
        let (lo, hi) = (int(it, a, 0)?, int(it, a, 1)?);
        if hi < lo {
            return Err(it.error("ValueError", "random_int(): Argument #1 ($min) must be less than or equal to argument #2 ($max)"));
        }
        Ok(Value::Int(secure_range(lo, hi)))
    });
    t.insert("mt_srand", |_, _, _| Ok(Value::Null));
    t.insert("mt_getrandmax", |_, _, _| Ok(Value::Int(2147483647)));
    t.insert("getrandmax", |_, _, _| Ok(Value::Int(2147483647)));
    t.insert("lcg_value", |_, _, _| Ok(Value::Float((random_u64() >> 11) as f64 / (1u64 << 53) as f64)));
    t.insert("base_convert", |it, _, a| {
        let txt = String::from_utf8_lossy(&s(it, a, 0)?).to_ascii_lowercase();
        let (from, to) = (int(it, a, 1)? as u32, int(it, a, 2)? as u32);
        let digits: String = txt.chars().filter(|c| c.is_digit(from)).collect();
        let n = u64::from_str_radix(&digits, from).unwrap_or(0);
        Ok(Value::str(&to_radix(n, to)))
    });
    t.insert("dechex", |it, _, a| Ok(Value::str(&format!("{:x}", int(it, a, 0)? as u64))));
    t.insert("decbin", |it, _, a| Ok(Value::str(&format!("{:b}", int(it, a, 0)? as u64))));
    t.insert("decoct", |it, _, a| Ok(Value::str(&format!("{:o}", int(it, a, 0)? as u64))));
    t.insert("hexdec", |it, _, a| from_radix(it, a, 16));
    t.insert("bindec", |it, _, a| from_radix(it, a, 2));
    t.insert("octdec", |it, _, a| from_radix(it, a, 8));

    // classes & functions
    t.insert("get_class", |it, _, a| match arg(a, 0) {
        Value::Object(o) => Ok(Value::str(&o.class.name)),
        other => Err(it.error(
            "TypeError",
            &format!("get_class(): Argument #1 ($object) must be of type object, {} given", other.type_name()),
        )),
    });
    t.insert("get_parent_class", |it, _, a| {
        let cls = match arg(a, 0) {
            Value::Object(o) => Some(o.class.clone()),
            Value::Str(n) => it.lookup_class(&String::from_utf8_lossy(&n)).ok(),
            _ => None,
        };
        Ok(cls.and_then(|c| c.parent.clone()).map(|p| Value::str(&p.name)).unwrap_or(Value::Bool(false)))
    });
    t.insert("get_object_vars", |it, f, a| {
        let Value::Object(o) = arg(a, 0) else { return Ok(Value::Null) };
        let mut out = PhpArray::new();
        for (k, v) in o.props.borrow().iter() {
            if it.prop_visible(f, &o, k) {
                out.insert(Key::from_bytes(k.as_bytes()), v.clone());
            }
        }
        Ok(Value::array(out))
    });
    t.insert("method_exists", |it, _, a| {
        let m = String::from_utf8_lossy(&s(it, a, 1)?).to_ascii_lowercase();
        let cls = match arg(a, 0) {
            Value::Object(o) => Some(o.class.clone()),
            Value::Str(n) => it.lookup_class(&String::from_utf8_lossy(&n)).ok(),
            _ => None,
        };
        Ok(Value::Bool(cls.is_some_and(|c| c.find_method(&m).is_some())))
    });
    t.insert("property_exists", |it, _, a| {
        let p = String::from_utf8_lossy(&s(it, a, 1)?).into_owned();
        Ok(Value::Bool(match arg(a, 0) {
            Value::Object(o) => o.class.find_prop(&p).is_some() || o.props.borrow().contains_key(&*p),
            Value::Str(n) => it.lookup_class(&String::from_utf8_lossy(&n)).ok().is_some_and(|c| c.find_prop(&p).is_some()),
            _ => false,
        }))
    });
    t.insert("class_exists", |it, _, a| class_kind_exists(it, a, phpx_syntax::ast::ClassKind::Class));
    t.insert("interface_exists", |it, _, a| class_kind_exists(it, a, phpx_syntax::ast::ClassKind::Interface));
    t.insert("trait_exists", |it, _, a| class_kind_exists(it, a, phpx_syntax::ast::ClassKind::Trait));
    t.insert("enum_exists", |it, _, a| class_kind_exists(it, a, phpx_syntax::ast::ClassKind::Enum));
    t.insert("function_exists", |it, f, a| {
        let n = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        Ok(Value::Bool(it.find_function(&f.ctx, &format!("\\{}", n.trim_start_matches('\\'))).is_some()))
    });
    t.insert("is_a", |it, _, a| {
        let target = it.lower.get(String::from_utf8_lossy(&s(it, a, 1)?).trim_start_matches('\\'));
        Ok(Value::Bool(match arg(a, 0) {
            Value::Object(o) => o.class.is_a(&target),
            Value::Str(n) if arg(a, 2).truthy() => it.lookup_class(&String::from_utf8_lossy(&n)).is_ok_and(|c| c.is_a(&target)),
            _ => false,
        }))
    });
    t.insert("is_subclass_of", |it, _, a| {
        let target = it.lower.get(String::from_utf8_lossy(&s(it, a, 1)?).trim_start_matches('\\'));
        let cls = match arg(a, 0) {
            Value::Object(o) => Some(o.class.clone()),
            Value::Str(n) => it.lookup_class(&String::from_utf8_lossy(&n)).ok(),
            _ => None,
        };
        Ok(Value::Bool(cls.is_some_and(|c| c.lname != target && c.is_a(&target))))
    });
    t.insert("class_implements", |it, _, a| {
        let cls = match arg(a, 0) {
            Value::Object(o) => o.class.clone(),
            Value::Str(n) => it.lookup_class(&String::from_utf8_lossy(&n))?,
            _ => return Ok(Value::Bool(false)),
        };
        let mut out = PhpArray::new();
        for anc in cls.ancestors.iter().skip(1) {
            if let Some(c) = it.classes.borrow().get(anc) {
                if c.kind == phpx_syntax::ast::ClassKind::Interface {
                    out.insert(Key::from_bytes(c.name.as_bytes()), Value::str(&c.name));
                }
            }
        }
        Ok(Value::array(out))
    });
    t.insert("spl_object_id", |_, _, a| Ok(Value::Int(arg(a, 0).as_object().map(|o| o.id as i64).unwrap_or(0))));
    t.insert("spl_object_hash", |_, _, a| {
        Ok(Value::str(&format!("{:032x}", arg(a, 0).as_object().map(|o| o.id as u128).unwrap_or(0))))
    });
    t.insert("spl_autoload_register", |it, f, a| {
        if !has(a, 0) || arg(a, 0).is_null() {
            return Ok(Value::Bool(true));
        }
        let c = callable(it, f, a, 0)?;
        if arg(a, 2).truthy() {
            it.autoloaders.borrow_mut().insert(0, c);
        } else {
            it.autoloaders.borrow_mut().push(c);
        }
        Ok(Value::Bool(true))
    });
    t.insert("spl_autoload_unregister", |_, _, _| Ok(Value::Bool(true)));
    t.insert("spl_autoload_functions", |it, _, _| {
        let fs: Vec<Value> = it.autoloaders.borrow().iter().map(|c| it.closure_object(c.clone())).collect();
        Ok(Value::array(PhpArray::from_list(fs)))
    });
    t.insert("iterator_to_array", |it, _, a| {
        let preserve = !has(a, 1) || arg(a, 1).truthy();
        match arg(a, 0) {
            Value::Array(x) => Ok(Value::Array(x)),
            Value::Object(o) => {
                let mut out = PhpArray::new();
                for (k, v) in it.iterate(&o)? {
                    if preserve {
                        out.insert(it.to_key(&k)?, v);
                    } else {
                        out.push(v);
                    }
                }
                Ok(Value::array(out))
            }
            _ => Err(it.error("TypeError", "iterator_to_array(): Argument #1 must be of type Traversable|array")),
        }
    });
    t.insert("call_user_func", |it, f, a| {
        let c = callable(it, f, a, 0)?;
        it.call_callable(&c, a[1..].to_vec(), vec![])
    });
    t.insert("call_user_func_array", |it, f, a| {
        let c = callable(it, f, a, 0)?;
        let args = arr(it, a, 1, "call_user_func_array")?;
        let mut pos = vec![];
        let mut named = vec![];
        for (k, v) in args.iter() {
            match k {
                Key::Int(_) => pos.push(v.clone()),
                Key::Str(n) => named.push((Rc::from(String::from_utf8_lossy(&n).as_ref()), v.clone())),
            }
        }
        it.call_callable(&c, pos, named)
    });
    t.insert("define", |it, _, a| {
        let n = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        if it.constants.borrow().contains_key(&n) {
            it.warn(&format!("Constant {n} already defined"));
            return Ok(Value::Bool(false));
        }
        it.constants.borrow_mut().insert(n, arg(a, 1));
        Ok(Value::Bool(true))
    });
    t.insert("defined", |it, _, a| {
        let n = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        Ok(Value::Bool(it.constants.borrow().contains_key(n.trim_start_matches('\\'))))
    });
    t.insert("constant", |it, f, a| {
        let n = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        if let Some((cls, c)) = n.split_once("::") {
            let class = it.lookup_class(cls)?;
            return it.get_class_const(&class, c);
        }
        it.constant(f, &format!("\\{}", n.trim_start_matches('\\')))
    });

    // output buffering
    t.insert("ob_start", |it, _, _| {
        it.out.borrow_mut().push(vec![]);
        Ok(Value::Bool(true))
    });
    t.insert("ob_get_clean", |it, _, _| {
        let mut out = it.out.borrow_mut();
        if out.len() < 2 {
            return Ok(Value::Bool(false));
        }
        Ok(Value::bytes(out.pop().unwrap()))
    });
    t.insert("ob_get_contents", |it, _, _| {
        let out = it.out.borrow();
        if out.len() < 2 {
            return Ok(Value::Bool(false));
        }
        Ok(Value::bytes(out.last().unwrap().clone()))
    });
    t.insert("ob_end_clean", |it, _, _| {
        let mut out = it.out.borrow_mut();
        if out.len() < 2 {
            return Ok(Value::Bool(false));
        }
        out.pop();
        Ok(Value::Bool(true))
    });
    t.insert("ob_end_flush", |it, _, _| {
        let mut out = it.out.borrow_mut();
        if out.len() < 2 {
            return Ok(Value::Bool(false));
        }
        let top = out.pop().unwrap();
        out.last_mut().unwrap().extend_from_slice(&top);
        Ok(Value::Bool(true))
    });
    t.insert("ob_get_level", |it, _, _| Ok(Value::Int(it.out.borrow().len() as i64 - 1)));
    t.insert("flush", |it, _, _| {
        if it.out.borrow().len() == 1 {
            it.flush();
        }
        Ok(Value::Null)
    });

    // environment
    // Extensions phpx implements natively. Others report false so Composer
    // polyfills (symfony/polyfill-*) fill the gaps.
    t.insert("extension_loaded", |it, _, a| {
        let n = String::from_utf8_lossy(&s(it, a, 0)?).to_ascii_lowercase();
        Ok(Value::Bool(EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(&n))))
    });
    t.insert("get_loaded_extensions", |_, _, _| {
        Ok(Value::array(PhpArray::from_list(EXTENSIONS.iter().map(|e| Value::str(e)).collect::<Vec<_>>())))
    });
    // Diagnostic: native stack bytes left in this green thread.
    t.insert("__phpx_stack_remaining", |_, _, _| Ok(Value::Int(phpx_runtime::stack_remaining().unwrap_or(0) as i64)));
    t.insert("php_sapi_name", |_, _, _| Ok(Value::str("cli")));
    t.insert("phpversion", |_, _, _| Ok(Value::str("8.5.10")));
    t.insert("error_reporting", |_, _, _| Ok(Value::Int(32767)));
    t.insert("ini_set", |_, _, _| Ok(Value::Bool(false)));
    t.insert("ini_get", |_, _, _| Ok(Value::Bool(false)));
    t.insert("set_error_handler", |_, _, _| Ok(Value::Null));
    t.insert("set_exception_handler", |_, _, _| Ok(Value::Null));
    t.insert("date_default_timezone_set", |_, _, _| Ok(Value::Bool(true)));
    t.insert("date_default_timezone_get", |_, _, _| Ok(Value::str("UTC")));
    t.insert("memory_get_usage", |_, _, _| Ok(Value::Int(2 * 1024 * 1024)));
    t.insert("memory_get_peak_usage", |_, _, _| Ok(Value::Int(2 * 1024 * 1024)));
    t.insert("gc_collect_cycles", |it, _, _| {
        // Destructors of cyclic garbage run, then the cycles are freed.
        let mut n = crate::gc::collect();
        it.run_destructors()?;
        n += crate::gc::collect();
        Ok(Value::Int(n as i64))
    });
    t.insert("gc_enable", |_, _, _| {
        crate::gc::set_enabled(true);
        Ok(Value::Null)
    });
    t.insert("gc_disable", |_, _, _| {
        crate::gc::set_enabled(false);
        Ok(Value::Null)
    });
    t.insert("gc_enabled", |_, _, _| Ok(Value::Bool(crate::gc::enabled())));
    t.insert("gc_mem_caches", |_, _, _| Ok(Value::Int(0)));
    t.insert("gc_status", |_, _, _| {
        let (runs, collected, threshold, roots) = crate::gc::status();
        let mut m = PhpArray::new();
        for (k, v) in [("runs", runs as i64), ("collected", collected as i64), ("threshold", threshold as i64), ("roots", roots as i64)] {
            m.insert(Key::Str(value::pstr(k)), Value::Int(v));
        }
        Ok(Value::array(m))
    });
    t.insert("getenv", |it, _, a| {
        if !has(a, 0) {
            let mut out = PhpArray::new();
            for (k, v) in std::env::vars() {
                out.insert(Key::from_bytes(k.as_bytes()), Value::str(&v));
            }
            return Ok(Value::array(out));
        }
        let n = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        Ok(std::env::var(n).map(|v| Value::str(&v)).unwrap_or(Value::Bool(false)))
    });
    t.insert("uniqid", |it, _, a| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let prefix = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        Ok(Value::str(&format!("{prefix}{:08x}{:05x}", now.as_secs(), now.subsec_micros())))
    });
    t.insert("random_bytes", |it, _, a| {
        let n = int(it, a, 0)?;
        if n < 1 {
            return Err(it.error("ValueError", "random_bytes(): Argument #1 ($length) must be greater than 0"));
        }
        Ok(Value::bytes(secure_bytes(n as usize)))
    });
    t.insert("bin2hex", |it, _, a| Ok(Value::str(&s(it, a, 0)?.iter().map(|b| format!("{b:02x}")).collect::<String>())));
    t.insert("hex2bin", |it, _, a| {
        let h = s(it, a, 0)?;
        if h.len() % 2 != 0 {
            it.warn("hex2bin(): Hexadecimal input string must have an even length");
            return Ok(Value::Bool(false));
        }
        let out: Option<Vec<u8>> = h
            .chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
            .collect();
        Ok(out.map(Value::bytes).unwrap_or(Value::Bool(false)))
    });

    // time
    t.insert("time", |_, _, _| Ok(Value::Int(now().as_secs() as i64)));
    t.insert("microtime", |_, _, a| {
        let d = now();
        if arg(a, 0).truthy() {
            Ok(Value::Float(d.as_secs_f64()))
        } else {
            Ok(Value::str(&format!("{:.8} {}", d.subsec_nanos() as f64 / 1e9, d.as_secs())))
        }
    });
    t.insert("hrtime", |_, _, a| {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let ns = START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as i64;
        if arg(a, 0).truthy() {
            Ok(Value::Int(ns))
        } else {
            Ok(Value::array(PhpArray::from_list([Value::Int(ns / 1_000_000_000), Value::Int(ns % 1_000_000_000)])))
        }
    });
    t.insert("date", |it, _, a| {
        let ts = if has(a, 1) && !arg(a, 1).is_null() { int(it, a, 1)? } else { now().as_secs() as i64 };
        Ok(Value::str(&format_date(&String::from_utf8_lossy(&s(it, a, 0)?), ts)))
    });
    t.insert("mktime", |it, _, a| {
        let n = now().as_secs() as i64;
        let (y, mo, d) = civil_from_days(n.div_euclid(86400));
        let sod = n.rem_euclid(86400);
        let get = |i: usize, dflt: i64| -> R<i64> { if has(a, i) && !arg(a, i).is_null() { int(it, a, i) } else { Ok(dflt) } };
        let (h, mi, sec) = (get(0, sod / 3600)?, get(1, sod / 60 % 60)?, get(2, sod % 60)?);
        let (mo, d, y) = (get(3, mo)?, get(4, d)?, get(5, y)?);
        Ok(Value::Int(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + sec))
    });
    t.insert("checkdate", |it, _, a| {
        let (m, d, y) = (int(it, a, 0)?, int(it, a, 1)?, int(it, a, 2)?);
        Ok(Value::Bool((1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m) && y >= 1))
    });

    // async
    t.insert("async", |it, f, a| {
        let c = callable(it, f, a, 0)?;
        Ok(crate::native::async_call(it, c))
    });
    t.insert("defer", |it, f, a| {
        let c = callable(it, f, a, 0)?;
        crate::native::defer_call(it, c);
        Ok(Value::Null)
    });
    t.insert("sleep", |it, _, a| {
        let secs = int(it, a, 0)?;
        if secs < 0 {
            return Err(it.error("ValueError", "sleep(): Argument #1 ($seconds) must be greater than or equal to 0"));
        }
        phpx_runtime::sleep(Duration::from_secs(secs as u64)).map_err(|e| crate::native::rt_error(it, e))?;
        Ok(Value::Int(0))
    });
    t.insert("usleep", |it, _, a| {
        let us = int(it, a, 0)?.max(0) as u64;
        phpx_runtime::sleep(Duration::from_micros(us)).map_err(|e| crate::native::rt_error(it, e))?;
        Ok(Value::Null)
    });
    t.insert("exit", |_, _, _| Err(Ctrl::Exit(0)));
    // Scope-reading functions: handled in call_named; these entries only make
    // them resolvable (function_exists, call_user_func gives an error).
    for name in ["compact", "extract", "eval", "func_get_args", "func_num_args", "get_called_class", "get_defined_vars"] {
        t.insert(name, |it, _, _| Err(it.error("Error", "Cannot call scope-reading function dynamically")));
    }
}

fn class_kind_exists(it: &Interp, a: &[Value], kind: phpx_syntax::ast::ClassKind) -> R<Value> {
    let n = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
    let autoload = !has(a, 1) || arg(a, 1).truthy();
    let c = if autoload {
        it.find_class(&n)?
    } else {
        it.classes.borrow().get(&*it.lower.get(n.trim_start_matches('\\'))).cloned()
    };
    Ok(Value::Bool(c.is_some_and(|c| c.kind == kind)))
}

fn min_max(it: &Interp, a: &[Value], want: std::cmp::Ordering, name: &str) -> R<Value> {
    let vals: Vec<Value> = if a.len() == 1 {
        match &a[0] {
            Value::Array(x) => x.values().collect(),
            other => {
                return Err(it.error(
                    "TypeError",
                    &format!("{name}(): Argument #1 ($value) must be of type array, {} given", other.type_name()),
                ))
            }
        }
    } else {
        a.to_vec()
    };
    if vals.is_empty() {
        return Err(it.error("ValueError", &format!("{name}(): Argument #1 ($value) must contain at least one element")));
    }
    let mut best = vals[0].clone();
    for v in &vals[1..] {
        if value::compare(v, &best) == Some(want) {
            best = v.clone();
        }
    }
    Ok(best)
}

/// Cryptographically secure bytes from the OS (random_bytes(), tokens).
pub fn secure_bytes(n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).expect("/dev/urandom unavailable");
    buf
}

pub fn random_u64() -> u64 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = Cell::new(
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9E3779B97F4A7C15) | 1,
        );
    }
    // xorshift64*: fine for rand()/shuffle(); not for secrets.
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        s.set(x);
        x.wrapping_mul(0x2545F4914F6CDD1D)
    })
}

/// Uniform integer in [lo, hi] from the OS CSPRNG (rejection sampling,
/// no modulo bias). For random_int().
pub fn secure_range(lo: i64, hi: i64) -> i64 {
    let span = (hi as i128 - lo as i128) as u128 + 1;
    if span > u64::MAX as u128 {
        return i64::from_le_bytes(secure_bytes(8).try_into().unwrap());
    }
    let span = span as u64;
    let zone = u64::MAX - (u64::MAX % span);
    loop {
        let r = u64::from_le_bytes(secure_bytes(8).try_into().unwrap());
        if r < zone {
            return (lo as i128 + (r % span) as i128) as i64;
        }
    }
}

fn rand_between(it: &Interp, a: &[Value]) -> R<Value> {
    let (lo, hi) = if has(a, 1) { (int(it, a, 0)?, int(it, a, 1)?) } else { (0, 2147483647) };
    if hi < lo {
        return Err(it.error("ValueError", "Argument #1 ($min) must be less than or equal to argument #2 ($max)"));
    }
    let span = (hi as i128 - lo as i128 + 1) as u128;
    Ok(Value::Int((lo as i128 + (random_u64() as u128 % span) as i128) as i64))
}

fn to_radix(mut n: u64, radix: u32) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut out = vec![];
    while n > 0 {
        out.push(std::char::from_digit((n % radix as u64) as u32, radix).unwrap());
        n /= radix as u64;
    }
    out.iter().rev().collect()
}

fn from_radix(it: &Interp, a: &[Value], radix: u32) -> R<Value> {
    let txt = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
    let digits: String = txt.chars().filter(|c| c.is_digit(radix)).collect();
    Ok(match i64::from_str_radix(&digits, radix) {
        Ok(n) => Value::Int(n),
        Err(_) if digits.is_empty() => Value::Int(0),
        Err(_) => Value::Float(digits.chars().fold(0f64, |acc, c| acc * radix as f64 + c.to_digit(radix).unwrap() as f64)),
    })
}

/// PHP's round(): half away from zero, after pre-rounding to 15
/// significant digits so 1.005 rounds to 1.01 like PHP.
pub fn php_round(v: f64, places: i64) -> f64 {
    if !v.is_finite() {
        return v;
    }
    let f = 10f64.powi(places.clamp(-300, 300) as i32);
    let x = v * f;
    let pre: f64 = format!("{x:.15e}").parse().unwrap_or(x);
    let r = pre.round();
    let out = r / f;
    if out.is_finite() { out } else { v }
}

fn now() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
}

// ---- date (UTC only) -----------------------------------------------------------

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    // Normalize month overflow like mktime.
    let (y, m) = (y + (m - 1).div_euclid(12), (m - 1).rem_euclid(12) + 1);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

pub fn format_date(fmt: &str, ts: i64) -> String {
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const MONTHS: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let days = ts.div_euclid(86400);
    let sod = ts.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let (h, mi, sec) = (sod / 3600, sod / 60 % 60, sod % 60);
    let dow = (days + 4).rem_euclid(7); // 1970-01-01 was a Thursday
    let doy = days - days_from_civil(y, 1, 1);
    // ISO-8601 week number and year.
    let iso_dow = if dow == 0 { 7 } else { dow };
    let thursday = days - iso_dow + 4;
    let (iso_y, _, _) = civil_from_days(thursday);
    let iso_week = (thursday - days_from_civil(iso_y, 1, 1)) / 7 + 1;
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        match c {
            'd' => out.push_str(&format!("{d:02}")),
            'D' => out.push_str(&DAYS[dow as usize][..3]),
            'j' => out.push_str(&d.to_string()),
            'l' => out.push_str(DAYS[dow as usize]),
            'N' => out.push_str(&iso_dow.to_string()),
            'S' => out.push_str(match d {
                1 | 21 | 31 => "st",
                2 | 22 => "nd",
                3 | 23 => "rd",
                _ => "th",
            }),
            'w' => out.push_str(&dow.to_string()),
            'z' => out.push_str(&doy.to_string()),
            'W' => out.push_str(&format!("{iso_week:02}")),
            'F' => out.push_str(MONTHS[m as usize - 1]),
            'M' => out.push_str(&MONTHS[m as usize - 1][..3]),
            'm' => out.push_str(&format!("{m:02}")),
            'n' => out.push_str(&m.to_string()),
            't' => out.push_str(&days_in_month(y, m).to_string()),
            'L' => out.push_str(if is_leap(y) { "1" } else { "0" }),
            'o' => out.push_str(&iso_y.to_string()),
            'Y' => out.push_str(&y.to_string()),
            'y' => out.push_str(&format!("{:02}", y % 100)),
            'a' => out.push_str(if h < 12 { "am" } else { "pm" }),
            'A' => out.push_str(if h < 12 { "AM" } else { "PM" }),
            'g' => out.push_str(&(if h % 12 == 0 { 12 } else { h % 12 }).to_string()),
            'G' => out.push_str(&h.to_string()),
            'h' => out.push_str(&format!("{:02}", if h % 12 == 0 { 12 } else { h % 12 })),
            'H' => out.push_str(&format!("{h:02}")),
            'i' => out.push_str(&format!("{mi:02}")),
            's' => out.push_str(&format!("{sec:02}")),
            'u' => out.push_str("000000"),
            'v' => out.push_str("000"),
            'e' => out.push_str("UTC"),
            'T' => out.push_str("UTC"),
            'P' => out.push_str("+00:00"),
            'p' => out.push('Z'),
            'O' => out.push_str("+0000"),
            'Z' => out.push('0'),
            'I' => out.push('0'),
            'U' => out.push_str(&ts.to_string()),
            'c' => out.push_str(&format_date("Y-m-d\\TH:i:sP", ts)),
            'r' => out.push_str(&format_date("D, d M Y H:i:s O", ts)),
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(format_date("Y-m-d H:i:s D N W", 0), "1970-01-01 00:00:00 Thu 4 01");
        assert_eq!(format_date("Y-m-d l jS F", 1_700_000_000), "2023-11-14 Tuesday 14th November");
        assert_eq!(days_from_civil(2024, 2, 29), 19782);
        assert_eq!(civil_from_days(19782), (2024, 2, 29));
    }

    #[test]
    fn rounding() {
        assert_eq!(php_round(1.005, 2), 1.01);
        assert_eq!(php_round(2.5, 0), 3.0);
        assert_eq!(php_round(-2.5, 0), -3.0);
        assert_eq!(php_round(1234.5678, -2), 1200.0);
    }
}
