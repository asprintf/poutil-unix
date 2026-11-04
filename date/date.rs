/* SPDX-License-Identifier: GPL-2.0-only */
/* date - write the date and time, or set it. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::time::Tm;

const SYNOPSIS: &str = "date [-u] [+format]\n       date [-u] mmddhhmm[[cc]yy]";

/* What the output is in the POSIX locale when no format is given. */
const DEFAULT_FORMAT: &[u8] = b"+%a %b %e %H:%M:%S %Z %Y";

/* The conversion specifiers of the spec, and which ones E and O may modify. */
const PLAIN: &[u8] = b"aAbBcCdDehHIjmMnprStTuUVwWxXyYZ%";
const MOD_E: &[u8] = b"cCxXyY";
const MOD_O: &[u8] = b"deHImMSuUVwWy";

/* A format, taken apart. */
#[derive(Debug, PartialEq, Eq)]
enum Piece {
    /* text copied as it is */
    Lit(Vec<u8>),
    /* a conversion specification for strftime, "%" included, e.g. "%Ey" */
    Spec(Vec<u8>),
}

/*
 * Split a format into text and conversion specifications. Err holds the
 * offending specification. A "%" with nothing after it, and a specifier the
 * spec does not list, are errors: what they mean is not defined.
 */
fn parse_format(f: &[u8]) -> Result<Vec<Piece>, Vec<u8>> {
    let mut out = Vec::new();
    let mut lit = Vec::new();
    let mut i = 0;
    while i < f.len() {
        if f[i] != b'%' {
            lit.push(f[i]);
            i += 1;
            continue;
        }
        let rest = &f[i + 1..];
        let (len, ok) = match rest {
            [] => (0, false),
            [b'E', c, ..] if MOD_E.contains(c) => (2, true),
            [b'O', c, ..] if MOD_O.contains(c) => (2, true),
            [b'E' | b'O', c, ..] => (if c.is_ascii() { 2 } else { 1 }, false),
            [c, ..] => (1, PLAIN.contains(c)),
        };
        if !ok {
            let mut bad = vec![b'%'];
            bad.extend_from_slice(&rest[..len]);
            return Err(bad);
        }
        if !lit.is_empty() {
            out.push(Piece::Lit(std::mem::take(&mut lit)));
        }
        let mut spec = vec![b'%'];
        spec.extend_from_slice(&rest[..len]);
        out.push(Piece::Spec(spec));
        i += 1 + len;
    }
    if !lit.is_empty() {
        out.push(Piece::Lit(lit));
    }
    Ok(out)
}

/*
 * One conversion through strftime. The empty result is a valid answer (no
 * time zone name) but strftime also returns 0 when the buffer is too small,
 * so the format is led by a character that is always produced.
 */
fn convert(spec: &[u8], tm: &Tm) -> Vec<u8> {
    let mut fmt = vec![b'x'];
    fmt.extend_from_slice(spec);
    let cf = posix::cstr(&fmt);
    let mut size = 256;
    loop {
        let mut buf = vec![0u8; size];
        let n = posix::time::strftime(&mut buf, &cf, tm);
        if n > 0 {
            return buf[1..n].to_vec();
        }
        if size >= 1 << 20 {
            return Vec::new();
        }
        size *= 4;
    }
}

/* The text for a broken-down time, without the final newline. */
fn expand(pieces: &[Piece], tm: &Tm) -> Vec<u8> {
    let mut out = Vec::new();
    for p in pieces {
        match p {
            Piece::Lit(t) => out.extend_from_slice(t),
            Piece::Spec(s) => match s[1] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'%' => out.push(b'%'),
                _ => out.extend_from_slice(&convert(s, tm)),
            },
        }
    }
    out
}

/* mmddhhmm[[cc]yy]: the time to set. Year is None when it is not given. */
#[derive(Debug, PartialEq, Eq)]
struct SetTime {
    year: Option<i64>,
    mon: u32,
    day: u32,
    hour: u32,
    min: u32,
}

fn two(s: &[u8]) -> u32 {
    (s[0] - b'0') as u32 * 10 + (s[1] - b'0') as u32
}

