//! Dates and times: DateTime, DateTimeImmutable, DateTimeZone (native
//! state), DateInterval / DatePeriod (PHP in prelude_date.php, with native
//! helpers), and date(), strtotime(), mktime() and friends.

mod abbr_map;
mod civil;
mod format;
mod fromformat;
mod interval;
mod more;
mod parse;
mod zone;
mod zones_table;

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::builtins::{arg, has, int, s};
use crate::class::Class;
use crate::interp::{BuiltinFn, Interp, NativeMethod, R};
use crate::value::{pstr, Internal, Key, NativeData, Obj, PhpArray, Value};

pub use civil::Civil;
pub use zone::Zone;

/// A DateTime / DateTimeImmutable: UTC instant plus the zone it's shown in.
#[derive(Clone, Debug)]
pub struct DateState {
    pub ts: i64,
    pub us: i64,
    pub zone: Zone,
}

impl DateState {
    fn local(&self) -> Civil {
        Civil::from_local(self.ts + self.zone.offset(self.ts) as i64, self.us)
    }

    /// Set from local wall-clock fields (normalizing overflow).
    fn set_local(&mut self, c: Civil) {
        let (local, us) = c.to_local();
        self.ts = self.zone.to_utc(local);
        self.us = us;
    }

    fn format(&self, f: &[u8]) -> Vec<u8> {
        format::format(f, self.ts, self.us, &self.zone)
    }
}

impl NativeData for DateState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        Some(Box::new(self.clone()))
    }
    fn view_props(&self) -> Vec<(Rc<str>, Value)> {
        vec![
            (Rc::from("date"), Value::bytes(self.format(b"Y-m-d H:i:s.u"))),
            (Rc::from("timezone_type"), Value::Int(self.zone.type_num())),
            (Rc::from("timezone"), Value::str(&self.zone.name())),
        ]
    }
    fn compare(&self, other: &dyn NativeData) -> Option<std::cmp::Ordering> {
        let o = other.as_any().downcast_ref::<DateState>()?;
        Some((self.ts, self.us).cmp(&(o.ts, o.us)))
    }
}

#[derive(Clone, Debug)]
pub struct ZoneState(pub Zone);

impl NativeData for ZoneState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        Some(Box::new(self.clone()))
    }
    fn view_props(&self) -> Vec<(Rc<str>, Value)> {
        vec![(Rc::from("timezone_type"), Value::Int(self.0.type_num())), (Rc::from("timezone"), Value::str(&self.0.name()))]
    }
}

fn now() -> (i64, i64) {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() as i64, d.subsec_micros() as i64)
}

/// The default time zone (date.timezone, else UTC).
pub fn default_zone(it: &Interp) -> Zone {
    it.ini_get("date.timezone").filter(|t| !t.is_empty()).and_then(|t| Zone::parse(&t)).unwrap_or(Zone::Id(chrono_tz::Tz::UTC))
}

fn lossy(b: &[u8]) -> String {
    crate::pstr::lossy(b).into_owned()
}

fn date_of(o: &Rc<Obj>) -> Option<DateState> {
    match &*o.try_internal()? {
        Internal::Data(d) => d.as_any().downcast_ref::<DateState>().cloned(),
        _ => None,
    }
}

fn zone_of(v: &Value) -> Option<Zone> {
    let Value::Object(o) = v else { return None };
    match &*o.try_internal()? {
        Internal::Data(d) => d.as_any().downcast_ref::<ZoneState>().map(|z| z.0.clone()),
        _ => None,
    }
}

fn state(it: &Interp, this: Option<&Rc<Obj>>) -> R<(Rc<Obj>, DateState)> {
    let o = this.ok_or_else(|| it.error("Error", "Non-static method called statically"))?.clone();
    match date_of(&o) {
        Some(d) => Ok((o, d)),
        None => Err(it.error("Error", &format!("The DateTime object has not been correctly initialized by its constructor"))),
    }
}

fn store(o: &Rc<Obj>, d: DateState) {
    o.set_internal(Internal::Data(Box::new(d)));
}

