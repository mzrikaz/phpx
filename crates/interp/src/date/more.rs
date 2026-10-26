//! strftime(), date_parse_from_format(), timezone_name_from_abbr(),
//! timezone_version_get() and the sun functions (timelib's astro.c).

use std::collections::HashMap;

use super::civil::{civil_from_days, days_from_civil, Civil};
use super::zone::Zone;
use super::{abbr_map, default_zone, fromformat};
use crate::builtins::{arg, has, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{pstr, Key, PhpArray, Value};

fn now() -> (i64, i64) {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() as i64, d.subsec_micros() as i64)
}

// ---- strftime ----------------------------------------------------------------------

fn strftime(it: &Interp, a: &[Value], gmt: bool) -> R<Value> {
    let name = if gmt { "gmstrftime" } else { "strftime" };
    it.deprecated(&format!("Function {name}() is deprecated since 8.1, use IntlDateFormatter::format() instead"));
    let fmt = s(it, a, 0)?.to_vec();
    if fmt.is_empty() {
        return Ok(Value::Bool(false));
    }
    let ts = match a.get(1) {
        Some(v) if !v.is_null() => it.to_int(v)?,
        _ => now().0,
    };
    let zone = if gmt { Zone::Id(chrono_tz::Tz::UTC) } else { default_zone(it) };
    let (offset, dst, abbr) = zone.at(ts);
    let c = Civil::from_local(ts + offset as i64, 0);
    let days = days_from_civil(c.y, c.m, c.d);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_sec = c.s as i32;
    tm.tm_min = c.i as i32;
    tm.tm_hour = c.h as i32;
    tm.tm_mday = c.d as i32;
    tm.tm_mon = (c.m - 1) as i32;
    tm.tm_year = (c.y - 1900) as i32;
    tm.tm_wday = (days + 4).rem_euclid(7) as i32;
    tm.tm_yday = (days - days_from_civil(c.y, 1, 1)) as i32;
    tm.tm_isdst = dst as i32;
    tm.tm_gmtoff = offset as libc::c_long;
    let zone_name = std::ffi::CString::new(if gmt { "GMT".to_string() } else { abbr }).unwrap_or_default();
    tm.tm_zone = zone_name.as_ptr();
    let cfmt = std::ffi::CString::new(fmt.iter().copied().take_while(|&c| c != 0).collect::<Vec<u8>>()).unwrap();
    let mut size = 256usize;
    loop {
        let mut buf = vec![0u8; size];
        let n = unsafe { libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, size, cfmt.as_ptr(), &tm) };
        if n > 0 || size > 1 << 20 {
            buf.truncate(n);
            return Ok(Value::bytes(buf));
        }
        size *= 4;
    }
}

// ---- date_parse_from_format ------------------------------------------------------------

fn parse_from_format(it: &Interp, a: &[Value]) -> R<Value> {
    let fmt = s(it, a, 0)?.to_vec();
    let text = s(it, a, 1)?.to_vec();
    let r = fromformat::parse(&fmt, &text);
    let mut out = PhpArray::new();
    let int_or_false = |v: Option<i64>| v.map_or(Value::Bool(false), Value::Int);
    out.insert(Key::Str(pstr("year")), int_or_false(r.y));
    out.insert(Key::Str(pstr("month")), int_or_false(r.m));
    out.insert(Key::Str(pstr("day")), int_or_false(r.d));
    out.insert(Key::Str(pstr("hour")), int_or_false(r.h));
    out.insert(Key::Str(pstr("minute")), int_or_false(r.i));
    out.insert(Key::Str(pstr("second")), int_or_false(r.s));
    out.insert(Key::Str(pstr("fraction")), r.us.map_or(Value::Bool(false), |us| Value::Float(us as f64 / 1e6)));
    super::errors_array(&r.warnings, &r.errors, &mut out);
    out.insert(Key::Str(pstr("is_localtime")), Value::Bool(r.zone.is_some()));
    if let Some(z) = &r.zone {
        let at = r.sse.unwrap_or_else(|| now().0);
        out.insert(Key::Str(pstr("zone_type")), Value::Int(z.type_num()));
        let (off, dst, abbr) = z.at(at);
        if z.type_num() != 3 {
            out.insert(Key::Str(pstr("zone")), Value::Int(off as i64 - if dst { 3600 } else { 0 }));
            out.insert(Key::Str(pstr("is_dst")), Value::Bool(dst));
        }
        match z.type_num() {
            2 => {
                out.insert(Key::Str(pstr("tz_abbr")), Value::str(&abbr));
            }
            3 => {
                out.insert(Key::Str(pstr("tz_id")), Value::str(&z.name()));
            }
            _ => {}
        }
    }
    if let Some(wd) = r.weekday {
        let mut rel = PhpArray::new();
        for k in ["year", "month", "day", "hour", "minute", "second"] {
            rel.insert(Key::Str(pstr(k)), Value::Int(0));
        }
        rel.insert(Key::Str(pstr("weekday")), Value::Int(wd));
        out.insert(Key::Str(pstr("relative")), Value::array(rel));
    }
    Ok(Value::array(out))
}

