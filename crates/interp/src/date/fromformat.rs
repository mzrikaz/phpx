//! DateTime::createFromFormat() and date_parse_from_format(): a port of
//! timelib's parse_from_format. Mismatches are recorded as errors at their
//! position and parsing carries on, as timelib does, so the error lists
//! (getLastErrors(), date_parse_from_format()) match PHP's.

use super::civil::{days_from_civil, days_in_month, weekday, Civil};
use super::zone::{abbreviation, identifier, parse_offset, Zone};
use super::DateState;

/// What the format matched. None = unset (TIMELIB_UNSET).
#[derive(Default, Clone)]
pub struct Parsed {
    pub y: Option<i64>,
    pub m: Option<i64>,
    pub d: Option<i64>,
    pub h: Option<i64>,
    pub i: Option<i64>,
    pub s: Option<i64>,
    pub us: Option<i64>,
    pub zone: Option<Zone>,
    /// `U`: seconds since the epoch.
    pub sse: Option<i64>,
    /// `D`/`l`: move to this weekday (0 = Sunday).
    pub weekday: Option<i64>,
    pub have_date: bool,
    pub have_time: bool,
    /// (position, message).
    pub errors: Vec<(usize, String)>,
    pub warnings: Vec<(usize, String)>,
}

const MONTHS: &[(&str, i64)] = &[
    ("jan", 1), ("feb", 2), ("mar", 3), ("apr", 4), ("may", 5), ("jun", 6), ("jul", 7), ("aug", 8), ("sep", 9), ("sept", 9), ("oct", 10), ("nov", 11), ("dec", 12),
    ("january", 1), ("february", 2), ("march", 3), ("april", 4), ("june", 6), ("july", 7), ("august", 8), ("september", 9), ("october", 10), ("november", 11), ("december", 12),
    ("i", 1), ("ii", 2), ("iii", 3), ("iv", 4), ("v", 5), ("vi", 6), ("vii", 7), ("viii", 8), ("ix", 9), ("x", 10), ("xi", 11), ("xii", 12),
];

const DAYS: &[(&str, i64)] = &[
    ("sun", 0), ("mon", 1), ("tue", 2), ("wed", 3), ("thu", 4), ("fri", 5), ("sat", 6),
    ("sunday", 0), ("monday", 1), ("tuesday", 2), ("wednesday", 3), ("thursday", 4), ("friday", 5), ("saturday", 6),
];

struct P<'a> {
    t: &'a [u8],
    p: usize,
}

impl P<'_> {
    fn cur(&self) -> u8 {
        self.t.get(self.p).copied().unwrap_or(0)
    }

    /// timelib_get_nr_ex: skip to the first digit, read up to `max`.
    fn nr(&mut self, max: usize) -> Option<(i64, usize)> {
        while !self.cur().is_ascii_digit() {
            if self.cur() == 0 {
                return None;
            }
            self.p += 1;
        }
        let start = self.p;
        while self.cur().is_ascii_digit() && self.p - start < max {
            self.p += 1;
        }
        let s = std::str::from_utf8(&self.t[start..self.p]).ok()?;
        Some((s.parse().unwrap_or(i64::MAX), self.p - start))
    }

    fn signed_nr(&mut self, max: usize) -> Option<i64> {
        let mut neg = false;
        while matches!(self.cur(), b'+' | b'-') {
            if self.cur() == b'-' {
                neg = !neg;
            }
            self.p += 1;
        }
        self.nr(max).map(|(n, _)| if neg { -n } else { n })
    }

    fn word(&mut self, stop: impl Fn(u8) -> bool) -> String {
        let start = self.p;
        while self.cur() != 0 && !stop(self.cur()) {
            self.p += 1;
        }
        String::from_utf8_lossy(&self.t[start..self.p]).to_ascii_lowercase()
    }
}

fn is_sep(c: u8) -> bool {
    matches!(c, b';' | b':' | b'/' | b'.' | b',' | b'-' | b'(' | b')')
}