/// `parse` + `apply` with PHP's error message.
fn parse_at(it: &Interp, text: &str, now_ts: i64, now_us: i64, base_zone: &Zone) -> Result<DateState, String> {
    let _ = it;
    match parse::parse(text) {
        Ok(p) => {
            let (ts, us, zone) = parse::apply(&p, now_ts, now_us, base_zone);
            Ok(DateState { ts, us, zone })
        }
        Err(e) => Err(format!("Failed to parse time string ({text}) at position {} ({}): {}", e.pos, e.ch, e.msg)),
    }
}

fn is_immutable(o: &Rc<Obj>) -> bool {
    o.class.is_a("datetimeimmutable")
}

/// The object a modifier works on: `$this`, or a clone for immutables.
fn target(it: &Interp, o: &Rc<Obj>) -> R<Rc<Obj>> {
    if is_immutable(o) {
        it.clone_object(o)
    } else {
        Ok(o.clone())
    }
}

fn new_object(it: &Interp, cls: &Rc<Class>, d: DateState) -> R<Value> {
    let o = it.new_instance(cls)?;
    store(&o, d);
    Ok(Value::Object(o))
}

fn zone_object(it: &Interp, z: Zone) -> R<Value> {
    let cls = it.lookup_class("DateTimeZone")?;
    let o = it.new_instance(&cls)?;
    o.set_internal(Internal::Data(Box::new(ZoneState(z))));
    Ok(Value::Object(o))
}

fn sarg(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(lossy(&it.to_str(&a.get(i).cloned().unwrap_or_default())?))
}

fn iarg(it: &Interp, a: &[Value], i: usize, dflt: i64) -> R<i64> {
    match a.get(i) {
        None | Some(Value::Null) => Ok(dflt),
        Some(v) => it.to_int(v),
    }
}

// ---- DateTime / DateTimeImmutable methods ------------------------------------

fn dt_construct(it: &Interp, this: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this.ok_or_else(|| it.error("Error", "constructor called statically"))?;
    let text = if a.is_empty() || a[0].is_null() { "now".to_string() } else { sarg(it, &a, 0)? };
    let zone = zone_of(&arg(&a, 1)).unwrap_or_else(|| default_zone(it));
    let (ts, us) = now();
    match parse_at(it, &text, ts, us, &zone) {
        Ok(d) => {
            set_last_errors(&[], &[]);
            store(o, d);
            Ok(Value::Null)
        }
        Err(msg) => {
            let _ = cls;
            Err(it.error("DateMalformedStringException", &msg))
        }
    }
}

fn dt_format(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let (_, d) = state(it, this)?;
    let f = it.to_str(&arg(&a, 0))?;
    Ok(Value::bytes(d.format(&f)))
}

fn dt_get_timestamp(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Int(state(it, this)?.1.ts))
}

fn dt_get_microsecond(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Int(state(it, this)?.1.us))
}

fn dt_get_offset(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let (_, d) = state(it, this)?;
    Ok(Value::Int(d.zone.offset(d.ts) as i64))
}

fn dt_get_timezone(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let (_, d) = state(it, this)?;
    zone_object(it, d.zone)
}

/// Run `g` on the target's state and return the target.
fn modify_with(it: &Interp, this: Option<&Rc<Obj>>, g: impl FnOnce(&mut DateState) -> R<()>) -> R<Value> {
    let (o, mut d) = state(it, this)?;
    let t = target(it, &o)?;
    g(&mut d)?;
    store(&t, d);
    Ok(Value::Object(t))
}

fn dt_set_timestamp(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let ts = iarg(it, &a, 0, 0)?;
    modify_with(it, this, |d| {
        d.ts = ts;
        d.us = 0;
        Ok(())
    })
}

fn dt_set_microsecond(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let us = iarg(it, &a, 0, 0)?;
    if !(0..1_000_000).contains(&us) {
        return Err(it.error("DateRangeError", "DateTime::setMicrosecond(): Argument #1 ($microsecond) must be between 0 and 999999, ".to_string().as_str()));
    }
    modify_with(it, this, |d| {
        d.us = us;
        Ok(())
    })
}

