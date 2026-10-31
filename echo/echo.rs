/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * echo - write arguments to standard output (XSI behavior).
 *
 * No options are recognized, not even "--" and "-n": they are strings. The
 * XSI backslash sequences are interpreted inside every operand.
 */

/*
 * Expand the operands. Returns the output bytes and whether the trailing
 * newline is suppressed (by "\c").
 */
fn expand(operands: &[Vec<u8>]) -> (Vec<u8>, bool) {
    let mut out = Vec::new();
    for (n, arg) in operands.iter().enumerate() {
        if n > 0 {
            out.push(b' ');
        }
        let mut i = 0;
        while i < arg.len() {
            let c = arg[i];
            i += 1;
            if c != b'\\' || i == arg.len() {
                out.push(c);
                continue;
            }
            let e = arg[i];
            i += 1;
            match e {
                b'a' => out.push(0x07),
                b'b' => out.push(0x08),
                b'c' => return (out, true),
                b'f' => out.push(0x0c),
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b't' => out.push(b'\t'),
                b'v' => out.push(0x0b),
                b'\\' => out.push(b'\\'),
                b'0' => {
                    /* \0num: zero, one, two or three octal digits */
                    let mut v: u32 = 0;
                    let mut k = 0;
                    while k < 3 && i < arg.len() && (b'0'..=b'7').contains(&arg[i]) {
                        v = v * 8 + (arg[i] - b'0') as u32;
                        i += 1;
                        k += 1;
                    }
                    out.push((v & 0xff) as u8);
                }
                _ => {
                    out.push(b'\\');
                    out.push(e);
                }
            }
        }
    }
    (out, false)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let (mut out, nonl) = expand(&args[1..]);
    if !nonl {
        out.push(b'\n');
    }
    posix::write_stdout(&prog, &out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(a: &[&str]) -> (Vec<u8>, bool) {
        let v: Vec<Vec<u8>> = a.iter().map(|s| s.as_bytes().to_vec()).collect();
        expand(&v)
    }

    #[test]
    fn plain_and_joined() {
        assert_eq!(e(&[]), (vec![], false));
        assert_eq!(e(&["a", "b c"]), (b"a b c".to_vec(), false));
        assert_eq!(e(&["-n", "--"]), (b"-n --".to_vec(), false));
    }

    #[test]
    fn escapes() {
        assert_eq!(e(&["a\\tb\\n"]).0, b"a\tb\n");
        assert_eq!(e(&["\\a\\b\\f\\r\\v\\\\"]).0, b"\x07\x08\x0c\r\x0b\\");
        assert_eq!(e(&["\\0101"]).0, b"A");
        assert_eq!(e(&["\\0"]).0, b"\0");
        assert_eq!(e(&["\\08"]).0, b"\x008");
        assert_eq!(e(&["\\q", "x\\"]), (b"\\q x\\".to_vec(), false));
    }

    #[test]
    fn backslash_c_stops_everything() {
        assert_eq!(e(&["ab\\cd", "ef"]), (b"ab".to_vec(), true));
    }
}
