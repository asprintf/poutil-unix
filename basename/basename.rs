/* SPDX-License-Identifier: GPL-2.0-only */
/* basename - return the non-directory portion of a pathname. */

const SYNOPSIS: &str = "basename string [suffix]";

/*
 * The steps of the specification, in order. A null string gives a null
 * string (the spec leaves "." or null open); "//" is processed like any
 * string of slashes.
 */
fn basename(s: &[u8], suffix: Option<&[u8]>) -> Vec<u8> {
    if s.is_empty() {
        return Vec::new();
    }
    if s.iter().all(|&c| c == b'/') {
        return b"/".to_vec();
    }
    let mut end = s.len();
    while s[end - 1] == b'/' {
        end -= 1;
    }
    let s = &s[..end];
    let s = match s.iter().rposition(|&c| c == b'/') {
        Some(i) => &s[i + 1..],
        None => s,
    };
    match suffix {
        Some(x) if x != s && s.ends_with(x) => s[..s.len() - x.len()].to_vec(),
        _ => s.to_vec(),
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let ops = posix::no_options(&args, SYNOPSIS);
    if ops.is_empty() || ops.len() > 2 {
        posix::usage(SYNOPSIS);
    }
    let mut out = basename(&ops[0], ops.get(1).map(|v| v.as_slice()));
    out.push(b'\n');
    posix::write_stdout(&prog, &out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(s: &str, x: Option<&str>) -> String {
        String::from_utf8(basename(s.as_bytes(), x.map(|v| v.as_bytes()))).unwrap()
    }

    #[test]
    fn components() {
        assert_eq!(b("/usr/bin/foo", None), "foo");
        assert_eq!(b("foo", None), "foo");
        assert_eq!(b("/usr/bin/", None), "bin");
        assert_eq!(b("a//b//", None), "b");
    }

    #[test]
    fn slashes_and_null() {
        assert_eq!(b("/", None), "/");
        assert_eq!(b("///", None), "/");
        assert_eq!(b("//", None), "/");
        assert_eq!(b("", None), "");
    }

    #[test]
    fn suffix() {
        assert_eq!(b("/a/b.c", Some(".c")), "b");
        assert_eq!(b("b.c", Some("b.c")), "b.c");
        assert_eq!(b("b.c", Some(".d")), "b.c");
        assert_eq!(b("b.c/", Some(".c")), "b");
        assert_eq!(b("b.c", Some("")), "b.c");
    }
}
