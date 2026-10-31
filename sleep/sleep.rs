/* SPDX-License-Identifier: GPL-2.0-only */
/* sleep - suspend execution for an interval. */

const SYNOPSIS: &str = "sleep time";

/* A non-negative decimal integer; very large values saturate. */
fn parse(s: &[u8]) -> Option<u64> {
    if s.is_empty() || !s.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut v: u64 = 0;
    for &c in s {
        v = v.saturating_mul(10).saturating_add((c - b'0') as u64);
    }
    Some(v)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let ops = posix::no_options(&args, SYNOPSIS);
    if ops.len() != 1 {
        posix::usage(SYNOPSIS);
    }
    let Some(mut rem) = parse(&ops[0]) else {
        posix::fail(&prog, "time must be a non-negative decimal integer");
    };
    /* sleep(3) returns early only if a handled signal arrives; keep going */
    while rem > 0 {
        let chunk = rem.min(u32::MAX as u64) as u32;
        let left = unsafe { libc::sleep(chunk) } as u64;
        rem = rem - chunk as u64 + left;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        assert_eq!(parse(b"0"), Some(0));
        assert_eq!(parse(b"007"), Some(7));
        assert_eq!(parse(b"99999999999999999999999"), Some(u64::MAX));
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"-1"), None);
        assert_eq!(parse(b"1.5"), None);
        assert_eq!(parse(b"1s"), None);
    }

    #[test]
    fn parsing_edges() {
        assert_eq!(parse(b"00"), Some(0));
        assert_eq!(parse(b"0000000000000000000000000001"), Some(1));
        assert_eq!(parse(b"18446744073709551615"), Some(u64::MAX));
        assert_eq!(parse(b"18446744073709551616"), Some(u64::MAX));
        for s in [" 1", "+1", "0x10", "1e3", "1 ", "1\n", "\u{663}"] {
            assert_eq!(parse(s.as_bytes()), None, "{s}");
        }
        assert_eq!(parse(b"0\0"), None);
        assert_eq!(parse(b"\xff"), None);
    }
}
