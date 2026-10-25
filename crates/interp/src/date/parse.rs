//! Date/time strings as strtotime() and `new DateTime()` read them: the
//! formats of PHP's timelib (absolute dates and times, relative phrases,
//! time zones), then applied to a base time.

use super::civil::{days_in_month, from_iso_week, weekday, Civil};
use super::zone::{abbreviation, identifier, parse_offset, Zone};

#[derive(Clone, Debug, Default)]
pub struct Rel {
    pub y: i64,
    pub m: i64,
    pub d: i64,
    pub h: i64,
    pub i: i64,
    pub s: i64,
    pub us: i64,
    /// Weekday (0 = Sunday) and amount: 0 "this", n > 0 the n-th next,
    /// n < 0 the n-th previous.
    pub weekday: Option<(i64, i64)>,
    /// Business days to add.
    pub weekdays: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FirstLast {
    First,
    Last,
}

#[derive(Clone, Debug, Default)]
pub struct Parsed {
    pub y: Option<i64>,
    pub m: Option<i64>,
    pub d: Option<i64>,
    pub h: Option<i64>,
    pub i: Option<i64>,
    pub s: Option<i64>,
    pub us: Option<i64>,
    pub zone: Option<Zone>,
    pub rel: Rel,
    pub have_rel: bool,
    /// `today`, `midnight`, weekday names: time becomes 00:00:00.
    pub reset_time: bool,
    pub first_last: Option<FirstLast>,
    /// "second monday of": (n, weekday); n = -1 for "last".
    pub nth_weekday_of: Option<(i64, i64)>,
    /// `@1700000000.5`
    pub timestamp: Option<(i64, i64)>,
}

#[derive(Debug)]
pub struct ParseError {
    pub pos: usize,
    pub ch: char,
    pub msg: &'static str,
}

const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
const DAYS: [&str; 7] = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];

fn month_of(w: &str) -> Option<i64> {
    if w == "sept" {
        return Some(9);
    }
    MONTHS.iter().position(|m| *m == w || (w.len() == 3 && m.starts_with(w))).map(|i| i as i64 + 1)
}

fn weekday_of(w: &str) -> Option<i64> {
    let abbr = |d: &str| match d {
        "tues" => Some(2),
        "wednes" => Some(3),
        "thur" | "thurs" => Some(4),
        _ => None,
    };
    DAYS.iter()
        .position(|d| *d == w || (w.len() == 3 && d.starts_with(w)) || d.strip_suffix("day") == Some(w))
        .map(|i| i as i64)
        .or_else(|| abbr(w))
}

fn ordinal(w: &str) -> Option<i64> {
    Some(match w {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "fifth" => 5,
        "sixth" => 6,
        "seventh" => 7,
        "eighth" => 8,
        "ninth" => 9,
        "tenth" => 10,
        "eleventh" => 11,
        "twelfth" => 12,
        "next" => 1,
        "last" | "previous" => -1,
        "this" => 0,
        _ => return None,
    })
}

enum Unit {
    Us,
    Ms,
    S,
    I,
    H,
    D,
    W,
    Fortnight,
    M,
    Y,
    Weekday,
    Day(i64),
}

fn unit_of(w: &str) -> Option<Unit> {
    Some(match w {
        "usec" | "usecs" | "microsecond" | "microseconds" | "µs" | "µsec" | "µsecs" => Unit::Us,
        "msec" | "msecs" | "millisecond" | "milliseconds" | "ms" => Unit::Ms,
        "sec" | "secs" | "second" | "seconds" => Unit::S,
        "min" | "mins" | "minute" | "minutes" => Unit::I,
        "hour" | "hours" => Unit::H,
        "day" | "days" => Unit::D,
        "week" | "weeks" => Unit::W,
        "fortnight" | "fortnights" | "forthnight" | "forthnights" => Unit::Fortnight,
        "month" | "months" => Unit::M,
        "year" | "years" => Unit::Y,
        "weekday" | "weekdays" => Unit::Weekday,
        w => Unit::Day(weekday_of(w)?),
    })
}