/// timelib_parse_zone, for the forms apps use: offsets, Z/UTC/GMT,
/// abbreviations and identifiers. None: not found.
fn parse_zone(p: &mut P) -> Option<Zone> {
    while matches!(p.cur(), b' ' | b'\t' | b'(') {
        p.p += 1;
    }
    let rest = &p.t[p.p..];
    if rest.len() >= 3 && rest[..3].eq_ignore_ascii_case(b"gmt") && matches!(rest.get(3), Some(b'+' | b'-')) {
        p.p += 3;
    }
    if matches!(p.cur(), b'+' | b'-') {
        let start = p.p;
        p.p += 1;
        while p.cur().is_ascii_digit() || p.cur() == b':' {
            p.p += 1;
        }
        return parse_offset(std::str::from_utf8(&p.t[start..p.p]).unwrap_or("")).map(Zone::Offset);
    }
    let start = p.p;
    while p.cur().is_ascii_alphanumeric() || matches!(p.cur(), b'/' | b'_' | b'-' | b'+') {
        p.p += 1;
    }
    let w = std::str::from_utf8(&p.t[start..p.p]).unwrap_or("");
    if w.is_empty() {
        return None;
    }
    if w.eq_ignore_ascii_case("z") {
        return Some(Zone::Abbr { name: "Z".into(), offset: 0, dst: false });
    }
    if w.eq_ignore_ascii_case("utc") {
        return Some(Zone::Id(chrono_tz::Tz::UTC));
    }
    if !w.contains('/') {
        if let Some((o, dst)) = abbreviation(w) {
            return Some(Zone::Abbr { name: w.to_ascii_uppercase(), offset: o, dst });
        }
    }
    identifier(w).map(Zone::Id)
}

fn valid_date(y: i64, m: i64, d: i64) -> bool {
    (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)
}

