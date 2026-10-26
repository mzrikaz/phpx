//! ext/intl: NumberFormatter (numfmt.rs, with ICU's locale data from
//! locales_table.rs), Locale's parsing helpers, idn_to_ascii/idn_to_utf8
//! and the intl error functions. Classes are declared in prelude_intl.php.

mod grapheme;
mod locales_table;
pub mod numfmt;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::{arg, has, int, s};
use crate::class::Class;
use crate::interp::{BuiltinFn, Interp, NativeMethod, R};
use crate::value::{pstr, Internal, Key, NativeData, Obj, PhpArray, Value};
use numfmt::{Ctx, Dec, LocaleData, Pattern};

// ---- errors ---------------------------------------------------------------------

const U_ZERO_ERROR: i64 = 0;
const U_ILLEGAL_ARGUMENT_ERROR: i64 = 1;
const U_PARSE_ERROR: i64 = 9;
const U_UNSUPPORTED_ERROR: i64 = 16;

fn error_name(code: i64) -> &'static str {
    match code {
        U_ZERO_ERROR => "U_ZERO_ERROR",
        U_ILLEGAL_ARGUMENT_ERROR => "U_ILLEGAL_ARGUMENT_ERROR",
        U_PARSE_ERROR => "U_PARSE_ERROR",
        U_UNSUPPORTED_ERROR => "U_UNSUPPORTED_ERROR",
        _ => "[BOGUS UErrorCode]",
    }
}

thread_local! {
    /// intl_get_error_code()/intl_get_error_message().
    static LAST: RefCell<(i64, String)> = RefCell::new((U_ZERO_ERROR, "U_ZERO_ERROR".into()));
    /// Locale::setDefault().
    static DEFAULT: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn set_global(code: i64, msg: &str) {
    LAST.with(|l| *l.borrow_mut() = (code, if code == U_ZERO_ERROR { "U_ZERO_ERROR".into() } else { format!("{msg}: {}", error_name(code)) }));
}

/// A new request starts with no error and the environment's locale.
pub(crate) fn reset() {
    set_global(U_ZERO_ERROR, "");
    DEFAULT.with(|d| *d.borrow_mut() = None);
}

/// ICU's default locale: intl.default_locale, else LC_ALL / LC_MESSAGES /
/// LANG (C and POSIX are en_US_POSIX).
fn default_locale(it: &Interp) -> String {
    if let Some(d) = DEFAULT.with(|d| d.borrow().clone()) {
        return d;
    }
    if let Some(v) = it.ini_get("intl.default_locale").filter(|v| !v.is_empty()) {
        return numfmt::canonicalize(&v).0;
    }
    let env = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())).unwrap_or_default();
    let base = env.split(['.', '@']).next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return "en_US_POSIX".into();
    }
    numfmt::canonicalize(base).0
}

// ---- NumberFormatter ---------------------------------------------------------------

const STYLE_PATTERN_DECIMAL: i64 = 0;
const STYLE_DECIMAL: i64 = 1;
const STYLE_CURRENCY: i64 = 2;
const STYLE_PERCENT: i64 = 3;
const STYLE_SCIENTIFIC: i64 = 4;
const STYLE_ACCOUNTING: i64 = 12;

struct Fmt {
    loc: &'static LocaleData,
    style: i64,
    pattern_src: String,
    /// The pattern was changed by attributes: getPattern() rebuilds it.
    pattern_dirty: bool,
    pattern: Pattern,
    symbols: [String; 12],
    currency: String,
    rounding_mode: i64,
    affixes: [Option<String>; 4],
    /// Fraction digits set explicitly (formatCurrency keeps them).
    frac_set: bool,
    parse_int_only: bool,
    lenient: bool,
    error: (i64, String),
}

struct FmtData(RefCell<Fmt>);

impl NativeData for FmtData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        let f = self.0.borrow();
        Some(Box::new(FmtData(RefCell::new(Fmt {
            loc: f.loc,
            style: f.style,
            pattern_src: f.pattern_src.clone(),
            pattern_dirty: f.pattern_dirty,
            pattern: f.pattern.clone(),
            symbols: f.symbols.clone(),
            currency: f.currency.clone(),
            rounding_mode: f.rounding_mode,
            affixes: f.affixes.clone(),
            frac_set: f.frac_set,
            parse_int_only: f.parse_int_only,
            lenient: f.lenient,
            error: f.error.clone(),
        }))))
    }
}

