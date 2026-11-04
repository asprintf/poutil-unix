/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * System calls as safe functions. They take references and slices instead of
 * raw pointers, and they keep the C convention for the result: -1 (or null,
 * or false) and errno on failure, read with std::io::Error::last_os_error().
 * Calls that give back a structure return io::Result instead.
 *
 * All the unsafe code of the utilities lives in libposix; the utilities
 * themselves have none.
 *
 * Some calls are safe only in a process with one thread, as the utilities
 * are: fork (the child of a threaded process may only call async-signal-safe
 * functions), getlogin and ttyname (static storage), and umask, set_umask
 * and ignore_signal (state of the whole process).
 */

use std::ffi::CStr;
use std::io;
use std::os::unix::io::{FromRawFd, OwnedFd, RawFd};

/*
 * C structures for which all bytes zero is a valid value (no references, no
 * niches: integers, floats and raw pointers only). Implementing it for
 * anything else is unsound, which is why the trait is unsafe.
 */
#[allow(clippy::missing_safety_doc)]
pub unsafe trait Pod: Sized {}

unsafe impl Pod for libc::stat {}
unsafe impl Pod for libc::tm {}
unsafe impl Pod for libc::rusage {}
unsafe impl Pod for libc::utsname {}
unsafe impl Pod for libc::winsize {}
unsafe impl Pod for libc::timespec {}
#[cfg(any(target_vendor = "apple", target_os = "freebsd"))]
unsafe impl Pod for libc::statfs {}
unsafe impl Pod for libc::statvfs {}
unsafe impl Pod for libc::regex_t {}
#[cfg(target_vendor = "apple")]
unsafe impl Pod for libc::attrlist {}

/* A structure of zero bytes, to be filled in by a system call. */
pub fn zeroed<T: Pod>() -> T {
    unsafe { std::mem::zeroed() }
}

fn result(r: libc::c_int) -> io::Result<()> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/* ---- file descriptors ---- */

pub fn close(fd: RawFd) -> libc::c_int {
    unsafe { libc::close(fd) }
}

pub fn open(path: &CStr, flags: libc::c_int, mode: libc::c_uint) -> RawFd {
    unsafe { libc::open(path.as_ptr(), flags, mode) }
}

pub fn openat(dirfd: RawFd, path: &CStr, flags: libc::c_int, mode: libc::c_uint) -> RawFd {
    unsafe { libc::openat(dirfd, path.as_ptr(), flags, mode) }
}

/* openat(2) that gives the descriptor an owner, which closes it. */
pub fn openat_owned(dirfd: RawFd, path: &CStr, flags: libc::c_int, mode: libc::c_uint) -> io::Result<OwnedFd> {
    let fd = openat(dirfd, path, flags, mode);
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

pub fn read(fd: RawFd, buf: &mut [u8]) -> isize {
    unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) }
}

pub fn write(fd: RawFd, buf: &[u8]) -> isize {
    unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) }
}

pub fn lseek(fd: RawFd, offset: i64, whence: libc::c_int) -> i64 {
    unsafe { libc::lseek(fd, offset as libc::off_t, whence) as i64 }
}

pub fn dup2(from: RawFd, to: RawFd) -> libc::c_int {
    let r = unsafe { libc::dup2(from, to) };
    if r >= 0 {
        crate::stdio_replaced(to);
    }
    r
}

/* A copy of fd that is at least min, closed on exec. */
pub fn dup_cloexec(fd: RawFd, min: RawFd) -> RawFd {
    unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, min) }
}

/* Is the descriptor open? A standard one that was closed at the start is not (see init). */
pub fn is_open(fd: RawFd) -> bool {
    !crate::stdio_closed(fd) && unsafe { libc::fcntl(fd, libc::F_GETFD) >= 0 }
}

pub fn isatty(fd: RawFd) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

/* The name of the terminal on fd (the caller clears errno first, to tell why it failed). */
pub fn ttyname(fd: RawFd) -> Option<Vec<u8>> {
    let p = unsafe { libc::ttyname(fd) };
    if p.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
    }
}

/* The width in columns of the terminal on fd, if it is one and says. */
pub fn terminal_columns(fd: RawFd) -> Option<usize> {
    let mut ws: libc::winsize = zeroed();
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
        Some(ws.ws_col as usize)
    } else {
        None
    }
}