struct P<'a> {
    s: &'a [u8],
    pos: usize,
    out: Parsed,
    have_date: bool,
    have_time: bool,
    have_zone: bool,
}

type Res<T> = Result<T, ParseError>;

impl<'a> P<'a> {
    fn err(&self, at: usize, msg: &'static str) -> ParseError {
        let ch = self.s.get(at).map(|&b| b as char).unwrap_or('\0');
        ParseError { pos: at, ch, msg }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn at(&self, i: usize) -> Option<u8> {
        self.s.get(i).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r' | b',')) {
            self.pos += 1;
        }
    }

    fn digits_at(&self, i: usize) -> usize {
        self.s[i.min(self.s.len())..].iter().take_while(|b| b.is_ascii_digit()).count()
    }

    fn num(&self, a: usize, n: usize) -> i64 {
        std::str::from_utf8(&self.s[a..a + n]).ok().and_then(|t| t.parse().ok()).unwrap_or(0)
    }

    /// Lowercase word at `i` (letters, and `/ _ -` inside zone names).
    fn word_at(&self, i: usize) -> (String, usize) {
        let mut j = i;
        while j < self.s.len() && (self.s[j].is_ascii_alphabetic() || self.s[j] >= 0x80) {
            j += 1;
        }
        (String::from_utf8_lossy(&self.s[i..j]).to_ascii_lowercase(), j)
    }

    fn set_date(&mut self, y: Option<i64>, m: i64, d: i64, at: usize) -> Res<()> {
        if self.have_date {
            return Err(self.err(at, "Double date specification"));
        }
        self.have_date = true;
        if let Some(y) = y {
            self.out.y = Some(y);
        }
        self.out.m = Some(m);
        self.out.d = Some(d);
        Ok(())
    }

    fn set_time(&mut self, h: i64, i: i64, s: i64, us: i64, at: usize) -> Res<()> {
        if self.have_time {
            return Err(self.err(at, "Double time specification"));
        }
        self.have_time = true;
        self.out.h = Some(h);
        self.out.i = Some(i);
        self.out.s = Some(s);
        self.out.us = Some(us);
        Ok(())
    }

    /// `today`, `tomorrow`, ...: drop a time given so far; 00:00:00 unless
    /// a later one follows.
    fn unset_time(&mut self) {
        self.have_time = false;
        self.out.h = None;
        self.out.i = None;
        self.out.s = None;
        self.out.us = None;
        self.out.reset_time = true;
    }

    fn set_zone(&mut self, z: Zone, at: usize) -> Res<()> {
        if self.have_zone {
            return Err(self.err(at, "Double timezone specification"));
        }
        self.have_zone = true;
        self.out.zone = Some(z);
        Ok(())
    }

    /// Two-digit years: 00-69 -> 2000s, 70-99 -> 1900s.
    fn year(&self, a: usize, n: usize) -> i64 {
        let y = self.num(a, n);
        if n <= 2 {
            if y < 70 { 2000 + y } else { 1900 + y }
        } else {
            y
        }
    }

    /// `.123456` / `:123` after seconds -> microseconds.
    fn fraction(&mut self) -> i64 {
        if matches!(self.peek(), Some(b'.' | b',' | b':')) && self.at(self.pos + 1).is_some_and(|b| b.is_ascii_digit()) {
            let a = self.pos + 1;
            let n = self.digits_at(a);
            self.pos = a + n;
            let digits = &self.s[a..a + n.min(6)];
            let mut us: i64 = std::str::from_utf8(digits).unwrap().parse().unwrap_or(0);
            for _ in digits.len()..6 {
                us *= 10;
            }
            return us;
        }
        0
    }