fn with_fmt<T>(it: &Interp, o: Option<&Rc<Obj>>, f: impl FnOnce(&mut Fmt) -> T) -> R<T> {
    let o = o.ok_or_else(|| it.error("Error", "Non-static method called statically"))?;
    let g = o.internal();
    match &*g {
        Internal::Data(d) => match d.as_any().downcast_ref::<FmtData>() {
            Some(d) => Ok(f(&mut d.0.borrow_mut())),
            None => Err(it.error("Error", "Object not initialized")),
        },
        _ => Err(it.error("Error", "Object not initialized")),
    }
}

/// Record an operation's outcome on the formatter and globally.
fn status(f: &mut Fmt, code: i64, msg: &str) {
    f.error = (code, if code == U_ZERO_ERROR { "U_ZERO_ERROR".into() } else { format!("{msg}: {}", error_name(code)) });
    set_global(code, msg);
}

fn new_fmt(it: &Interp, func: &str, locale: &str, style: i64, pattern: Option<String>) -> R<Result<Fmt, ()>> {
    let name = if locale.is_empty() { default_locale(it) } else { locale.to_string() };
    let Some(loc) = numfmt::resolve(&name) else {
        return Err(it.error("ValueError", &format!("{func}: Argument #1 ($locale) \"{locale}\" is invalid")));
    };
    let src = match style {
        STYLE_PATTERN_DECIMAL => match pattern {
            Some(p) if !p.is_empty() => p,
            _ => "#".into(),
        },
        STYLE_DECIMAL => loc.patterns[0].into(),
        STYLE_PERCENT => loc.patterns[1].into(),
        STYLE_CURRENCY => loc.patterns[2].into(),
        STYLE_ACCOUNTING => loc.patterns[3].into(),
        STYLE_SCIENTIFIC => loc.patterns[4].into(),
        _ => return Ok(Err(())),
    };
    let Some(pattern) = Pattern::parse(&src) else { return Ok(Err(())) };
    let currency = numfmt::keyword_currency(&name).unwrap_or_else(|| loc.currency.to_string());
    let mut f = Fmt {
        loc,
        style,
        pattern_src: src,
        pattern_dirty: false,
        pattern,
        symbols: loc.symbols.map(String::from),
        currency,
        rounding_mode: 4,
        affixes: Default::default(),
        frac_set: false,
        parse_int_only: false,
        lenient: false,
        error: (U_ZERO_ERROR, "U_ZERO_ERROR".into()),
    };
    if f.pattern.is_currency() {
        apply_currency_digits(&mut f);
    }
    Ok(Ok(f))
}

/// A currency style shows the currency's fraction digits.
fn apply_currency_digits(f: &mut Fmt) {
    if f.frac_set || f.pattern.sig.is_some() {
        return;
    }
    let d = numfmt::currency_digits(&f.currency);
    f.pattern.min_frac = d;
    f.pattern.max_frac = d;
}

fn ctx(f: &Fmt) -> Ctx<'_> {
    Ctx { loc: f.loc, locale: f.loc.name, pattern: &f.pattern, symbols: &f.symbols, currency: &f.currency, rounding_mode: f.rounding_mode, affixes: &f.affixes }
}

/// The value to format: (negative, digits or None for NaN, infinite).
fn number_of(v: &Value, ty: i64) -> (bool, Option<Dec>, bool) {
    let as_int = |n: i64| (n < 0, Some(Dec::from_u128(n.unsigned_abs() as u128)), false);
    match (v, ty) {
        (Value::Int(n), 0 | 2) => as_int(*n),
        (Value::Int(n), 1) => as_int(*n as i32 as i64),
        (Value::Float(x), 1) => as_int(if x.is_finite() { *x as i64 as i32 as i64 } else { 0 }),
        (Value::Float(x), 2) => as_int(if x.is_finite() { *x as i64 } else { 0 }),
        (Value::Int(n), _) => float_of(*n as f64),
        (Value::Float(x), _) => float_of(*x),
        _ => as_int(0),
    }
}

fn float_of(x: f64) -> (bool, Option<Dec>, bool) {
    if x.is_nan() {
        return (false, None, false);
    }
    (x.is_sign_negative(), Some(if x.is_infinite() { Dec::from_u128(0) } else { Dec::from_f64(x) }), x.is_infinite())
}

fn format_value(f: &Fmt, v: &Value, ty: i64) -> String {
    let (neg, dec, inf) = number_of(v, ty);
    numfmt::format(&ctx(f), neg, dec, inf)
}

