//! DateTime::createFromFormat().

use super::civil::Civil;
use super::zone::{abbreviation, identifier, parse_offset, Zone};
use super::DateState;

#[derive(Default)]
struct F {
    y: Option<i64>,
    m: Option<i64>,
    d: Option<i64>,
    doy: Option<i64>,
    h: Option<i64>,
    i: Option<i64>,
    s: Option<i64>,
    us: Option<i64>,
    pm: Option<bool>,
    zone: Option<Zone>,
    ts: Option<i64>,
}

const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

pub fn parse(fmt: &[u8], text: &[u8], now_ts: i64, now_us: i64, zone: &Zone) -> Result<DateState, String> {
    let mut f = F::default();
    let mut p = 0usize;
    let mut reset_all = false;
    let mut reset_unset = false;
    let mut allow_trailing = false;
    let digits = |p: usize, max: usize| -> (i64, usize) {
        let mut j = p;
        while j < text.len() && j - p < max && text[j].is_ascii_digit() {
            j += 1;
        }
        (std::str::from_utf8(&text[p..j]).ok().and_then(|s| s.parse().ok()).unwrap_or(0), j - p)
    };
    let mut k = 0;
    while k < fmt.len() {
        let c = fmt[k];
        k += 1;
        macro_rules! num {
            ($max:expr, $min:expr) => {{
                let (n, len) = digits(p, $max);
                if len < $min {
                    return Err("A two digit value could not be found".into());
                }
                p += len;
                n
            }};
        }
        match c {
            b'd' | b'j' => f.d = Some(num!(2, 1)),
            b'S' => p = (p + 2).min(text.len()),
            b'z' => f.doy = Some(num!(3, 1)),
            b'm' | b'n' => f.m = Some(num!(2, 1)),
            b'Y' => {
                let neg = text.get(p) == Some(&b'-');
                if neg {
                    p += 1;
                }
                let y = num!(4, 1);
                f.y = Some(if neg { -y } else { y });
            }
            b'y' => {
                let y = num!(2, 2);
                f.y = Some(if y < 70 { 2000 + y } else { 1900 + y });
            }
            b'a' | b'A' => {
                let w = text.get(p..p + 2).map(|s| s.to_ascii_lowercase());
                match w.as_deref() {
                    Some(b"am") => f.pm = Some(false),
                    Some(b"pm") => f.pm = Some(true),
                    _ => return Err("A meridian could not be found".into()),
                }
                p += 2;
            }
            b'g' | b'h' | b'G' | b'H' => f.h = Some(num!(2, 1)),
            b'i' => f.i = Some(num!(2, 2)),
            b's' => f.s = Some(num!(2, 2)),
            b'v' => f.us = Some(num!(3, 3) * 1000),
            b'u' => {
                let (n, len) = digits(p, 9);
                if len == 0 {
                    return Err("A six digit microsecond value could not be found".into());
                }
                p += len;
                let mut us = n;
                if len > 6 {
                    us /= 10i64.pow(len as u32 - 6);
                } else {
                    us *= 10i64.pow(6 - len as u32);
                }
                f.us = Some(us);
            }
            b'D' | b'l' => {
                while p < text.len() && text[p].is_ascii_alphabetic() {
                    p += 1;
                }
            }
            b'M' | b'F' => {
                let start = p;
                while p < text.len() && text[p].is_ascii_alphabetic() {
                    p += 1;
                }
                let w = String::from_utf8_lossy(&text[start..p]).to_ascii_lowercase();
                let m = MONTHS.iter().position(|m| *m == w || (w.len() == 3 && m.starts_with(&w)));
                match m {
                    Some(m) => f.m = Some(m as i64 + 1),
                    None => return Err("A textual month could not be found".into()),
                }
            }
            b'U' => {
                let neg = text.get(p) == Some(&b'-');
                if neg {
                    p += 1;
                }
                let n = num!(20, 1);
                f.ts = Some(if neg { -n } else { n });
            }
            b'e' | b'T' | b'O' | b'P' | b'p' => {
                let start = p;
                if matches!(text.get(p), Some(b'+' | b'-')) {
                    p += 1;
                    while p < text.len() && (text[p].is_ascii_digit() || text[p] == b':') {
                        p += 1;
                    }
                    let o = parse_offset(std::str::from_utf8(&text[start..p]).unwrap_or("")).ok_or("The timezone could not be found in the database")?;
                    f.zone = Some(Zone::Offset(o));
                } else {
                    while p < text.len() && (text[p].is_ascii_alphanumeric() || matches!(text[p], b'/' | b'_' | b'-')) {
                        p += 1;
                    }
                    let w = std::str::from_utf8(&text[start..p]).unwrap_or("");
                    f.zone = Some(if w.eq_ignore_ascii_case("z") {
                        Zone::Abbr { name: "Z".into(), offset: 0, dst: false }
                    } else if w.eq_ignore_ascii_case("utc") {
                        Zone::Id(chrono_tz::Tz::UTC)
                    } else if let Some((o, dst)) = abbreviation(w).filter(|_| !w.contains('/')) {
                        Zone::Abbr { name: w.to_ascii_uppercase(), offset: o, dst }
                    } else {
                        Zone::Id(identifier(w).ok_or("The timezone could not be found in the database")?)
                    });
                }
            }
            b'!' => reset_all = true,
            b'|' => reset_unset = true,
            b'+' => allow_trailing = true,
            b'#' => {
                if matches!(text.get(p), Some(b';' | b':' | b'/' | b'.' | b',' | b'-' | b'(' | b')')) {
                    p += 1;
                } else {
                    return Err("The separation symbol ([;:/.,-]) could not be found".into());
                }
            }
            b'?' => {
                if p >= text.len() {
                    return Err("Unexpected data found.".into());
                }
                p += 1;
            }
            b'*' => {
                while p < text.len() && !matches!(text[p], b' ' | b',' | b';' | b':' | b'/' | b'.' | b'-' | b'(' | b')') && !text[p].is_ascii_digit() {
                    p += 1;
                }
            }
            b' ' => {
                while p < text.len() && matches!(text[p], b' ' | b'\t') {
                    p += 1;
                }
            }
            b'\\' => {
                let lit = fmt.get(k).copied();
                k += 1;
                if text.get(p).copied() != lit {
                    return Err("The escaped character could not be found".into());
                }
                p += 1;
            }
            lit => {
                if text.get(p) != Some(&lit) {
                    return Err("The separation symbol could not be found".into());
                }
                p += 1;
            }
        }
    }
    if p < text.len() && !allow_trailing {
        return Err("Trailing data".into());
    }
    let zone = f.zone.clone().unwrap_or_else(|| zone.clone());
    if let Some(ts) = f.ts {
        let z = f.zone.clone().unwrap_or(Zone::Offset(0));
        return Ok(DateState { ts, us: f.us.unwrap_or(0), zone: z });
    }
    let now = Civil::from_local(now_ts + zone.offset(now_ts) as i64, now_us);
    let epoch = Civil { y: 1970, m: 1, d: 1, h: 0, i: 0, s: 0, us: 0 };
    let base = if reset_all || reset_unset { epoch } else { now };
    let any_time = f.h.is_some() || f.i.is_some() || f.s.is_some() || f.us.is_some();
    let t0 = |v: Option<i64>, b: i64| v.unwrap_or(if any_time { 0 } else { b });
    let mut h = t0(f.h, base.h);
    if let Some(pm) = f.pm {
        h = match (h, pm) {
            (12, false) => 0,
            (12, true) => 12,
            (h, true) => h + 12,
            (h, false) => h,
        };
    }
    let mut c = Civil {
        y: f.y.unwrap_or(base.y),
        m: f.m.unwrap_or(base.m),
        d: f.d.unwrap_or(base.d),
        h,
        i: t0(f.i, base.i),
        s: t0(f.s, base.s),
        us: f.us.unwrap_or(if any_time || reset_all || reset_unset { 0 } else { base.us }),
    };
    if let Some(doy) = f.doy {
        c.m = 1;
        c.d = doy + 1;
    }
    let (local, us) = c.to_local();
    Ok(DateState { ts: zone.to_utc(local), us, zone })
}