    /// After a clock time: optional `am`/`pm` (also `a.m.`).
    fn meridian(&mut self, h: i64) -> Res<Option<i64>> {
        let save = self.pos;
        while self.peek() == Some(b' ') {
            self.pos += 1;
        }
        let c = self.peek().map(|b| b.to_ascii_lowercase());
        if matches!(c, Some(b'a' | b'p')) {
            let mut j = self.pos + 1;
            if self.at(j) == Some(b'.') {
                j += 1;
            }
            if self.at(j).is_some_and(|b| b.eq_ignore_ascii_case(&b'm')) {
                j += 1;
                if self.at(j) == Some(b'.') {
                    j += 1;
                }
                if !self.at(j).is_some_and(|b| b.is_ascii_alphabetic()) {
                    if !(1..=12).contains(&h) {
                        return Err(self.err(save, "Unexpected character"));
                    }
                    self.pos = j;
                    let pm = c == Some(b'p');
                    return Ok(Some(match (h, pm) {
                        (12, false) => 0,
                        (12, true) => 12,
                        (h, true) => h + 12,
                        (h, false) => h,
                    }));
                }
            }
        }
        self.pos = save;
        Ok(None)
    }

    /// `HH:MM[:SS[.frac]]` starting at `a` (hour digits `n`).
    fn clock(&mut self, a: usize, n: usize) -> Res<bool> {
        let h = self.num(a, n);
        let mut j = a + n;
        if self.at(j) != Some(b':') && self.at(j) != Some(b'.') {
            return Ok(false);
        }
        let mn = self.digits_at(j + 1);
        if mn != 2 {
            return Ok(false);
        }
        let i = self.num(j + 1, 2);
        j += 3;
        let mut s = 0;
        if matches!(self.at(j), Some(b':' | b'.')) && self.digits_at(j + 1) == 2 {
            s = self.num(j + 1, 2);
            j += 3;
        }
        self.pos = j;
        let us = self.fraction();
        let h = match self.meridian(h)? {
            Some(h) => h,
            None => h,
        };
        if h > 24 || i > 59 || s > 60 {
            return Err(self.err(a, "Unexpected character"));
        }
        self.set_time(h, i, s, us, a)?;
        Ok(true)
    }

    fn relative(&mut self, amount: i64, unit: Unit) {
        let r = &mut self.out.rel;
        match unit {
            Unit::Us => r.us += amount,
            Unit::Ms => r.us += amount * 1000,
            Unit::S => r.s += amount,
            Unit::I => r.i += amount,
            Unit::H => r.h += amount,
            Unit::D => r.d += amount,
            Unit::W => r.d += amount * 7,
            Unit::Fortnight => r.d += amount * 14,
            Unit::M => r.m += amount,
            Unit::Y => r.y += amount,
            Unit::Weekday => r.weekdays += amount,
            Unit::Day(wd) => {
                // "+2 monday": the second Monday from now.
                r.weekday = Some((wd, amount));
                self.out.reset_time = true;
            }
        }
        self.out.have_rel = true;
    }

    /// `[+-]N unit` / `N unit`; false if no unit follows.
    fn try_relative_number(&mut self, sign: i64, a: usize, n: usize) -> bool {
        let amount = sign * self.num(a, n);
        let mut j = a + n;
        while self.at(j) == Some(b' ') || self.at(j) == Some(b'\t') {
            j += 1;
        }
        let (w, end) = self.word_at(j);
        match unit_of(&w) {
            Some(u) => {
                self.pos = end;
                self.relative(amount, u);
                true
            }
            None => false,
        }
    }

    /// Optional ordinal suffix after a day number.
    fn day_suffix(&mut self) {
        let (w, end) = self.word_at(self.pos);
        if matches!(w.as_str(), "st" | "nd" | "rd" | "th") {
            self.pos = end;
        }
    }

    /// After `month` (name): `[ day[suffix] ][,] [ year ]`.
    fn after_month_name(&mut self, m: i64, at: usize) -> Res<()> {
        let save = self.pos;
        let mut j = self.pos;
        while matches!(self.at(j), Some(b' ' | b'-' | b'.' | b'\t')) {
            j += 1;
        }
        let n = self.digits_at(j);
        if n == 4 && self.at(j + 4) != Some(b':') {
            // "January 2024": the first of the month.
            self.pos = j + 4;
            let y = self.num(j, 4);
            return self.set_date(Some(y), m, 1, at);
        }
        if (1..=2).contains(&n) && self.at(j + n) != Some(b':') {
            let d = self.num(j, n);
            self.pos = j + n;
            self.day_suffix();
            // Optional year.
            let mut k = self.pos;
            while matches!(self.at(k), Some(b' ' | b',' | b'-' | b'.' | b'\t')) {
                k += 1;
            }
            let yn = self.digits_at(k);
            if (yn == 4 || (yn == 2 && self.at(k - 1) == Some(b'-'))) && self.at(k + yn) != Some(b':') {
                let y = self.year(k, yn);
                self.pos = k + yn;
                return self.set_date(Some(y), m, d, at);
            }
            return self.set_date(None, m, d, at);
        }
        self.pos = save;
        // Month alone: keeps the day.
        if self.have_date {
            return Err(self.err(at, "Double date specification"));
        }
        self.have_date = true;
        self.out.m = Some(m);
        Ok(())
    }