/// Rebuild a pattern string from its parts (getPattern() after changes).
fn to_pattern(f: &Fmt) -> String {
    let p = &f.pattern;
    let affix = |parts: &[numfmt::Affix]| {
        parts
            .iter()
            .map(|a| match a {
                numfmt::Affix::Lit(l) => {
                    if l.chars().any(|c| "#0123456789@.,;%‰¤-+E'*".contains(c)) {
                        format!("'{}'", l.replace('\'', "''"))
                    } else {
                        l.clone()
                    }
                }
                numfmt::Affix::Currency(n) => "¤".repeat(*n),
                numfmt::Affix::Percent => "%".into(),
                numfmt::Affix::Permill => "‰".into(),
                numfmt::Affix::Minus => "-".into(),
                numfmt::Affix::Plus => "+".into(),
            })
            .collect::<String>()
    };
    let mut num = String::new();
    let min_int = p.min_int.max(1);
    if p.grouping_used && p.grouping > 0 {
        let width = (p.grouping + if p.grouping2 != p.grouping { p.grouping2 } else { 0 } + 1).max(min_int + 1);
        let mut chars = vec![];
        for i in 0..width {
            if i > 0 && (i == p.grouping || (p.grouping2 != p.grouping && i > p.grouping && (i - p.grouping) % p.grouping2.max(1) == 0)) {
                chars.push(',');
            }
            chars.push(if i < min_int { '0' } else { '#' });
        }
        num.extend(chars.iter().rev());
    } else {
        num.push_str(&"0".repeat(min_int as usize));
    }
    if p.max_frac > 0 || p.decimal_always {
        num.push('.');
        num.push_str(&"0".repeat(p.min_frac as usize));
        num.push_str(&"#".repeat(p.max_frac.saturating_sub(p.min_frac) as usize));
    }
    let mut out = format!("{}{}{}", affix(&p.pos_prefix), num, affix(&p.pos_suffix));
    if let Some((np, ns)) = &p.neg {
        out = format!("{out};{}{}{}", affix(np), num, affix(ns));
    }
    out
}

fn get_attr(f: &Fmt, a: i64) -> Option<Value> {
    let p = &f.pattern;
    Some(match a {
        0 => Value::Int(f.parse_int_only as i64),
        1 => Value::Int(p.grouping_used as i64),
        2 => Value::Int(p.decimal_always as i64),
        3 => Value::Int(if p.max_int == u32::MAX { 2000000000 } else { p.max_int as i64 }),
        4 | 5 => Value::Int(p.min_int as i64),
        6 => Value::Int(p.max_frac as i64),
        7 | 8 => Value::Int(p.min_frac as i64),
        9 => Value::Int(p.multiplier),
        10 => Value::Int(if p.grouping_used { p.grouping as i64 } else { 0 }),
        11 => Value::Int(f.rounding_mode),
        12 => Value::Float(0.0),
        14 => Value::Int(0),
        15 => Value::Int(if p.grouping2 != p.grouping { p.grouping2 as i64 } else { 0 }),
        16 => Value::Int(p.sig.is_some() as i64),
        17 if p.sig.is_some() => Value::Int(p.sig.unwrap().0 as i64),
        18 if p.sig.is_some() => Value::Int(p.sig.unwrap().1 as i64),
        19 => Value::Int(f.lenient as i64),
        _ => return None,
    })
}

fn set_attr(f: &mut Fmt, a: i64, v: f64) -> bool {
    let n = v as i64;
    let u = n.clamp(0, u32::MAX as i64 - 1) as u32;
    let p = &mut f.pattern;
    match a {
        0 => f.parse_int_only = n != 0,
        1 => p.grouping_used = n != 0,
        2 => p.decimal_always = n != 0,
        3 => {
            p.max_int = u;
            p.min_int = p.min_int.min(u);
        }
        4 => {
            p.min_int = u;
            p.max_int = p.max_int.max(u);
        }
        5 => {
            p.min_int = u;
            p.max_int = u;
        }
        6 => {
            p.max_frac = u;
            p.min_frac = p.min_frac.min(u);
            f.frac_set = true;
        }
        7 => {
            p.min_frac = u;
            p.max_frac = p.max_frac.max(u);
            f.frac_set = true;
        }
        8 => {
            p.min_frac = u;
            p.max_frac = u;
            f.frac_set = true;
        }
        9 => p.multiplier = n,
        10 => {
            p.grouping = u;
            if p.grouping2 == 0 {
                p.grouping2 = u;
            }
        }
        11 if (0..=8).contains(&n) => f.rounding_mode = n,
        15 => p.grouping2 = u,
        16 => {
            p.sig = if n != 0 { Some(p.sig.unwrap_or((1, 6))) } else { None };
        }
        17 => {
            let (_, max) = p.sig.unwrap_or((1, 6));
            p.sig = Some((u.max(1), max.max(u)));
        }
        18 => {
            let (min, _) = p.sig.unwrap_or((1, 6));
            p.sig = Some((min.min(u.max(1)), u.max(1)));
        }
        19 => f.lenient = n != 0,
        12 | 13 | 14 => {}
        _ => return false,
    }
    f.pattern_dirty = true;
    true
}

