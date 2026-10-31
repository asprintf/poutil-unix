/* SPDX-License-Identifier: GPL-2.0-only */
/* time - time a simple command. */

use posix::getopt::{Getopt, Opt};
use std::io::Write;
use std::time::Instant;

const SYNOPSIS: &str = "time [-p] utility [argument...]";

/* %.{prec}f in the current locale, so that LC_NUMERIC picks the radix character */
fn float(v: f64, prec: usize) -> String {
    let fmt = posix::cstr(format!("%.{prec}f").as_bytes());
    let mut buf = [0u8; 64];
    let n = unsafe { libc::snprintf(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), fmt.as_ptr(), v) };
    String::from_utf8_lossy(&buf[..(n.max(0) as usize).min(buf.len() - 1)]).into_owned()
}

/* The format required by -p: seconds with two decimals. */
fn fmt_posix(real: f64, user: f64, sys: f64) -> String {
    format!("real {}\nuser {}\nsys {}\n", float(real, 2), float(user, 2), float(sys, 2))
}

fn minutes(s: f64) -> String {
    /* round to milliseconds first, or 59.9996 would print as "0m60.000s" */
    let ms = (s * 1000.0).round() as u64;
    format!("{}m{}s", ms / 60000, float((ms % 60000) as f64 / 1000.0, 3))
}

/* Without -p the format is up to the implementation. */
fn fmt_default(real: f64, user: f64, sys: f64) -> String {
    format!("\nreal\t{}\nuser\t{}\nsys\t{}\n", minutes(real), minutes(user), minutes(sys))
}

fn secs(t: libc::timeval) -> f64 {
    t.tv_sec as f64 + t.tv_usec as f64 / 1e6
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut posix_fmt = false;
    let mut g = Getopt::new(&args, b"p");
    while let Some(o) = g.next() {
        match o {
            Opt::Flag(b'p') => posix_fmt = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let ops = g.operands();
    if ops.is_empty() {
        posix::usage(SYNOPSIS);
    }
    let start = Instant::now();
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        posix::fail(&prog, &posix::strerror(&std::io::Error::last_os_error()));
    }
    if pid == 0 {
        posix::exec_utility(&prog, ops, None);
    }
    let mut status: libc::c_int = 0;
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    loop {
        let r = unsafe { libc::wait4(pid, &mut status, 0, &mut ru) };
        if r == pid {
            break;
        }
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EINTR) {
            posix::fail(&prog, &posix::strerror(&e));
        }
    }
    let real = start.elapsed().as_secs_f64();
    let (user, sys) = (secs(ru.ru_utime), secs(ru.ru_stime));
    let text = if posix_fmt { fmt_posix(real, user, sys) } else { fmt_default(real, user, sys) };
    let _ = std::io::stderr().write_all(text.as_bytes());
    let code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        128 + libc::WTERMSIG(status)
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(fmt_posix(1.5, 0.25, 0.0), "real 1.50\nuser 0.25\nsys 0.00\n");
        assert_eq!(minutes(0.0), "0m0.000s");
        assert_eq!(minutes(93.75), "1m33.750s");
        assert_eq!(
            fmt_default(61.0, 0.5, 0.0),
            "\nreal\t1m1.000s\nuser\t0m0.500s\nsys\t0m0.000s\n"
        );
    }

    #[test]
    fn float_and_posix_format_for_long_runs() {
        assert_eq!(float(0.0, 2), "0.00");
        assert_eq!(float(1234567.891, 2), "1234567.89");
        assert_eq!(fmt_posix(3661.5, 0.0, 0.0), "real 3661.50\nuser 0.00\nsys 0.00\n");
        assert_eq!(fmt_posix(59.999, 0.004, 0.006), "real 60.00\nuser 0.00\nsys 0.01\n");
    }

    #[test]
    fn minutes_never_show_sixty_seconds() {
        assert_eq!(minutes(60.0), "1m0.000s");
        assert_eq!(minutes(59.9996), "1m0.000s");
        assert_eq!(minutes(119.9996), "2m0.000s");
        assert_eq!(minutes(0.0004), "0m0.000s");
        assert_eq!(minutes(0.0005), "0m0.001s");
        assert_eq!(minutes(3600.0), "60m0.000s");
    }
}