/* ---- file names ---- */

pub fn unlink(path: &CStr) -> libc::c_int {
    unsafe { libc::unlink(path.as_ptr()) }
}

pub fn unlinkat(dirfd: RawFd, path: &CStr, flags: libc::c_int) -> libc::c_int {
    unsafe { libc::unlinkat(dirfd, path.as_ptr(), flags) }
}

pub fn mkdir(path: &CStr, mode: libc::mode_t) -> libc::c_int {
    unsafe { libc::mkdir(path.as_ptr(), mode) }
}

pub fn mkdirat(dirfd: RawFd, path: &CStr, mode: libc::mode_t) -> libc::c_int {
    unsafe { libc::mkdirat(dirfd, path.as_ptr(), mode) }
}

pub fn mkfifo(path: &CStr, mode: libc::mode_t) -> libc::c_int {
    unsafe { libc::mkfifo(path.as_ptr(), mode) }
}

pub fn mknod(path: &CStr, mode: libc::mode_t, dev: libc::dev_t) -> libc::c_int {
    unsafe { libc::mknod(path.as_ptr(), mode, dev) }
}

pub fn link(from: &CStr, to: &CStr) -> libc::c_int {
    unsafe { libc::link(from.as_ptr(), to.as_ptr()) }
}

/* linkat(2) between two directory descriptors (or AT_FDCWD); flags 0 or AT_SYMLINK_FOLLOW. */
pub fn linkat(olddirfd: RawFd, old: &CStr, newdirfd: RawFd, new: &CStr, flags: libc::c_int) -> libc::c_int {
    unsafe { libc::linkat(olddirfd, old.as_ptr(), newdirfd, new.as_ptr(), flags) }
}

pub fn symlink(target: &CStr, path: &CStr) -> libc::c_int {
    unsafe { libc::symlink(target.as_ptr(), path.as_ptr()) }
}

/* readlink(2) into buf; the number of bytes, or -1. */
pub fn readlink(path: &CStr, buf: &mut [u8]) -> isize {
    unsafe { libc::readlink(path.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) }
}

/* The canonical absolute name of path (realpath(3)). */
pub fn realpath(path: &CStr) -> Option<Vec<u8>> {
    let r = unsafe { libc::realpath(path.as_ptr(), std::ptr::null_mut()) };
    if r.is_null() {
        return None;
    }
    let v = unsafe { CStr::from_ptr(r) }.to_bytes().to_vec();
    unsafe { libc::free(r.cast()) };
    Some(v)
}

pub fn pathconf(path: &CStr, name: libc::c_int) -> libc::c_long {
    unsafe { libc::pathconf(path.as_ptr(), name) }
}

pub fn access(path: &CStr, mode: libc::c_int) -> libc::c_int {
    unsafe { libc::access(path.as_ptr(), mode) }
}

pub fn faccessat(dirfd: RawFd, path: &CStr, mode: libc::c_int, flags: libc::c_int) -> libc::c_int {
    unsafe { libc::faccessat(dirfd, path.as_ptr(), mode, flags) }
}

/* ---- file attributes ---- */

/*
 * Does the file have an extended access control list (a link itself, not what
 * it points to, when is_link)? Only macOS is covered; other systems say no.
 */
#[cfg(target_os = "macos")]
pub fn has_acl(path: &CStr, is_link: bool) -> bool {
    extern "C" {
        fn acl_get_file(path: *const libc::c_char, acl_type: libc::c_int) -> *mut libc::c_void;
        fn acl_get_link_np(path: *const libc::c_char, acl_type: libc::c_int) -> *mut libc::c_void;
        fn acl_free(obj: *mut libc::c_void) -> libc::c_int;
    }
    /* ACL_TYPE_EXTENDED of <sys/acl.h> */
    const EXTENDED: libc::c_int = 0x100;
    let acl = if is_link {
        unsafe { acl_get_link_np(path.as_ptr(), EXTENDED) }
    } else {
        unsafe { acl_get_file(path.as_ptr(), EXTENDED) }
    };
    if acl.is_null() {
        return false;
    }
    unsafe { acl_free(acl) };
    true
}

#[cfg(not(target_os = "macos"))]
pub fn has_acl(_path: &CStr, _is_link: bool) -> bool {
    false
}

