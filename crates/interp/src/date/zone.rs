//! Time zones as PHP models them: a UTC offset (type 1), an abbreviation
//! with a fixed offset (type 2) or a tz database identifier (type 3).

use std::collections::HashMap;
use std::sync::OnceLock;

use chrono::{DateTime, LocalResult, NaiveDateTime, Offset, TimeZone};
use chrono_tz::{OffsetComponents, OffsetName, Tz};

#[derive(Clone, Debug, PartialEq)]
pub enum Zone {
    /// Seconds east of UTC.
    Offset(i32),
    /// Abbreviation as written (upper-cased), its total offset and DST flag.
    Abbr { name: String, offset: i32, dst: bool },
    Id(Tz),
}

/// Abbreviations timelib knows (the common subset): name, offset, DST.
const ABBRS: &[(&str, i32, bool)] = &[
    ("utc", 0, false),
    ("gmt", 0, false),
    ("z", 0, false),
    ("est", -18000, false),
    ("edt", -14400, true),
    ("cst", -21600, false),
    ("cdt", -18000, true),
    ("mst", -25200, false),
    ("mdt", -21600, true),
    ("pst", -28800, false),
    ("pdt", -25200, true),
    ("akst", -32400, false),
    ("akdt", -28800, true),
    ("hst", -36000, false),
    ("ast", -14400, false),
    ("adt", -10800, true),
    ("nst", -12600, false),
    ("ndt", -9000, true),
    ("wet", 0, false),
    ("west", 3600, true),
    ("bst", 3600, true),
    ("cet", 3600, false),
    ("cest", 7200, true),
    ("met", 3600, false),
    ("mest", 7200, true),
    ("eet", 7200, false),
    ("eest", 10800, true),
    ("msk", 10800, false),
    ("sast", 7200, false),
    ("pkt", 18000, false),
    ("wib", 25200, false),
    ("hkt", 28800, false),
    ("sgt", 28800, false),
    ("awst", 28800, false),
    ("jst", 32400, false),
    ("kst", 32400, false),
    ("acst", 34200, false),
    ("acdt", 37800, true),
    ("aest", 36000, false),
    ("aedt", 39600, true),
    ("nzst", 43200, false),
    ("nzdt", 46800, true),
];

fn ids() -> &'static HashMap<String, Tz> {
    static IDS: OnceLock<HashMap<String, Tz>> = OnceLock::new();
    IDS.get_or_init(|| chrono_tz::TZ_VARIANTS.iter().map(|t| (t.name().to_ascii_lowercase(), *t)).collect())
}

/// A tz database identifier, case-insensitively.
pub fn identifier(s: &str) -> Option<Tz> {
    ids().get(&s.to_ascii_lowercase()).copied()
}

pub fn abbreviation(s: &str) -> Option<(i32, bool)> {
    let l = s.to_ascii_lowercase();
    ABBRS.iter().find(|(n, ..)| *n == l).map(|&(_, o, d)| (o, d))
}

/// `+05:30`, `-0800`, `+5`, `+05`, `-5:30` -> seconds.
pub fn parse_offset(s: &str) -> Option<i32> {
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit() || b == b':') {
        return None;
    }
    let (h, m, sec) = if let Some((h, rest)) = rest.split_once(':') {
        let (m, s) = rest.split_once(':').unwrap_or((rest, "0"));
        (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?, s.parse::<i32>().ok()?)
    } else {
        match rest.len() {
            1 | 2 => (rest.parse().ok()?, 0, 0),
            3 => (rest[..1].parse().ok()?, rest[1..].parse().ok()?, 0),
            4 => (rest[..2].parse().ok()?, rest[2..].parse().ok()?, 0),
            6 => (rest[..2].parse().ok()?, rest[2..4].parse().ok()?, rest[4..].parse().ok()?),
            _ => return None,
        }
    };
    if h > 99 || m > 59 || sec > 59 {
        return None;
    }
    Some(sign * (h * 3600 + m * 60 + sec))
}

pub fn format_offset(off: i32, colon: bool) -> String {
    let sign = if off < 0 { '-' } else { '+' };
    let a = off.unsigned_abs();
    let (h, m, s) = (a / 3600, a / 60 % 60, a % 60);
    let mut out = if colon { format!("{sign}{h:02}:{m:02}") } else { format!("{sign}{h:02}{m:02}") };
    if s != 0 {
        out.push_str(&if colon { format!(":{s:02}") } else { format!("{s:02}") });
    }
    out
}