fn int_arg(it: &Interp, a: &[Value], i: usize) -> R<i64> {
    int(it, a, i)
}

fn str_arg(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(crate::pstr::lossy(&s(it, a, i)?).into_owned())
}

fn construct(it: &Interp, o: &Rc<Obj>, a: &[Value], func: &str) -> R<bool> {
    let locale = str_arg(it, a, 0)?;
    let style = int_arg(it, a, 1)?;
    let pattern = match arg(a, 2) {
        Value::Null => None,
        v => Some(crate::pstr::lossy(&it.to_str(&v)?).into_owned()),
    };
    match new_fmt(it, func, &locale, style, pattern)? {
        Ok(f) => {
            o.set_internal(Internal::Data(Box::new(FmtData(RefCell::new(f)))));
            set_global(U_ZERO_ERROR, "");
            Ok(true)
        }
        Err(()) => Ok(false),
    }
}

fn nf_construct(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = o.unwrap();
    if !construct(it, o, &a, "NumberFormatter::__construct()")? {
        set_global(U_UNSUPPORTED_ERROR, "NumberFormatter::__construct(): number formatter creation failed");
        return Err(it.error("IntlException", "NumberFormatter::__construct(): number formatter creation failed"));
    }
    Ok(Value::Null)
}

fn nf_create(it: &Interp, _: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = it.new_instance(cls)?;
    if !construct(it, &o, &a, "NumberFormatter::create()")? {
        set_global(U_UNSUPPORTED_ERROR, "NumberFormatter::create(): number formatter creation failed");
        return Ok(Value::Null);
    }
    Ok(Value::Object(o))
}

fn nf_format(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let v = arg(&a, 0);
    let v = match v {
        Value::Int(_) | Value::Float(_) => v,
        Value::Bool(b) => Value::Int(b as i64),
        Value::Null => Value::Int(0),
        Value::Str(ref b) => match crate::value::parse_numeric(b) {
            Some(crate::value::Num::Int(n)) => Value::Int(n),
            Some(crate::value::Num::Float(x)) => Value::Float(x),
            None => return Err(it.error("TypeError", &format!("NumberFormatter::format(): Argument #1 ($num) must be of type int|float, {} given", v.type_name()))),
        },
        other => return Err(it.error("TypeError", &format!("NumberFormatter::format(): Argument #1 ($num) must be of type int|float, {} given", other.type_name()))),
    };
    let ty = if has(&a, 1) { int_arg(it, &a, 1)? } else { 0 };
    if !(0..=3).contains(&ty) {
        return Err(it.error("ValueError", "NumberFormatter::format(): Argument #2 ($type) must be a NumberFormatter::TYPE_* constant"));
    }
    with_fmt(it, o, |f| {
        let s = format_value(f, &v, ty);
        status(f, U_ZERO_ERROR, "");
        Value::str(&s)
    })
}

fn nf_format_currency(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let amount = it.to_float(&arg(&a, 0))?;
    let currency = str_arg(it, &a, 1)?.to_ascii_uppercase();
    with_fmt(it, o, |f| {
        let saved = (f.currency.clone(), f.pattern.min_frac, f.pattern.max_frac);
        f.currency = currency;
        if f.pattern.is_currency() {
            apply_currency_digits(f);
        }
        let s = format_value(f, &Value::Float(amount), 3);
        (f.currency, f.pattern.min_frac, f.pattern.max_frac) = saved;
        status(f, U_ZERO_ERROR, "");
        Value::str(&s)
    })
}

/// parse() with the offset passed and returned explicitly (the PHP
/// wrapper writes it back): [value, offset].
fn nf_parse(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let text = str_arg(it, &a, 0)?;
    let ty = if has(&a, 1) { int_arg(it, &a, 1)? } else { 3 };
    let start = match arg(&a, 2) {
        Value::Null => 0,
        v => it.to_int(&v)?.clamp(0, text.len() as i64) as usize,
    };
    if !(0..=3).contains(&ty) || ty == 0 {
        return Err(it.error("ValueError", "NumberFormatter::parse(): Argument #2 ($type) must be a NumberFormatter::TYPE_* constant"));
    }
    with_fmt(it, o, |f| match numfmt::parse(&ctx(f), &text, start) {
        Some((v, end)) => {
            status(f, U_ZERO_ERROR, "");
            let v = match ty {
                1 => Value::Int(v as i64 as i32 as i64),
                2 => Value::Int(v as i64),
                _ => Value::Float(v),
            };
            Value::array(PhpArray::from_list(vec![v, Value::Int(end as i64)]))
        }
        None => {
            status(f, U_PARSE_ERROR, "Number parsing failed");
            Value::array(PhpArray::from_list(vec![Value::Bool(false), Value::Int(start as i64)]))
        }
    })
}

