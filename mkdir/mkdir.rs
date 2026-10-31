/* SPDX-License-Identifier: GPL-2.0-only */
/* mkdir - make directories. */

use posix::getopt::{Getopt, Opt};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

const SYNOPSIS: &str = "mkdir [-p] [-m mode] dir...";

/*
 * The proper prefixes of path that "mkdir -p" has to make sure exist, each
 * ending where a component ends: "/a/b/c" gives "/a", "/a/b".
 */
fn prefixes(path: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < path.len() {
        while i < path.len() && path[i] == b'/' {
            i += 1;
        }
        let start = i;
        while i < path.len() && path[i] != b'/' {
            i += 1;
        }
        if i > start {
            let rest_has_component = path[i..].iter().any(|&c| c != b'/');
            if rest_has_component {
                out.push(&path[..i]);
            }
        }
    }
    out
}

fn is_dir(p: &[u8]) -> bool {
    std::fs::metadata(OsStr::from_bytes(p)).map(|m| m.is_dir()).unwrap_or(false)
}

fn make(path: &[u8], mode: u32) -> std::io::Result<()> {
    if unsafe { libc::mkdir(posix::cstr(path).as_ptr(), mode as libc::mode_t) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn chmod(path: &[u8], mode: u32) -> std::io::Result<()> {
    if unsafe { libc::chmod(posix::cstr(path).as_ptr(), mode as libc::mode_t) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut parents = false;
    let mut mode_arg: Option<Vec<u8>> = None;
    let mut g = Getopt::new(&args, b"pm:");
    while let Some(o) = g.next() {
        match o {
            Opt::Flag(b'p') => parents = true,
            Opt::Arg(b'm', v) => mode_arg = Some(v),
            _ => posix::usage(SYNOPSIS),
        }
    }
    if g.operands().is_empty() {
        posix::usage(SYNOPSIS);
    }
    let mask = unsafe {
        let m = libc::umask(0);
        libc::umask(m);
        m as u32
    };
    /* symbolic modes are relative to a=rwx */
    let mode = match &mode_arg {
        Some(m) => match posix::mode::parse(m, 0o777, true, mask) {
            Some(v) => Some(v),
            None => posix::fail(&prog, "invalid mode"),
        },
        None => None,
    };
    let mut status = 0;
    for dir in g.operands() {
        let mut result = Ok(());
        let mut failed_at: &[u8] = dir;
        if parents && is_dir(dir) {
            continue;
        }
        if parents {
            for pre in prefixes(dir) {
                if is_dir(pre) {
                    continue;
                }
                /* made with no permissions, then opened up for ourselves */
                let r = make(pre, 0).and_then(|_| chmod(pre, (0o300 | !mask) & 0o777));
                if let Err(e) = r {
                    if !is_dir(pre) {
                        result = Err(e);
                        failed_at = pre;
                        break;
                    }
                }
            }
        }
        if result.is_ok() {
            /* never more permissive than requested, even for a moment */
            result = match mode {
                Some(m) => make(dir, m & 0o777).and_then(|_| chmod(dir, m)),
                None => make(dir, 0o777),
            };
        }
        if let Err(e) = result {
            posix::warn_path(&prog, failed_at, &posix::strerror(&e));
            status = 1;
        }
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(p: &str) -> Vec<String> {
        prefixes(p.as_bytes()).iter().map(|v| String::from_utf8(v.to_vec()).unwrap()).collect()
    }

    #[test]
    fn prefix_list() {
        assert_eq!(pre("a/b/c"), ["a", "a/b"]);
        assert_eq!(pre("/a/b/c"), ["/a", "/a/b"]);
        assert_eq!(pre("a"), Vec::<String>::new());
        assert_eq!(pre("a/"), Vec::<String>::new());
        assert_eq!(pre("a//b//"), ["a"]);
        assert_eq!(pre("../x/y"), ["..", "../x"]);
    }
}