fn naive_utc(ts: i64) -> Option<NaiveDateTime> {
    DateTime::from_timestamp(ts, 0).map(|d| d.naive_utc())
}

impl Zone {
    /// What DateTimeZone's constructor accepts.
    pub fn parse(s: &str) -> Option<Zone> {
        if let Some(o) = parse_offset(s) {
            return Some(Zone::Offset(o));
        }
        if s.eq_ignore_ascii_case("utc") {
            return Some(Zone::Id(Tz::UTC));
        }
        if let Some((offset, dst)) = abbreviation(s) {
            return Some(Zone::Abbr { name: s.to_ascii_uppercase(), offset, dst });
        }
        identifier(s).map(Zone::Id)
    }

    pub fn type_num(&self) -> i64 {
        match self {
            Zone::Offset(_) => 1,
            Zone::Abbr { .. } => 2,
            Zone::Id(_) => 3,
        }
    }

    pub fn name(&self) -> String {
        match self {
            Zone::Offset(o) => format_offset(*o, true),
            Zone::Abbr { name, .. } => name.clone(),
            Zone::Id(t) => t.name().to_string(),
        }
    }

    /// (offset, is DST, abbreviation) at UTC instant `ts`.
    pub fn at(&self, ts: i64) -> (i32, bool, String) {
        match self {
            Zone::Offset(o) => (*o, false, format_offset(*o, true)),
            Zone::Abbr { name, offset, dst } => (*offset, *dst, name.clone()),
            Zone::Id(t) => match naive_utc(ts) {
                Some(n) => {
                    let off = t.offset_from_utc_datetime(&n);
                    let total = off.fix().local_minus_utc();
                    let dst = off.dst_offset().num_seconds() != 0;
                    let abbr = off.abbreviation().map(str::to_string).unwrap_or_else(|| short_offset(total));
                    (total, dst, abbr)
                }
                None => (0, false, "UTC".into()),
            },
        }
    }

    pub fn offset(&self, ts: i64) -> i32 {
        match self {
            Zone::Offset(o) => *o,
            Zone::Abbr { offset, .. } => *offset,
            Zone::Id(_) => self.at(ts).0,
        }
    }

    /// The UTC instant of local wall-clock `local` (seconds as if UTC).
    /// In a DST gap the time moves forward; in an overlap the first
    /// (earlier) instant wins, as in PHP.
    pub fn to_utc(&self, local: i64) -> i64 {
        match self {
            Zone::Offset(o) => local - *o as i64,
            Zone::Abbr { offset, .. } => local - *offset as i64,
            Zone::Id(t) => {
                let Some(n) = naive_utc(local) else { return local };
                match t.from_local_datetime(&n) {
                    LocalResult::Single(d) => d.timestamp(),
                    LocalResult::Ambiguous(a, b) => a.timestamp().min(b.timestamp()),
                    LocalResult::None => {
                        // Use the offset in force just before the gap.
                        let before = self.offset(local - 86400);
                        local - before as i64
                    }
                }
            }
        }
    }
}

/// `+03`, `-0330`: how tzdata names zones without an abbreviation.
fn short_offset(off: i32) -> String {
    let sign = if off < 0 { '-' } else { '+' };
    let a = off.unsigned_abs();
    if a % 3600 == 0 {
        format!("{sign}{:02}", a / 3600)
    } else {
        format!("{sign}{:02}{:02}", a / 3600, a / 60 % 60)
    }
}

/// DateTimeZone::listIdentifiers($group, $country): PHP's identifiers in
/// its order. ALL_WITH_BC (4095) adds the backward-compatible aliases;
/// PER_COUNTRY (4096) picks a country's canonical zones.
pub fn list_identifiers(group: i64, country: &str) -> Vec<&'static str> {
    const GROUPS: &[(i64, &str)] = &[
        (1, "Africa/"),
        (2, "America/"),
        (4, "Antarctica/"),
        (8, "Arctic/"),
        (16, "Asia/"),
        (32, "Atlantic/"),
        (64, "Australia/"),
        (128, "Europe/"),
        (256, "Indian/"),
        (512, "Pacific/"),
    ];
    let zones = super::zones_table::ZONES.iter();
    match group {
        4095 => zones.map(|z| z.0).collect(),
        4096 => zones.filter(|z| z.1 && z.2 == country).map(|z| z.0).collect(),
        g => zones
            .filter(|z| z.1 && (GROUPS.iter().any(|(bit, p)| g & bit != 0 && z.0.starts_with(p)) || (g & 1024 != 0 && z.0 == "UTC")))
            .map(|z| z.0)
            .collect(),
    }
}
