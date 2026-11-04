/* SPDX-License-Identifier: GPL-2.0-only */
/* uname - return the system name. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};

const SYNOPSIS: &str = "uname [-amnrsv]";

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    /* sysname, nodename, release, version, machine, in that output order */
    let mut want = [false; 5];
    let mut any = false;
    let mut g = Getopt::new(&args, b"amnrsv");
    for o in g.by_ref() {
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
    let u = match posix::sys::uname() {
        Ok(u) => u,
        Err(e) => posix::fail(&prog, &posix::strerror(&e)),
    };
    let vals = [u.sysname, u.nodename, u.release, u.version, u.machine];
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