    /// A token starting with a digit.
    fn number(&mut self) -> Res<()> {
        let a = self.pos;
        let n = self.digits_at(a);
        let next = self.at(a + n);
        // ISO-8601 / MySQL: YYYY-MM-DD, YYYY-MM, YYYY-Www[-D]
        if n == 4 && next == Some(b'-') {
            let b = a + 5;
            if self.at(b).is_some_and(|c| c.eq_ignore_ascii_case(&b'w')) && self.digits_at(b + 1) == 2 {
                let y = self.num(a, 4);
                let w = self.num(b + 1, 2);
                let mut j = b + 3;
                let mut dow = 1;
                if self.at(j) == Some(b'-') && self.digits_at(j + 1) == 1 {
                    dow = self.num(j + 1, 1);
                    j += 2;
                }
                self.pos = j;
                let days = from_iso_week(y, w, dow);
                let (yy, mm, dd) = super::civil::civil_from_days(days);
                return self.set_date(Some(yy), mm, dd, a);
            }
            let mn = self.digits_at(b);
            if (1..=2).contains(&mn) {
                let m = self.num(b, mn);
                let c = b + mn;
                if self.at(c) == Some(b'-') && (1..=2).contains(&self.digits_at(c + 1)) {
                    let dn = self.digits_at(c + 1);
                    let d = self.num(c + 1, dn);
                    self.pos = c + 1 + dn;
                    if m > 12 || d > 31 {
                        return Err(self.err(a, "Unexpected character"));
                    }
                    self.set_date(Some(self.num(a, 4)), m, d, a)?;
                    // `T` separator before a time.
                    if self.peek().is_some_and(|c| c.eq_ignore_ascii_case(&b't')) && self.at(self.pos + 1).is_some_and(|c| c.is_ascii_digit()) {
                        self.pos += 1;
                    }
                    return Ok(());
                }
                if m <= 12 && !self.at(c).is_some_and(|x| x.is_ascii_digit()) {
                    self.pos = c;
                    return self.set_date(Some(self.num(a, 4)), m, 1, a);
                }
            }
        }
        // YYYY/MM/DD
        if n == 4 && next == Some(b'/') {
            let b = a + 5;
            let mn = self.digits_at(b);
            if (1..=2).contains(&mn) && self.at(b + mn) == Some(b'/') {
                let dn = self.digits_at(b + mn + 1);
                if (1..=2).contains(&dn) {
                    let (m, d) = (self.num(b, mn), self.num(b + mn + 1, dn));
                    self.pos = b + mn + 1 + dn;
                    return self.set_date(Some(self.num(a, 4)), m, d, a);
                }
            }
        }
        // MM/DD[/YYYY]
        if (1..=2).contains(&n) && next == Some(b'/') {
            let b = a + n + 1;
            let dn = self.digits_at(b);
            if (1..=2).contains(&dn) {
                let (m, d) = (self.num(a, n), self.num(b, dn));
                let c = b + dn;
                if self.at(c) == Some(b'/') {
                    let yn = self.digits_at(c + 1);
                    if yn >= 1 {
                        let y = self.year(c + 1, yn);
                        self.pos = c + 1 + yn;
                        return self.set_date(Some(y), m, d, a);
                    }
                }
                self.pos = c;
                return self.set_date(None, m, d, a);
            }
        }
        // DD-MM-YYYY, DD.MM.YYYY, DD.MM.YY, DD-Mon-YYYY
        if (1..=2).contains(&n) && matches!(next, Some(b'-' | b'.')) {
            let b = a + n + 1;
            let mn = self.digits_at(b);
            let sep = next.unwrap();
            if (1..=2).contains(&mn) && self.at(b + mn) == Some(sep) {
                let yn = self.digits_at(b + mn + 1);
                if yn == 4 || (yn == 2 && sep == b'.') {
                    let (d, m) = (self.num(a, n), self.num(b, mn));
                    let y = self.year(b + mn + 1, yn);
                    self.pos = b + mn + 1 + yn;
                    return self.set_date(Some(y), m, d, a);
                }
            }
            if sep == b'-' && mn == 0 {
                let (w, end) = self.word_at(b);
                if let Some(m) = month_of(&w) {
                    let d = self.num(a, n);
                    self.pos = end;
                    let mut y = None;
                    if self.peek() == Some(b'-') {
                        let yn = self.digits_at(self.pos + 1);
                        if yn > 0 {
                            y = Some(self.year(self.pos + 1, yn));
                            self.pos += 1 + yn;
                        }
                    }
                    return self.set_date(y, m, d, a);
                }
            }
        }
        // Clock time.
        if (1..=2).contains(&n) && matches!(next, Some(b':')) {
            if self.clock(a, n)? {
                return Ok(());
            }
        }
        // YYYYMMDD / YYYYMMDDHHMMSS
        if n == 8 || n == 14 {
            let (y, m, d) = (self.num(a, 4), self.num(a + 4, 2), self.num(a + 6, 2));
            if (1..=12).contains(&m) && (1..=31).contains(&d) {
                self.pos = a + n;
                self.set_date(Some(y), m, d, a)?;
                if n == 14 {
                    self.set_time(self.num(a + 8, 2), self.num(a + 10, 2), self.num(a + 12, 2), 0, a)?;
                } else if self.peek().is_some_and(|c| c.eq_ignore_ascii_case(&b't')) {
                    self.pos += 1;
                }
                return Ok(());
            }
        }
        // "3pm", "11 am"
        if (1..=2).contains(&n) {
            self.pos = a + n;
            let h = self.num(a, n);
            if let Some(h) = self.meridian(h)? {
                return self.set_time(h, 0, 0, 0, a);
            }
            self.pos = a;
        }
        // Relative: "3 days", "1 week ago".
        if self.try_relative_number(1, a, n) {
            return Ok(());
        }
        // "15 January [2024]", "1st of march"
        if (1..=2).contains(&n) {
            let d = self.num(a, n);
            self.pos = a + n;
            self.day_suffix();
            let mut j = self.pos;
            while matches!(self.at(j), Some(b' ' | b'-' | b'.' | b'\t')) {
                j += 1;
            }
            let (w, end) = self.word_at(j);
            let (w, end) = if w == "of" {
                let mut k = end;
                while self.at(k) == Some(b' ') {
                    k += 1;
                }
                self.word_at(k)
            } else {
                (w, end)
            };
            if let Some(m) = month_of(&w) {
                self.pos = end;
                let mut k = self.pos;
                while matches!(self.at(k), Some(b' ' | b',' | b'-' | b'.' | b'\t')) {
                    k += 1;
                }
                let yn = self.digits_at(k);
                if yn == 4 && self.at(k + 4) != Some(b':') {
                    let y = self.num(k, 4);
                    self.pos = k + 4;
                    return self.set_date(Some(y), m, d, a);
                }
                return self.set_date(None, m, d, a);
            }
            self.pos = a;
        }
        // "1530" -> 15:30 (timelib reads four digits as a 24-hour time).
        if n == 4 && !self.have_time {
            let (h, i) = (self.num(a, 2), self.num(a + 2, 2));
            if h <= 24 && i <= 59 {
                self.pos = a + 4;
                return self.set_time(h, i, 0, 0, a);
            }
        }
        // A bare year after a date ("Sat Jan 15 2024").
        if n == 4 && self.have_date && self.out.y.is_none() {
            self.out.y = Some(self.num(a, 4));
            self.pos = a + 4;
            return Ok(());
        }
        Err(self.err(a, "Unexpected character"))
    }

