//! Proleptic Gregorian calendar arithmetic on plain integers.

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    // Month overflow normalizes like mktime.
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

pub fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i64, m: i64) -> i64 {
    let (y, m) = (y + (m - 1).div_euclid(12), (m - 1).rem_euclid(12) + 1);
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// 0 = Sunday.
pub fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

/// Local wall-clock time broken down. `sec` counts seconds since the epoch
/// as if the zone were UTC.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Civil {
    pub y: i64,
    pub m: i64,
    pub d: i64,
    pub h: i64,
    pub i: i64,
    pub s: i64,
    pub us: i64,
}

impl Civil {
    pub fn from_local(sec: i64, us: i64) -> Civil {
        let (y, m, d) = civil_from_days(sec.div_euclid(86400));
        let sod = sec.rem_euclid(86400);
        Civil { y, m, d, h: sod / 3600, i: sod / 60 % 60, s: sod % 60, us }
    }

    /// Back to local epoch seconds, normalizing every field's overflow
    /// (month 13, day 0, hour 25, negative seconds, ...), like timelib.
    pub fn to_local(&self) -> (i64, i64) {
        let us_carry = self.us.div_euclid(1_000_000);
        let us = self.us.rem_euclid(1_000_000);
        let days = days_from_civil(self.y, self.m, 1) + self.d - 1;
        (days * 86400 + self.h * 3600 + self.i * 60 + self.s + us_carry, us)
    }

    /// Fold month overflow into the year and day overflow into months, so
    /// the fields are a real date again.
    pub fn normalized(&self) -> Civil {
        let (sec, us) = self.to_local();
        Civil::from_local(sec, us)
    }
}

/// ISO-8601 (year, week) of a day number.
pub fn iso_week(days: i64) -> (i64, i64) {
    let dow = weekday(days);
    let iso_dow = if dow == 0 { 7 } else { dow };
    let thursday = days - iso_dow + 4;
    let (iso_y, _, _) = civil_from_days(thursday);
    (iso_y, (thursday - days_from_civil(iso_y, 1, 1)) / 7 + 1)
}

/// Day number of ISO year `y`, week `w`, weekday `dow` (1 = Monday).
pub fn from_iso_week(y: i64, w: i64, dow: i64) -> i64 {
    let jan4 = days_from_civil(y, 1, 4);
    let wd = weekday(jan4);
    let monday = jan4 - if wd == 0 { 6 } else { wd - 1 };
    monday + (w - 1) * 7 + (dow - 1)
}
