/* SPDX-License-Identifier: GPL-2.0-only */
/* unlink - call the unlink function. */

#![forbid(unsafe_code)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

const SYNOPSIS: &str = "unlink file";

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let ops = posix::no_options(&args, SYNOPSIS);
    if ops.len() != 1 {
        posix::usage(SYNOPSIS);
    }
    if let Err(e) = std::fs::remove_file(OsStr::from_bytes(&ops[0])) {
        posix::warn_path(&prog, &ops[0], &posix::strerror(&e));
        std::process::exit(1);
    }
}