    /// `+N unit` / `-N unit`, or a UTC offset.
    fn signed(&mut self) -> Res<()> {
        let a = self.pos;
        let sign = if self.s[a] == b'-' { -1 } else { 1 };
        let mut b = a + 1;
        while self.at(b) == Some(b' ') {
            b += 1;
        }
        let n = self.digits_at(b);
        if n == 0 {
            // "+ week"? not a format.
            return Err(self.err(a, "Unexpected character"));
        }
        if self.try_relative_number(sign, b, n) {
            return Ok(());
        }
        // UTC offset: +HH, +HHMM, +HH:MM
        let mut j = b + n;
        if self.at(j) == Some(b':') && self.digits_at(j + 1) == 2 {
            j += 3;
        }
        let text = std::str::from_utf8(&self.s[a..j]).unwrap();
        match parse_offset(text) {
            Some(o) => {
                self.pos = j;
                self.set_zone(Zone::Offset(o), a)
            }
            None => Err(self.err(a, "Unexpected character")),
        }
    }

    /// `of <month spec>` after "first day" / "first monday".
    fn month_spec(&mut self) -> Res<()> {
        self.skip_space();
        let start = self.pos;
        let (w, end) = self.word_at(self.pos);
        if let Some(m) = month_of(&w) {
            self.pos = end;
            return self.after_month_name(m, start);
        }
        if let Some(n) = ordinal(&w) {
            let mut j = end;
            while self.at(j) == Some(b' ') {
                j += 1;
            }
            let (u, e2) = self.word_at(j);
            if u == "month" {
                self.pos = e2;
                self.out.rel.m += n;
                self.out.have_rel = true;
                return Ok(());
            }
            if u == "year" {
                self.pos = e2;
                self.out.rel.y += n;
                self.out.have_rel = true;
                return Ok(());
            }
        }
        Ok(())
    }

