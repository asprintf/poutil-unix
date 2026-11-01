/* SPDX-License-Identifier: GPL-2.0-only */
/* rmdir - remove directories. */

use posix::getopt::{Getopt, Opt};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

const SYNOPSIS: &str = "rmdir [-p] dir...";

/*
 * Drop trailing slashes. A pathname that begins with exactly two slashes may
 * have implementation-defined semantics, so "//" is kept as it is and not
 * folded into "/". Three or more slashes are the root.
 */
fn trim_slashes(p: &[u8]) -> &[u8] {
    if p == b"//" {
        return p;
    }
    let mut n = p.len();
    while n > 1 && p[n - 1] == b'/' {
        n -= 1;
    }
    &p[..n]
}

/*
 * The directories "rmdir -p" removes for an operand, in order: the operand
 * itself, then each parent in turn ("rmdir -p $(dirname dir)"), up to but
 * not including "." and "/" (however many slashes the root is spelled with).
 */
fn chain(path: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut p = trim_slashes(path);
    loop {
        out.push(p.to_vec());
        let Some(i) = p.iter().rposition(|&c| c == b'/') else { break };
        let parent = trim_slashes(&p[..i]);
        if parent.iter().all(|&c| c == b'/') {
            break;
        }
        p = parent;
    }
    out
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut parents = false;
    let mut g = Getopt::new(&args, b"p");
    while let Some(o) = g.next() {
        match o {
            Opt::Flag(b'p') => parents = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    if g.operands().is_empty() {
        posix::usage(SYNOPSIS);
    }
    let mut status = 0;
    for dir in g.operands() {
        let targets = if parents { chain(dir) } else { vec![dir.clone()] };
        for t in targets {
            if let Err(e) = std::fs::remove_dir(OsStr::from_bytes(&t)) {
                posix::warn_path(&prog, &t, &posix::strerror(&e));
                status = 1;
                break;
            }
        }
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(p: &str) -> Vec<String> {
        chain(p.as_bytes()).iter().map(|v| String::from_utf8(v.clone()).unwrap()).collect()
    }

    #[test]
    fn parent_chain() {
        assert_eq!(c("a/b/c"), ["a/b/c", "a/b", "a"]);
        assert_eq!(c("a"), ["a"]);
        assert_eq!(c("a/"), ["a"]);
        assert_eq!(c("a//b//"), ["a//b", "a"]);
        assert_eq!(c("/a/b"), ["/a/b", "/a"]);
        assert_eq!(c("/a"), ["/a"]);
        assert_eq!(c("./a"), ["./a", "."]);
    }

    #[test]
    fn chain_edges() {
        assert_eq!(c("a//b//c"), ["a//b//c", "a//b", "a"]);
        assert_eq!(c("/"), ["/"]);
        assert_eq!(c("///"), ["/"]);
        assert_eq!(c("//"), ["//"]);
        assert_eq!(c("////"), ["/"]);
        assert_eq!(c("."), ["."]);
        assert_eq!(c("a/."), ["a/.", "a"]);
        assert_eq!(c("a/b/.."), ["a/b/..", "a/b", "a"]);
        assert_eq!(c("../a"), ["../a", ".."]);
    }

    #[test]
    fn chain_never_reaches_the_root() {
        assert_eq!(c("//a"), ["//a"]);
        assert_eq!(c("///a"), ["///a"]);
        assert_eq!(c("//a/b"), ["//a/b", "//a"]);
        assert_eq!(c("///a///b///"), ["///a///b", "///a"]);
    }
}
