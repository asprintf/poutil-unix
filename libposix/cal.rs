/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Calendar arithmetic shared by touch and date: the proleptic Gregorian
 * calendar, seconds since the Epoch, and the two-digit-year rule of
 * "-t" and "mmddhhmm[[cc]yy]".
 */

pub fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

/* Days from 1970-01-01 to the given day. */
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/*
 * Seconds since the Epoch of a calendar time read as UTC. A second of 60
 * is the second after 59, as in the spec.
 */
pub fn epoch_utc(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    days_from_civil(y, mo, d) * 86400 + h as i64 * 3600 + mi as i64 * 60 + s as i64
}

/* The same calendar time read in the local time zone (TZ), through mktime. */
pub fn epoch_local(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<i64> {
    let year = i32::try_from(y - 1900).ok()?;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year;
    tm.tm_mon = mo as i32 - 1;
    tm.tm_mday = d as i32;
    tm.tm_hour = h as i32;
    tm.tm_min = mi as i32;
    tm.tm_sec = s as i32;
    tm.tm_isdst = -1;
    let t = unsafe { libc::mktime(&mut tm) } as i64;
    if t == -1 {
        /* -1 is also the instant before the Epoch: tell the two apart by converting back */
        let tt = t as libc::time_t;
        let mut back: libc::tm = unsafe { std::mem::zeroed() };
        if unsafe { libc::localtime_r(&tt, &mut back) }.is_null() {
            return None;
        }
        if back.tm_year != year
            || back.tm_mon != mo as i32 - 1
            || back.tm_mday != d as i32
            || back.tm_hour != h as i32
            || back.tm_min != mi as i32
            || back.tm_sec != s as i32
        {
            return None;
        }
    }
    Some(t)
}

/*
 * "If TZ is unset or null" the system default zone is used (touch -t, date).
 * C libraries differ on a null TZ (macOS reads it as UTC), so a null one is
 * removed: both cases then mean the same.
 */
pub fn null_tz_is_unset() {
    if let Some(v) = std::env::var_os("TZ") {
        if v.is_empty() {
            unsafe { libc::unsetenv(c"TZ".as_ptr()) };
        }
    }
}

/* A two-digit year: 69 to 99 is 19YY, 00 to 68 is 20YY. */
pub fn two_digit_year(yy: i64) -> i64 {
    if yy >= 69 {
        1900 + yy
    } else {
        2000 + yy
    }
}

/* Month 1-12, a day that exists in it, hour 0-23, minute 0-59, second 0-60. */
pub fn valid(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> bool {
    (1..=12).contains(&mo) && d >= 1 && d <= days_in_month(y, mo) && h <= 23 && mi <= 59 && s <= 60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_epochs() {
        assert_eq!(epoch_utc(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(epoch_utc(1969, 12, 31, 23, 59, 59), -1);
        assert_eq!(epoch_utc(2038, 1, 1, 0, 0, 0), 2145916800);
        assert_eq!(epoch_utc(1900, 1, 1, 0, 0, 0), -2208988800);
        assert_eq!(epoch_utc(2000, 2, 29, 12, 0, 0), 951825600);
        assert_eq!(epoch_utc(2007, 11, 12, 10, 15, 30), 1194862530);
        assert_eq!(epoch_utc(1998, 12, 31, 23, 59, 60), epoch_utc(1999, 1, 1, 0, 0, 0));
        assert_eq!(epoch_utc(2106, 2, 7, 6, 28, 16), 4294967296);
    }

    #[test]
    fn day_by_day_walk() {
        let mut days = 0i64;
        for y in 1970..2101 {
            for mo in 1..=12 {
                assert_eq!(days_from_civil(y, mo, 1), days, "{y}-{mo}");
                days += days_in_month(y, mo) as i64;
            }
        }
        let mut days = 0i64;
        for y in (1900..1970).rev() {
            for mo in (1..=12).rev() {
                days -= days_in_month(y, mo) as i64;
                assert_eq!(days_from_civil(y, mo, 1), days, "{y}-{mo}");
            }
        }
    }

    #[test]
    fn leap_years_and_months() {
        assert!(is_leap(2000) && is_leap(2024) && !is_leap(1900) && !is_leap(2023) && is_leap(1600));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2023, 4), 30);
        assert_eq!(days_in_month(2023, 12), 31);
    }

    #[test]
    fn century_rule() {
        assert_eq!(two_digit_year(69), 1969);
        assert_eq!(two_digit_year(99), 1999);
        assert_eq!(two_digit_year(0), 2000);
        assert_eq!(two_digit_year(68), 2068);
    }

    #[test]
    fn ranges() {
        assert!(valid(2024, 2, 29, 23, 59, 60));
        assert!(!valid(2023, 2, 29, 0, 0, 0));
        assert!(!valid(2023, 13, 1, 0, 0, 0));
        assert!(!valid(2023, 0, 1, 0, 0, 0));
        assert!(!valid(2023, 1, 0, 0, 0, 0));
        assert!(!valid(2023, 1, 1, 24, 0, 0));
        assert!(!valid(2023, 1, 1, 0, 60, 0));
        assert!(!valid(2023, 1, 1, 0, 0, 61));
    }
}