/* Eight, ten or twelve digits. Without a century the 69 to 99 / 00 to 68 rule applies. */
fn parse_set(s: &[u8]) -> Option<SetTime> {
    if !s.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let year = match s.len() {
        8 => None,
        10 => Some(posix::cal::two_digit_year(two(&s[8..10]) as i64)),
        12 => Some(two(&s[8..10]) as i64 * 100 + two(&s[10..12]) as i64),
        _ => return None,
    };
    Some(SetTime { year, mon: two(&s[0..2]), day: two(&s[2..4]), hour: two(&s[4..6]), min: two(&s[6..8]) })
}

/* Is every field in range, the day one that the month has? */
fn set_valid(t: &SetTime, cur_year: i64) -> bool {
    posix::cal::valid(t.year.unwrap_or(cur_year), t.mon, t.day, t.hour, t.min, 0)
}

fn local_tm(secs: i64) -> Option<Tm> {
    /* a time_t that cannot hold the value is no time at all, not a wrapped one */
    let t = libc::time_t::try_from(secs).ok()?;
    posix::time::localtime(t)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut utc = false;
    let mut g = Getopt::new(&args, b"u");
    for o in g.by_ref() {
        match o {
            Opt::Flag(b'u') => utc = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let operands = g.operands();
    if operands.len() > 1 {
        posix::usage(SYNOPSIS);
    }
    posix::cal::null_tz_is_unset();
    if utc {
        /* "as if the TZ environment variable was set to the string UTC0" */
        posix::time::use_utc();
    }
    let mut secs = match posix::time::now() {
        Ok(s) => s,
        Err(e) => posix::fail(&prog, &format!("cannot read the clock: {}", posix::strerror(&e))),
    };
    let mut format = DEFAULT_FORMAT;
    if let Some(op) = operands.first() {
        if op.first() == Some(&b'+') {
            format = op;
        } else if let Some(t) = parse_set(op) {
            let cur = match local_tm(secs) {
                Some(tm) => tm.tm_year as i64 + 1900,
                None => posix::fail(&prog, "cannot work out the current year"),
            };
            if !set_valid(&t, cur) {
                posix::warn_path(&prog, op, "invalid date");
                std::process::exit(2);
            }
            let (y, mo, d, h, mi) = (t.year.unwrap_or(cur), t.mon, t.day, t.hour, t.min);
            let Some(new) = (if utc {
                Some(posix::cal::epoch_utc(y, mo, d, h, mi, 0))
            } else {
                posix::cal::epoch_local(y, mo, d, h, mi, 0)
            }) else {
                posix::warn_path(&prog, op, "date out of range");
                std::process::exit(1);
            };
            let Some(tv_sec) = libc::time_t::try_from(new).ok() else {
                posix::warn_path(&prog, op, "date out of range");
                std::process::exit(1);
            };
            if let Err(e) = posix::time::set_realtime(tv_sec) {
                posix::warn(&prog, &format!("cannot set date: {}", posix::strerror(&e)));
                std::process::exit(1);
            }
            /* what was set is written, like the display of the time */
            secs = new;
        } else {
            posix::usage(SYNOPSIS);
        }
    }
    let pieces = match parse_format(&format[1..]) {
        Ok(p) => p,
        Err(bad) => {
            posix::warn_path(&prog, &bad, "invalid conversion specification");
            std::process::exit(1);
        }
    };
    let Some(tm) = local_tm(secs) else {
        posix::fail(&prog, "the time cannot be represented");
    };
    let mut out = expand(&pieces, &tm);
    out.push(b'\n');
    posix::write_stdout(&prog, &out);
}

#[cfg(test)]
mod tests {
    use super::*;

    /* a broken-down UTC time; the zone name can be overridden (a static C string) */
    fn tm_at(secs: i64, zone: Option<&'static std::ffi::CStr>) -> Tm {
        let t = secs as libc::time_t;
        let mut tm = posix::time::gmtime(t).expect("gmtime");
        if let Some(z) = zone {
            tm.set_zone(z);
        }
        tm
    }

    fn fmt(f: &str, secs: i64) -> String {
        let p = parse_format(f.as_bytes()).unwrap();
        String::from_utf8(expand(&p, &tm_at(secs, None))).unwrap()
    }

    fn utc(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
        posix::cal::epoch_utc(y, mo, d, h, mi, s)
    }

    /* EXAMPLES 1: $ date  ->  Tue Jun 26 09:58:10 PDT 1990 */
    #[test]
    fn example_default_output() {
        let tm = tm_at(utc(1990, 6, 26, 9, 58, 10), Some(c"PDT"));
        let p = parse_format(&DEFAULT_FORMAT[1..]).unwrap();
        assert_eq!(expand(&p, &tm), b"Tue Jun 26 09:58:10 PDT 1990");
    }

    /* EXAMPLES 1: $ date "+DATE: %m/%d/%y%nTIME: %H:%M:%S"  ->  DATE: 11/02/91 TIME: 13:36:16 */
    #[test]
    fn example_date_and_time_lines() {
        assert_eq!(fmt("DATE: %m/%d/%y%nTIME: %H:%M:%S", utc(1991, 11, 2, 13, 36, 16)), "DATE: 11/02/91\nTIME: 13:36:16");
    }

    /* EXAMPLES 1: $ date "+TIME: %r"  ->  TIME: 01:36:32 PM */
    #[test]
    fn example_twelve_hour_clock() {
        assert_eq!(fmt("TIME: %r", utc(1991, 11, 2, 13, 36, 32)), "TIME: 01:36:32 PM");
    }

    /* STDOUT: the POSIX-locale output equals "+%a %b %e %H:%M:%S %Z %Y" */
    #[test]
    fn default_format_is_the_documented_one() {
        assert_eq!(DEFAULT_FORMAT, b"+%a %b %e %H:%M:%S %Z %Y");
    }

    #[test]
    fn every_specifier_in_the_posix_locale() {
        let t = utc(2009, 2, 3, 4, 5, 6); /* Tuesday, Feb 3 2009, 04:05:06 */
        for (f, want) in [
            ("%a", "Tue"),
            ("%A", "Tuesday"),
            ("%b", "Feb"),
            ("%B", "February"),
            ("%C", "20"),
            ("%d", "03"),
            ("%D", "02/03/09"),
            ("%e", " 3"),
            ("%h", "Feb"),
            ("%H", "04"),
            ("%I", "04"),
            ("%j", "034"),
            ("%m", "02"),
            ("%M", "05"),
            ("%n", "\n"),
            ("%p", "AM"),
            ("%r", "04:05:06 AM"),
            ("%S", "06"),
            ("%t", "\t"),
            ("%T", "04:05:06"),
            ("%u", "2"),
            ("%U", "05"),
            ("%V", "06"),
            ("%w", "2"),
            ("%W", "05"),
            ("%y", "09"),
            ("%Y", "2009"),
            ("%%", "%"),
            ("%c", "Tue Feb  3 04:05:06 2009"),
            ("%x", "02/03/09"),
            ("%X", "04:05:06"),
        ] {
            assert_eq!(fmt(f, t), want, "{f}");
        }
    }

    /* %Z is the zone name of the broken-down time, whatever the C library calls UTC ("GMT" in glibc) */
    #[test]
    fn zone_name_is_the_one_in_the_broken_down_time() {
        let p = parse_format(b"%Z").unwrap();
        let tm = tm_at(utc(2009, 2, 3, 4, 5, 6), Some(c"UTC"));
        assert_eq!(expand(&p, &tm), b"UTC");
        let tm = tm_at(utc(2009, 2, 3, 4, 5, 6), Some(c"GMT"));
        assert_eq!(expand(&p, &tm), b"GMT");
    }

    /* %E and %O fall back to the plain value when the locale has no alternatives */
    #[test]
    fn modified_specifiers_equal_the_plain_ones_in_the_posix_locale() {
        let t = utc(2009, 12, 31, 15, 5, 6);
        for (m, p) in [
            ("%Ec", "%c"), ("%EC", "%C"), ("%Ex", "%x"), ("%EX", "%X"), ("%Ey", "%y"), ("%EY", "%Y"),
            ("%Od", "%d"), ("%Oe", "%e"), ("%OH", "%H"), ("%OI", "%I"), ("%Om", "%m"), ("%OM", "%M"),
            ("%OS", "%S"), ("%Ou", "%u"), ("%OU", "%U"), ("%OV", "%V"), ("%Ow", "%w"), ("%OW", "%W"), ("%Oy", "%y"),
        ] {
            assert_eq!(fmt(m, t), fmt(p, t), "{m}");
            assert!(!fmt(m, t).is_empty(), "{m}");
        }
    }

    #[test]
    fn literal_text_is_copied_whole() {
        assert_eq!(fmt("", 0), "");
        assert_eq!(fmt("plain", 0), "plain");
        assert_eq!(fmt("a%%b%%", 0), "a%b%");
        assert_eq!(fmt("%Y-%m-%d %H:%M:%S", 0), "1970-01-01 00:00:00");
        assert_eq!(fmt("100%%%Y", 0), "100%1970");
        assert_eq!(fmt("%Y%Y", 0), "19701970");
        let raw = parse_format(&[0xff, b'%', b'Y', 0xfe, 0x80]).unwrap();
        assert_eq!(expand(&raw, &tm_at(0, None)), [0xff, b'1', b'9', b'7', b'0', 0xfe, 0x80]);
    }

    #[test]
    fn no_time_zone_name_is_no_characters() {
        let mut tm = tm_at(0, None);
        tm.set_zone(c"");
        /* the daylight saving flag is "not known": glibc would name the zone of TZ from it otherwise */
        tm.set_isdst(-1);
        let p = parse_format(b"[%Z]").unwrap();
        assert_eq!(expand(&p, &tm), b"[]");
    }

    #[test]
    fn bad_specifiers_are_refused_whole() {
        for (f, bad) in [
            (&b"%"[..], &b"%"[..]),
            (b"abc%", b"%"),
            (b"%Q", b"%Q"),
            (b"%s", b"%s"),
            (b"%z", b"%z"),
            (b"%N", b"%N"),
            (b"%-d", b"%-"),
            (b"%Ea", b"%Ea"),
            (b"%Oc", b"%Oc"),
            (b"%EE", b"%EE"),
            (b"%E", b"%E"),
            (b"%O", b"%O"),
            (b"%Y%k", b"%k"),
            (b"%Y %_d", b"%_"),
            (b"%5Y", b"%5"),
        ] {
            assert_eq!(parse_format(f), Err(bad.to_vec()), "{}", String::from_utf8_lossy(f));
        }
    }

    #[test]
    fn parse_format_splits_text_and_specs() {
        assert_eq!(
            parse_format(b"a%Yb%Ecc").unwrap(),
            vec![Piece::Lit(b"a".to_vec()), Piece::Spec(b"%Y".to_vec()), Piece::Lit(b"b".to_vec()), Piece::Spec(b"%Ec".to_vec()), Piece::Lit(b"c".to_vec())]
        );
        assert_eq!(parse_format(b"").unwrap(), vec![]);
        assert_eq!(parse_format(b"%%").unwrap(), vec![Piece::Spec(b"%%".to_vec())]);
    }

    #[test]
    fn set_forms() {
        let s = |x: &str| parse_set(x.as_bytes());
        assert_eq!(s("11121015"), Some(SetTime { year: None, mon: 11, day: 12, hour: 10, min: 15 }));
        assert_eq!(s("1112101507"), Some(SetTime { year: Some(2007), mon: 11, day: 12, hour: 10, min: 15 }));
        assert_eq!(s("1112101599"), Some(SetTime { year: Some(1999), mon: 11, day: 12, hour: 10, min: 15 }));
        assert_eq!(s("1112101569"), Some(SetTime { year: Some(1969), mon: 11, day: 12, hour: 10, min: 15 }));
        assert_eq!(s("1112101568"), Some(SetTime { year: Some(2068), mon: 11, day: 12, hour: 10, min: 15 }));
        assert_eq!(s("111210152007"), Some(SetTime { year: Some(2007), mon: 11, day: 12, hour: 10, min: 15 }));
        assert_eq!(s("111210151999"), Some(SetTime { year: Some(1999), mon: 11, day: 12, hour: 10, min: 15 }));
        for bad in ["", "1112101", "111210150", "11121015070", "1112101507200", "1112101a", "1112 1015", "+1112101", "11121015.00"] {
            assert_eq!(s(bad), None, "{bad}");
        }
    }

    #[test]
    fn set_ranges() {
        let ok = |x: &str, cur| parse_set(x.as_bytes()).map(|t| set_valid(&t, cur));
        assert_eq!(ok("11121015", 2026), Some(true));
        assert_eq!(ok("02291200", 2024), Some(true));
        assert_eq!(ok("02291200", 2023), Some(false));
        assert_eq!(ok("0229120023", 2023), Some(false));
        assert_eq!(ok("0229120024", 2023), Some(true));
        assert_eq!(ok("13011200", 2026), Some(false));
        assert_eq!(ok("00011200", 2026), Some(false));
        assert_eq!(ok("01001200", 2026), Some(false));
        assert_eq!(ok("01322400", 2026), Some(false));
        assert_eq!(ok("01012400", 2026), Some(false));
        assert_eq!(ok("01011260", 2026), Some(false));
        assert_eq!(ok("12312359", 2026), Some(true));
        assert_eq!(ok("01010000", 2026), Some(true));
    }

    /* The numeric fields, checked against an independent computation over every day of 1900 to 2200. */
    fn weekday(days: i64) -> i64 {
        (days + 4).rem_euclid(7) /* 1970-01-01 was a Thursday; 0 = Sunday */
    }

    fn weeks_in_iso_year(y: i64) -> i64 {
        let p = |y: i64| (y + y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400)).rem_euclid(7);
        if p(y) == 4 || p(y - 1) == 3 {
            53
        } else {
            52
        }
    }

    fn iso_week(y: i64, yday1: i64, wday: i64) -> i64 {
        let isowd = if wday == 0 { 7 } else { wday };
        let w = (yday1 - isowd + 10).div_euclid(7);
        if w < 1 {
            weeks_in_iso_year(y - 1)
        } else if w > weeks_in_iso_year(y) {
            1
        } else {
            w
        }
    }

    #[test]
    fn numeric_specifiers_agree_with_an_independent_computation() {
        use posix::cal::*;
        let mut checked = 0;
        for y in 1900..=2200i64 {
            let mut yday = 0i64;
            for mo in 1..=12u32 {
                for d in 1..=days_in_month(y, mo) {
                    let days = days_from_civil(y, mo, d);
                    let wd = weekday(days);
                    let t = days * 86400 + 13 * 3600 + 5 * 60 + 9;
                    let tm = tm_at(t, None);
                    let get = |f: &str| {
                        let p = parse_format(f.as_bytes()).unwrap();
                        String::from_utf8(expand(&p, &tm)).unwrap()
                    };
                    let want_u = (yday + 7 - wd) / 7;
                    let want_w = (yday + 7 - (wd + 6) % 7) / 7;
                    let date = format!("{y}-{mo}-{d}");
                    assert_eq!(get("%Y"), format!("{y}"), "{date}");
                    assert_eq!(get("%C"), format!("{:02}", y / 100), "{date}");
                    assert_eq!(get("%y"), format!("{:02}", y % 100), "{date}");
                    assert_eq!(get("%m"), format!("{mo:02}"), "{date}");
                    assert_eq!(get("%d"), format!("{d:02}"), "{date}");
                    assert_eq!(get("%e"), format!("{d:2}"), "{date}");
                    assert_eq!(get("%j"), format!("{:03}", yday + 1), "{date}");
                    assert_eq!(get("%w"), format!("{wd}"), "{date}");
                    assert_eq!(get("%u"), format!("{}", if wd == 0 { 7 } else { wd }), "{date}");
                    assert_eq!(get("%U"), format!("{want_u:02}"), "{date}");
                    assert_eq!(get("%W"), format!("{want_w:02}"), "{date}");
                    assert_eq!(get("%V"), format!("{:02}", iso_week(y, yday + 1, wd)), "{date}");
                    assert_eq!(get("%D"), format!("{mo:02}/{d:02}/{:02}", y % 100), "{date}");
                    assert_eq!(get("%H:%M:%S"), "13:05:09", "{date}");
                    assert_eq!(get("%T"), "13:05:09", "{date}");
                    assert_eq!(get("%I %p"), "01 PM", "{date}");
                    yday += 1;
                    checked += 1;
                }
            }
        }
        assert!(checked > 100_000);
    }

    #[test]
    fn hours_of_the_twelve_hour_clock() {
        for (h, want) in [(0, "12 AM"), (1, "01 AM"), (11, "11 AM"), (12, "12 PM"), (13, "01 PM"), (23, "11 PM")] {
            assert_eq!(fmt("%I %p", utc(2020, 1, 1, h, 0, 0)), want);
        }
    }

    #[test]
    fn the_seconds_field_and_negative_times() {
        assert_eq!(fmt("%S", utc(2020, 1, 1, 0, 0, 59)), "59");
        assert_eq!(fmt("%Y-%m-%d %H:%M:%S", -1), "1969-12-31 23:59:59");
        assert_eq!(fmt("%Y", utc(12345, 1, 1, 0, 0, 0)), "12345");
    }

    #[test]
    fn a_long_conversion_is_not_cut() {
        /* %c can be long in some locales; the buffer grows until it fits */
        let tm = tm_at(0, None);
        assert!(convert(b"%c", &tm).len() > 10);
    }
}
