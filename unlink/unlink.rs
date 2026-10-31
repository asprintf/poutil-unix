/* SPDX-License-Identifier: GPL-2.0-only */
/* unlink - call the unlink function. */

use std::ffi::CString;

const SYNOPSIS: &str = "unlink file";

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let ops = posix::no_options(&args, SYNOPSIS);
    if ops.len() != 1 {
        posix::usage(SYNOPSIS);
    }
    /* argv strings cannot contain a NUL byte */
    let path = CString::new(ops[0].clone()).unwrap();
    if unsafe { libc::unlink(path.as_ptr()) } < 0 {
        let e = std::io::Error::last_os_error();
        posix::warn_path(&prog, &ops[0], &posix::strerror(&e));
        std::process::exit(1);
    }
}
