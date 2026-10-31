/* SPDX-License-Identifier: GPL-2.0-only */
/* nohup - invoke a utility immune to hangups. */

use posix::getopt::Getopt;
use std::io::Write;

const SYNOPSIS: &str = "nohup utility [argument...]";

fn isatty(fd: i32) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

fn is_closed(fd: i32) -> bool {
    unsafe { libc::fcntl(fd, libc::F_GETFD) < 0 }
}

/* Open path for appending, creating it with mode 0600 whatever the umask. */
fn open_append(path: &[u8]) -> Option<i32> {
    let c = posix::cstr(path);
    unsafe {
        let old = libc::umask(0o177);
        let fd = libc::open(
            c.as_ptr(),
            libc::O_WRONLY | libc::O_APPEND | libc::O_CREAT,
            0o600 as libc::c_uint,
        );
        libc::umask(old);
        if fd < 0 { None } else { Some(fd) }
    }
}

/* nohup.out in the current directory, else in $HOME */
fn open_nohup_out() -> Option<(i32, Vec<u8>)> {
    if let Some(fd) = open_append(b"nohup.out") {
        return Some((fd, b"nohup.out".to_vec()));
    }
    let mut p = std::env::var_os("HOME").map(|h| std::os::unix::ffi::OsStringExt::into_vec(h))?;
    if p.is_empty() {
        return None;
    }
    p.extend_from_slice(b"/nohup.out");
    open_append(&p).map(|fd| (fd, p))
}

/* Errors of nohup itself are exit status 127. */
fn die(prog: &[u8], msg: &str) -> ! {
    posix::warn(prog, msg);
    std::process::exit(127)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut g = Getopt::new(&args, b"");
    if g.next().is_some() || g.operands().is_empty() {
        let _ = writeln!(std::io::stderr(), "usage: {SYNOPSIS}");
        std::process::exit(127);
    }
    unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
    let out_tty = isatty(1);
    let out_closed = is_closed(1);
    if out_tty || out_closed {
        let Some((fd, name)) = open_nohup_out() else {
            die(&prog, "cannot open nohup.out");
        };
        let mut msg = prog.clone();
        msg.extend_from_slice(b": appending output to ");
        msg.extend_from_slice(&name);
        msg.push(b'\n');
        let _ = std::io::stderr().write_all(&msg);
        unsafe {
            if fd != 1 {
                libc::dup2(fd, 1);
            }
            if isatty(2) && fd != 2 {
                libc::dup2(fd, 2);
            }
            if fd > 2 {
                libc::close(fd);
            }
        }
    } else if isatty(2) {
        unsafe { libc::dup2(1, 2) };
    }
    posix::exec_utility(&prog, g.operands(), None);
}
