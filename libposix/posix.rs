/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Shared helpers for the POSIX utilities: byte-oriented argument handling,
 * POSIX getopt, and process startup conventions.
 */

pub mod getopt;
pub mod mode;

use std::ffi::CString;
use std::fs::File;
use std::io::Write;
use std::mem::ManuallyDrop;
use std::os::unix::io::FromRawFd;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::os::unix::ffi::OsStringExt;

static SIGPIPE_WAS_IGNORED: AtomicBool = AtomicBool::new(false);
/* bit n set: descriptor n was closed when the process started */
static CLOSED_STDIO: AtomicU8 = AtomicU8::new(0);

/*
 * Rust's runtime sets SIGPIPE to SIG_IGN before main, which loses the
 * disposition inherited from our parent, and reopens any of the descriptors
 * 0, 1 and 2 that were closed as /dev/null. This constructor runs earlier
 * and records whether the signal arrived ignored and which descriptors
 * were closed.
 */
extern "C" fn save_sigpipe() {
    unsafe {
        for fd in 0..3 {
            if libc::fcntl(fd, libc::F_GETFD) < 0 {
                CLOSED_STDIO.fetch_or(1 << fd, Ordering::Relaxed);
            }
        }
        let mut sa: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(libc::SIGPIPE, std::ptr::null(), &mut sa) == 0
            && sa.sa_sigaction == libc::SIG_IGN
        {
            SIGPIPE_WAS_IGNORED.store(true, Ordering::Relaxed);
        }
    }
}

#[used]
#[cfg_attr(target_vendor = "apple", link_section = "__DATA,__mod_init_func")]
#[cfg_attr(not(target_vendor = "apple"), link_section = ".init_array")]
static SAVE_SIGPIPE: extern "C" fn() = save_sigpipe;

/*
 * Rust's runtime installs handlers for SIGSEGV and SIGBUS (stack overflow
 * detection) that swallow signals sent with kill(). Restore the default
 * action, unless the signal was inherited as ignored (the runtime leaves
 * those alone).
 */
fn default_unless_ignored(sig: libc::c_int) {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(sig, std::ptr::null(), &mut sa) == 0
            && sa.sa_sigaction != libc::SIG_IGN
        {
            libc::signal(sig, libc::SIG_DFL);
        }
    }
}

/*
 * Must be the first call in every utility's main.
 *
 * Rust ignores SIGPIPE by default; POSIX utilities expect the default
 * disposition so that "util | head" terminates quietly. A SIGPIPE that
 * was inherited as ignored stays ignored, so that a write to a broken
 * pipe fails with EPIPE and is reported.
 */
pub fn init() {
    std::hint::black_box(&SAVE_SIGPIPE);
    /* undo the runtime's /dev/null replacement of closed descriptors */
    let closed = CLOSED_STDIO.load(Ordering::Relaxed);
    for fd in 0..3 {
        if closed & (1 << fd) != 0 {
            unsafe { libc::close(fd) };
        }
    }
    default_unless_ignored(libc::SIGSEGV);
    default_unless_ignored(libc::SIGBUS);
    unsafe {
        let h = if SIGPIPE_WAS_IGNORED.load(Ordering::Relaxed) {
            libc::SIG_IGN
        } else {
            libc::SIG_DFL
        };
        libc::signal(libc::SIGPIPE, h);
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    }
}

/* Command-line arguments as raw bytes (args[0] is the program name). */
pub fn args() -> Vec<Vec<u8>> {
    std::env::args_os().map(|a| a.into_vec()).collect()
}

/* The last component of args[0], for diagnostics. */
pub fn progname(args: &[Vec<u8>]) -> Vec<u8> {
    let a = args.first().map(|v| v.as_slice()).unwrap_or(b"?");
    match a.iter().rposition(|&c| c == b'/') {
        Some(i) => a[i + 1..].to_vec(),
        None => a.to_vec(),
    }
}

/*
 * Print "prog: msg" to standard error. Only the fact that a diagnostic is
 * written is specified by POSIX, not its wording.
 */
pub fn warn(prog: &[u8], msg: &str) {
    let mut e = std::io::stderr().lock();
    let _ = e.write_all(prog);
    let _ = writeln!(e, ": {msg}");
}

/* Print "prog: path: msg" to standard error. */
pub fn warn_path(prog: &[u8], path: &[u8], msg: &str) {
    let mut e = std::io::stderr().lock();
    let _ = e.write_all(prog);
    let _ = e.write_all(b": ");
    let _ = e.write_all(path);
    let _ = writeln!(e, ": {msg}");
}

/* Print a diagnostic and exit with status 1. */
pub fn fail(prog: &[u8], msg: &str) -> ! {
    warn(prog, msg);
    std::process::exit(1)
}

/*
 * Print "usage: synopsis" and exit with status 2 (">1" so that utilities
 * like tty, whose status 1 means something else, stay correct).
 */