fn dt_set_timezone(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let Some(z) = zone_of(&arg(&a, 0)) else {
        return Err(it.error("TypeError", "DateTime::setTimezone(): Argument #1 ($timezone) must be of type DateTimeZone"));
    };
    modify_with(it, this, |d| {
        d.zone = z;
        Ok(())
    })
}

fn dt_set_date(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let (y, m, dd) = (iarg(it, &a, 0, 0)?, iarg(it, &a, 1, 1)?, iarg(it, &a, 2, 1)?);
    modify_with(it, this, |d| {
        let c = d.local();
        d.set_local(Civil { y, m, d: dd, ..c });
        Ok(())
    })
}

fn dt_set_iso_date(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let (y, w, dow) = (iarg(it, &a, 0, 0)?, iarg(it, &a, 1, 1)?, iarg(it, &a, 2, 1)?);
    modify_with(it, this, |d| {
        let c = d.local();
        let (yy, mm, dd) = civil::civil_from_days(civil::from_iso_week(y, w, dow));
        d.set_local(Civil { y: yy, m: mm, d: dd, ..c });
        Ok(())
    })
}

fn dt_set_time(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let (h, i, s, us) = (iarg(it, &a, 0, 0)?, iarg(it, &a, 1, 0)?, iarg(it, &a, 2, 0)?, iarg(it, &a, 3, 0)?);
    modify_with(it, this, |d| {
        let c = d.local();
        d.set_local(Civil { h, i, s, us, ..c });
        Ok(())
    })
}

fn dt_modify(it: &Interp, this: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let text = sarg(it, &a, 0)?;
    let (o, d) = state(it, this)?;
    match parse::parse(&text) {
        Ok(p) => {
            let t = target(it, &o)?;
            let (ts, us, zone) = parse::apply(&p, d.ts, d.us, &d.zone);
            store(&t, DateState { ts, us, zone });
            Ok(Value::Object(t))
        }
        Err(e) => {
            let msg = format!("Failed to parse time string ({text}) at position {} ({}): {}", e.pos, e.ch, e.msg);
            let class = if cls.is_a("datetimeimmutable") { "DateTimeImmutable" } else { "DateTime" };
            Err(it.error("DateMalformedStringException", &format!("{class}::modify(): {msg}")))
        }
    }
}

fn dt_add(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let iv = interval::read(it, &arg(&a, 0))?;
    modify_with(it, this, |d| {
        interval::add(d, &iv, 1);
        Ok(())
    })
}

fn dt_sub(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let iv = interval::read(it, &arg(&a, 0))?;
    modify_with(it, this, |d| {
        interval::add(d, &iv, -1);
        Ok(())
    })
}

fn dt_diff(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let (_, x) = state(it, this)?;
    let Value::Object(other) = arg(&a, 0) else {
        return Err(it.error("TypeError", "DateTime::diff(): Argument #1 ($targetObject) must be of type DateTimeInterface"));
    };
    let Some(y) = date_of(&other) else { return Err(it.error("Error", "The DateTimeInterface object has not been correctly initialized")) };
    let absolute = a.get(1).is_some_and(|v| v.truthy());
    interval::diff(it, &x, &y, absolute)
}

fn dt_create_from_format(it: &Interp, _: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let f = it.to_str(&arg(&a, 0))?;
    let text = it.to_str(&arg(&a, 1))?;
    let zone = zone_of(&arg(&a, 2)).unwrap_or_else(|| default_zone(it));
    let (ts, us) = now();
    let r = fromformat::parse(&f, &text);
    set_last_errors(&r.warnings, &r.errors);
    if !r.errors.is_empty() {
        return Ok(Value::Bool(false));
    }
    new_object(it, cls, fromformat::to_state(&r, ts, us, &zone))
}