fn nf_parse_currency(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let text = str_arg(it, &a, 0)?;
    let start = match arg(&a, 1) {
        Value::Null => 0,
        v => it.to_int(&v)?.clamp(0, text.len() as i64) as usize,
    };
    with_fmt(it, o, |f| match numfmt::parse(&ctx(f), &text, start) {
        Some((v, end)) => {
            status(f, U_ZERO_ERROR, "");
            Value::array(PhpArray::from_list(vec![Value::Float(v), Value::str(&f.currency), Value::Int(end as i64)]))
        }
        None => {
            status(f, U_PARSE_ERROR, "Number parsing failed");
            Value::array(PhpArray::from_list(vec![Value::Bool(false), Value::Null, Value::Int(start as i64)]))
        }
    })
}

fn nf_set_attribute(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let attr = int_arg(it, &a, 0)?;
    let v = it.to_float(&arg(&a, 1))?;
    with_fmt(it, o, |f| {
        let ok = set_attr(f, attr, v);
        if ok {
            status(f, U_ZERO_ERROR, "");
        } else {
            status(f, U_UNSUPPORTED_ERROR, "NumberFormatter::setAttribute(): Error setting attribute value");
        }
        Value::Bool(ok)
    })
}

fn nf_get_attribute(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let attr = int_arg(it, &a, 0)?;
    with_fmt(it, o, |f| match get_attr(f, attr) {
        Some(v) => {
            status(f, U_ZERO_ERROR, "");
            v
        }
        None => {
            status(f, U_UNSUPPORTED_ERROR, "NumberFormatter::getAttribute(): Error getting attribute value");
            Value::Bool(false)
        }
    })
}

fn nf_get_text_attribute(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let attr = int_arg(it, &a, 0)?;
    with_fmt(it, o, |f| {
        let sym = numfmt::currency_symbol(f.loc.name, &f.currency);
        let c = ctx(f);
        let render = |parts: &[numfmt::Affix]| {
            let mut s = String::new();
            for p in parts {
                match p {
                    numfmt::Affix::Lit(l) => s.push_str(l),
                    numfmt::Affix::Currency(1) => s.push_str(&sym),
                    numfmt::Affix::Currency(_) => s.push_str(&f.currency),
                    numfmt::Affix::Percent => s.push_str(&c.symbols[numfmt::SYM_PERCENT]),
                    numfmt::Affix::Permill => s.push_str(&c.symbols[numfmt::SYM_PERMILL]),
                    numfmt::Affix::Minus => s.push_str(&c.symbols[numfmt::SYM_MINUS]),
                    numfmt::Affix::Plus => s.push_str(&c.symbols[numfmt::SYM_PLUS]),
                }
            }
            s
        };
        let p = &f.pattern;
        let v = match attr {
            0 => Some(f.affixes[0].clone().unwrap_or_else(|| render(&p.pos_prefix))),
            1 => Some(f.affixes[1].clone().unwrap_or_else(|| render(&p.pos_suffix))),
            2 => Some(f.affixes[2].clone().unwrap_or_else(|| match &p.neg {
                Some((np, _)) => render(np),
                None => format!("{}{}", f.symbols[numfmt::SYM_MINUS], render(&p.pos_prefix)),
            })),
            3 => Some(f.affixes[3].clone().unwrap_or_else(|| match &p.neg {
                Some((_, ns)) => render(ns),
                None => render(&p.pos_suffix),
            })),
            4 => Some("*".to_string()),
            5 => Some(f.currency.clone()),
            _ => None,
        };
        match v {
            Some(v) => {
                status(f, U_ZERO_ERROR, "");
                Value::str(&v)
            }
            None => {
                status(f, U_UNSUPPORTED_ERROR, "NumberFormatter::getTextAttribute(): Error getting attribute value");
                Value::Bool(false)
            }
        }
    })
}

