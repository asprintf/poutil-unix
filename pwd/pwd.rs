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
    while let Some(o) = g.next() {
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