pub fn stat(path: &CStr) -> io::Result<libc::stat> {
    let mut st: libc::stat = zeroed();
    result(unsafe { libc::stat(path.as_ptr(), &mut st) }).map(|_| st)
}

pub fn lstat(path: &CStr) -> io::Result<libc::stat> {
    let mut st: libc::stat = zeroed();
    result(unsafe { libc::lstat(path.as_ptr(), &mut st) }).map(|_| st)
}

pub fn fstat(fd: RawFd) -> io::Result<libc::stat> {
    let mut st: libc::stat = zeroed();
    result(unsafe { libc::fstat(fd, &mut st) }).map(|_| st)
}

pub fn fstatat(dirfd: RawFd, path: &CStr, flags: libc::c_int) -> io::Result<libc::stat> {
    let mut st: libc::stat = zeroed();
    result(unsafe { libc::fstatat(dirfd, path.as_ptr(), &mut st, flags) }).map(|_| st)
}

pub fn chmod(path: &CStr, mode: libc::mode_t) -> libc::c_int {
    unsafe { libc::chmod(path.as_ptr(), mode) }
}

/* flags must be 0: AT_SYMLINK_NOFOLLOW is not available on every system for chmod. */
pub fn fchmodat(dirfd: RawFd, path: &CStr, mode: libc::mode_t) -> libc::c_int {
    unsafe { libc::fchmodat(dirfd, path.as_ptr(), mode, 0) }
}


/* flags: 0 or AT_SYMLINK_NOFOLLOW (not supported everywhere for chmod) */
pub fn fchmodat_flags(dirfd: RawFd, path: &CStr, mode: libc::mode_t, flags: libc::c_int) -> libc::c_int {
    unsafe { libc::fchmodat(dirfd, path.as_ptr(), mode, flags) }
}

fn errno_is(e: &io::Error, codes: &[libc::c_int]) -> bool {
    e.raw_os_error().is_some_and(|n| codes.contains(&n))
}

/*
 * chmod of an entry seen as not a symbolic link (st), without following one
 * put there since: open with O_NOFOLLOW and fchmod() the descriptor, with the
 * mode taken from mode_of(stat of the opened file). Only regular files and
 * directories are opened. If neither read nor write is allowed, use
 * fchmodat(AT_SYMLINK_NOFOLLOW), and where that is not supported, plain
 * fchmodat() (the one case left open to the race).
 */
pub fn chmod_entry(
    dirfd: RawFd,
    name: &CStr,
    st: &libc::stat,
    mode_of: &mut dyn FnMut(&libc::stat) -> Option<libc::mode_t>,
) -> io::Result<()> {
    let kind = st.st_mode & libc::S_IFMT;
    if kind == libc::S_IFREG || kind == libc::S_IFDIR {
        for access in [libc::O_RDONLY, libc::O_WRONLY] {
            let flags = access | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC;
            match openat_owned(dirfd, name, flags, 0) {
                Ok(fd) => {
                    let now = fstat(std::os::fd::AsRawFd::as_raw_fd(&fd))?;
                    let Some(m) = mode_of(&now) else {
                        return Err(io::Error::from_raw_os_error(libc::EINVAL));
                    };
                    return result(fchmod(std::os::fd::AsRawFd::as_raw_fd(&fd), m));
                }
                /* a symbolic link now */
                Err(e) if errno_is(&e, &[libc::ELOOP, libc::EMLINK]) => {
                    return Err(io::Error::from_raw_os_error(libc::ELOOP));
                }
                Err(e) if errno_is(&e, &[libc::EACCES]) => continue,
                Err(_) => break,
            }
        }
    }
    let now = fstatat(dirfd, name, libc::AT_SYMLINK_NOFOLLOW)?;
    if now.st_mode & libc::S_IFMT == libc::S_IFLNK {
        return Err(io::Error::from_raw_os_error(libc::ELOOP));
    }
    let Some(m) = mode_of(&now) else {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    };
    if fchmodat_flags(dirfd, name, m, libc::AT_SYMLINK_NOFOLLOW) == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    if errno_is(&e, &[libc::ENOSYS, libc::EINVAL]) || e.raw_os_error() == Some(libc::ENOTSUP) || e.raw_os_error() == Some(libc::EOPNOTSUPP) {
        return result(fchmodat_flags(dirfd, name, m, 0));
    }
    Err(e)
}

