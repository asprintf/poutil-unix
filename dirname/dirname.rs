/* SPDX-License-Identifier: GPL-2.0-only */
/* dirname - return the directory portion of a pathname. */

const SYNOPSIS: &str = "dirname string";

/*
 * The steps of the specification, in order. A result of "//" is taken to
 * continue through steps 7 and 8, giving "/".
 */
fn dirname(s: &[u8]) -> Vec<u8> {
    let mut v = s.to_vec();
    if v != b"//" {
        if !v.is_empty() && v.iter().all(|&c| c == b'/') {
            return b"/".to_vec();
        }
        while v.last() == Some(&b'/') {
            v.pop();
        }
        if !v.contains(&b'/') {
            return b".".to_vec();
        }
        while v.last().is_some_and(|&c| c != b'/') {
            v.pop();
        }
    }
    while v.last() == Some(&b'/') {
        v.pop();
    }
    if v.is_empty() {
        v.push(b'/');
    }
    v
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let ops = posix::no_options(&args, SYNOPSIS);
    if ops.len() != 1 {
        posix::usage(SYNOPSIS);
    }
    let mut out = dirname(&ops[0]);
    out.push(b'\n');
    posix::write_stdout(&prog, &out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> String {
        String::from_utf8(dirname(s.as_bytes())).unwrap()
    }

    #[test]
    fn spec_table() {
        assert_eq!(d("/usr/lib"), "/usr");
        assert_eq!(d("/usr/"), "/");
        assert_eq!(d("usr"), ".");
        assert_eq!(d("/"), "/");
        assert_eq!(d("."), ".");
        assert_eq!(d(".."), ".");
    }

    #[test]
    fn odd_inputs() {
        assert_eq!(d(""), ".");
        assert_eq!(d("//"), "/");
        assert_eq!(d("///"), "/");
        assert_eq!(d("a//b"), "a");
        assert_eq!(d("a/b//"), "a");
        assert_eq!(d("//a"), "/");
        assert_eq!(d("/a"), "/");
        assert_eq!(d("a/b/c"), "a/b");
    }

    #[test]
    fn trailing_and_repeated_slashes() {
        assert_eq!(d("///a///b///"), "///a");
        assert_eq!(d("a/"), ".");
        assert_eq!(d("./"), ".");
        assert_eq!(d("/a/"), "/");
        assert_eq!(d("//a//"), "/");
        assert_eq!(d("/a/b/"), "/a");
        assert_eq!(d("a/."), "a");
        assert_eq!(d("../x"), "..");
    }

    #[test]
    fn spaces_and_bytes() {
        assert_eq!(d("a b/c d"), "a b");
        assert_eq!(dirname(b"\xff/x"), b"\xff");
        assert_eq!(dirname(b"\xff"), b".");
    }
}
