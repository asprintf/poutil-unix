/* SPDX-License-Identifier: GPL-2.0-only */
/* touch - change file access and modification times. */

use posix::getopt::{Getopt, Opt};
use std::ffi::CStr;

const SYNOPSIS: &str = "touch [-acm] [-r ref_file|-t time|-d date_time] file...";

/* A broken-down time as written in a -t or -d option-argument. */
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Civil {
    year: i64,
    mon: u32,
    day: u32,
    hour: u32,
    min: u32,
    /* 0 to 60: 60 is the second after 59, as the spec has it */
    sec: u32,
    nsec: u32,
}

/* Seconds since the Epoch of a calendar time read as UTC. */
fn utc_epoch(c: &Civil) -> i64 {
    posix::cal::epoch_utc(c.year, c.mon, c.day, c.hour, c.min, c.sec)
}

/* The calendar time read in the local time zone (TZ), through mktime. */
fn local_epoch(c: &Civil) -> Option<i64> {
    posix::cal::epoch_local(c.year, c.mon, c.day, c.hour, c.min, c.sec)
}

fn digits(s: &[u8]) -> bool {
    !s.is_empty() && s.iter().all(|c| c.is_ascii_digit())
}

fn num(s: &[u8]) -> u32 {
    s.iter().fold(0u32, |a, &c| a * 10 + (c - b'0') as u32)
}

/* Check the field ranges of the spec, and that the day exists in that month. */
fn valid(c: &Civil) -> bool {
    posix::cal::valid(c.year, c.mon, c.day, c.hour, c.min, c.sec)
}

/*
 * -t time: [[CC]YY]MMDDhhmm[.SS]. cur_year is the current year, for the
 * form without one. A two-digit year of 69 to 99 is 19YY, 00 to 68 is 20YY.
 */
fn parse_t(s: &[u8], cur_year: i64) -> Option<Civil> {
    let (main, secs) = match s.iter().position(|&c| c == b'.') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    if !digits(main) {
        return None;
    }
    let sec = match secs {
        None => 0,
        Some(x) if x.len() == 2 && digits(x) => num(x),
        Some(_) => return None,
    };
    let (year, rest) = match main.len() {
        8 => (cur_year, main),
        10 => {
            let yy = num(&main[..2]) as i64;
            (posix::cal::two_digit_year(yy), &main[2..])
        }
        12 => (num(&main[..4]) as i64, &main[4..]),
        _ => return None,
    };
    let c = Civil {
        year,
        mon: num(&rest[0..2]),
        day: num(&rest[2..4]),
        hour: num(&rest[4..6]),
        min: num(&rest[6..8]),
        sec,
        nsec: 0,
    };
    if valid(&c) {
        Some(c)
    } else {
        None
    }
}

/*
 * -d date_time: YYYY-MM-DDThh:mm:SS[.frac|,frac][Z], the T may be a space.
 * The year has four or more digits. Returns the time and whether it is UTC.
 * A fraction beyond nanoseconds is cut off.
 */
fn parse_d(s: &[u8]) -> Option<(Civil, bool)> {
    let ylen = s.iter().position(|&c| c == b'-')?;
    if ylen < 4 || ylen > 9 || !digits(&s[..ylen]) {
        return None;
    }
    let year = s[..ylen].iter().fold(0i64, |a, &c| a * 10 + (c - b'0') as i64);
    let r = &s[ylen..];
    /* -MM-DD?hh:mm:SS */
    if r.len() < 15 || r[0] != b'-' || r[3] != b'-' || (r[6] != b'T' && r[6] != b' ') || r[9] != b':' || r[12] != b':' {
        return None;
    }
    let two = |i: usize| -> Option<u32> {
        if r[i].is_ascii_digit() && r[i + 1].is_ascii_digit() {
            Some(num(&r[i..i + 2]))
        } else {
            None
        }
    };
    let (mon, day, hour, min, sec) = (two(1)?, two(4)?, two(7)?, two(10)?, two(13)?);
    let mut rest = &r[15..];
    let mut nsec = 0u32;
    if let Some(&sep) = rest.first() {
        if sep == b'.' || sep == b',' {
            let n = rest[1..].iter().take_while(|c| c.is_ascii_digit()).count();
            if n == 0 {
                return None;
            }
            let mut scale = 100_000_000u32;
            for &d in rest[1..1 + n].iter().take(9) {
                nsec += (d - b'0') as u32 * scale;
                scale /= 10;
            }
            rest = &rest[1 + n..];
        }
    }
    let utc = match rest {
        [] => false,
        [b'Z'] => true,
        _ => return None,
    };
    let c = Civil { year, mon, day, hour, min, sec, nsec };
    if valid(&c) {
        Some((c, utc))
    } else {
        None
    }
}