thread_local! {
    /// DateTime::getLastErrors(): the last parse's warnings and errors.
    static LAST_ERRORS: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

pub fn reset() {
    LAST_ERRORS.with(|l| *l.borrow_mut() = None);
}

/// The warning_count/warnings/error_count/errors entries of a parse.
pub(crate) fn errors_array(warnings: &[(usize, String)], errors: &[(usize, String)], out: &mut PhpArray) {
    let list = |v: &[(usize, String)]| {
        let mut a = PhpArray::new();
        for (pos, msg) in v {
            a.insert(Key::Int(*pos as i64), Value::str(msg));
        }
        Value::array(a)
    };
    out.insert(Key::Str(pstr("warning_count")), Value::Int(warnings.len() as i64));
    out.insert(Key::Str(pstr("warnings")), list(warnings));
    out.insert(Key::Str(pstr("error_count")), Value::Int(errors.len() as i64));
    out.insert(Key::Str(pstr("errors")), list(errors));
}

/// Record a parse for getLastErrors() (false when it was clean).
pub(crate) fn set_last_errors(warnings: &[(usize, String)], errors: &[(usize, String)]) {
    let v = if warnings.is_empty() && errors.is_empty() {
        None
    } else {
        let mut a = PhpArray::new();
        errors_array(warnings, errors, &mut a);
        Some(Value::array(a))
    };
    LAST_ERRORS.with(|l| *l.borrow_mut() = v);
}

fn dt_create_from_interface(it: &Interp, _: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let Value::Object(o) = arg(&a, 0) else { return Err(it.error("TypeError", "Argument #1 must be of type DateTimeInterface")) };
    let d = date_of(&o).ok_or_else(|| it.error("Error", "The DateTimeInterface object has not been correctly initialized"))?;
    new_object(it, cls, d)
}

fn dt_get_last_errors(_: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(LAST_ERRORS.with(|l| l.borrow().clone()).unwrap_or(Value::Bool(false)))
}

/// The three `date`/`timezone_type`/`timezone` entries back to a state.
fn from_parts(it: &Interp, arr: &Value) -> Option<DateState> {
    let Value::Array(a) = arr else { return None };
    let get = |k: &str| a.get(&Key::Str(pstr(k)));
    let date = lossy(&it.to_str(&get("date")?).ok()?);
    let zone = Zone::parse(&lossy(&it.to_str(&get("timezone")?).ok()?))?;
    let p = parse::parse(&date).ok()?;
    let (ts, us, _) = parse::apply(&p, 0, 0, &zone);
    Some(DateState { ts, us, zone })
}

fn dt_set_state(it: &Interp, _: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    match from_parts(it, &arg(&a, 0)) {
        Some(d) => new_object(it, cls, d),
        None => Err(it.error("Error", &format!("Invalid serialization data for {} object", cls.name))),
    }
}

fn dt_serialize(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let (o, d) = state(it, this)?;
    let mut arr = PhpArray::new();
    for (k, v) in d.view_props() {
        arr.insert(Key::Str(pstr(&k)), v);
    }
    // Subclass properties come along too.
    for (k, v) in o.prop_list() {
        arr.insert(Key::Str(pstr(&k)), v);
    }
    Ok(Value::array(arr))
}

fn dt_unserialize(it: &Interp, this: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this.ok_or_else(|| it.error("Error", "called statically"))?;
    let data = arg(&a, 0);
    let d = from_parts(it, &data).ok_or_else(|| it.error("Error", &format!("Invalid serialization data for {} object", cls.name)))?;
    store(o, d);
    if let Value::Array(arr) = &data {
        for (k, v) in arr.iter() {
            if let Key::Str(k) = k {
                let k = lossy(&k);
                if !matches!(k.as_str(), "date" | "timezone_type" | "timezone") {
                    o.set_prop(&k, v.clone());
                }
            }
        }
    }
    Ok(Value::Null)
}

fn dt_wakeup(_: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Null)
}

// ---- DateTimeZone ---------------------------------------------------------------

fn tz_construct(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this.ok_or_else(|| it.error("Error", "called statically"))?;
    let name = sarg(it, &a, 0)?;
    match Zone::parse(&name) {
        Some(z) => {
            o.set_internal(Internal::Data(Box::new(ZoneState(z))));
            Ok(Value::Null)
        }
        None => Err(it.error("DateInvalidTimeZoneException", &format!("DateTimeZone::__construct(): Unknown or bad timezone ({name})"))),
    }
}

