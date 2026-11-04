/* SPDX-License-Identifier: GPL-2.0-only */
/* logname - return the user's login name. */

#![forbid(unsafe_code)]

const SYNOPSIS: &str = "logname";

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    if !posix::no_options(&args, SYNOPSIS).is_empty() {
        posix::usage(SYNOPSIS);
    }
    let Some(mut out) = posix::sys::getlogin() else {
        posix::fail(&prog, "no login name");
    };
    out.push(b'\n');
    posix::write_stdout(&prog, &out);
}