pub fn fchmod(fd: RawFd, mode: libc::mode_t) -> libc::c_int {
    unsafe { libc::fchmod(fd, mode) }
}

pub fn chown(path: &CStr, uid: u32, gid: u32) -> libc::c_int {
    unsafe { libc::chown(path.as_ptr(), uid, gid) }
}

pub fn fchown(fd: RawFd, uid: u32, gid: u32) -> libc::c_int {
    unsafe { libc::fchown(fd, uid, gid) }
}

/* flags is 0 or AT_SYMLINK_NOFOLLOW; an ID of u32::MAX (-1) leaves that ID alone. */
pub fn fchownat(dirfd: RawFd, path: &CStr, uid: u32, gid: u32, flags: libc::c_int) -> libc::c_int {
    unsafe { libc::fchownat(dirfd, path.as_ptr(), uid, gid, flags) }
}

pub fn lchown(path: &CStr, uid: u32, gid: u32) -> libc::c_int {
    unsafe { libc::lchown(path.as_ptr(), uid, gid) }
}

/* Set the access and modification times; times[0] and times[1]. */
pub fn utimensat(dirfd: RawFd, path: &CStr, times: &[libc::timespec; 2], flags: libc::c_int) -> libc::c_int {
    unsafe { libc::utimensat(dirfd, path.as_ptr(), times.as_ptr(), flags) }
}

pub fn futimens(fd: RawFd, times: &[libc::timespec; 2]) -> libc::c_int {
    unsafe { libc::futimens(fd, times.as_ptr()) }
}

/* The process's file mode creation mask. */
pub fn umask() -> u32 {
    unsafe {
        let m = libc::umask(0);
        libc::umask(m);
        m as u32
    }
}

/* Set the mask; the old one. */
pub fn set_umask(mask: u32) -> u32 {
    unsafe { libc::umask(mask as libc::mode_t) as u32 }
}

/* ---- directories ---- */

/* Collect the names of an open directory stream and close it. */
fn names_of(dir: *mut libc::DIR) -> io::Result<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let r = loop {
        crate::clear_errno();
        let ent = unsafe { libc::readdir(dir) };
        if ent.is_null() {
            let e = io::Error::last_os_error();
            break if e.raw_os_error() == Some(0) { Ok(()) } else { Err(e) };
        }
        out.push(unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) }.to_bytes().to_vec());
    };
    unsafe { libc::closedir(dir) };
    r.map(|_| out)
}

/* Every name in a directory, "." and ".." included, in directory order. */
pub fn read_dir(path: &CStr) -> io::Result<Vec<Vec<u8>>> {
    let dir = unsafe { libc::opendir(path.as_ptr()) };
    if dir.is_null() {
        return Err(io::Error::last_os_error());
    }
    names_of(dir)
}

/* The same for an open directory descriptor, which stays open (a copy is read). */
pub fn read_dir_fd(fd: RawFd) -> io::Result<Vec<Vec<u8>>> {
    let d = dup_cloexec(fd, 3);
    if d < 0 {
        return Err(io::Error::last_os_error());
    }
    let dir = unsafe { libc::fdopendir(d) };
    if dir.is_null() {
        let e = io::Error::last_os_error();
        close(d);
        return Err(e);
    }
    names_of(dir)
}

/* ---- processes ---- */

pub fn kill(pid: libc::pid_t, sig: libc::c_int) -> libc::c_int {
    unsafe { libc::kill(pid, sig) }
}

/* Ignore a signal. */
pub fn ignore_signal(sig: libc::c_int) {
    unsafe { libc::signal(sig, libc::SIG_IGN) };
}

/* fork(2). Single thread only (see above): the child may then do anything before it execs. */
pub fn fork() -> libc::pid_t {
    unsafe { libc::fork() }
}

/* wait4(2) for one child: the result, the wait status and the resources it used. */
pub fn wait4(pid: libc::pid_t) -> (libc::pid_t, libc::c_int, libc::rusage) {
    let mut status: libc::c_int = 0;
    let mut ru: libc::rusage = zeroed();
    let r = unsafe { libc::wait4(pid, &mut status, 0, &mut ru) };
    (r, status, ru)
}

