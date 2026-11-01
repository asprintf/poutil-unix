/* SPDX-License-Identifier: GPL-2.0-only */
/* tty - return the user's terminal name. */

const SYNOPSIS: &str = "tty";

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    if !posix::no_options(&args, SYNOPSIS).is_empty() {
        posix::usage(SYNOPSIS);
    }
    if unsafe { libc::isatty(0) } != 1 {
        posix::write_stdout(&prog, b"not a tty\n");
        std::process::exit(1);
    }
    posix::clear_errno();
    let p = unsafe { libc::ttyname(0) };
    if p.is_null() {
        /* errno says why: no entry for the device under /dev, no memory, ... */
        let e = std::io::Error::last_os_error();
        posix::warn(&prog, &format!("cannot determine the terminal name: {}", posix::strerror(&e)));
        std::process::exit(2);
    }
    let mut out = unsafe { std::ffi::CStr::from_ptr(p) }.to_bytes().to_vec();
    out.push(b'\n');
    posix::write_stdout(&prog, &out);
}
