/* SPDX-License-Identifier: GPL-2.0-only */
/* kill - terminate or signal processes. */

const SYNOPSIS: &str = "kill -s signal_name pid...\n       kill -l [exit_status]\n       kill [-signal_name] pid...\n       kill [-signal_number] pid...";

const SIGNALS: &[(&str, i32)] = &[
    ("HUP", libc::SIGHUP),
    ("INT", libc::SIGINT),
    ("QUIT", libc::SIGQUIT),
    ("ILL", libc::SIGILL),
    ("TRAP", libc::SIGTRAP),
    ("ABRT", libc::SIGABRT),
    ("FPE", libc::SIGFPE),
    ("KILL", libc::SIGKILL),
    ("BUS", libc::SIGBUS),
    ("SEGV", libc::SIGSEGV),
    ("SYS", libc::SIGSYS),
    ("PIPE", libc::SIGPIPE),
    ("ALRM", libc::SIGALRM),
    ("TERM", libc::SIGTERM),
    ("URG", libc::SIGURG),
    ("STOP", libc::SIGSTOP),
    ("TSTP", libc::SIGTSTP),
    ("CONT", libc::SIGCONT),
    ("CHLD", libc::SIGCHLD),
    ("TTIN", libc::SIGTTIN),
    ("TTOU", libc::SIGTTOU),
    ("IO", libc::SIGIO),
    ("XCPU", libc::SIGXCPU),
    ("XFSZ", libc::SIGXFSZ),
    ("VTALRM", libc::SIGVTALRM),
    ("PROF", libc::SIGPROF),
    ("WINCH", libc::SIGWINCH),
    ("USR1", libc::SIGUSR1),
    ("USR2", libc::SIGUSR2),
    #[cfg(target_vendor = "apple")]
    ("EMT", libc::SIGEMT),
    #[cfg(target_vendor = "apple")]
    ("INFO", libc::SIGINFO),
];

fn name_of(n: i32) -> Option<&'static str> {
    SIGNALS.iter().find(|s| s.1 == n).map(|s| s.0)
}

/* A signal name (case-independent, SIG prefix optional), "0", or a number. */
fn parse_signal(s: &[u8]) -> Option<i32> {
    if !s.is_empty() && s.iter().all(|c| c.is_ascii_digit()) {
        let n: i32 = std::str::from_utf8(s).ok()?.parse().ok()?;
        return (n == 0 || name_of(n).is_some()).then_some(n);
    }
    let up = s.to_ascii_uppercase();
    let name = up.strip_prefix(b"SIG").unwrap_or(&up);
    SIGNALS.iter().find(|x| x.0.as_bytes() == name).map(|x| x.1)
}

/* An exit status that names a signal: the number itself or 128 + number. */
fn signal_for_status(n: u32) -> Option<&'static str> {
    name_of(n as i32).or_else(|| if n > 128 { name_of((n - 128) as i32) } else { None })
}

fn list(prog: &[u8], rest: &[Vec<u8>]) -> i32 {
    match rest {
        [] => {
            let mut all: Vec<_> = SIGNALS.iter().collect();
            all.sort_by_key(|s| s.1);
            let names: Vec<&str> = all.iter().map(|s| s.0).collect();
            posix::write_stdout(prog, format!("{}\n", names.join(" ")).as_bytes());
            0
        }
        [st] => {
            let parsed = std::str::from_utf8(st).ok().and_then(|s| s.parse::<u32>().ok());
            match parsed.and_then(signal_for_status) {
                Some(n) => {
                    posix::write_stdout(prog, format!("{n}\n").as_bytes());
                    0
                }
                None => {
                    posix::warn_path(prog, st, "not a signal number or exit status");
                    1
                }
            }
        }
        _ => posix::usage(SYNOPSIS),
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut rest = &args[1..];
    let mut sig = libc::SIGTERM;
    let bad = |s: &[u8]| -> ! {
        posix::warn_path(&prog, s, "invalid signal");
        std::process::exit(2)
    };
    match rest.first().map(|v| v.as_slice()) {
        None => posix::usage(SYNOPSIS),
        Some(b"-l") => std::process::exit(list(&prog, &rest[1..])),
        Some(b"-s") => {
            let Some(name) = rest.get(1) else { posix::usage(SYNOPSIS) };
            sig = parse_signal(name).unwrap_or_else(|| bad(name));
            rest = &rest[2..];
        }
        Some(b"--") => {}
        Some(a) if a.len() > 1 && a[0] == b'-' => {
            /* -signal_name and -signal_number first, then -sNAME */
            if let Some(n) = parse_signal(&a[1..]) {
                sig = n;
            } else if a[1] == b's' || a[1] == b'S' {
                sig = parse_signal(&a[2..]).unwrap_or_else(|| bad(&a[2..]));
            } else {
                bad(&a[1..]);
            }
            rest = &rest[1..];
        }
        Some(_) => {}
    }
    if rest.first().map(|v| v.as_slice()) == Some(b"--") {
        rest = &rest[1..];
    }
    if rest.is_empty() {
        posix::usage(SYNOPSIS);
    }
    let mut status = 0;
    for p in rest {
        let pid = std::str::from_utf8(p).ok().and_then(|s| s.parse::<i32>().ok());
        let Some(pid) = pid else {
            posix::warn_path(&prog, p, "invalid process id");
            status = 1;
            continue;
        };
        if unsafe { libc::kill(pid, sig) } < 0 {
            let e = std::io::Error::last_os_error();
            posix::warn_path(&prog, p, &posix::strerror(&e));
            status = 1;
        }
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_numbers() {
        assert_eq!(parse_signal(b"TERM"), Some(libc::SIGTERM));
        assert_eq!(parse_signal(b"term"), Some(libc::SIGTERM));
        assert_eq!(parse_signal(b"SIGKILL"), Some(libc::SIGKILL));
        assert_eq!(parse_signal(b"0"), Some(0));
        assert_eq!(parse_signal(b"9"), Some(9));
        assert_eq!(parse_signal(b"1000"), None);
        assert_eq!(parse_signal(b"BOGUS"), None);
        assert_eq!(parse_signal(b""), None);
    }

    #[test]
    fn spec_numbers() {
        for (n, name) in [(1, "HUP"), (2, "INT"), (3, "QUIT"), (6, "ABRT"), (9, "KILL"), (14, "ALRM"), (15, "TERM")] {
            assert_eq!(name_of(n), Some(name));
        }
    }

    #[test]
    fn exit_status_to_name() {
        assert_eq!(signal_for_status(15), Some("TERM"));
        assert_eq!(signal_for_status(143), Some("TERM"));
        assert_eq!(signal_for_status(129), Some("HUP"));
        assert_eq!(signal_for_status(0), None);
        assert_eq!(signal_for_status(200), None);
    }
}
