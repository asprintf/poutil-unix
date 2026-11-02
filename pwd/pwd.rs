/* SPDX-License-Identifier: GPL-2.0-only */
/* pwd - return the working directory name. */

use posix::getopt::{Getopt, Opt};
use std::ffi::OsStr;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;

const SYNOPSIS: &str = "pwd [-L|-P]";

/*
 * Whether pwd is an absolute pathname of the current directory with no
 * dot or dot-dot components, as -L requires of $PWD.
 */
fn logical_ok(pwd: &[u8]) -> bool {
    if pwd.first() != Some(&b'/') {
        return false;
    }
    if pwd.split(|&c| c == b'/').any(|c| c == b"." || c == b"..") {
        return false;
    }
    let a = std::fs::metadata(OsStr::from_bytes(pwd));
    let b = std::fs::metadata(".");
    match (a, b) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut logical = true;
    let mut g = Getopt::new(&args, b"LP");
    for o in g.by_ref() {
        match o {
            Opt::Flag(b'L') => logical = true,
            Opt::Flag(b'P') => logical = false,
            _ => posix::usage(SYNOPSIS),
        }
    }
    if !g.operands().is_empty() {
        posix::usage(SYNOPSIS);
    }
    let mut out = Vec::new();
    if logical {
        if let Some(p) = std::env::var_os("PWD") {
            let p = p.into_vec();
            if logical_ok(&p) {
                out = p;
            }
        }
    }
    if out.is_empty() {
        match std::env::current_dir() {
            Ok(p) => out = p.into_os_string().into_vec(),
            Err(e) => posix::fail(&prog, &posix::strerror(&e)),
        }
    }
    out.push(b'\n');
    posix::write_stdout(&prog, &out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    fn cwd() -> Vec<u8> {
        std::env::current_dir().unwrap().into_os_string().into_vec()
    }

    #[test]
    fn accepts_the_real_directory_in_its_spellings() {
        let real = cwd();
        assert!(logical_ok(&real));
        let mut with_slash = real.clone();
        with_slash.push(b'/');
        assert!(logical_ok(&with_slash));
        let mut double = b"/".to_vec();
        double.extend_from_slice(&real);
        assert!(logical_ok(&double));
    }

    #[test]
    fn rejects_everything_else() {
        let real = cwd();
        assert!(!logical_ok(b""));
        assert!(!logical_ok(b"relative"));
        assert!(!logical_ok(b"/"));
        assert!(!logical_ok(b"/nonexistent/dir"));
        assert!(!logical_ok(b"/tmp/.."));
        assert!(!logical_ok(b"/./tmp"));
        let mut dot = real.clone();
        dot.extend_from_slice(b"/.");
        assert!(!logical_ok(&dot));
        let mut dotdot = real.clone();
        dotdot.extend_from_slice(b"/../");
        dotdot.extend_from_slice(real.rsplit(|&c| c == b'/').next().unwrap());
        assert!(!logical_ok(&dotdot));
    }

    #[test]
    fn accepts_a_symlink_to_the_directory() {
        let link = format!("/tmp/poutil-pwd-link-{}", std::process::id());
        std::os::unix::fs::symlink(OsStr::from_bytes(&cwd()), &link).unwrap();
        let ok = logical_ok(link.as_bytes());
        std::fs::remove_file(&link).unwrap();
        assert!(ok);
    }
}