fn nf_set_text_attribute(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let attr = int_arg(it, &a, 0)?;
    let v = str_arg(it, &a, 1)?;
    with_fmt(it, o, |f| {
        let ok = match attr {
            0..=3 => {
                f.affixes[attr as usize] = Some(v);
                true
            }
            5 => {
                f.currency = v.to_ascii_uppercase();
                if f.pattern.is_currency() {
                    apply_currency_digits(f);
                }
                true
            }
            _ => false,
        };
        if ok {
            status(f, U_ZERO_ERROR, "");
        } else {
            status(f, U_UNSUPPORTED_ERROR, "NumberFormatter::setTextAttribute(): Error setting text attribute");
        }
        Value::Bool(ok)
    })
}

/// NumberFormatter's symbol constants, in the order of LocaleData::symbols.
fn symbol_index(sym: i64) -> Option<usize> {
    Some(match sym {
        0 => numfmt::SYM_DECIMAL,
        1 => numfmt::SYM_GROUP,
        3 => numfmt::SYM_PERCENT,
        4 => numfmt::SYM_ZERO,
        6 => numfmt::SYM_MINUS,
        7 => numfmt::SYM_PLUS,
        10 => numfmt::SYM_MONEY_DECIMAL,
        11 => numfmt::SYM_EXP,
        12 => numfmt::SYM_PERMILL,
        14 => numfmt::SYM_INF,
        15 => numfmt::SYM_NAN,
        17 => numfmt::SYM_MONEY_GROUP,
        _ => return None,
    })
}

fn nf_get_symbol(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let sym = int_arg(it, &a, 0)?;
    with_fmt(it, o, |f| {
        let v = match sym {
            2 => Some(";".to_string()),
            5 => Some("#".to_string()),
            8 if f.currency == "XXX" => Some("¤".to_string()),
            8 => Some(numfmt::currency_symbol(f.loc.name, &f.currency)),
            9 => Some(f.currency.clone()),
            13 => Some("*".to_string()),
            16 => Some("@".to_string()),
            s => symbol_index(s).map(|i| f.symbols[i].clone()),
        };
        match v {
            Some(v) => {
                status(f, U_ZERO_ERROR, "");
                Value::str(&v)
            }
            None => {
                status(f, U_ILLEGAL_ARGUMENT_ERROR, "NumberFormatter::getSymbol(): Invalid symbol value");
                Value::Bool(false)
            }
        }
    })
}

fn nf_set_symbol(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let sym = int_arg(it, &a, 0)?;
    let v = str_arg(it, &a, 1)?;
    with_fmt(it, o, |f| match symbol_index(sym) {
        Some(i) => {
            f.symbols[i] = v;
            status(f, U_ZERO_ERROR, "");
            Value::Bool(true)
        }
        None => {
            status(f, U_ILLEGAL_ARGUMENT_ERROR, "NumberFormatter::setSymbol(): Invalid symbol value");
            Value::Bool(false)
        }
    })
}

fn nf_get_pattern(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    with_fmt(it, o, |f| Value::str(&if f.pattern_dirty { to_pattern(f) } else { f.pattern_src.clone() }))
}

fn nf_set_pattern(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let p = str_arg(it, &a, 0)?;
    with_fmt(it, o, |f| match Pattern::parse(&p) {
        Some(pat) => {
            f.pattern = pat;
            f.pattern_src = p;
            f.pattern_dirty = false;
            status(f, U_ZERO_ERROR, "");
            Value::Bool(true)
        }
        None => {
            status(f, U_PARSE_ERROR, "NumberFormatter::setPattern(): Error setting pattern value");
            Value::Bool(false)
        }
    })
}

fn nf_get_locale(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    with_fmt(it, o, |f| Value::str(f.loc.name))
}

fn nf_error_code(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    with_fmt(it, o, |f| Value::Int(f.error.0))
}

fn nf_error_message(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    with_fmt(it, o, |f| Value::str(&f.error.1))
}

// ---- Locale -------------------------------------------------------------------------

/// (language, script, region, variants) of a locale id.
fn subtags(locale: &str) -> (String, String, String, Vec<String>) {
    let (canon, _) = numfmt::canonicalize(locale);
    let mut parts = canon.split('_');
    let lang = parts.next().unwrap_or("").to_string();
    let (mut script, mut region, mut variants) = (String::new(), String::new(), vec![]);
    for p in parts {
        if script.is_empty() && region.is_empty() && variants.is_empty() && p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic()) {
            script = p.to_string();
        } else if region.is_empty() && variants.is_empty() && (p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()) || p.len() == 3 && p.chars().all(|c| c.is_ascii_digit())) {
            region = p.to_string();
        } else {
            variants.push(p.to_string());
        }
    }
    (lang, script, region, variants)
}

fn locale_arg(it: &Interp, a: &[Value]) -> R<String> {
    let l = str_arg(it, a, 0)?;
    Ok(if l.is_empty() { default_locale(it) } else { l })
}

