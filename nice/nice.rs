/* SPDX-License-Identifier: GPL-2.0-only */
/* nice - invoke a utility with an altered nice value. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};

const SYNOPSIS: &str = "nice [-n increment] utility [argument...]";

/* A decimal integer with an optional sign; huge values saturate. */
fn parse_increment(s: &[u8]) -> Option<i32> {
    let (neg, digits) = match s.first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() || !digits.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut v: i64 = 0;
    for &c in digits {
        v = (v * 10 + (c - b'0') as i64).min(i32::MAX as i64);
    }
    Some(if neg { -v } else { v } as i32)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut inc = 10;
    let mut g = Getopt::new(&args, b"n:");
    for o in g.by_ref() {
        match o {
            Opt::Arg(b'n', v) => match parse_increment(&v) {
                Some(n) => inc = n,
                None => posix::usage(SYNOPSIS),
            },
            _ => posix::usage(SYNOPSIS),
        }
    }
    let ops = g.operands();
    if ops.is_empty() {
        posix::usage(SYNOPSIS);
    }
    /* nice(3) can return -1 as a valid value, so errno tells the failure */
    posix::clear_errno();
    let r = posix::sys::nice(inc);
    if r == -1 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error().unwrap_or(0) != 0 {
            /* not privileged: warn, and run the utility regardless */
            posix::warn(&prog, &format!("cannot set niceness: {}", posix::strerror(&e)));
        }
    }
    posix::exec_utility(&prog, ops, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increments() {
        assert_eq!(parse_increment(b"5"), Some(5));
        assert_eq!(parse_increment(b"+5"), Some(5));
        assert_eq!(parse_increment(b"-5"), Some(-5));
        assert_eq!(parse_increment(b"0"), Some(0));
        assert_eq!(parse_increment(b"99999999999999999999"), Some(i32::MAX));
        assert_eq!(parse_increment(b"-99999999999999999999"), Some(-i32::MAX));
        for s in ["", "-", "+", "1.5", "x", "1x", " 1"] {
            assert_eq!(parse_increment(s.as_bytes()), None, "{s}");
        }
    }

    #[test]
    fn increment_edges() {
        assert_eq!(parse_increment(b"-0"), Some(0));
        assert_eq!(parse_increment(b"+0"), Some(0));
        assert_eq!(parse_increment(b"007"), Some(7));
        assert_eq!(parse_increment(b"2147483647"), Some(i32::MAX));
        assert_eq!(parse_increment(b"2147483648"), Some(i32::MAX));
        assert_eq!(parse_increment(b"-2147483648"), Some(-i32::MAX));
        for s in ["--5", "+-5", "5 ", "0x5", "\u{663}", "1e1"] {
            assert_eq!(parse_increment(s.as_bytes()), None, "{s}");
        }
        assert_eq!(parse_increment(b"\xff"), None);
    }
}
