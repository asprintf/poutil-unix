/* SPDX-License-Identifier: GPL-2.0-only */
/* logname - return the user's login name. */

const SYNOPSIS: &str = "logname";

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    if !posix::no_options(&args, SYNOPSIS).is_empty() {
        posix::usage(SYNOPSIS);
    }
    let p = unsafe { libc::getlogin() };
    if p.is_null() {
        posix::fail(&prog, "no login name");
    }
    let mut out = unsafe { std::ffi::CStr::from_ptr(p) }.to_bytes().to_vec();
    out.push(b'\n');
    posix::write_stdout(&prog, &out);
}
