//! DateTime::format() / date().

use super::civil::{days_from_civil, days_in_month, is_leap, iso_week, weekday, Civil};
use super::zone::{format_offset, Zone};

pub const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
pub const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// Format UTC instant `ts`/`us` as seen in `zone`.
pub fn format(fmt: &[u8], ts: i64, us: i64, zone: &Zone) -> Vec<u8> {
    let (off, dst, abbr) = zone.at(ts);
    let local = ts + off as i64;
    let c = Civil::from_local(local, us);
    let days = local.div_euclid(86400);
    let dow = weekday(days);
    let mut out = Vec::with_capacity(fmt.len() * 2);
    let push = |s: &str, out: &mut Vec<u8>| out.extend_from_slice(s.as_bytes());
    let mut i = 0;
    while i < fmt.len() {
        let ch = fmt[i];
        i += 1;
        match ch {
            b'd' => push(&format!("{:02}", c.d), &mut out),
            b'D' => push(&DAYS[dow as usize][..3], &mut out),
            b'j' => push(&c.d.to_string(), &mut out),
            b'l' => push(DAYS[dow as usize], &mut out),
            b'N' => push(&(if dow == 0 { 7 } else { dow }).to_string(), &mut out),
            b'S' => push(
                match c.d {
                    1 | 21 | 31 => "st",
                    2 | 22 => "nd",
                    3 | 23 => "rd",
                    _ => "th",
                },
                &mut out,
            ),
            b'w' => push(&dow.to_string(), &mut out),
            b'z' => push(&(days - days_from_civil(c.y, 1, 1)).to_string(), &mut out),
            b'W' => push(&format!("{:02}", iso_week(days).1), &mut out),
            b'F' => push(MONTHS[c.m as usize - 1], &mut out),
            b'M' => push(&MONTHS[c.m as usize - 1][..3], &mut out),
            b'm' => push(&format!("{:02}", c.m), &mut out),
            b'n' => push(&c.m.to_string(), &mut out),
            b't' => push(&days_in_month(c.y, c.m).to_string(), &mut out),
            b'L' => push(if is_leap(c.y) { "1" } else { "0" }, &mut out),
            b'o' => push(&iso_week(days).0.to_string(), &mut out),
            b'X' | b'x' => {
                let y = c.y;
                let s = if y < 0 {
                    format!("-{:04}", -y)
                } else if y >= 10000 || ch == b'X' {
                    format!("+{y:04}")
                } else {
                    format!("{y:04}")
                };
                push(&s, &mut out)
            }
            b'Y' => push(&if c.y < 0 { format!("-{:04}", -c.y) } else { format!("{:04}", c.y) }, &mut out),
            b'y' => push(&format!("{:02}", c.y.rem_euclid(100)), &mut out),
            b'a' => push(if c.h < 12 { "am" } else { "pm" }, &mut out),
            b'A' => push(if c.h < 12 { "AM" } else { "PM" }, &mut out),
            b'B' => {
                let b = ((ts + 3600).rem_euclid(86400) * 10 / 864) % 1000;
                push(&format!("{b:03}"), &mut out)
            }
            b'g' => push(&(if c.h % 12 == 0 { 12 } else { c.h % 12 }).to_string(), &mut out),
            b'G' => push(&c.h.to_string(), &mut out),
            b'h' => push(&format!("{:02}", if c.h % 12 == 0 { 12 } else { c.h % 12 }), &mut out),
            b'H' => push(&format!("{:02}", c.h), &mut out),
            b'i' => push(&format!("{:02}", c.i), &mut out),
            b's' => push(&format!("{:02}", c.s), &mut out),
            b'u' => push(&format!("{:06}", c.us), &mut out),
            b'v' => push(&format!("{:03}", c.us / 1000), &mut out),
            b'e' => push(&zone.name(), &mut out),
            b'I' => push(if dst { "1" } else { "0" }, &mut out),
            b'O' => push(&format_offset(off, false), &mut out),
            b'P' => push(&format_offset(off, true), &mut out),
            b'p' => push(&if off == 0 { "Z".to_string() } else { format_offset(off, true) }, &mut out),
            // An offset zone has no abbreviation: GMT+hhmm.
            b'T' if matches!(zone, Zone::Offset(_)) => {
                let a = off.unsigned_abs();
                push(&format!("GMT{}{:02}{:02}", if off < 0 { '-' } else { '+' }, a / 3600, a % 3600 / 60), &mut out)
            }
            b'T' => push(&abbr, &mut out),
            b'Z' => push(&off.to_string(), &mut out),
            b'c' => out.extend(format(b"Y-m-d\\TH:i:sP", ts, us, zone)),
            b'r' => out.extend(format(b"D, d M Y H:i:s O", ts, us, zone)),
            b'U' => push(&ts.to_string(), &mut out),
            b'\\' => {
                if i < fmt.len() {
                    out.push(fmt[i]);
                    i += 1;
                }
            }
            other => out.push(other),
        }
    }
    out
}
