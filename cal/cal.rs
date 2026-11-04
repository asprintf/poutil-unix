/* SPDX-License-Identifier: GPL-2.0-only */
/* cal - print a calendar. */

#![forbid(unsafe_code)]

use std::io::Write;

const SYNOPSIS: &str = "cal [[month] year]";

/* the width of one month in columns: seven days of two digits and the blanks between them */
const WIDTH: usize = 20;
/* a month is 20 columns; three of them with two blanks between make the 64 of a row of the year, 60 centre the year title */
const YEAR_WIDTH: usize = 3 * WIDTH;

/*
 * The calendar the standard asks for: the Julian calendar from January 1 of
 * the year 1 through September 2, 1752, the Gregorian one from September 14,
 * 1752 on, as though it had been adopted on that day. The days between do
 * not exist.
 */
fn is_julian(y: i64, m: u32, d: u32) -> bool {
    (y, m, d) < (1752, 9, 14)
}

/* Leap years: every fourth year up to 1752 (Julian), the Gregorian rule after it. */
fn is_leap(y: i64) -> bool {
    if y <= 1752 {
        y % 4 == 0
    } else {
        y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
    }
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/* The days of a month that exist: all of them but in September 1752, which has no 3rd to 13th. */
fn days_of(y: i64, m: u32) -> Vec<u32> {
    (1..=days_in_month(y, m)).filter(|&d| !(y == 1752 && m == 9 && (3..=13).contains(&d))).collect()
}

/* The day of the week, 0 for Sunday, from the Julian day number in the calendar that was in use. */
fn weekday(y: i64, m: u32, d: u32) -> usize {
    let a = i64::from((14 - m) / 12);
    let y2 = y + 4800 - a;
    let m2 = i64::from(m) + 12 * a - 3;
    let base = i64::from(d) + (153 * m2 + 2) / 5 + 365 * y2 + y2 / 4;
    let jdn = if is_julian(y, m, d) { base - 32083 } else { base - y2 / 100 + y2 / 400 - 32045 };
    ((jdn + 1) % 7) as usize
}

/* The weeks of a month, Sunday first: 0 is a blank. */
fn weeks(y: i64, m: u32) -> Vec<[u32; 7]> {
    let mut rows: Vec<[u32; 7]> = Vec::new();
    for d in days_of(y, m) {
        let col = weekday(y, m, d);
        if rows.is_empty() || col == 0 {
            rows.push([0; 7]);
        }
        if let Some(row) = rows.last_mut() {
            row[col] = d;
        }
    }
    rows
}

/* The names that LC_TIME gives: the twelve months and the seven days of the week, Sunday first. */
struct Names {
    months: Vec<Vec<u8>>,
    days: Vec<Vec<u8>>,
}

impl Names {
    /* The names of the C locale. */
    fn c() -> Names {
        let months = [
            "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
        ];
        let days = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
        Names {
            months: months.iter().map(|s| s.as_bytes().to_vec()).collect(),
            days: days.iter().map(|s| s.as_bytes().to_vec()).collect(),
        }
    }

    /* The names of the current locale, from strftime(3); an empty answer is the C name. */
    fn locale() -> Names {
        let mut n = Names::c();
        for (i, name) in n.months.iter_mut().enumerate() {
            if let Some(s) = strftime(c"%B", 0, i as i32) {
                *name = s;
            }
        }
        for (i, name) in n.days.iter_mut().enumerate() {
            if let Some(s) = strftime(c"%a", i as i32, 0) {
                *name = s;
            }
        }
        n
    }
}

/* One strftime conversion for a weekday and a month (0-based); None if it gives nothing. */
fn strftime(fmt: &std::ffi::CStr, wday: i32, mon: i32) -> Option<Vec<u8>> {
    let mut tm: libc::tm = posix::sys::zeroed();
    tm.tm_wday = wday;
    tm.tm_mon = mon;
    tm.tm_mday = 1;
    let mut buf = [0u8; 256];
    let n = posix::time::strftime(&mut buf, fmt, &tm);
    if n == 0 {
        return None;
    }
    Some(buf[..n].to_vec())
}

/* Append blanks to s up to the given number of characters. */
fn pad(s: &mut Vec<u8>, cols: usize) {
    let have = posix::mb::count_chars(s);
    s.resize(s.len() + cols.saturating_sub(have), b' ');
}

/* s centred in the given width (the extra blank, if any, goes to the right), not padded on the right. */
fn centre(s: &[u8], cols: usize) -> Vec<u8> {
    let lead = cols.saturating_sub(posix::mb::count_chars(s)) / 2;
    let mut out = vec![b' '; lead];
    out.extend_from_slice(s);
    out
}

/* A day name cut to its first two characters, and made two columns wide. */
fn short_day(name: &[u8]) -> Vec<u8> {
    let mut s = name[..posix::mb::skip_chars(name, 2)].to_vec();
    if posix::mb::count_chars(&s) < 2 {
        s.insert(0, b' ');
    }
    s
}

/* One month as lines of exactly WIDTH columns: the title, the day names, the weeks. */
fn month_block(y: i64, m: u32, names: &Names, with_year: bool) -> Vec<Vec<u8>> {
    let mut title = names.months[m as usize - 1].clone();
    if with_year {
        title.extend_from_slice(format!(" {y}").as_bytes());
    }
    let mut lines = Vec::new();
    let mut t = centre(&title, WIDTH);
    pad(&mut t, WIDTH);
    lines.push(t);
    let mut head: Vec<u8> = Vec::new();
    for (i, d) in names.days.iter().enumerate() {
        if i > 0 {
            head.push(b' ');
        }
        head.extend_from_slice(&short_day(d));
    }
    pad(&mut head, WIDTH);
    lines.push(head);
    for w in weeks(y, m) {
        let mut line: Vec<u8> = Vec::new();
        for (i, &d) in w.iter().enumerate() {
            if i > 0 {
                line.push(b' ');
            }
            if d == 0 {
                line.extend_from_slice(b"  ");
            } else {
                line.extend_from_slice(format!("{d:2}").as_bytes());
            }
        }
        lines.push(line);
    }
    lines
}

/* The blanks at the end of a line are not written. */
fn trim_end(mut line: Vec<u8>) -> Vec<u8> {
    while line.last() == Some(&b' ') {
        line.pop();
    }
    line
}

fn finish(lines: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    for l in lines {
        out.extend_from_slice(&trim_end(l));
        out.push(b'\n');
    }
    out
}

/* The calendar of one month. */
fn month_calendar(y: i64, m: u32, names: &Names) -> Vec<u8> {
    finish(month_block(y, m, names, true))
}

/* The calendar of a year: its number, then the twelve months, three side by side. */
fn year_calendar(y: i64, names: &Names) -> Vec<u8> {
    let mut lines = vec![centre(y.to_string().as_bytes(), YEAR_WIDTH)];
    for band in 0..4u32 {
        if band > 0 {
            lines.push(Vec::new());
        }
        let blocks: Vec<Vec<Vec<u8>>> = (1..=3).map(|i| month_block(y, band * 3 + i, names, false)).collect();
        for r in 0..blocks.iter().map(Vec::len).max().unwrap_or(0) {
            let mut line: Vec<u8> = Vec::new();
            for (i, b) in blocks.iter().enumerate() {
                if i > 0 {
                    line.extend_from_slice(b"  ");
                }
                match b.get(r) {
                    Some(l) => line.extend_from_slice(l),
                    None => line.resize(line.len() + WIDTH, b' '),
                }
            }
            lines.push(line);
        }
    }
    finish(lines)
}

/* What the operands ask for. */
enum Request {
    Current,
    Year(i64),
    Month(i64, u32),
}

enum Bad {
    Usage,
    Message(String),
}

/* A decimal integer of digits only; one too big for u32 is as big as one that does not fit a month or a year. */
fn decimal(s: &[u8]) -> Option<u32> {
    if s.is_empty() || !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut n: u32 = 0;
    for &d in s {
        n = n.saturating_mul(10).saturating_add(u32::from(d - b'0'));
    }
    Some(n)
}

fn year_operand(s: &[u8]) -> Result<i64, Bad> {
    match decimal(s) {
        Some(n @ 1..=9999) => Ok(i64::from(n)),
        _ => Err(Bad::Message(format!("{}: not a year from 1 to 9999", String::from_utf8_lossy(s)))),
    }
}

fn month_operand(s: &[u8]) -> Result<u32, Bad> {
    match decimal(s) {
        Some(n @ 1..=12) => Ok(n),
        _ => Err(Bad::Message(format!("{}: not a month from 1 to 12", String::from_utf8_lossy(s)))),
    }
}

fn request(operands: &[Vec<u8>]) -> Result<Request, Bad> {
    match operands {
        [] => Ok(Request::Current),
        [y] => Ok(Request::Year(year_operand(y)?)),
        [m, y] => {
            let m = month_operand(m)?;
            Ok(Request::Month(year_operand(y)?, m))
        }
        _ => Err(Bad::Usage),
    }
}

/* The calendar for the operands; now is asked only when there are none. */
fn calendar<F: FnOnce() -> Option<(i64, u32)>>(operands: &[Vec<u8>], now: F, names: &Names) -> Result<Vec<u8>, Bad> {
    Ok(match request(operands)? {
        Request::Current => match now() {
            Some((y, m)) if (1..=9999).contains(&y) => month_calendar(y, m, names),
            _ => return Err(Bad::Message("cannot find the current month".to_string())),
        },
        Request::Year(y) => year_calendar(y, names),
        Request::Month(y, m) => month_calendar(y, m, names),
    })
}

/* The current year and month in the time zone of TZ. */
fn today() -> Option<(i64, u32)> {
    posix::cal::null_tz_is_unset();
    posix::time::tzset();
    let tm = posix::time::localtime(posix::time::now() as libc::time_t)?;
    Some((i64::from(tm.tm_year) + 1900, (tm.tm_mon + 1) as u32))
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let operands = posix::no_options(&args, SYNOPSIS);
    match calendar(operands, today, &Names::locale()) {
        Ok(out) => {
            let mut w = posix::Stdout::new();
            if let Err(e) = w.write_all(&out).and_then(|_| w.flush()) {
                posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
            }
        }
        Err(Bad::Usage) => posix::usage(SYNOPSIS),
        Err(Bad::Message(m)) => posix::fail(&prog, &m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /* the locale is process-wide: tests that change it hold this */
    static LOCALE: Mutex<()> = Mutex::new(());

    const SUN: usize = 0;
    const MON: usize = 1;
    const TUE: usize = 2;
    const WED: usize = 3;
    const THU: usize = 4;
    const FRI: usize = 5;
    const SAT: usize = 6;

    fn month(y: i64, m: u32) -> String {
        String::from_utf8(month_calendar(y, m, &Names::c())).unwrap()
    }

    fn year(y: i64) -> String {
        String::from_utf8(year_calendar(y, &Names::c())).unwrap()
    }

    fn ops(v: &[&str]) -> Vec<Vec<u8>> {
        v.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    fn run(v: &[&str], now: Option<(i64, u32)>) -> Result<String, String> {
        match calendar(&ops(v), || now, &Names::c()) {
            Ok(o) => Ok(String::from_utf8(o).unwrap()),
            Err(Bad::Usage) => Err("usage".to_string()),
            Err(Bad::Message(m)) => Err(m),
        }
    }

    #[test]
    fn days_of_the_week_of_known_dates() {
        assert_eq!(weekday(2000, 1, 1), SAT);
        assert_eq!(weekday(2024, 2, 29), THU);
        assert_eq!(weekday(1970, 1, 1), THU);
        assert_eq!(weekday(2026, 11, 2), MON);
        assert_eq!(weekday(9999, 12, 31), FRI);
        assert_eq!(weekday(1900, 3, 1), THU);
        /* the Julian calendar: the 1st of January of the year 1 and the day Columbus sailed */
        assert_eq!(weekday(1, 1, 1), SAT);
        assert_eq!(weekday(1492, 8, 3), FRI);
        assert_eq!(weekday(1066, 10, 14), SAT);
    }

    #[test]
    fn the_change_of_calendar_loses_eleven_days_and_no_weekday() {
        /* September 2, 1752 was a Wednesday and the next day was the 14th, a Thursday */
        assert_eq!(weekday(1752, 9, 2), WED);
        assert_eq!(weekday(1752, 9, 14), THU);
        assert_eq!(weekday(1752, 9, 1), TUE);
        assert_eq!(weekday(1752, 9, 30), SAT);
        assert_eq!(weekday(1752, 10, 1), SUN);
        assert_eq!(days_of(1752, 9), [vec![1, 2], (14..=30).collect::<Vec<u32>>()].concat());
        assert_eq!(days_of(1752, 9).len(), 19);
        assert_eq!(days_of(1752, 8).len(), 31);
        assert_eq!(days_of(1752, 10).len(), 31);
    }

    #[test]
    fn leap_years_follow_the_calendar_in_use() {
        /* Julian: every fourth year, centuries too */
        assert!(is_leap(4) && is_leap(100) && is_leap(1500) && is_leap(1700));
        assert!(!is_leap(1) && !is_leap(1699) && !is_leap(1701));
        assert!(is_leap(1752));
        /* Gregorian after it */
        assert!(!is_leap(1800) && !is_leap(1900) && !is_leap(2100));
        assert!(is_leap(2000) && is_leap(2024) && is_leap(2400));
        assert!(!is_leap(2023) && !is_leap(2026));
        assert_eq!(days_in_month(1700, 2), 29);
        assert_eq!(days_in_month(1800, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(9999, 12), 31);
        assert_eq!(days_in_month(2026, 4), 30);
    }

    #[test]
    fn a_month_has_the_expected_shape() {
        assert_eq!(
            month(2024, 2),
            "   February 2024\nSu Mo Tu We Th Fr Sa\n             1  2  3\n 4  5  6  7  8  9 10\n\
             11 12 13 14 15 16 17\n18 19 20 21 22 23 24\n25 26 27 28 29\n"
        );
    }

    #[test]
    fn september_1752_has_no_days_3_to_13() {
        assert_eq!(
            month(1752, 9),
            "   September 1752\nSu Mo Tu We Th Fr Sa\n       1  2 14 15 16\n17 18 19 20 21 22 23\n24 25 26 27 28 29 30\n"
        );
    }

    #[test]
    fn the_first_month_of_the_first_year_is_julian() {
        assert_eq!(
            month(1, 1),
            "     January 1\nSu Mo Tu We Th Fr Sa\n                   1\n 2  3  4  5  6  7  8\n 9 10 11 12 13 14 15\n\
             16 17 18 19 20 21 22\n23 24 25 26 27 28 29\n30 31\n"
        );
    }

    #[test]
    fn the_last_month_is_gregorian() {
        /* December 31, 9999 is a Friday, so the 1st is a Wednesday */
        assert_eq!(
            month(9999, 12),
            "   December 9999\nSu Mo Tu We Th Fr Sa\n          1  2  3  4\n 5  6  7  8  9 10 11\n\
             12 13 14 15 16 17 18\n19 20 21 22 23 24 25\n26 27 28 29 30 31\n"
        );
    }

    #[test]
    fn a_month_of_four_weeks_has_four_rows() {
        /* February 2026 starts on a Sunday and has 28 days */
        let m = month(2026, 2);
        assert_eq!(m.lines().count(), 2 + 4);
        assert!(!m.contains("  \n"));
    }

    #[test]
    fn a_month_of_six_weeks_has_six_rows() {
        /* March 2025 starts on a Saturday and has 31 days */
        assert_eq!(month(2025, 3).lines().count(), 2 + 6);
    }

    #[test]
    fn no_line_ends_in_a_blank() {
        for y in [1, 1752, 2000, 2024, 9999] {
            for m in 1..=12 {
                assert!(!month(y, m).lines().any(|l| l.ends_with(' ')), "{y}-{m}");
            }
            assert!(!year(y).lines().any(|l| l.ends_with(' ')), "{y}");
        }
    }

    #[test]
    fn every_day_that_exists_is_shown_once_in_its_column() {
        for (y, m) in [(2024, 2), (2026, 11), (1752, 9), (1752, 10), (1, 12), (1700, 2), (1800, 2)] {
            let mut shown = Vec::new();
            for (r, w) in weeks(y, m).iter().enumerate() {
                for (c, &d) in w.iter().enumerate() {
                    if d != 0 {
                        assert_eq!(weekday(y, m, d), c, "{y}-{m}-{d} row {r}");
                        shown.push(d);
                    }
                }
            }
            assert_eq!(shown, days_of(y, m), "{y}-{m}");
        }
    }

    #[test]
    fn a_year_shows_twelve_months_three_across() {
        let y = year(2026);
        let lines: Vec<&str> = y.lines().collect();
        assert_eq!(lines[0], "                            2026");
        assert_eq!(lines[1], "      January               February               March");
        assert_eq!(lines[2], "Su Mo Tu We Th Fr Sa  Su Mo Tu We Th Fr Sa  Su Mo Tu We Th Fr Sa");
        assert_eq!(lines[3], "             1  2  3   1  2  3  4  5  6  7   1  2  3  4  5  6  7");
        for name in [
            "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
        ] {
            assert_eq!(y.matches(name).count(), 1, "{name}");
        }
        /* four bands of months, a blank line between them */
        assert_eq!(y.lines().filter(|l| l.is_empty()).count(), 3);
        assert!(!y.ends_with("\n\n"));
    }

    #[test]
    fn a_year_number_is_not_padded() {
        assert!(year(83).lines().next().unwrap().trim() == "83");
        assert!(month(83, 3).starts_with("      March 83\n"));
    }

    #[test]
    fn september_1752_in_the_year_1752() {
        let y = year(1752);
        assert!(y.contains("       1  2 14 15 16"));
        /* the 3rd to the 13th of that month are nowhere in the Sept block: its second week starts at 17 */
        assert!(y.contains("17 18 19 20 21 22 23"));
    }

    #[test]
    fn no_operand_is_the_current_month() {
        let out = run(&[], Some((2026, 11))).unwrap();
        assert_eq!(out, month(2026, 11));
        assert!(out.starts_with("   November 2026\n"));
    }

    #[test]
    fn the_clock_is_not_asked_when_there_are_operands() {
        assert!(calendar(&ops(&["2000"]), || panic!("no clock"), &Names::c()).is_ok());
        assert!(calendar(&ops(&["2", "2000"]), || panic!("no clock"), &Names::c()).is_ok());
    }

    #[test]
    fn a_year_alone_is_the_twelve_months_of_that_year() {
        /* RATIONALE: "cal 2000" is not the current month of the year 2000 */
        let out = run(&["2000"], Some((2026, 11))).unwrap();
        assert_eq!(out, year(2000));
        assert_eq!(out.matches("Su Mo Tu We Th Fr Sa").count(), 12);
    }

    #[test]
    fn a_month_and_a_year_are_one_month() {
        assert_eq!(run(&["2", "2024"], None).unwrap(), month(2024, 2));
        assert_eq!(run(&["02", "2024"], None).unwrap(), month(2024, 2));
        assert_eq!(run(&["12", "1999"], None).unwrap(), month(1999, 12));
    }

    #[test]
    fn cal_83_is_the_year_83() {
        /* APPLICATION USAGE: "cal 83" refers to A.D. 83, not 1983 */
        let out = run(&["83"], None).unwrap();
        assert_eq!(out.lines().next().unwrap().trim(), "83");
        assert_eq!(out, year(83));
        assert!(!out.contains("1983"));
        assert_eq!(run(&["3", "83"], None).unwrap(), month(83, 3));
    }

    #[test]
    fn the_first_operand_alone_is_a_year_even_when_it_could_be_a_month() {
        assert_eq!(run(&["12"], None).unwrap(), year(12));
        assert_eq!(run(&["1"], None).unwrap(), year(1));
    }

    #[test]
    fn months_and_years_out_of_range_are_errors() {
        for bad in [
            vec!["0"], vec!["10000"], vec!["13", "2020"], vec!["0", "2020"], vec!["1", "0"], vec!["1", "10000"], vec!["2020", "1"],
            vec!["abc"], vec!["1", "x"], vec!["x", "2020"], vec!["-1"], vec!["+5"], vec!["1.5"], vec![""], vec!["1", ""],
            vec!["99999999999999999999"], vec!["1", "99999999999999999999"], vec!["12 "], vec![" 12"],
        ] {
            assert!(run(&bad, None).is_err(), "{bad:?}");
        }
        assert!(run(&["1", "9999"], None).is_ok());
        assert!(run(&["12", "1"], None).is_ok());
        assert!(run(&["9999"], None).is_ok());
    }

    #[test]
    fn three_operands_are_a_usage_error() {
        assert_eq!(run(&["1", "2", "3"], None), Err("usage".to_string()));
        assert_eq!(run(&["1", "2000", "x"], None), Err("usage".to_string()));
    }

    #[test]
    fn a_clock_that_gives_nothing_is_an_error() {
        assert!(run(&[], None).is_err());
        assert!(run(&[], Some((0, 5))).is_err());
        assert!(run(&[], Some((10000, 5))).is_err());
    }

    #[test]
    fn every_month_of_every_century_year_has_the_right_number_of_days() {
        for y in (100..=9900).step_by(100) {
            let total: usize = (1..=12).map(|m| days_of(y, m).len()).sum();
            assert_eq!(total, if is_leap(y) { 366 } else { 365 }, "{y}");
        }
        let total: usize = (1..=12).map(|m| days_of(1752, m).len()).sum();
        assert_eq!(total, 366 - 11);
    }

    #[test]
    fn weekdays_advance_by_one_each_day_across_the_change() {
        /* from 1752-01-01 to 1753-12-31, day by day, except the 11 days that do not exist */
        let mut prev: Option<usize> = None;
        for y in 1751..=1753i64 {
            for m in 1..=12u32 {
                for d in days_of(y, m) {
                    let w = weekday(y, m, d);
                    if let Some(p) = prev {
                        assert_eq!(w, (p + 1) % 7, "{y}-{m}-{d}");
                    }
                    prev = Some(w);
                }
            }
        }
    }

    #[test]
    fn the_names_come_from_lc_time() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let c = Names::locale();
        assert_eq!(c.months[0], b"January");
        assert_eq!(c.days[0], b"Sun");
        assert_eq!(short_day(&c.days[0]), b"Su");
        if !posix::locale::try_set("fr_FR.UTF-8") {
            return;
        }
        let fr = Names::locale();
        /* the locale decides: French month names are not capitalised */
        assert_eq!(fr.months[2], b"mars");
        assert_eq!(short_day(&fr.days[1]), b"lu");
        let out = String::from_utf8(month_calendar(2020, 3, &fr)).unwrap();
        assert!(out.starts_with("     mars 2020\ndi lu ma me je ve sa\n"));
        /* two characters, not two bytes: e-acute is two bytes */
        assert_eq!(short_day("\u{e9}t\u{e9}".as_bytes()), "\u{e9}t".as_bytes());
        posix::locale::set_c();
    }

    #[test]
    fn day_names_are_two_characters() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(short_day(b"Sun"), b"Su");
        assert_eq!(short_day(b"S"), b" S");
        assert_eq!(short_day(b"Mo"), b"Mo");
    }
}
