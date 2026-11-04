/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * File mode operands as defined for chmod (XCU chmod, EXTENDED DESCRIPTION):
 * an octal number or a symbolic_mode. Also used by mkdir -m and mkfifo -m.
 */

const USER: u32 = 0o4700;
const GROUP: u32 = 0o2070;
const OTHER: u32 = 0o0007;
const STICKY: u32 = 0o1000;

/*
 * Apply the mode operand s to the current mode bits cur and return the new
 * ones, or None if s is not a valid mode. is_dir matters only for the "X"
 * perm and umask only for clauses without a wholist.
 */
pub fn parse(s: &[u8], cur: u32, is_dir: bool, umask: u32) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    if s.iter().all(|c| (b'0'..=b'7').contains(c)) {
        let mut v: u32 = 0;
        for &c in s {
            v = v * 8 + (c - b'0') as u32;
            if v > 0o7777 {
                return None;
            }
        }
        return Some(v);
    }
    let mut mode = cur & 0o7777;
    for clause in s.split(|&c| c == b',') {
        mode = apply_clause(clause, mode, cur, is_dir, umask & 0o777)?;
    }
    Some(mode)
}

fn apply_clause(clause: &[u8], mut mode: u32, orig: u32, is_dir: bool, umask: u32) -> Option<u32> {
    let mut i = 0;
    let mut who: u32 = 0;
    let mut who_given = false;
    while i < clause.len() {
        match clause[i] {
            b'u' => who |= USER,
            b'g' => who |= GROUP,
            b'o' => who |= OTHER,
            b'a' => who |= USER | GROUP | OTHER,
            _ => break,
        }
        who_given = true;
        i += 1;
    }
    if i >= clause.len() {
        return None;
    }
    /* sticky only goes with all three classes (a, or no wholist) */
    let all = USER | GROUP | OTHER;
    let wmask = if !who_given || who & all == all { all | STICKY } else { who };
    while i < clause.len() {
        let op = clause[i];
        if !matches!(op, b'+' | b'-' | b'=') {
            return None;
        }
        i += 1;
        let mut bits: u32 = 0;
        /* a perm symbol was read: the bits may still be none (s with o, X on a file without x) */
        let mut seen = false;
        while i < clause.len() && !matches!(clause[i], b'+' | b'-' | b'=') {
            match clause[i] {
                b'r' => bits |= 0o444,
                b'w' => bits |= 0o222,
                b'x' => bits |= 0o111,
                b'X' => {
                    if is_dir || orig & 0o111 != 0 {
                        bits |= 0o111
                    }
                }
                b's' => {
                    if !who_given || who & USER != 0 {
                        bits |= 0o4000
                    }
                    if !who_given || who & GROUP != 0 {
                        bits |= 0o2000
                    }
                }
                b't' => bits |= STICKY,
                c @ (b'u' | b'g' | b'o') => {
                    /* permcopy is a single symbol and must follow the op directly */
                    if seen || (i + 1 < clause.len() && !matches!(clause[i + 1], b'+' | b'-' | b'=')) {
                        return None;
                    }
                    let shift = match c {
                        b'u' => 6,
                        b'g' => 3,
                        _ => 0,
                    };
                    bits = ((mode >> shift) & 7) * 0o111;
                }
                _ => return None,
            }
            seen = true;
            i += 1;
        }
        bits &= wmask;
        if !who_given {
            bits &= !umask;
        }
        match op {
            b'+' => mode |= bits,
            b'-' => mode &= !bits,
            _ => {
                mode &= !(if who_given { wmask } else { 0o7777 });
                mode |= bits;
            }
        }
    }
    Some(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str, cur: u32) -> Option<u32> {
        parse(s.as_bytes(), cur, false, 0o022)
    }

    #[test]
    fn a_permcopy_stands_alone_even_when_the_perm_before_it_adds_nothing() {
        /* s with who o changes nothing, X on a file without x adds nothing: still perms before the u */
        for m in ["o+su", "o+sg", "u+Xg", "g+Xo", "o=su", "o-sg", "u+Xu", "o+sXu", "u+xu", "u+tu", "u+ug", "u+uu"] {
            assert_eq!(p(m, 0o644), None, "{m}");
            assert_eq!(parse(m.as_bytes(), 0o755, true, 0o022), None, "{m} on a directory");
        }
        /* a permcopy after an op of its own, or alone, is fine */
        assert_eq!(p("o+s,g=u", 0o640), Some(0o660));
        assert_eq!(p("u+X-g", 0o640), Some(0o240));
        assert_eq!(p("u+Xx=g", 0o640), Some(0o440));
        assert_eq!(p("o+s+u", 0o640), Some(0o646));
    }

    #[test]
    fn octal() {
        assert_eq!(p("755", 0), Some(0o755));
        assert_eq!(p("0", 0o777), Some(0));
        assert_eq!(p("4755", 0), Some(0o4755));
        assert_eq!(p("10000", 0), None);
        assert_eq!(p("789", 0), None);
    }

    #[test]
    fn symbolic_basic() {
        assert_eq!(p("u=rwx,go=rx", 0), Some(0o755));
        assert_eq!(p("a+x", 0o600), Some(0o711));
        assert_eq!(p("go-w", 0o777), Some(0o755));
        assert_eq!(p("a=r", 0o777), Some(0o444));
        assert_eq!(p("u+r-w", 0o200), Some(0o400));
        assert_eq!(p("+", 0o640), Some(0o640));
        assert_eq!(p("u=", 0o777), Some(0o077));
    }

    #[test]
    fn umask_applies_without_who() {
        assert_eq!(p("+w", 0o400), Some(0o600));
        assert_eq!(p("=rwx", 0), Some(0o755));
        assert_eq!(p("-r", 0o777), Some(0o333));
        assert_eq!(p("a+w", 0), Some(0o222));
    }

    #[test]
    fn copy_and_special() {
        assert_eq!(p("g=u", 0o700), Some(0o770));
        assert_eq!(p("o=g", 0o750), Some(0o755));
        assert_eq!(p("u+s", 0o700), Some(0o4700));
        assert_eq!(p("g+s", 0o700), Some(0o2700));
        assert_eq!(p("o+s", 0o700), Some(0o700));
        assert_eq!(p("a+t", 0o700), Some(0o1700));
        assert_eq!(p("a+X", 0o600), Some(0o600));
        assert_eq!(p("a+X", 0o700), Some(0o711));
        assert_eq!(parse(b"a+X", 0o600, true, 0), Some(0o711));
    }

    #[test]
    fn invalid() {
        for s in ["", "u", "ug", "u+z", "u+x,", ",u+x", "u+xu", "+rg", "u+gx", "8"] {
            assert_eq!(p(s, 0), None, "{s}");
        }
    }

    #[test]
    fn octal_edges() {
        assert_eq!(p("0000", 0o777), Some(0));
        assert_eq!(p("7777", 0), Some(0o7777));
        assert_eq!(p("00000007777", 0), Some(0o7777));
        assert_eq!(p("17777", 0), None);
        assert_eq!(p("777777777777", 0), None);
    }

    #[test]
    fn symbolic_from_the_mkdir_cases() {
        assert_eq!(p("u=rwx,g=rx,o=", 0), Some(0o750));
        assert_eq!(p("u=rwx,go=", 0o777), Some(0o700));
        assert_eq!(p("a=rx,u+w", 0), Some(0o755));
        assert_eq!(p("u=rwx,g=u-w", 0), Some(0o750));
        assert_eq!(p("u+s,g+s", 0), Some(0o6000));
        assert_eq!(p("o+rwx", 0o700), Some(0o707));
        assert_eq!(p("a-w", 0o777), Some(0o555));
        assert_eq!(p("g+w", 0o755), Some(0o775));
        assert_eq!(p("a+x,u-w", 0o666), Some(0o577));
        assert_eq!(p("a=", 0o777), Some(0));
        assert_eq!(p("-", 0o640), Some(0o640));
    }

    #[test]
    fn without_who_the_umask_is_honoured() {
        assert_eq!(p("+x", 0), Some(0o111));
        assert_eq!(p("=r", 0o777), Some(0o444));
        assert_eq!(parse(b"+w", 0, false, 0o077), Some(0o200));
        assert_eq!(parse(b"=rwx", 0, false, 0o077), Some(0o700));
    }

    #[test]
    fn special_bits_and_file_type_bits() {
        assert_eq!(p("a+x", 0o4600), Some(0o4711));
        assert_eq!(p("u-s", 0o4755), Some(0o755));
        assert_eq!(p("a-t", 0o1777), Some(0o777));
        assert_eq!(p("a+x", 0o170600), Some(0o711));
    }

    #[test]
    fn more_invalid() {
        for s in ["rwx", " 755", "755 ", "7 5", "0x10", "u=rwx,,g=r", "u=rwxg", "a", "=rwx,", "\u{e9}"] {
            assert_eq!(p(s, 0), None, "{s}");
        }
        assert_eq!(parse(b"\xff", 0, false, 0), None);
    }
}