/* How a command that was run ended. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wait {
    Exited(i32),
    Signaled(i32),
}

/*
 * Run a command found through PATH (execvp) and wait for it. An error is the
 * failure to run it: the errno of fork, or of the exec itself, which the child
 * sends back through a pipe that closes on a successful exec, so that it
 * cannot be taken for an exit status of the command. With stdin_null the
 * command reads /dev/null instead of our standard input. Single thread only.
 */
pub fn run_command(argv: &[Vec<u8>], stdin_null: bool) -> io::Result<Wait> {
    if argv.is_empty() {
        return Err(io::Error::from_raw_os_error(libc::ENOENT));
    }
    let mut cargs = Vec::with_capacity(argv.len());
    for a in argv {
        match std::ffi::CString::new(a.as_slice()) {
            Ok(c) => cargs.push(c),
            Err(_) => return Err(io::Error::from_raw_os_error(libc::EINVAL)),
        }
    }
    let mut ptrs: Vec<*const libc::c_char> = cargs.iter().map(|c| c.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    let mut fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    for fd in fds {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    let null = if stdin_null { open(c"/dev/null", libc::O_RDONLY | libc::O_CLOEXEC, 0) } else { -1 };
    let pid = fork();
    if pid < 0 {
        let e = io::Error::last_os_error();
        close(fds[0]);
        close(fds[1]);
        if null >= 0 {
            close(null);
        }
        return Err(e);
    }
    if pid == 0 {
        /* the child does nothing but what is safe before an exec: no allocation */
        unsafe {
            if null >= 0 {
                libc::dup2(null, 0);
            }
            crate::inherited_signals();
            libc::execvp(ptrs[0], ptrs.as_ptr());
            let e: libc::c_int = io::Error::last_os_error().raw_os_error().unwrap_or(libc::EINVAL);
            libc::write(fds[1], (&e as *const libc::c_int).cast(), std::mem::size_of::<libc::c_int>());
            libc::_exit(127)
        }
    }
    close(fds[1]);
    if null >= 0 {
        close(null);
    }
    let mut buf = [0u8; std::mem::size_of::<libc::c_int>()];
    let n = loop {
        let n = read(fds[0], &mut buf);
        if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        break n;
    };
    close(fds[0]);
    let exec_errno = if n == buf.len() as isize { Some(libc::c_int::from_ne_bytes(buf)) } else { None };
    let status = loop {
        let (r, st, _) = wait4(pid);
        if r == pid {
            break st;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    if let Some(e) = exec_errno {
        return Err(io::Error::from_raw_os_error(e));
    }
    if libc::WIFSIGNALED(status) {
        Ok(Wait::Signaled(libc::WTERMSIG(status)))
    } else {
        Ok(Wait::Exited(libc::WEXITSTATUS(status)))
    }
}

/* {LINE_MAX}, 2048 if the system does not say */
pub fn line_max() -> usize {
    let n = unsafe { libc::sysconf(libc::_SC_LINE_MAX) };
    if n > 0 {
        n as usize
    } else {
        2048
    }
}

/* {ARG_MAX}: the most the system takes for the arguments and the environment of a command, if it says. */
pub fn arg_max() -> Option<usize> {
    let n = unsafe { libc::sysconf(libc::_SC_ARG_MAX) };
    if n > 0 {
        Some(n as usize)
    } else {
        None
    }
}

pub fn nice(increment: libc::c_int) -> libc::c_int {
    unsafe { libc::nice(increment) }
}

/* The login name of the user of the controlling terminal. */
pub fn getlogin() -> Option<Vec<u8>> {
    let p = unsafe { libc::getlogin() };
    if p.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
    }
}

/* The fields of uname(3), as bytes. */
pub struct Uname {
    pub sysname: Vec<u8>,
    pub nodename: Vec<u8>,
    pub release: Vec<u8>,
    pub version: Vec<u8>,
    pub machine: Vec<u8>,
}

pub fn uname() -> io::Result<Uname> {
    let mut u: libc::utsname = zeroed();
    result(unsafe { libc::uname(&mut u) })?;
    let field = |p: &[libc::c_char]| unsafe { CStr::from_ptr(p.as_ptr()) }.to_bytes().to_vec();
    Ok(Uname {
        sysname: field(&u.sysname),
        nodename: field(&u.nodename),
        release: field(&u.release),
        version: field(&u.version),
        machine: field(&u.machine),
    })
}