pub fn usage(synopsis: &str) -> ! {
    let _ = writeln!(std::io::stderr().lock(), "usage: {synopsis}");
    std::process::exit(2)
}

/*
 * The system's message for an I/O error (strerror(3)), without Rust's
 * "(os error N)" suffix.
 */
pub fn strerror(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(n) => unsafe {
            std::ffi::CStr::from_ptr(libc::strerror(n))
                .to_string_lossy()
                .into_owned()
        },
        None => e.to_string(),
    }
}

/*
 * Write all of data to standard output; on failure report it and exit 1.
 * Not std::io::stdout(), which treats a closed descriptor (EBADF) as success.
 */
pub fn write_stdout(prog: &[u8], data: &[u8]) {
    if let Err(e) = borrow_fd(1).write_all(data) {
        fail(prog, &format!("write error: {}", strerror(&e)));
    }
}

/*
 * For utilities that take no options: accept a leading "--", reject any
 * other option (usage error) and return the operands.
 */
pub fn no_options<'a>(args: &'a [Vec<u8>], synopsis: &str) -> &'a [Vec<u8>] {
    let mut g = getopt::Getopt::new(args, b"");
    if g.next().is_some() {
        usage(synopsis);
    }
    g.operands()
}

/* A descriptor we do not own: never closed, never buffered. */
pub fn borrow_fd(fd: i32) -> ManuallyDrop<File> {
    ManuallyDrop::new(unsafe { File::from_raw_fd(fd) })
}

/* Command-line bytes as a C string; arguments never contain a NUL. */
pub fn cstr(b: &[u8]) -> CString {
    CString::new(b).expect("NUL in argument")
}

/* Set errno to 0, to tell an error return of -1 from a valid one. */
pub fn clear_errno() {
    unsafe {
        #[cfg(target_vendor = "apple")]
        {
            *libc::__error() = 0;
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            *libc::__errno_location() = 0;
        }
    }
}

#[cfg(target_vendor = "apple")]
unsafe fn set_environ(p: *mut *mut libc::c_char) {
    *libc::_NSGetEnviron() = p;
}

#[cfg(not(target_vendor = "apple"))]
unsafe fn set_environ(p: *mut *mut libc::c_char) {
    extern "C" {
        static mut environ: *mut *mut libc::c_char;
    }
    environ = p;
}

/*
 * Replace this process by the utility argv[0], found through PATH, with
 * the environment env (default: the current one). On failure print a
 * diagnostic and exit 127 (not found) or 126 (found but not invocable), as
 * env, nice, nohup and time specify.
 *
 * This is execvp(3) directly, not std::process::Command: that one resets
 * SIGPIPE and the signal mask in the child, which would lose inherited
 * dispositions.
 */
pub fn exec_utility(prog: &[u8], argv: &[Vec<u8>], env: Option<&[Vec<u8>]>) -> ! {
    let cargs: Vec<CString> = argv.iter().map(|a| cstr(a)).collect();
    let mut pargs: Vec<*const libc::c_char> = cargs.iter().map(|a| a.as_ptr()).collect();
    pargs.push(std::ptr::null());
    if let Some(env) = env {
        let mut penv: Vec<*mut libc::c_char> =
            env.iter().map(|e| cstr(e).into_raw()).collect();
        penv.push(std::ptr::null_mut());
        unsafe { set_environ(penv.as_mut_ptr()) };
        std::mem::forget(penv);
    }
    unsafe { libc::execvp(pargs[0], pargs.as_ptr()) };
    let e = std::io::Error::last_os_error();
    warn_path(prog, &argv[0], &strerror(&e));
    let missing = matches!(e.raw_os_error(), Some(libc::ENOENT) | Some(libc::ENOTDIR));
    std::process::exit(if missing { 127 } else { 126 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progname_is_the_last_component() {
        assert_eq!(progname(&[b"/usr/bin/cat".to_vec()]), b"cat");
        assert_eq!(progname(&[b"cat".to_vec()]), b"cat");
        assert_eq!(progname(&[b"./x/y".to_vec(), b"z".to_vec()]), b"y");
        assert_eq!(progname(&[]), b"?");
    }

    #[test]
    fn strerror_has_no_rust_suffix() {
        let e = std::io::Error::from_raw_os_error(libc::ENOENT);
        assert_eq!(strerror(&e), "No such file or directory");
        let e = std::io::Error::from_raw_os_error(libc::EACCES);
        assert_eq!(strerror(&e), "Permission denied");
        let e = std::io::Error::new(std::io::ErrorKind::Other, "boom");
        assert_eq!(strerror(&e), "boom");
    }

    #[test]
    fn cstr_keeps_every_byte() {
        assert_eq!(cstr(b"abc").as_bytes(), b"abc");
        assert_eq!(cstr(b"\xff\xfe").as_bytes(), b"\xff\xfe");
        assert_eq!(cstr(b"").as_bytes(), b"");
    }
}