    /// A token starting with a letter.
    fn word(&mut self) -> Res<()> {
        let a = self.pos;
        let (w, end) = self.word_at(a);
        // Zone identifiers: Europe/Paris, America/Argentina/Buenos_Aires
        if matches!(self.at(end), Some(b'/' | b'_')) {
            let mut j = end;
            while j < self.s.len() && (self.s[j].is_ascii_alphanumeric() || matches!(self.s[j], b'/' | b'_' | b'-' | b'+')) {
                j += 1;
            }
            let id = std::str::from_utf8(&self.s[a..j]).unwrap();
            if let Some(tz) = identifier(id) {
                self.pos = j;
                return self.set_zone(Zone::Id(tz), a);
            }
            return Err(self.err(a, "The timezone could not be found in the database"));
        }
        self.pos = end;
        match w.as_str() {
            "now" => return Ok(()),
            "today" | "midnight" => {
                self.unset_time();
                return Ok(());
            }
            "noon" => {
                self.out.reset_time = true;
                if !self.have_time {
                    self.have_time = true;
                    self.out.h = Some(12);
                    self.out.i = Some(0);
                    self.out.s = Some(0);
                    self.out.us = Some(0);
                }
                return Ok(());
            }
            "tomorrow" | "yesterday" => {
                self.out.rel.d += if w == "tomorrow" { 1 } else { -1 };
                self.out.have_rel = true;
                self.unset_time();
                return Ok(());
            }
            "ago" => {
                let r = &mut self.out.rel;
                for f in [&mut r.y, &mut r.m, &mut r.d, &mut r.h, &mut r.i, &mut r.s, &mut r.us, &mut r.weekdays] {
                    *f = -*f;
                }
                if let Some((_, n)) = &mut r.weekday {
                    *n = -*n;
                }
                return Ok(());
            }
            "t" if self.peek().is_some_and(|c| c.is_ascii_digit()) => return Ok(()),
            "of" | "at" | "on" | "and" => return Ok(()),
            _ => {}
        }
        if let Some(m) = month_of(&w) {
            return self.after_month_name(m, a);
        }
        if let Some(wd) = weekday_of(&w) {
            // A weekday on its own: this one, or the next.
            if self.out.rel.weekday.is_none() {
                self.out.rel.weekday = Some((wd, 0));
            }
            self.out.have_rel = true;
            self.out.reset_time = true;
            return Ok(());
        }
        if let Some(n) = ordinal(&w) {
            let mut j = self.pos;
            while self.at(j) == Some(b' ') {
                j += 1;
            }
            let (u, e2) = self.word_at(j);
            // "first day of", "last day of"
            if u == "day" && (w == "first" || w == "last") {
                let mut k = e2;
                while self.at(k) == Some(b' ') {
                    k += 1;
                }
                let (of, e3) = self.word_at(k);
                if of == "of" {
                    self.pos = e3;
                    self.out.first_last = Some(if w == "first" { FirstLast::First } else { FirstLast::Last });
                    self.out.have_rel = true;
                    return self.month_spec();
                }
            }
            if let Some(wd) = weekday_of(&u) {
                let mut k = e2;
                while self.at(k) == Some(b' ') {
                    k += 1;
                }
                let (of, e3) = self.word_at(k);
                if of == "of" && w != "this" && w != "next" && w != "previous" {
                    // "second monday of next month"
                    self.pos = e3;
                    self.out.nth_weekday_of = Some((n, wd));
                    self.out.have_rel = true;
                    self.out.reset_time = true;
                    return self.month_spec();
                }
                self.pos = e2;
                self.out.rel.weekday = Some((wd, n));
                self.out.have_rel = true;
                self.out.reset_time = true;
                return Ok(());
            }
            if let Some(unit) = unit_of(&u) {
                self.pos = e2;
                self.relative(n, unit);
                return Ok(());
            }
        }
        // Zone abbreviations (UTC, GMT, Z, EST, CEST, ...)
        if w == "utc" || w == "gmt" {
            // "GMT+2" style offsets.
            if matches!(self.peek(), Some(b'+' | b'-')) && self.digits_at(self.pos + 1) > 0 {
                let b = self.pos;
                let mut j = b + 1 + self.digits_at(b + 1);
                if self.at(j) == Some(b':') && self.digits_at(j + 1) == 2 {
                    j += 3;
                }
                if let Some(o) = parse_offset(std::str::from_utf8(&self.s[b..j]).unwrap()) {
                    self.pos = j;
                    return self.set_zone(Zone::Offset(o), a);
                }
            }
            let z = if w == "utc" { Zone::Id(chrono_tz::Tz::UTC) } else { Zone::Abbr { name: "GMT".into(), offset: 0, dst: false } };
            return self.set_zone(z, a);
        }
        if let Some((offset, dst)) = abbreviation(&w) {
            return self.set_zone(Zone::Abbr { name: w.to_ascii_uppercase(), offset, dst }, a);
        }
        if let Some(tz) = identifier(&w) {
            return self.set_zone(Zone::Id(tz), a);
        }
        Err(self.err(a, "The timezone could not be found in the database"))
    }