fn tz_state(it: &Interp, this: Option<&Rc<Obj>>) -> R<Zone> {
    let o = this.ok_or_else(|| it.error("Error", "called statically"))?;
    zone_of(&Value::Object(o.clone())).ok_or_else(|| it.error("Error", "The DateTimeZone object has not been correctly initialized by its constructor"))
}

fn tz_get_name(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::str(&tz_state(it, this)?.name()))
}

fn tz_get_offset(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let z = tz_state(it, this)?;
    let ts = match arg(&a, 0) {
        Value::Object(o) => date_of(&o).map(|d| d.ts).unwrap_or(0),
        _ => 0,
    };
    Ok(Value::Int(z.offset(ts) as i64))
}

fn tz_get_location(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let z = tz_state(it, this)?;
    if z.type_num() != 3 {
        return Ok(Value::Bool(false));
    }
    let mut arr = PhpArray::new();
    for (k, v) in [("country_code", Value::str("??")), ("latitude", Value::Float(0.0)), ("longitude", Value::Float(0.0)), ("comments", Value::str(""))] {
        arr.insert(Key::Str(pstr(k)), v);
    }
    Ok(Value::array(arr))
}

fn tz_get_transitions(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let z = tz_state(it, this)?;
    let begin = iarg(it, &a, 0, i64::MIN / 2)?;
    let end = iarg(it, &a, 1, i64::MAX / 2)?;
    let entry = |ts: i64| {
        let (off, dst, abbr) = z.at(ts);
        let mut e = PhpArray::new();
        e.insert(Key::Str(pstr("ts")), Value::Int(ts));
        e.insert(Key::Str(pstr("time")), Value::bytes(format::format(b"Y-m-d\\TH:i:sO", ts, 0, &Zone::Offset(0))));
        e.insert(Key::Str(pstr("offset")), Value::Int(off as i64));
        e.insert(Key::Str(pstr("isdst")), Value::Bool(dst));
        e.insert(Key::Str(pstr("abbr")), Value::str(&abbr));
        Value::array(e)
    };
    let mut out = vec![entry(begin)];
    // Scan for offset changes hour by hour within a bounded range.
    if end - begin <= 20 * 366 * 86400 {
        let mut prev = z.at(begin);
        let mut t = begin + 3600;
        while t <= end {
            let cur = z.at(t);
            if cur.0 != prev.0 || cur.1 != prev.1 {
                // Narrow to the second.
                let (mut lo, mut hi) = (t - 3600, t);
                while hi - lo > 1 {
                    let mid = lo + (hi - lo) / 2;
                    if z.at(mid).0 == prev.0 && z.at(mid).1 == prev.1 {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                out.push(entry(hi));
                prev = cur;
            }
            t += 3600;
        }
    }
    Ok(Value::array(PhpArray::from_list(out)))
}

fn tz_list_identifiers(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    list_identifiers(it, &a, "DateTimeZone::listIdentifiers")
}

/// timezone_identifiers_list() / DateTimeZone::listIdentifiers().
fn list_identifiers(it: &Interp, a: &[Value], func: &str) -> R<Value> {
    let group = match arg(a, 0) {
        Value::Null => 2047,
        v => it.to_int(&v)?,
    };
    let country = match arg(a, 1) {
        Value::Null => String::new(),
        v => lossy(&it.to_str(&v)?),
    };
    if group == 4096 && country.len() != 2 {
        return Err(it.error(
            "ValueError",
            &format!("{func}(): Argument #2 ($countryCode) must be a two-letter ISO 3166-1 compatible country code when argument #1 ($timezoneGroup) is DateTimeZone::PER_COUNTRY"),
        ));
    }
    Ok(Value::array(PhpArray::from_list(zone::list_identifiers(group, &country).into_iter().map(Value::str))))
}

fn tz_list_abbreviations(_: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::array(PhpArray::new()))
}

fn tz_serialize(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let z = tz_state(it, this)?;
    let mut arr = PhpArray::new();
    for (k, v) in ZoneState(z).view_props() {
        arr.insert(Key::Str(pstr(&k)), v);
    }
    Ok(Value::array(arr))
}

fn tz_unserialize(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this.ok_or_else(|| it.error("Error", "called statically"))?;
    let Value::Array(arr) = arg(&a, 0) else { return Err(it.error("Error", "Invalid serialization data for DateTimeZone object")) };
    let name = arr.get(&Key::Str(pstr("timezone"))).map(|v| it.to_str(&v)).transpose()?.map(|b| lossy(&b)).unwrap_or_default();
    let z = Zone::parse(&name).ok_or_else(|| it.error("Error", "Invalid serialization data for DateTimeZone object"))?;
    o.set_internal(Internal::Data(Box::new(ZoneState(z))));
    Ok(Value::Null)
}

fn tz_set_state(it: &Interp, _: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = it.new_instance(cls)?;
    tz_unserialize(it, Some(&o), cls, a)?;
    Ok(Value::Object(o))
}

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    let mut v: Vec<(&'static str, &'static str, NativeMethod)> = vec![];
    for c in ["datetime", "datetimeimmutable"] {
        v.extend([
            (c, "__construct", dt_construct as NativeMethod),
            (c, "format", dt_format),
            (c, "gettimestamp", dt_get_timestamp),
            (c, "getmicrosecond", dt_get_microsecond),
            (c, "getoffset", dt_get_offset),
            (c, "gettimezone", dt_get_timezone),
            (c, "settimestamp", dt_set_timestamp),
            (c, "setmicrosecond", dt_set_microsecond),
            (c, "settimezone", dt_set_timezone),
            (c, "setdate", dt_set_date),
            (c, "setisodate", dt_set_iso_date),
            (c, "settime", dt_set_time),
            (c, "modify", dt_modify),
            (c, "add", dt_add),
            (c, "sub", dt_sub),
            (c, "diff", dt_diff),
            (c, "static createfromformat", dt_create_from_format),
            (c, "static createfrominterface", dt_create_from_interface),
            (c, "static getlasterrors", dt_get_last_errors),
            (c, "static __set_state", dt_set_state),
            (c, "__serialize", dt_serialize),
            (c, "__unserialize", dt_unserialize),
            (c, "__wakeup", dt_wakeup),
        ]);
    }
    v.push(("datetime", "static createfromimmutable", dt_create_from_interface));
    v.push(("datetimeimmutable", "static createfrommutable", dt_create_from_interface));
    v.extend([
        ("datetimezone", "__construct", tz_construct as NativeMethod),
        ("datetimezone", "getname", tz_get_name),
        ("datetimezone", "getoffset", tz_get_offset),
        ("datetimezone", "getlocation", tz_get_location),
        ("datetimezone", "gettransitions", tz_get_transitions),
        ("datetimezone", "static listidentifiers", tz_list_identifiers),
        ("datetimezone", "static listabbreviations", tz_list_abbreviations),
        ("datetimezone", "__serialize", tz_serialize),
        ("datetimezone", "__unserialize", tz_unserialize),
        ("datetimezone", "static __set_state", tz_set_state),
    ]);
    v
}