pub fn parse(fmt: &[u8], text: &[u8]) -> Parsed {
    let mut r = Parsed::default();
    let mut p = P { t: text, p: 0 };
    let mut allow_extra = false;
    let mut k = 0;
    macro_rules! err {
        ($pos:expr, $msg:expr) => {
            r.errors.push(($pos, $msg.to_string()))
        };
    }
    while k < fmt.len() && p.cur() != 0 {
        let begin = p.p;
        let c = fmt[k];
        macro_rules! check_number {
            () => {
                if !p.cur().is_ascii_digit() {
                    err!(begin, "Unexpected data found.");
                }
            };
        }
        match c {
            b'D' | b'l' => {
                let w = p.word(|c| matches!(c, b' ' | b',' | b'\t' | b';' | b':' | b'/' | b'.' | b'-' | b'(' | b')'));
                match DAYS.iter().find(|d| d.0 == w) {
                    Some(d) => r.weekday = Some(d.1),
                    None => err!(begin, "A textual day could not be found"),
                }
            }
            b'd' | b'j' => {
                check_number!();
                match p.nr(2) {
                    Some((n, _)) => {
                        r.d = Some(n);
                        r.have_date = true;
                    }
                    None => err!(begin, "A two digit day could not be found"),
                }
            }
            b'S' => {
                if !p.cur().is_ascii_whitespace() {
                    let two = p.t.get(p.p..p.p + 2).map(|s| s.to_ascii_lowercase());
                    if matches!(two.as_deref(), Some(b"nd" | b"rd" | b"st" | b"th")) {
                        p.p += 2;
                    }
                }
            }
            b'z' => {
                check_number!();
                if r.y.is_none() {
                    err!(begin, "A 'day of year' can only come after a year has been found");
                }
                match p.nr(3) {
                    Some((n, _)) => {
                        if let Some(y) = r.y {
                            r.have_date = true;
                            let (yy, mm, dd) = super::civil::civil_from_days(days_from_civil(y, 1, 1) + n);
                            r.y = Some(yy);
                            r.m = Some(mm);
                            r.d = Some(dd);
                        }
                    }
                    None => err!(begin, "A three digit day-of-year could not be found"),
                }
            }
            b'm' | b'n' => {
                check_number!();
                match p.nr(2) {
                    Some((n, _)) => {
                        r.m = Some(n);
                        r.have_date = true;
                    }
                    None => err!(begin, "A two digit month could not be found"),
                }
            }
            b'M' | b'F' => {
                let w = p.word(|c| !c.is_ascii_alphabetic());
                match MONTHS.iter().find(|m| m.0 == w) {
                    Some(m) => {
                        r.m = Some(m.1);
                        r.have_date = true;
                    }
                    None => err!(begin, "A textual month could not be found"),
                }
            }
            b'y' => {
                check_number!();
                match p.nr(2) {
                    Some((n, len)) => {
                        r.have_date = true;
                        r.y = Some(if len >= 4 || n >= 100 { n } else if n < 70 { n + 2000 } else { n + 1900 });
                    }
                    None => err!(begin, "A two digit year could not be found"),
                }
            }
            b'Y' => {
                check_number!();
                match p.nr(4) {
                    Some((n, _)) => {
                        r.y = Some(n);
                        r.have_date = true;
                    }
                    None => err!(begin, "A four digit year could not be found"),
                }
            }
            b'X' | b'x' => {
                if !matches!(p.cur(), b'+' | b'-' | b'0'..=b'9') {
                    err!(begin, "Unexpected data found.");
                }
                match p.signed_nr(19) {
                    Some(n) => {
                        r.y = Some(n);
                        r.have_date = true;
                    }
                    None => err!(begin, "An expanded digit year could not be found"),
                }
            }
            b'h' | b'g' => {
                check_number!();
                match p.nr(2) {
                    Some((n, _)) if n > 12 => {
                        r.h = Some(n);
                        err!(begin, "Hour cannot be higher than 12");
                    }
                    Some((n, _)) => {
                        r.h = Some(n);
                        r.have_time = true;
                    }
                    None => err!(begin, "A two digit hour could not be found"),
                }
            }
            b'H' | b'G' => {
                check_number!();
                match p.nr(2) {
                    Some((n, _)) => {
                        r.h = Some(n);
                        r.have_time = true;
                    }
                    None => err!(begin, "A two digit hour could not be found"),
                }
            }
            b'a' | b'A' => {
                if r.h.is_none() {
                    err!(begin, "Meridian can only come after an hour has been found");
                }
                while p.cur() != 0 && !matches!(p.cur(), b'A' | b'a' | b'P' | b'p') {
                    p.p += 1;
                }
                let mut ok = p.cur() != 0;
                let pm = matches!(p.cur(), b'P' | b'p');
                if ok {
                    p.p += 1;
                    if p.cur() == b'.' {
                        p.p += 1;
                        if matches!(p.cur(), b'm' | b'M') {
                            p.p += 1;
                            if p.cur() == b'.' {
                                p.p += 1;
                            } else {
                                ok = false;
                            }
                        } else {
                            ok = false;
                        }
                    } else if matches!(p.cur(), b'm' | b'M') {
                        p.p += 1;
                    } else {
                        ok = false;
                    }
                }
                if !ok {
                    err!(begin, "A meridian could not be found");
                } else {
                    r.have_time = true;
                    if let Some(h) = r.h {
                        r.h = Some(match (pm, h) {
                            (false, 12) => 0,
                            (true, 12) => 12,
                            (true, h) => h + 12,
                            (false, h) => h,
                        });
                    }
                }
            }
            b'i' => {
                check_number!();
                match p.nr(2) {
                    Some((n, 2)) => {
                        r.i = Some(n);
                        r.have_time = true;
                    }
                    _ => err!(begin, "A two digit minute could not be found"),
                }
            }
            b's' => {
                check_number!();
                match p.nr(2) {
                    Some((n, 2)) => {
                        r.s = Some(n);
                        r.have_time = true;
                    }
                    _ => err!(begin, "A two digit second could not be found"),
                }
            }
            b'u' => {
                check_number!();
                match p.nr(6) {
                    Some((n, len)) if len >= 1 => r.us = Some(n * 10i64.pow(6 - len as u32)),
                    _ => err!(begin, "A six digit microsecond could not be found"),
                }
            }
            b'v' => {
                check_number!();
                match p.nr(3) {
                    Some((n, len)) if len >= 1 => r.us = Some(n * 10i64.pow(3 - len as u32) * 1000),
                    _ => err!(begin, "A three digit millisecond could not be found"),
                }
            }
            b' ' => {
                while matches!(p.cur(), b' ' | b'\t') {
                    p.p += 1;
                }
            }
            b'U' => {
                if !matches!(p.cur(), b'+' | b'-' | b'0'..=b'9') {
                    err!(begin, "Unexpected data found.");
                }
                if let Some(n) = p.signed_nr(24) {
                    r.sse = Some(n);
                    r.zone = Some(Zone::Offset(0));
                    let c = Civil::from_local(n, 0);
                    (r.y, r.m, r.d, r.h, r.i, r.s) = (Some(c.y), Some(c.m), Some(c.d), Some(c.h), Some(c.i), Some(c.s));
                }
            }
            b'#' => {
                if is_sep(p.cur()) {
                    p.p += 1;
                } else {
                    err!(begin, "The separation symbol ([;:/.,-]) could not be found");
                }
            }
            b';' | b':' | b'/' | b'.' | b',' | b'-' | b'(' | b')' => {
                if p.cur() == c {
                    p.p += 1;
                } else {
                    err!(begin, "The separation symbol could not be found");
                }
            }
            b'!' => reset_all(&mut r),
            b'|' => reset_unset(&mut r),
            b'?' => p.p += 1,
            b'\\' => {
                if k + 1 >= fmt.len() {
                    err!(begin, "Escaped character expected");
                } else {
                    k += 1;
                    if p.cur() == fmt[k] {
                        p.p += 1;
                    } else {
                        err!(begin, "The escaped character could not be found");
                    }
                }
            }
            b'*' => {
                p.p += 1;
                while p.cur() != 0 && !matches!(p.cur(), b' ' | b'\t' | b'.' | b',' | b':' | b';' | b'/' | b'-' | b'0'..=b'9') {
                    p.p += 1;
                }
            }
            b'+' => allow_extra = true,
            b'e' | b'T' | b'O' | b'P' | b'p' => match parse_zone(&mut p) {
                Some(z) => r.zone = Some(z),
                None => err!(begin, "The timezone could not be found in the database"),
            },
            lit => {
                if p.cur() != lit {
                    err!(begin, "The format separator does not match");
                }
                p.p += 1;
            }
        }
        k += 1;
    }
    if p.p < text.len() {
        if allow_extra {
            r.warnings.push((p.p, "Trailing data".into()));
        } else {
            err!(p.p, "Trailing data");
        }
    }
    while k < fmt.len() {
        match fmt[k] {
            b'!' => reset_all(&mut r),
            b'|' => reset_unset(&mut r),
            b'+' => {}
            _ => {
                err!(p.p, "Not enough data available to satisfy format");
                break;
            }
        }
        k += 1;
    }
    if r.h.is_some() || r.i.is_some() || r.s.is_some() || r.us.is_some() {
        r.h.get_or_insert(0);
        r.i.get_or_insert(0);
        r.s.get_or_insert(0);
        r.us.get_or_insert(0);
    }
    if let (Some(h), Some(i), Some(s)) = (r.h, r.i, r.s) {
        if !(0..24).contains(&h) || !(0..60).contains(&i) || !(0..60).contains(&s) {
            r.warnings.push((p.p, "The parsed time was invalid".into()));
        }
    }
    if let (Some(y), Some(m), Some(d)) = (r.y, r.m, r.d) {
        if !valid_date(y, m, d) {
            r.warnings.push((p.p, "The parsed date was invalid".into()));
        }
    }
    r
}

