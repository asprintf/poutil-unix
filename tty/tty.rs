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
        /* status 1 means "not a terminal": a write error is 2 */
        posix::write_stdout_or(&prog, b"not a tty\n", 2);
        std::process::exit(1);
    }
    let mut out = match posix::sys::ttyname(0) {
        Ok(name) => name,
        /* no entry for the device under /dev, no memory, ... */
        Err(e) => {
            posix::warn(&prog, &format!("cannot determine the terminal name: {}", posix::strerror(&e)));
            std::process::exit(2);
        }
    };
    out.push(b'\n');
    posix::write_stdout_or(&prog, &out, 2);
}
