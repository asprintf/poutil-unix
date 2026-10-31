/* SPDX-License-Identifier: GPL-2.0-only */
/* uname - return the system name. */

use posix::getopt::{Getopt, Opt};
use std::ffi::CStr;

const SYNOPSIS: &str = "uname [-amnrsv]";

fn field(p: *const libc::c_char) -> Vec<u8> {
    unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    /* sysname, nodename, release, version, machine, in that output order */
    let mut want = [false; 5];
    let mut any = false;
    let mut g = Getopt::new(&args, b"amnrsv");
    while let Some(o) = g.next() {
        any = true;
        match o {
            Opt::Flag(b'a') => want = [true; 5],
            Opt::Flag(b's') => want[0] = true,
            Opt::Flag(b'n') => want[1] = true,
            Opt::Flag(b'r') => want[2] = true,
            Opt::Flag(b'v') => want[3] = true,
            Opt::Flag(b'm') => want[4] = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    if !g.operands().is_empty() {
        posix::usage(SYNOPSIS);
    }
    if !any {
        want[0] = true;
    }
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut u) } < 0 {
        posix::fail(&prog, &posix::strerror(&std::io::Error::last_os_error()));
    }
    let vals = [
        field(u.sysname.as_ptr()),
        field(u.nodename.as_ptr()),
        field(u.release.as_ptr()),
        field(u.version.as_ptr()),
        field(u.machine.as_ptr()),
    ];
    let mut out = Vec::new();
    for (i, v) in vals.iter().enumerate() {
        if want[i] {
            if !out.is_empty() {
                out.push(b' ');
            }
            out.extend_from_slice(v);
        }
    }
    out.push(b'\n');
    posix::write_stdout(&prog, &out);
}