    fn run(&mut self) -> Res<()> {
        loop {
            self.skip_space();
            let Some(c) = self.peek() else { return Ok(()) };
            match c {
                b'@' => {
                    let a = self.pos;
                    let mut j = a + 1;
                    let neg = self.at(j) == Some(b'-');
                    if neg {
                        j += 1;
                    }
                    let n = self.digits_at(j);
                    if n == 0 {
                        return Err(self.err(a, "Unexpected character"));
                    }
                    let mut ts = self.num(j, n);
                    if neg {
                        ts = -ts;
                    }
                    self.pos = j + n;
                    let us = if self.peek() == Some(b'.') { self.fraction() } else { 0 };
                    let us = if neg { -us } else { us };
                    self.out.timestamp = Some((ts, us));
                    self.have_date = true;
                    self.have_time = true;
                    self.have_zone = true;
                }
                b'0'..=b'9' => self.number()?,
                b'+' | b'-' => self.signed()?,
                c if c.is_ascii_alphabetic() || c >= 0x80 => self.word()?,
                b'.' | b'/' | b'(' | b')' => self.pos += 1,
                _ => return Err(self.err(self.pos, "Unexpected character")),
            }
        }
    }
}

pub fn parse(s: &str) -> Result<Parsed, ParseError> {
    let mut p = P { s: s.as_bytes(), pos: 0, out: Parsed::default(), have_date: false, have_time: false, have_zone: false };
    p.run()?;
    Ok(p.out)
}