// ---- abbreviations -----------------------------------------------------------------

fn name_from_abbr(abbr: &str, offset: i64, isdst: i64) -> Option<&'static str> {
    if abbr.eq_ignore_ascii_case("utc") || abbr.eq_ignore_ascii_case("gmt") {
        return Some("UTC");
    }
    let mut first = None;
    for e in abbr_map::LOOKUP {
        if e.0.eq_ignore_ascii_case(abbr) {
            if first.is_none() {
                first = Some(e);
                if offset == -1 {
                    break;
                }
            }
            if e.2 as i64 == offset {
                first = Some(e);
                break;
            }
        }
    }
    if let Some(e) = first {
        return (!e.3.is_empty()).then_some(e.3);
    }
    abbr_map::FALLBACK.iter().find(|e| e.2 as i64 == offset && e.1 as i64 == isdst).map(|e| e.3)
}

// ---- sun ---------------------------------------------------------------------------

fn rev(x: f64) -> f64 {
    x - 360.0 * (x / 360.0).floor()
}

fn rev180(x: f64) -> f64 {
    x - 360.0 * (x / 360.0 + 0.5).floor()
}

fn sind(x: f64) -> f64 {
    x.to_radians().sin()
}

fn cosd(x: f64) -> f64 {
    x.to_radians().cos()
}

fn sun_ra_dec(d: f64) -> (f64, f64, f64) {
    let m = rev(356.0470 + 0.9856002585 * d);
    let w = 282.9404 + 4.70935E-5 * d;
    let e = 0.016709 - 1.151E-9 * d;
    let ea = m + e * (180.0 / std::f64::consts::PI) * sind(m) * (1.0 + e * cosd(m));
    let x = cosd(ea) - e;
    let y = (1.0 - e * e).sqrt() * sind(ea);
    let r = (x * x + y * y).sqrt();
    let v = y.atan2(x).to_degrees();
    let mut lon = v + w;
    if lon >= 360.0 {
        lon -= 360.0;
    }
    let (x, y) = (r * cosd(lon), r * sind(lon));
    let obl = 23.4393 - 3.563E-7 * d;
    let z = y * sind(obl);
    let y = y * cosd(obl);
    (y.atan2(x).to_degrees(), z.atan2((x * x + y * y).sqrt()).to_degrees(), r)
}

struct RiseSet {
    rc: i32,
    h_rise: f64,
    h_set: f64,
    rise: i64,
    set: i64,
    transit: i64,
}

/// timelib_astro_rise_set_altitude for the local day of `ts` in `zone`.
fn rise_set(ts: i64, zone: &Zone, lon: f64, lat: f64, altit: f64, upper_limb: bool) -> RiseSet {
    let local = Civil::from_local(ts + zone.offset(ts) as i64, 0);
    let noon_local = days_from_civil(local.y, local.m, local.d) * 86400 + 12 * 3600;
    let t_loc = zone.to_utc(noon_local);
    let t_utc = days_from_civil(local.y, local.m, local.d) * 86400;
    let d = (t_utc as f64 / 86400.0 + 2440587.5 - 2451545.0) + 2.0 - lon / 360.0;
    let sidtime = rev(rev((180.0 + 356.0470 + 282.9404) + (0.9856002585 + 4.70935E-5) * d) + 180.0 + lon);
    let (sra, sdec, sr) = sun_ra_dec(d);
    let tsouth = 12.0 - rev180(sidtime - sra) / 15.0;
    let sradius = 0.2666 / sr;
    let altit = if upper_limb { altit - sradius } else { altit };
    let cost = (sind(altit) - sind(lat) * sind(sdec)) / (cosd(lat) * cosd(sdec));
    let transit = (t_utc as f64 + tsouth * 3600.0) as i64;
    let mut r = RiseSet { rc: 0, h_rise: 0.0, h_set: 0.0, rise: 0, set: 0, transit };
    if cost >= 1.0 {
        r.rc = -1;
        r.rise = transit;
        r.set = transit;
    } else if cost <= -1.0 {
        r.rc = 1;
        r.rise = t_loc - 12 * 3600;
        r.set = t_loc + 12 * 3600;
    } else {
        let t = cost.acos().to_degrees() / 15.0;
        r.rise = ((tsouth - t) * 3600.0 + t_utc as f64) as i64;
        r.set = ((tsouth + t) * 3600.0 + t_utc as f64) as i64;
        r.h_rise = tsouth - t;
        r.h_set = tsouth + t;
    }
    r
}