/* What the times of each file are to be set to. */
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Source {
    Now,
    At(i64, i64),
    /* access and modification times of the reference file */
    Pair((i64, i64), (i64, i64)),
}

/* The (seconds, nanoseconds) pairs for utimensat: access, then modification. */
fn times_for(a: bool, m: bool, src: Source) -> [(i64, i64); 2] {
    let omit = (0, libc::UTIME_OMIT as i64);
    let now = (0, libc::UTIME_NOW as i64);
    let (at, mt) = match src {
        Source::Now => (now, now),
        Source::At(s, n) => ((s, n), (s, n)),
        Source::Pair(x, y) => (x, y),
    };
    [if a { at } else { omit }, if m { mt } else { omit }]
}

fn to_timespec(t: (i64, i64)) -> libc::timespec {
    libc::timespec { tv_sec: t.0 as libc::time_t, tv_nsec: t.1 as libc::c_long }
}

/* Why a file could not be touched. */
enum Fail {
    /* report it and go on with the next file */
    Soft(std::io::Error),
    /* the time cannot be a file timestamp: report it and stop */
    Range(std::io::Error),
}

fn classify(e: std::io::Error) -> Fail {
    match e.raw_os_error() {
        Some(libc::EINVAL) | Some(libc::EOVERFLOW) => Fail::Range(e),
        _ => Fail::Soft(e),
    }
}

/* Is this an explicit time (not "now", not "leave it")? */
fn is_explicit(t: &(i64, i64)) -> bool {
    t.1 != libc::UTIME_NOW as i64 && t.1 != libc::UTIME_OMIT as i64
}

/* The access and modification times a file has now (symbolic links followed). */
fn stored(path: &CStr) -> Option<[(i64, i64); 2]> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatat(libc::AT_FDCWD, path.as_ptr(), &mut st, 0) } < 0 {
        return None;
    }
    Some([
        (st.st_atime as i64, st.st_atime_nsec as i64),
        (st.st_mtime as i64, st.st_mtime_nsec as i64),
    ])
}

fn stored_fd(fd: libc::c_int) -> Option<[(i64, i64); 2]> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } < 0 {
        return None;
    }
    Some([
        (st.st_atime as i64, st.st_atime_nsec as i64),
        (st.st_mtime as i64, st.st_mtime_nsec as i64),
    ])
}

/*
 * Some file systems do not refuse a time they cannot hold: macOS (APFS)
 * clamps it to about the years 1677 and 2262 and reports success. Whatever
 * a file system keeps is rounded to its granularity (a second, two seconds
 * on FAT), never by more than that, so a stored second more than two away
 * from the requested one means the time was not representable.
 */
fn held(want: &[(i64, i64); 2], got: &[(i64, i64); 2]) -> bool {
    (0..2).all(|i| !is_explicit(&want[i]) || (want[i].0 - got[i].0).abs() <= 2)
}

fn out_of_range() -> Fail {
    Fail::Range(std::io::Error::from_raw_os_error(libc::ERANGE))
}

/*
 * Steps 1 and 2 of the DESCRIPTION. The file is looked up first with
 * utimensat; only when it does not exist is it created, with mode 0666
 * less the umask and no truncation (so a file that appears in between is
 * never emptied), and its times are set through the descriptor.
 */