/// Apply a parse to a base: `base` is the current local time in `zone`.
/// Returns (UTC seconds, microseconds, zone of the result).
pub fn apply(p: &Parsed, now_ts: i64, now_us: i64, base_zone: &Zone) -> (i64, i64, Zone) {
    let (zone, base) = match p.timestamp {
        Some((ts, us)) => {
            (Zone::Offset(0), Civil::from_local(ts, us))
        }
        None => {
            // The base's wall clock (in the base zone) is read in the
            // string's zone, as timelib does.
            let zone = p.zone.clone().unwrap_or_else(|| base_zone.clone());
            let local = now_ts + base_zone.offset(now_ts) as i64;
            (zone, Civil::from_local(local, now_us))
        }
    };
    let mut c = base;
    if p.timestamp.is_none() {
        if let Some(y) = p.y {
            c.y = y;
        }
        if let Some(m) = p.m {
            c.m = m;
        }
        if let Some(d) = p.d {
            c.d = d;
        }
        let have_date = p.y.is_some() || p.m.is_some() || p.d.is_some();
        if let Some(h) = p.h {
            c.h = h;
            c.i = p.i.unwrap_or(0);
            c.s = p.s.unwrap_or(0);
            c.us = p.us.unwrap_or(0);
        } else if have_date || p.reset_time {
            c.h = 0;
            c.i = 0;
            c.s = 0;
            c.us = 0;
        }
    }
    // A weekday name moves to that day before the relative offsets apply.
    if let Some((wd, n)) = p.rel.weekday {
        c = c.normalized();
        let (sec, _) = c.to_local();
        let cur = weekday(sec.div_euclid(86400));
        let delta = if n == 0 {
            (wd - cur).rem_euclid(7)
        } else if n > 0 {
            let first = (wd - cur).rem_euclid(7);
            let first = if first == 0 { 7 } else { first };
            first + (n - 1) * 7
        } else {
            let back = (cur - wd).rem_euclid(7);
            let back = if back == 0 { 7 } else { back };
            -(back + (-n - 1) * 7)
        };
        c.d += delta;
    }
    c.y += p.rel.y;
    c.m += p.rel.m;
    c.d += p.rel.d;
    c.h += p.rel.h;
    c.i += p.rel.i;
    c.s += p.rel.s;
    c.us += p.rel.us;
    match p.first_last {
        Some(FirstLast::First) => c.d = 1,
        Some(FirstLast::Last) => {
            let ym = Civil { d: 1, ..c }.normalized();
            c = Civil { y: ym.y, m: ym.m, d: days_in_month(ym.y, ym.m), ..c };
        }
        None => {}
    }
    if let Some((n, wd)) = p.nth_weekday_of {
        let first = Civil { d: 1, ..c }.normalized();
        let (sec, _) = first.to_local();
        let wd1 = weekday(sec.div_euclid(86400));
        let day = if n > 0 {
            1 + (wd - wd1).rem_euclid(7) + (n - 1) * 7
        } else {
            let last = days_in_month(first.y, first.m);
            let wdl = (wd1 + last - 1).rem_euclid(7);
            last - (wdl - wd).rem_euclid(7)
        };
        c = Civil { y: first.y, m: first.m, d: day, ..c };
    }
    if p.rel.weekdays != 0 {
        let mut c2 = c.normalized();
        let step = p.rel.weekdays.signum();
        let mut left = p.rel.weekdays.abs();
        while left > 0 {
            c2.d += step;
            c2 = c2.normalized();
            let (sec, _) = c2.to_local();
            let wd = weekday(sec.div_euclid(86400));
            if wd != 0 && wd != 6 {
                left -= 1;
            }
        }
        c = c2;
    }
    let (local, us) = c.to_local();
    (zone.to_utc(local), us, zone)
}
