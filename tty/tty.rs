/* SPDX-License-Identifier: GPL-2.0-only */
/* tty - return the user's terminal name. */

#![forbid(unsafe_code)]

const SYNOPSIS: &str = "tty";

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    if !posix::no_options(&args, SYNOPSIS).is_empty() {
        posix::usage(SYNOPSIS);
    }
    if !posix::sys::isatty(0) {
        posix::write_stdout(&prog, b"not a tty\n");
        std::process::exit(1);
    }
    posix::clear_errno();
    let Some(mut out) = posix::sys::ttyname(0) else {
        /* errno says why: no entry for the device under /dev, no memory, ... */
        let e = std::io::Error::last_os_error();
        posix::warn(&prog, &format!("cannot determine the terminal name: {}", posix::strerror(&e)));
        std::process::exit(2);
    };
    out.push(b'\n');
    posix::write_stdout(&prog, &out);
}
