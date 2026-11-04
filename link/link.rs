/* SPDX-License-Identifier: GPL-2.0-only */
/* link - call the link function. */

#![forbid(unsafe_code)]

use std::ffi::CString;

const SYNOPSIS: &str = "link file1 file2";

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let ops = posix::no_options(&args, SYNOPSIS);
    if ops.len() != 2 {
        posix::usage(SYNOPSIS);
    }
    /* argv strings cannot contain a NUL byte */
    let a = CString::new(ops[0].clone()).unwrap();
    let b = CString::new(ops[1].clone()).unwrap();
    if posix::sys::link(&a, &b) < 0 {
        let e = std::io::Error::last_os_error();
        posix::warn_path(&prog, &ops[1], &posix::strerror(&e));
        std::process::exit(1);
    }
}