fn touch_one(path: &CStr, times: &[(i64, i64); 2], create: bool) -> Result<(), Fail> {
    let ts = [to_timespec(times[0]), to_timespec(times[1])];
    let explicit = is_explicit(&times[0]) || is_explicit(&times[1]);
    let before = if explicit { stored(path) } else { None };
    if unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), ts.as_ptr(), 0) } == 0 {
        if explicit {
            if let Some(got) = stored(path) {
                if !held(times, &got) {
                    /* put the old times back, so that the failure leaves the file as it was */
                    if let Some(b) = before {
                        let back = [to_timespec(b[0]), to_timespec(b[1])];
                        unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), back.as_ptr(), 0) };
                    }
                    return Err(out_of_range());
                }
            }
        }
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    if e.raw_os_error() != Some(libc::ENOENT) {
        return Err(classify(e));
    }
    if !create {
        return Ok(());
    }
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_NOCTTY | libc::O_CLOEXEC;
    let fd = unsafe { libc::open(path.as_ptr(), flags, 0o666 as libc::c_uint) };
    if fd < 0 {
        return Err(Fail::Soft(std::io::Error::last_os_error()));
    }
    let r = unsafe { libc::futimens(fd, ts.as_ptr()) };
    let e = std::io::Error::last_os_error();
    let kept = if r == 0 && explicit { stored_fd(fd) } else { None };
    unsafe { libc::close(fd) };
    if r < 0 {
        return Err(classify(e));
    }
    if let Some(got) = kept {
        if !held(times, &got) {
            return Err(out_of_range());
        }
    }
    Ok(())
}