fn reset_all(r: &mut Parsed) {
    (r.y, r.m, r.d, r.h, r.i, r.s, r.us) = (Some(1970), Some(1), Some(1), Some(0), Some(0), Some(0), Some(0));
}

fn reset_unset(r: &mut Parsed) {
    r.y.get_or_insert(1970);
    r.m.get_or_insert(1);
    r.d.get_or_insert(1);
    r.h.get_or_insert(0);
    r.i.get_or_insert(0);
    r.s.get_or_insert(0);
    r.us.get_or_insert(0);
}

/// createFromFormat's date: holes filled from now (time included:
/// TIMELIB_OVERRIDE_TIME), in the parsed zone or `zone`.
pub fn to_state(r: &Parsed, now_ts: i64, now_us: i64, zone: &Zone) -> DateState {
    if let (Some(sse), None) = (r.sse, r.weekday) {
        let z = r.zone.clone().unwrap_or(Zone::Offset(0));
        return DateState { ts: sse, us: r.us.unwrap_or(0), zone: z };
    }
    let zone = r.zone.clone().unwrap_or_else(|| zone.clone());
    let now = Civil::from_local(now_ts + zone.offset(now_ts) as i64, now_us);
    let any = r.y.is_some() || r.m.is_some() || r.d.is_some() || r.h.is_some() || r.i.is_some() || r.s.is_some();
    let mut c = Civil {
        y: r.y.unwrap_or(now.y),
        m: r.m.unwrap_or(now.m),
        d: r.d.unwrap_or(now.d),
        h: r.h.unwrap_or(now.h),
        i: r.i.unwrap_or(now.i),
        s: r.s.unwrap_or(now.s),
        us: r.us.unwrap_or(if any { 0 } else { now.us }),
    };
    if let Some(wd) = r.weekday {
        let days = days_from_civil(c.y, c.m, c.d);
        let delta = (wd - weekday(days)).rem_euclid(7);
        let (y, m, d) = super::civil::civil_from_days(days + delta);
        (c.y, c.m, c.d) = (y, m, d);
    }
    let (local, us) = c.to_local();
    DateState { ts: zone.to_utc(local), us, zone }
}
