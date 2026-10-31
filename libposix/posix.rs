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

/*
 * An input file operand: "-" is standard input (borrowed, never closed),
 * anything else is opened for reading. Derefs to File either way.
 */
pub enum Input {
    Stdin(ManuallyDrop<File>),
    Named(File),
}

impl std::ops::Deref for Input {
    type Target = File;
    fn deref(&self) -> &File {
        match self {
            Input::Stdin(f) => f,
            Input::Named(f) => f,
        }
    }
}

impl std::ops::DerefMut for Input {
    fn deref_mut(&mut self) -> &mut File {
        match self {
            Input::Stdin(f) => f,
            Input::Named(f) => f,
        }
    }
}

impl Input {
    pub fn is_stdin(&self) -> bool {
        matches!(self, Input::Stdin(_))
    }
}

pub fn open_input(name: &[u8]) -> std::io::Result<Input> {
    if name == b"-" {
        return Ok(Input::Stdin(borrow_fd(0)));
    }
    File::open(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(name))
        .map(Input::Named)
}

/* One read(2) that is retried after a signal interrupted it. */
pub fn read_block<R: std::io::Read>(r: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match r.read(buf) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

/* Index of the first byte c in hay (memchr(3)). */
pub fn find_byte(c: u8, hay: &[u8]) -> Option<usize> {
    let p = unsafe { libc::memchr(hay.as_ptr() as *const libc::c_void, c as libc::c_int, hay.len()) };
    if p.is_null() {
        None
    } else {
        Some(p as usize - hay.as_ptr() as usize)
    }
}

/*
 * How many bytes of hay are c. Eight bytes at a time in a plain u64: a byte
 * of x = word ^ pattern is zero exactly where the input byte was c, and
 * ((x & 0x7f..) + 0x7f.. | x) & 0x80.. has the top bit set in every byte of
 * x that is not zero (the 0x7f addition cannot carry into the next byte).
 * The per-byte results are summed in the eight lanes of one u64 for at most
 * 255 words before the lanes are added up. It is several times faster than
 * a byte loop, which the size-optimised build does not vectorise.
 */
pub fn count_byte(c: u8, hay: &[u8]) -> u64 {
    const LO: u64 = 0x0101_0101_0101_0101;
    const M7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const HI: u64 = 0x8080_8080_8080_8080;
    const EVEN: u64 = 0x00ff_00ff_00ff_00ff;
    let pat = LO.wrapping_mul(c as u64);
    let mut total = 0u64;
    for block in hay.chunks(255 * 8) {
        let words = block.chunks_exact(8);
        let rest = words.remainder();
        let mut lanes = 0u64;
        for w in words {
            let mut a = [0u8; 8];
            a.copy_from_slice(w);
            let x = u64::from_ne_bytes(a) ^ pat;
            let nonzero = (((x & M7).wrapping_add(M7)) | x) & HI;
            lanes += (!nonzero & HI) >> 7;
        }
        let pairs = (lanes & EVEN) + ((lanes >> 8) & EVEN);
        total += pairs.wrapping_mul(0x0001_0001_0001_0001) >> 48;
        total += rest.iter().filter(|&&b| b == c).count() as u64;
    }
    total
}

/*
 * A non-empty string of decimal digits as a number. A value too big for 64
 * bits becomes u64::MAX, which every caller treats as "more than the file
 * can hold".
 */
pub fn parse_decimal(s: &[u8]) -> Option<u64> {
    if s.is_empty() || !s.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut n: u64 = 0;
    for &c in s {
        n = n.saturating_mul(10).saturating_add((c - b'0') as u64);
    }
    Some(n)
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
    fn find_byte_is_memchr() {
        assert_eq!(find_byte(b'\n', b"ab\ncd\n"), Some(2));
        assert_eq!(find_byte(b'\n', b"abcd"), None);
        assert_eq!(find_byte(b'\n', b""), None);
        assert_eq!(find_byte(0, b"a\0b"), Some(1));
        assert_eq!(find_byte(0xff, b"a\xff"), Some(1));
    }

    #[test]
    fn count_byte_matches_a_plain_loop() {
        let naive = |c: u8, h: &[u8]| h.iter().filter(|&&b| b == c).count() as u64;
        /* every length around the word and block sizes, every alignment */
        let mut x: u32 = 1;
        let data: Vec<u8> = (0..9000)
            .map(|_| {
                x = x.wrapping_mul(1103515245).wrapping_add(12345);
                match (x >> 16) % 5 {
                    0 => b'\n',
                    1 => 0x0a ^ 0x80,
                    2 => 0x8a,
                    3 => 0x0b,
                    _ => (x >> 8) as u8,
                }
            })
            .collect();
        for start in 0..9 {
            for len in (0..40).chain(2030..2050).chain(4080..4100).chain([8000, 8990]) {
                let h = &data[start..(start + len).min(data.len())];
                for c in [b'\n', 0x0a ^ 0x80, 0x8a, 0, 0xff, 0x0b] {
                    assert_eq!(count_byte(c, h), naive(c, h), "start {start} len {len} byte {c}");
                }
            }
        }
        assert_eq!(count_byte(b'\n', b""), 0);
        assert_eq!(count_byte(b'\n', &[b'\n'; 100_000]), 100_000);
        assert_eq!(count_byte(0, &[0u8; 100_003]), 100_003);
        assert_eq!(count_byte(b'\n', &[0xffu8; 100_000]), 0);
        /* bytes next to the wanted one, differing in one bit, are not it */
        assert_eq!(count_byte(b'\n', &[0x0b, 0x8a, 0x0a ^ 0x01, 0x0a ^ 0x40, 0x0a]), 1);
    }

    #[test]
    fn parse_decimal_takes_only_digits() {
        assert_eq!(parse_decimal(b"0"), Some(0));
        assert_eq!(parse_decimal(b"0042"), Some(42));
        assert_eq!(parse_decimal(b""), None);
        assert_eq!(parse_decimal(b"+1"), None);
        assert_eq!(parse_decimal(b"-1"), None);
        assert_eq!(parse_decimal(b"1 "), None);
        assert_eq!(parse_decimal(b"1.5"), None);
        assert_eq!(parse_decimal(b"\xff"), None);
        assert_eq!(parse_decimal(b"18446744073709551615"), Some(u64::MAX));
        assert_eq!(parse_decimal(b"99999999999999999999999999"), Some(u64::MAX));
    }

    #[test]
    fn open_input_dash_is_stdin_and_a_missing_file_is_an_error() {
        assert!(open_input(b"-").unwrap().is_stdin());
        assert!(!open_input(b"/").unwrap().is_stdin());
        assert!(open_input(b"/nonexistent/poutil").is_err());
    }

    #[test]
    fn cstr_keeps_every_byte() {
        assert_eq!(cstr(b"abc").as_bytes(), b"abc");
        assert_eq!(cstr(b"\xff\xfe").as_bytes(), b"\xff\xfe");
        assert_eq!(cstr(b"").as_bytes(), b"");
    }
}