fn lc_get_default(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::str(&default_locale(it)))
}

fn lc_set_default(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let l = str_arg(it, &a, 0)?;
    DEFAULT.with(|d| *d.borrow_mut() = Some(numfmt::canonicalize(&l).0));
    Ok(Value::Bool(true))
}

fn lc_canonicalize(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let l = locale_arg(it, &a)?;
    let (base, kw) = numfmt::canonicalize(&l);
    Ok(Value::str(&if kw.is_empty() { base } else { format!("{base}@{kw}") }))
}

fn lc_primary_language(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    Ok(Value::str(&subtags(&locale_arg(it, &a)?).0))
}

fn lc_script(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    Ok(Value::str(&subtags(&locale_arg(it, &a)?).1))
}

fn lc_region(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    Ok(Value::str(&subtags(&locale_arg(it, &a)?).2))
}

fn lc_parse(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let (lang, script, region, variants) = subtags(&locale_arg(it, &a)?);
    let mut out = PhpArray::new();
    let mut put = |k: &str, v: &str| {
        if !v.is_empty() {
            out.insert(Key::Str(pstr(k)), Value::str(v));
        }
    };
    put("language", &lang);
    put("script", &script);
    put("region", &region);
    for (i, v) in variants.iter().enumerate() {
        put(&format!("variant{i}"), v);
    }
    Ok(Value::array(out))
}

fn lc_compose(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let Value::Array(m) = arg(&a, 0) else { return Ok(Value::Bool(false)) };
    let get = |k: &str| m.get(&Key::Str(pstr(k))).map(|v| it.to_string_lossy(&v)).transpose();
    let Some(lang) = get("language")? else { return Ok(Value::Bool(false)) };
    let mut parts = vec![lang];
    for k in ["script", "region"] {
        if let Some(v) = get(k)? {
            parts.push(v);
        }
    }
    for i in 0..15 {
        if let Some(v) = get(&format!("variant{i}"))? {
            parts.push(v);
        }
    }
    Ok(Value::str(&parts.join("_")))
}

fn lc_keywords(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let (_, kw) = numfmt::canonicalize(&locale_arg(it, &a)?);
    let mut pairs: Vec<(String, String)> = kw.split(';').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    pairs.sort();
    let mut out = PhpArray::new();
    for (k, v) in pairs {
        out.insert(Key::Str(pstr(&k)), Value::str(&v));
    }
    Ok(if out.is_empty() { Value::Null } else { Value::array(out) })
}

fn lc_accept_from_http(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let header = str_arg(it, &a, 0)?;
    let mut best: Option<(f64, usize, String)> = None;
    for (i, part) in header.split(',').enumerate() {
        let mut it2 = part.split(';');
        let tag = it2.next().unwrap_or("").trim();
        if tag.is_empty() || tag == "*" {
            continue;
        }
        let q = it2.find_map(|p| p.trim().strip_prefix("q=").and_then(|q| q.parse::<f64>().ok())).unwrap_or(1.0);
        if numfmt::resolve(tag).is_none() {
            continue;
        }
        if best.as_ref().is_none_or(|b| q > b.0) {
            best = Some((q, i, numfmt::canonicalize(tag).0));
        }
    }
    Ok(best.map_or(Value::Bool(false), |b| Value::str(&b.2)))
}

fn lc_is_rtl(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let (lang, script, _, _) = subtags(&locale_arg(it, &a)?);
    let rtl_script = matches!(script.as_str(), "Arab" | "Hebr" | "Thaa" | "Syrc" | "Nkoo" | "Adlm" | "Rohg");
    let rtl_lang = script.is_empty() && matches!(lang.as_str(), "ar" | "he" | "fa" | "ur" | "ps" | "yi" | "dv" | "ckb" | "sd" | "ug" | "syr" | "ks" | "mzn" | "lrc");
    Ok(Value::Bool(rtl_script || rtl_lang))
}