// ---- functions ----------------------------------------------------------------------

fn date_fn(it: &Interp, a: &[Value], zone: Zone) -> R<Value> {
    let f = s(it, a, 0)?;
    let ts = if has(a, 1) && !arg(a, 1).is_null() { int(it, a, 1)? } else { now().0 };
    Ok(Value::bytes(format::format(&f, ts, 0, &zone)))
}

fn mktime_fn(it: &Interp, a: &[Value], zone: Zone) -> R<Value> {
    let (ts, _) = now();
    let c = Civil::from_local(ts + zone.offset(ts) as i64, 0);
    let get = |i: usize, d: i64| -> R<i64> { if has(a, i) && !arg(a, i).is_null() { int(it, a, i) } else { Ok(d) } };
    let mut y = get(5, c.y)?;
    if has(a, 5) && !arg(a, 5).is_null() {
        // Two-digit years like mktime: 0-69 -> 2000s, 70-100 -> 1900s.
        if (0..70).contains(&y) {
            y += 2000;
        } else if (70..=100).contains(&y) {
            y += 1900;
        }
    }
    let c = Civil { y, m: get(3, c.m)?, d: get(4, c.d)?, h: get(0, c.h)?, i: get(1, c.i)?, s: get(2, c.s)?, us: 0 };
    let (local, _) = c.to_local();
    Ok(Value::Int(zone.to_utc(local)))
}