fn ini_f(it: &Interp, k: &str, default: f64) -> f64 {
    it.ini_get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn sunrise_sunset(it: &Interp, a: &[Value], sunset: bool) -> R<Value> {
    let name = if sunset { "date_sunset" } else { "date_sunrise" };
    it.deprecated(&format!("Function {name}() is deprecated since 8.1, use date_sun_info() instead"));
    let ts = it.to_int(&arg(a, 0))?;
    let fmt = if has(a, 1) { it.to_int(&a[1])? } else { 1 };
    let opt = |i: usize| -> R<Option<f64>> {
        match a.get(i) {
            Some(v) if !v.is_null() => Ok(Some(it.to_float(v)?)),
            _ => Ok(None),
        }
    };
    let lat = opt(2)?.unwrap_or_else(|| ini_f(it, "date.default_latitude", 31.7667));
    let lon = opt(3)?.unwrap_or_else(|| ini_f(it, "date.default_longitude", 35.2333));
    let zenith = opt(4)?.unwrap_or_else(|| ini_f(it, if sunset { "date.sunset_zenith" } else { "date.sunrise_zenith" }, 90.833333));
    if !(0..=2).contains(&fmt) {
        return Err(it.error("ValueError", &format!("{name}(): Argument #2 ($returnFormat) must be one of SUNFUNCS_RET_TIMESTAMP, SUNFUNCS_RET_STRING, or SUNFUNCS_RET_DOUBLE")));
    }
    if !lat.is_finite() || !lon.is_finite() {
        return Ok(Value::Bool(false));
    }
    let zone = default_zone(it);
    let gmt_offset = opt(5)?.unwrap_or_else(|| zone.offset(now().0) as f64 / 3600.0);
    let r = rise_set(ts, &zone, lon, lat, 90.0 - zenith, true);
    if r.rc != 0 {
        return Ok(Value::Bool(false));
    }
    if fmt == 0 {
        return Ok(Value::Int(if sunset { r.set } else { r.rise }));
    }
    let mut n = (if sunset { r.h_set } else { r.h_rise }) + gmt_offset;
    if !(0.0..=24.0).contains(&n) {
        n -= (n / 24.0).floor() * 24.0;
    }
    if !(0.0..=24.0).contains(&n) {
        return Ok(Value::Bool(false));
    }
    Ok(if fmt == 1 { Value::str(&format!("{:02}:{:02}", n as i64, (60.0 * (n - n.trunc())) as i64)) } else { Value::Float(n) })
}

fn sun_info(it: &Interp, a: &[Value]) -> R<Value> {
    let ts = it.to_int(&arg(a, 0))?;
    let lat = it.to_float(&arg(a, 1))?;
    let lon = it.to_float(&arg(a, 2))?;
    if !lat.is_finite() {
        return Err(it.error("ValueError", "date_sun_info(): Argument #2 ($latitude) must be finite"));
    }
    if !lon.is_finite() {
        return Err(it.error("ValueError", "date_sun_info(): Argument #3 ($longitude) must be finite"));
    }
    let zone = default_zone(it);
    let mut out = PhpArray::new();
    let mut put = |k: &str, v: Value| {
        out.insert(Key::Str(pstr(k)), v);
    };
    let r = rise_set(ts, &zone, lon, lat, -35.0 / 60.0, true);
    match r.rc {
        -1 => {
            put("sunrise", Value::Bool(false));
            put("sunset", Value::Bool(false));
        }
        1 => {
            put("sunrise", Value::Bool(true));
            put("sunset", Value::Bool(true));
        }
        _ => {
            put("sunrise", Value::Int(r.rise));
            put("sunset", Value::Int(r.set));
        }
    }
    put("transit", Value::Int(r.transit));
    for (alt, begin, end) in [(-6.0, "civil_twilight_begin", "civil_twilight_end"), (-12.0, "nautical_twilight_begin", "nautical_twilight_end"), (-18.0, "astronomical_twilight_begin", "astronomical_twilight_end")] {
        let r = rise_set(ts, &zone, lon, lat, alt, false);
        match r.rc {
            -1 => {
                put(begin, Value::Bool(false));
                put(end, Value::Bool(false));
            }
            1 => {
                put(begin, Value::Bool(true));
                put(end, Value::Bool(true));
            }
            _ => {
                put(begin, Value::Int(r.rise));
                put(end, Value::Int(r.set));
            }
        }
    }
    let _ = civil_from_days;
    Ok(Value::array(out))
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("strftime", |it, _, a| strftime(it, a, false));
    t.insert("gmstrftime", |it, _, a| strftime(it, a, true));
    t.insert("date_parse_from_format", |it, _, a| parse_from_format(it, a));
    t.insert("timezone_name_from_abbr", |it, _, a| {
        let abbr = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        let off = if has(a, 1) { it.to_int(&a[1])? } else { -1 };
        let dst = if has(a, 2) { it.to_int(&a[2])? } else { -1 };
        Ok(name_from_abbr(&abbr, off, dst).map(Value::str).unwrap_or(Value::Bool(false)))
    });
    t.insert("timezone_version_get", |_, _, _| Ok(Value::str("2025.2")));
    t.insert("date_sunrise", |it, _, a| sunrise_sunset(it, a, false));
    t.insert("date_sunset", |it, _, a| sunrise_sunset(it, a, true));
    t.insert("date_sun_info", |it, _, a| sun_info(it, a));
}