fn current_year() -> i64 {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    tm.tm_year as i64 + 1900
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    R,
    T,
    D,
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    posix::cal::null_tz_is_unset();
    let (mut a, mut m, mut c) = (false, false, false);
    let mut src_opt: Option<(Kind, Vec<u8>)> = None;
    let mut g = Getopt::new(&args, b"acmr:t:d:");
    while let Some(o) = g.next() {
        let (kind, arg) = match o {
            Opt::Flag(b'a') => {
                a = true;
                continue;
            }
            Opt::Flag(b'c') => {
                c = true;
                continue;
            }
            Opt::Flag(b'm') => {
                m = true;
                continue;
            }
            Opt::Arg(b'r', v) => (Kind::R, v),
            Opt::Arg(b't', v) => (Kind::T, v),
            Opt::Arg(b'd', v) => (Kind::D, v),
            _ => posix::usage(SYNOPSIS),
        };
        /* the three sources exclude each other; the same one repeated: the last wins */
        if let Some((k, _)) = &src_opt {
            if *k != kind {
                posix::usage(SYNOPSIS);
            }
        }
        src_opt = Some((kind, arg));
    }
    let operands = g.operands();
    if operands.is_empty() {
        posix::usage(SYNOPSIS);
    }
    if !a && !m {
        a = true;
        m = true;
    }
    let src = match src_opt {
        None => Source::Now,
        Some((Kind::R, r)) => {
            let cr = posix::cstr(&r);
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::stat(cr.as_ptr(), &mut st) } < 0 {
                let e = std::io::Error::last_os_error();
                posix::warn_path(&prog, &r, &posix::strerror(&e));
                std::process::exit(1);
            }
            Source::Pair(
                (st.st_atime as i64, st.st_atime_nsec as i64),
                (st.st_mtime as i64, st.st_mtime_nsec as i64),
            )
        }
        Some((Kind::T, v)) => {
            let Some(civ) = parse_t(&v, current_year()) else {
                posix::warn_path(&prog, &v, "invalid time");
                std::process::exit(2);
            };
            let Some(sec) = local_epoch(&civ) else {
                posix::warn_path(&prog, &v, "time out of range");
                std::process::exit(1);
            };
            Source::At(sec, 0)
        }
        Some((Kind::D, v)) => {
            let Some((civ, utc)) = parse_d(&v) else {
                posix::warn_path(&prog, &v, "invalid date_time");
                std::process::exit(2);
            };
            let sec = if utc { Some(utc_epoch(&civ)) } else { local_epoch(&civ) };
            let Some(sec) = sec else {
                posix::warn_path(&prog, &v, "time out of range");
                std::process::exit(1);
            };
            Source::At(sec, civ.nsec as i64)
        }
    };
    let times = times_for(a, m, src);
    let mut status = 0;
    for f in operands {
        match touch_one(&posix::cstr(f), &times, !c) {
            Ok(()) => {}
            Err(Fail::Soft(e)) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                status = 1;
            }
            Err(Fail::Range(e)) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                std::process::exit(1);
            }
        }
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("touch-test-{}-{}", std::process::id(), n));
            std::fs::create_dir(&p).unwrap();
            Tmp(p)
        }
        fn c(&self, s: &str) -> std::ffi::CString {
            posix::cstr(self.0.join(s).as_os_str().as_bytes())
        }
        fn path(&self, s: &str) -> PathBuf {
            self.0.join(s)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn civ(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Civil {
        Civil { year: y, mon: mo, day: d, hour: h, min: mi, sec: s, nsec: 0 }
    }

    /* the spec's EXAMPLES: touch -t 200711121015 cathy */
    #[test]
    fn example_cathy_t_without_seconds() {
        assert_eq!(parse_t(b"200711121015", 2026), Some(civ(2007, 11, 12, 10, 15, 0)));
    }

    /* touch -t 200711121015.30 drepper */
    #[test]
    fn example_drepper_t_with_seconds() {
        assert_eq!(parse_t(b"200711121015.30", 2026), Some(civ(2007, 11, 12, 10, 15, 30)));
    }

    /* touch -t 0711121015.30 ebb9: a two-digit year of 07 is 2007 */
    #[test]
    fn example_ebb9_two_digit_year() {
        assert_eq!(parse_t(b"0711121015.30", 2026), Some(civ(2007, 11, 12, 10, 15, 30)));
    }

    /* touch -d 2007-11-12T10:15:30 dwc: local time */
    #[test]
    fn example_dwc_d_local() {
        assert_eq!(parse_d(b"2007-11-12T10:15:30"), Some((civ(2007, 11, 12, 10, 15, 30), false)));
    }

    /* touch -d 2007-11-12T10:15:30Z nick: UTC */
    #[test]
    fn example_nick_d_utc() {
        let (c, utc) = parse_d(b"2007-11-12T10:15:30Z").unwrap();
        assert!(utc);
        assert_eq!(utc_epoch(&c), 1194862530);
    }

    /* touch -d 2007-11-12T10:15:30,002 gwc: a comma fraction, local time */
    #[test]
    fn example_gwc_comma_fraction() {
        let (c, utc) = parse_d(b"2007-11-12T10:15:30,002").unwrap();
        assert!(!utc);
        assert_eq!((c.sec, c.nsec), (30, 2_000_000));
    }

    /* touch -d "2007-11-12 10:15:30.002Z" ajosey: a space for the T, a period fraction, UTC */
    #[test]
    fn example_ajosey_space_and_z() {
        let (c, utc) = parse_d(b"2007-11-12 10:15:30.002Z").unwrap();
        assert!(utc);
        assert_eq!((utc_epoch(&c), c.nsec), (1194862530, 2_000_000));
    }

    /* touch -a -r mark eggert: only the access time is taken from the reference */
    #[test]
    fn example_eggert_a_r() {
        let t = times_for(true, false, Source::Pair((10, 1), (20, 2)));
        assert_eq!(t[0], (10, 1));
        assert_eq!(t[1].1, libc::UTIME_OMIT as i64);
    }

    #[test]
    fn t_forms_and_century_rule() {
        assert_eq!(parse_t(b"07121015", 1999), Some(civ(1999, 7, 12, 10, 15, 0)));
        assert_eq!(parse_t(b"6901010000", 0), Some(civ(1969, 1, 1, 0, 0, 0)));
        assert_eq!(parse_t(b"9912312359", 0), Some(civ(1999, 12, 31, 23, 59, 0)));
        assert_eq!(parse_t(b"0001010000", 0), Some(civ(2000, 1, 1, 0, 0, 0)));
        assert_eq!(parse_t(b"6801010000", 0), Some(civ(2068, 1, 1, 0, 0, 0)));
        assert_eq!(parse_t(b"190001010000.00", 0), Some(civ(1900, 1, 1, 0, 0, 0)));
        assert_eq!(parse_t(b"203801010000.00", 0), Some(civ(2038, 1, 1, 0, 0, 0)));
        assert_eq!(parse_t(b"01010000.60", 2001), Some(civ(2001, 1, 1, 0, 0, 60)));
    }

    #[test]
    fn t_rejects() {
        for bad in [
            &b""[..], b".", b"1", b"1234567", b"123456789", b"12345678901", b"1234567890123", b"0101000a",
            b"13010000", b"00010000", b"01320000", b"01002400", b"01010060", b"01010000.61", b"01010000.1",
            b"01010000.", b"01010000.123", b"0101 0000", b"-1010000", b"+1010000", b"02300000", b"02310000",
            b"01010000.6x", b".01010000",
        ] {
            assert_eq!(parse_t(bad, 2001), None, "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn t_day_exists_in_the_month() {
        assert!(parse_t(b"02290000", 2024).is_some());
        assert!(parse_t(b"02290000", 2023).is_none());
        assert!(parse_t(b"190002290000", 0).is_none());
        assert!(parse_t(b"200002290000", 0).is_some());
        assert!(parse_t(b"04310000", 2001).is_none());
        assert!(parse_t(b"04300000", 2001).is_some());
    }

    #[test]
    fn d_forms() {
        assert_eq!(parse_d(b"2011-01-01T00:00:00"), Some((civ(2011, 1, 1, 0, 0, 0), false)));
        assert_eq!(parse_d(b"2038-01-01 00:00:00"), Some((civ(2038, 1, 1, 0, 0, 0), false)));
        assert_eq!(parse_d(b"1970-01-01T00:00:00"), Some((civ(1970, 1, 1, 0, 0, 0), false)));
        assert_eq!(parse_d(b"12345-01-01T00:00:00Z").unwrap().0.year, 12345);
        assert_eq!(parse_d(b"0001-01-01T00:00:00Z").unwrap().0.year, 1);
        assert_eq!(parse_d(b"2007-11-12T10:15:60").unwrap().0.sec, 60);
    }

    #[test]
    fn d_fractions() {
        let nsec = |s: &str| parse_d(s.as_bytes()).unwrap().0.nsec;
        assert_eq!(nsec("2007-11-12T10:15:30.5"), 500_000_000);
        assert_eq!(nsec("2007-11-12T10:15:30,5"), 500_000_000);
        assert_eq!(nsec("2007-11-12T10:15:30.000000001"), 1);
        assert_eq!(nsec("2007-11-12T10:15:30.1234567891"), 123_456_789);
        assert_eq!(nsec("2007-11-12T10:15:30.99999999999999"), 999_999_999);
        assert_eq!(nsec("2007-11-12T10:15:30.0Z"), 0);
        assert_eq!(nsec("2007-11-12T10:15:30"), 0);
    }

    #[test]
    fn d_rejects() {
        for bad in [
            "", "2007", "2007-11-12", "2007-11-12T", "2007-11-12T10:15", "2007-11-12T10:15:3", "07-11-12T10:15:30",
            "207-11-12T10:15:30", "2007-1-12T10:15:30", "2007-11-12t10:15:30", "2007-11-12  10:15:30",
            "2007-11-12T10:15:30.", "2007-11-12T10:15:30,", "2007-11-12T10:15:30z", "2007-11-12T10:15:30 Z",
            "2007-11-12T10:15:30+01:00", "2007-11-12T10:15:30Zx", "2007-13-12T10:15:30", "2007-11-32T10:15:30",
            "2007-02-30T10:15:30", "2007-11-12T24:15:30", "2007-11-12T10:60:30", "2007-11-12T10:15:61",
            "+2007-11-12T10:15:30", "-2007-11-12T10:15:30", "2007-11-12T10:15:30.5.5", "2007-11-12T10:15:30.5Z.",
            "2007/11/12T10:15:30", "2007-11-12T10-15-30", "20x7-11-12T10:15:30", "2007-11-12T1a:15:30",
            "2007-11-12T10:15:30\n",
        ] {
            assert!(parse_d(bad.as_bytes()).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn utc_epoch_through_the_wrapper() {
        assert_eq!(utc_epoch(&civ(2007, 11, 12, 10, 15, 30)), 1194862530);
        assert_eq!(utc_epoch(&civ(1998, 12, 31, 23, 59, 60)), utc_epoch(&civ(1999, 1, 1, 0, 0, 0)));
    }

    #[test]
    fn times_selection() {
        let omit = libc::UTIME_OMIT as i64;
        let now = libc::UTIME_NOW as i64;
        assert_eq!(times_for(true, true, Source::Now), [(0, now), (0, now)]);
        assert_eq!(times_for(true, false, Source::Now), [(0, now), (0, omit)]);
        assert_eq!(times_for(false, true, Source::Now), [(0, omit), (0, now)]);
        assert_eq!(times_for(true, true, Source::At(5, 7)), [(5, 7), (5, 7)]);
        assert_eq!(times_for(false, true, Source::At(5, 7)), [(0, omit), (5, 7)]);
        assert_eq!(times_for(true, true, Source::Pair((1, 2), (3, 4))), [(1, 2), (3, 4)]);
        assert_eq!(times_for(false, true, Source::Pair((1, 2), (3, 4))), [(0, omit), (3, 4)]);
    }

    fn meta_times(p: &PathBuf) -> ((i64, i64), (i64, i64)) {
        let m = std::fs::metadata(p).unwrap();
        ((m.atime(), m.atime_nsec()), (m.mtime(), m.mtime_nsec()))
    }

    #[test]
    fn creates_a_missing_file_empty_with_the_requested_times() {
        let t = Tmp::new();
        let times = times_for(true, true, Source::At(1194862530, 123_000_000));
        assert!(touch_one(&t.c("new"), &times, true).is_ok());
        let m = std::fs::metadata(t.path("new")).unwrap();
        assert!(m.is_file() && m.len() == 0);
        assert_eq!(meta_times(&t.path("new")), ((1194862530, 123_000_000), (1194862530, 123_000_000)));
        /* mode is 0666 less the umask */
        let um = unsafe {
            let u = libc::umask(0);
            libc::umask(u);
            u as u32
        };
        assert_eq!(m.mode() & 0o777, 0o666 & !um);
    }

    #[test]
    fn dash_c_creates_nothing_and_is_not_an_error() {
        let t = Tmp::new();
        let times = times_for(true, true, Source::Now);
        assert!(touch_one(&t.c("absent"), &times, false).is_ok());
        assert!(!t.path("absent").exists());
        assert!(touch_one(&t.c("nodir/absent"), &times, false).is_ok());
    }

    #[test]
    fn existing_file_is_not_truncated_and_a_or_m_leave_the_other_alone() {
        let t = Tmp::new();
        std::fs::write(t.path("f"), b"data").unwrap();
        let first = times_for(true, true, Source::At(1000, 0));
        assert!(touch_one(&t.c("f"), &first, true).is_ok());
        assert_eq!(meta_times(&t.path("f")), ((1000, 0), (1000, 0)));
        assert!(touch_one(&t.c("f"), &times_for(true, false, Source::At(2000, 5)), true).is_ok());
        assert_eq!(meta_times(&t.path("f")), ((2000, 5), (1000, 0)));
        assert!(touch_one(&t.c("f"), &times_for(false, true, Source::At(3000, 6)), true).is_ok());
        assert_eq!(meta_times(&t.path("f")), ((2000, 5), (3000, 6)));
        assert_eq!(std::fs::read(t.path("f")).unwrap(), b"data");
    }

    #[test]
    fn reference_pair_copies_each_time() {
        let t = Tmp::new();
        std::fs::write(t.path("ref"), b"").unwrap();
        assert!(touch_one(&t.c("ref"), &[(4000, 11), (5000, 22)], true).is_ok());
        let (a, m) = meta_times(&t.path("ref"));
        std::fs::write(t.path("g"), b"").unwrap();
        assert!(touch_one(&t.c("g"), &times_for(true, true, Source::Pair(a, m)), true).is_ok());
        assert_eq!(meta_times(&t.path("g")), ((4000, 11), (5000, 22)));
    }

    #[test]
    fn times_before_the_epoch_are_accepted_when_the_file_system_can_hold_them() {
        let t = Tmp::new();
        std::fs::write(t.path("old"), b"").unwrap();
        let times = times_for(true, true, Source::At(-2208988800, 500_000_000));
        match touch_one(&t.c("old"), &times, true) {
            Ok(()) => assert_eq!(meta_times(&t.path("old")).1, (-2208988800, 500_000_000)),
            Err(Fail::Range(_)) => {}
            Err(Fail::Soft(e)) => panic!("{e}"),
        }
    }

    #[test]
    fn a_missing_directory_is_an_error_when_creating() {
        let t = Tmp::new();
        let times = times_for(true, true, Source::Now);
        assert!(matches!(touch_one(&t.c("nodir/f"), &times, true), Err(Fail::Soft(_))));
        std::fs::write(t.path("plain"), b"").unwrap();
        assert!(matches!(touch_one(&t.c("plain/f"), &times, true), Err(Fail::Soft(_))));
        /* ... and with -c: ENOENT is silent, ENOTDIR is not */
        assert!(touch_one(&t.c("nodir/f"), &times, false).is_ok());
        assert!(matches!(touch_one(&t.c("plain/f"), &times, false), Err(Fail::Soft(_))));
    }

    #[test]
    fn a_time_the_file_system_cannot_hold_stops_the_run() {
        let t = Tmp::new();
        std::fs::write(t.path("f"), b"").unwrap();
        let times = times_for(true, true, Source::At(i64::MAX / 2, 0));
        assert!(matches!(touch_one(&t.c("f"), &times, true), Err(Fail::Range(_))));
        /* the same through the descriptor of a file being created */
        assert!(matches!(touch_one(&t.c("g"), &times, true), Err(Fail::Range(_))));
    }

    #[test]
    fn a_clamped_time_is_refused_and_the_old_times_come_back() {
        let t = Tmp::new();
        std::fs::write(t.path("f"), b"").unwrap();
        assert!(touch_one(&t.c("f"), &times_for(true, true, Source::At(1000, 0)), true).is_ok());
        /* the year 9999: APFS would clamp it to 2262 and say yes */
        let far = times_for(true, true, Source::At(253402300799, 0));
        assert!(matches!(touch_one(&t.c("f"), &far, true), Err(Fail::Range(_))));
        assert_eq!(meta_times(&t.path("f")), ((1000, 0), (1000, 0)));
        /* a time that is held, however far, is fine: 2038 and the year 2200 */
        for s in [2145916800i64, 7258118400] {
            assert!(touch_one(&t.c("f"), &times_for(true, true, Source::At(s, 0)), true).is_ok(), "{s}");
            assert_eq!(meta_times(&t.path("f")).1 .0, s);
        }
    }

    #[test]
    fn held_tolerance() {
        let w = [(100, 0), (100, 0)];
        assert!(held(&w, &[(100, 0), (100, 0)]));
        assert!(held(&w, &[(98, 0), (102, 5)]));
        assert!(!held(&w, &[(97, 0), (100, 0)]));
        assert!(!held(&w, &[(100, 0), (103, 0)]));
        /* fields that are not explicit are not compared */
        let part = [(0, libc::UTIME_OMIT as i64), (100, 0)];
        assert!(held(&part, &[(5000, 0), (100, 0)]));
        assert!(held(&[(0, libc::UTIME_NOW as i64); 2], &[(5000, 0), (7, 0)]));
    }

    #[test]
    fn dangling_link_creates_its_target() {
        let t = Tmp::new();
        std::os::unix::fs::symlink("target", t.path("lnk")).unwrap();
        let times = times_for(true, true, Source::Now);
        assert!(touch_one(&t.c("lnk"), &times, true).is_ok());
        assert!(t.path("target").is_file());
        std::fs::remove_file(t.path("target")).unwrap();
        assert!(touch_one(&t.c("lnk"), &times, false).is_ok());
        assert!(!t.path("target").exists());
    }

    #[test]
    fn the_current_year_is_sane() {
        let y = current_year();
        assert!((2024..2200).contains(&y));
    }
}