fn getdate_arr(ts: i64, zone: &Zone) -> Value {
    let off = zone.offset(ts) as i64;
    let local = ts + off;
    let c = Civil::from_local(local, 0);
    let days = local.div_euclid(86400);
    let dow = civil::weekday(days);
    let mut a = PhpArray::new();
    for (k, v) in [
        ("seconds", Value::Int(c.s)),
        ("minutes", Value::Int(c.i)),
        ("hours", Value::Int(c.h)),
        ("mday", Value::Int(c.d)),
        ("wday", Value::Int(dow)),
        ("mon", Value::Int(c.m)),
        ("year", Value::Int(c.y)),
        ("yday", Value::Int(days - civil::days_from_civil(c.y, 1, 1))),
        ("weekday", Value::str(format::DAYS[dow as usize])),
        ("month", Value::str(format::MONTHS[c.m as usize - 1])),
    ] {
        a.insert(Key::Str(pstr(k)), v);
    }
    a.insert(Key::Int(0), Value::Int(ts));
    Value::array(a)
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    more::register(t);
    t.insert("date", |it, _, a| date_fn(it, a, default_zone(it)));
    t.insert("gmdate", |it, _, a| date_fn(it, a, Zone::Offset(0)));
    t.insert("mktime", |it, _, a| mktime_fn(it, a, default_zone(it)));
    t.insert("gmmktime", |it, _, a| mktime_fn(it, a, Zone::Offset(0)));
    t.insert("strtotime", |it, _, a| {
        let text = lossy(&s(it, a, 0)?);
        let (ts, us) = if has(a, 1) && !arg(a, 1).is_null() { (int(it, a, 1)?, 0) } else { now() };
        Ok(match parse::parse(&text) {
            Ok(p) if !text.trim().is_empty() => Value::Int(parse::apply(&p, ts, us, &default_zone(it)).0),
            _ => Value::Bool(false),
        })
    });
    t.insert("idate", |it, _, a| {
        let f = s(it, a, 0)?;
        if f.len() != 1 {
            return Err(it.error("ValueError", "idate(): Argument #1 ($format) must be one character"));
        }
        let ts = if has(a, 1) && !arg(a, 1).is_null() { int(it, a, 1)? } else { now().0 };
        let zone = default_zone(it);
        let code: &[u8] = match f[0] {
            b'B' => b"B",
            b'd' => b"j",
            b'h' => b"g",
            b'H' => b"G",
            b'i' => b"i",
            b'I' => b"I",
            b'L' => b"L",
            b'm' => b"n",
            b'N' => b"N",
            b'o' => b"o",
            b's' => b"s",
            b't' => b"t",
            b'U' => b"U",
            b'w' => b"w",
            b'W' => b"W",
            b'y' => b"y",
            b'Y' => b"Y",
            b'z' => b"z",
            b'Z' => b"Z",
            _ => return Err(it.error("ValueError", "idate(): Argument #1 ($format) must be a valid date format character")),
        };
        let out = lossy(&format::format(code, ts, 0, &zone));
        Ok(Value::Int(out.trim_start_matches('0').parse().unwrap_or(0)))
    });
    t.insert("getdate", |it, _, a| {
        let ts = if has(a, 0) && !arg(a, 0).is_null() { int(it, a, 0)? } else { now().0 };
        Ok(getdate_arr(ts, &default_zone(it)))
    });
    t.insert("localtime", |it, _, a| {
        let ts = if has(a, 0) && !arg(a, 0).is_null() { int(it, a, 0)? } else { now().0 };
        let assoc = has(a, 1) && arg(a, 1).truthy();
        let zone = default_zone(it);
        let (off, dst, _) = zone.at(ts);
        let local = ts + off as i64;
        let c = Civil::from_local(local, 0);
        let days = local.div_euclid(86400);
        let vals = [
            ("tm_sec", c.s),
            ("tm_min", c.i),
            ("tm_hour", c.h),
            ("tm_mday", c.d),
            ("tm_mon", c.m - 1),
            ("tm_year", c.y - 1900),
            ("tm_wday", civil::weekday(days)),
            ("tm_yday", days - civil::days_from_civil(c.y, 1, 1)),
            ("tm_isdst", dst as i64),
        ];
        let mut arr = PhpArray::new();
        for (k, v) in vals {
            if assoc {
                arr.insert(Key::Str(pstr(k)), Value::Int(v));
            } else {
                arr.push(Value::Int(v));
            }
        }
        Ok(Value::array(arr))
    });
    t.insert("date_default_timezone_set", |it, _, a| {
        let name = lossy(&s(it, a, 0)?);
        match Zone::parse(&name) {
            Some(z) if z.type_num() == 3 || name.eq_ignore_ascii_case("utc") => {
                it.ini.borrow_mut().set("date.timezone", &z.name());
                Ok(Value::Bool(true))
            }
            _ => {
                it.notice(&format!("date_default_timezone_set(): Timezone ID '{name}' is invalid"));
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("date_default_timezone_get", |it, _, _| Ok(Value::str(&default_zone(it).name())));
    t.insert("__phpx_interval_parse", |it, _, a| interval::parse_fn(it, a));
    t.insert("__phpx_interval_format", |it, _, a| interval::format_fn(it, a));
    t.insert("__phpx_interval_from_string", |it, _, a| interval::from_string_fn(it, a));
    t.insert("date_parse", |it, _, a| {
        let text = lossy(&s(it, a, 0)?);
        Ok(date_parse(&text))
    });
    t.insert("timezone_identifiers_list", |it, _, a| list_identifiers(it, &a, "timezone_identifiers_list"));
}

fn date_parse(text: &str) -> Value {
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: Value| {
        a.insert(Key::Str(pstr(k)), v);
    };
    let (p, err) = match parse::parse(text) {
        Ok(p) => (p, None),
        Err(e) => (parse::Parsed::default(), Some(e)),
    };
    let f = |v: Option<i64>| v.map(Value::Int).unwrap_or(Value::Bool(false));
    put("year", f(p.y));
    put("month", f(p.m));
    put("day", f(p.d));
    put("hour", f(p.h));
    put("minute", f(p.i));
    put("second", f(p.s));
    put("fraction", p.us.map(|u| Value::Float(u as f64 / 1e6)).unwrap_or(Value::Bool(false)));
    put("warning_count", Value::Int(0));
    put("warnings", Value::array(PhpArray::new()));
    let mut errs = PhpArray::new();
    if let Some(e) = &err {
        errs.insert(Key::Int(e.pos as i64), Value::str(e.msg));
    }
    put("error_count", Value::Int(errs.len() as i64));
    put("errors", Value::array(errs));
    put("is_localtime", Value::Bool(p.zone.is_some()));
    if let Some(z) = &p.zone {
        put("zone_type", Value::Int(z.type_num()));
        put("zone", Value::Int(z.offset(0) as i64));
        put("is_dst", Value::Bool(false));
        if let Zone::Abbr { name, .. } = z {
            put("tz_abbr", Value::str(name));
        }
        if let Zone::Id(t) = z {
            put("tz_id", Value::str(t.name()));
        }
    }
    if p.have_rel {
        let mut r = PhpArray::new();
        for (k, v) in [("year", p.rel.y), ("month", p.rel.m), ("day", p.rel.d), ("hour", p.rel.h), ("minute", p.rel.i), ("second", p.rel.s)] {
            r.insert(Key::Str(pstr(k)), Value::Int(v));
        }
        if let Some((wd, _)) = p.rel.weekday {
            r.insert(Key::Str(pstr("weekday")), Value::Int(wd));
        }
        put("relative", Value::array(r));
    }
    Value::array(a)
}