// ---- registration ------------------------------------------------------------------

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![
        ("numberformatter", "__construct", nf_construct),
        ("numberformatter", "static create", nf_create),
        ("numberformatter", "format", nf_format),
        ("numberformatter", "formatcurrency", nf_format_currency),
        ("numberformatter", "__phpx_parse", nf_parse),
        ("numberformatter", "__phpx_parse_currency", nf_parse_currency),
        ("numberformatter", "setattribute", nf_set_attribute),
        ("numberformatter", "getattribute", nf_get_attribute),
        ("numberformatter", "settextattribute", nf_set_text_attribute),
        ("numberformatter", "gettextattribute", nf_get_text_attribute),
        ("numberformatter", "setsymbol", nf_set_symbol),
        ("numberformatter", "getsymbol", nf_get_symbol),
        ("numberformatter", "setpattern", nf_set_pattern),
        ("numberformatter", "getpattern", nf_get_pattern),
        ("numberformatter", "getlocale", nf_get_locale),
        ("numberformatter", "geterrorcode", nf_error_code),
        ("numberformatter", "geterrormessage", nf_error_message),
        ("locale", "static getdefault", lc_get_default),
        ("locale", "static setdefault", lc_set_default),
        ("locale", "static canonicalize", lc_canonicalize),
        ("locale", "static getprimarylanguage", lc_primary_language),
        ("locale", "static getscript", lc_script),
        ("locale", "static getregion", lc_region),
        ("locale", "static parselocale", lc_parse),
        ("locale", "static composelocale", lc_compose),
        ("locale", "static getkeywords", lc_keywords),
        ("locale", "static acceptfromhttp", lc_accept_from_http),
        ("locale", "static isrighttoleft", lc_is_rtl),
    ]
}

/// A procedural alias: the first argument is the object.
macro_rules! method_fn {
    ($t:expr, $name:expr, $method:expr) => {
        $t.insert($name, |it, _, a| {
            let o = match a.first() {
                Some(Value::Object(o)) => o.clone(),
                _ => return Err(it.error("TypeError", &format!("{}(): Argument #1 ($formatter) must be of type NumberFormatter", $name))),
            };
            let cls = o.class.clone();
            $method(it, Some(&o), &cls, a[1..].to_vec())
        });
    };
}

/// A procedural alias of a static method.
macro_rules! static_fn {
    ($t:expr, $name:expr, $method:expr) => {
        $t.insert($name, |it, _, a| {
            let cls = it.classes.borrow().get("locale").cloned().expect("Locale class");
            $method(it, None, &cls, a.to_vec())
        });
    };
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    grapheme::register(t);
    t.insert("__phpx_intl_missing", |it, _, a| {
        let m = crate::pstr::lossy(&s(it, a, 0)?).into_owned();
        Err(it.error("Error", &format!("{m}() is not supported by phpx yet")))
    });
    t.insert("intl_get_error_code", |_, _, _| Ok(Value::Int(LAST.with(|l| l.borrow().0))));
    t.insert("intl_get_error_message", |_, _, _| Ok(Value::str(&LAST.with(|l| l.borrow().1.clone()))));
    t.insert("intl_is_failure", |it, _, a| Ok(Value::Bool(int(it, a, 0)? > 0)));
    t.insert("intl_error_name", |it, _, a| Ok(Value::str(error_name(int(it, a, 0)?))));
    t.insert("numfmt_create", |it, _, a| {
        let cls = it.classes.borrow().get("numberformatter").cloned().expect("NumberFormatter class");
        nf_create(it, None, &cls, a.to_vec())
    });
    method_fn!(t, "numfmt_format", nf_format);
    method_fn!(t, "numfmt_format_currency", nf_format_currency);
    method_fn!(t, "numfmt_set_attribute", nf_set_attribute);
    method_fn!(t, "numfmt_get_attribute", nf_get_attribute);
    method_fn!(t, "numfmt_set_text_attribute", nf_set_text_attribute);
    method_fn!(t, "numfmt_get_text_attribute", nf_get_text_attribute);
    method_fn!(t, "numfmt_set_symbol", nf_set_symbol);
    method_fn!(t, "numfmt_get_symbol", nf_get_symbol);
    method_fn!(t, "numfmt_set_pattern", nf_set_pattern);
    method_fn!(t, "numfmt_get_pattern", nf_get_pattern);
    method_fn!(t, "numfmt_get_locale", nf_get_locale);
    method_fn!(t, "numfmt_get_error_code", nf_error_code);
    method_fn!(t, "numfmt_get_error_message", nf_error_message);
    static_fn!(t, "locale_get_default", lc_get_default);
    static_fn!(t, "locale_set_default", lc_set_default);
    static_fn!(t, "locale_canonicalize", lc_canonicalize);
    static_fn!(t, "locale_get_primary_language", lc_primary_language);
    static_fn!(t, "locale_get_script", lc_script);
    static_fn!(t, "locale_get_region", lc_region);
    static_fn!(t, "locale_parse", lc_parse);
    static_fn!(t, "locale_compose", lc_compose);
    static_fn!(t, "locale_get_keywords", lc_keywords);
    static_fn!(t, "locale_accept_from_http", lc_accept_from_http);
    static_fn!(t, "locale_is_right_to_left", lc_is_rtl);
}
