//! Approximate Northeastern term dates, America/New_York, without a timezone database.
//!
//! Port of `TermCalendar.approximateWindow`.

use crate::model::{Season, Term};

/// Inclusive start and end, as seconds since 1970.
pub fn approximate_window(term_code: &str) -> Option<(f64, f64)> {
    let term = Term { code: term_code.to_string(), banner_description: String::new() };
    if term.code.len() != 6 {
        return None;
    }
    let year = term.calendar_year();
    let ((sm, sd), (em, ed)) = match term.season() {
        Season::Fall => ((9, 1), (12, 23)),
        Season::Winter => ((1, 1), (3, 31)),
        Season::Spring => ((1, 3), (4, 30)),
        Season::Summer1 => ((5, 1), (6, 30)),
        Season::SummerFull => ((5, 1), (8, 20)),
        Season::Summer2 => ((6, 25), (8, 20)),
    };
    let start = unix_midnight_new_york(year, sm, sd)?;
    let end_day = unix_midnight_new_york(year, em, ed)?;
    Some((start, end_day + 86_400.0 - 1.0))
}

fn unix_midnight_new_york(year: i32, month: u32, day: u32) -> Option<f64> {
    let utc = civil_to_unix(year, month, day)?;
    let offset = if is_us_dst(year, month, day) { 4 * 3600 } else { 5 * 3600 };
    Some((utc + offset) as f64)
}

/// US DST (2007 rule): second Sunday in March through the first Sunday in November.
fn is_us_dst(year: i32, month: u32, day: u32) -> bool {
    if !(3..=11).contains(&month) {
        return false;
    }
    if month > 3 && month < 11 {
        return true;
    }
    if month == 3 {
        return day >= nth_sunday(year, 3, 2);
    }
    day < nth_sunday(year, 11, 1)
}

fn nth_sunday(year: i32, month: u32, n: u32) -> u32 {
    let first = weekday(year, month, 1);
    let first_sunday = if first == 0 { 1 } else { 8 - first };
    first_sunday + (n - 1) * 7
}

/// 0 = Sunday … 6 = Saturday. Howard Hinnant’s civil-from-days inverse, weekday only.
fn weekday(year: i32, month: u32, day: u32) -> u32 {
    let unix = civil_to_unix(year, month, day).unwrap_or(0);
    ((unix.div_euclid(86_400) + 4).rem_euclid(7)) as u32
}

fn civil_to_unix(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as u64;
    let days = (era as i64) * 146_097 + doe as i64 - 719_468;
    Some(days * 86_400)
}
