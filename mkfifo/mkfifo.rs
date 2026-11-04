/* SPDX-License-Identifier: GPL-2.0-only */
/* mkfifo - make FIFO special files. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};

const SYNOPSIS: &str = "mkfifo [-m mode] file...";

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut mode_arg: Option<Vec<u8>> = None;
    let mut g = Getopt::new(&args, b"m:");
    for o in g.by_ref() {
        match o {
            Opt::Arg(b'm', v) => mode_arg = Some(v),
            _ => posix::usage(SYNOPSIS),
        }
    }
    if g.operands().is_empty() {
        posix::usage(SYNOPSIS);
    }
    /* symbolic modes are relative to a=rw; the umask is read (it can only be read by setting it) only for -m */
    let mode = match &mode_arg {
        Some(m) => {
            let mask = posix::sys::umask();
            match posix::mode::parse(m, 0o666, false, mask) {
                Some(v) => Some(v),
                None => posix::fail(&prog, "invalid mode"),
            }
        }
        None => None,
    };
    let mut status = 0;
    for f in g.operands() {
        let c = posix::cstr(f);
        /* never more permissive than requested, even for a moment */
        let create = mode.map_or(0o666, |m| m & 0o777);
        let mut r = posix::sys::mkfifo(&c, create as libc::mode_t);
        if r == 0 {
            if let Some(m) = mode {
                r = posix::sys::chmod(&c, m as libc::mode_t);
            }
        }
        if r < 0 {
            let e = std::io::Error::last_os_error();
            posix::warn_path(&prog, f, &posix::strerror(&e));
            status = 1;
        }
    }
    std::process::exit(status);
}
