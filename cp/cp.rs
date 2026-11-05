/* SPDX-License-Identifier: GPL-2.0-only */
/* cp - copy files. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use std::ffi::CStr;
use std::os::fd::RawFd;

const SYNOPSIS: &str = "cp [-Pfip] source_file target_file\n       cp [-Pfip] source_file... target\n       cp -R [-H|-L|-P] [-fip] source_file... target";

/* How much is read and written at a time. */
const BLOCK: usize = 128 * 1024;

/* Which symbolic links -R follows; the spec leaves the default open and -P is the safe choice. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Link {
    Default,
    H,
    L,
    P,
}

#[derive(Clone, Copy)]
struct Opts {
    force: bool,
    interactive: bool,
    preserve: bool,
    recursive: bool,
    link: Link,
}

impl Opts {
    /* Is a symbolic link named as an operand followed (true) or copied as a link? */
    fn follow_operand(&self) -> bool {
        if self.recursive {
            matches!(self.link, Link::H | Link::L)
        } else {
            self.link != Link::P
        }
    }

    /* Is a symbolic link met inside a hierarchy followed? Only with -L. */
    fn follow_traversed(&self) -> bool {
        self.recursive && self.link == Link::L
    }
}

type Id = (u64, u64);

fn id_of(st: &libc::stat) -> Id {
    (st.st_dev as u64, st.st_ino as u64)
}

fn kind(st: &libc::stat) -> libc::mode_t {
    st.st_mode & libc::S_IFMT
}

fn is_dir(st: &libc::stat) -> bool {
    kind(st) == libc::S_IFDIR
}

fn last_err() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn stat_path(path: &[u8], follow: bool) -> std::io::Result<libc::stat> {
    let c = posix::cstr(path);
    if follow {
        posix::sys::stat(&c)
    } else {
        posix::sys::lstat(&c)
    }
}

fn fstat(fd: RawFd) -> std::io::Result<libc::stat> {
    posix::sys::fstat(fd)
}

/* The last component of a pathname, without trailing slashes ("" for a pathname of only slashes). */
fn last_component(p: &[u8]) -> &[u8] {
    let mut n = p.len();
    while n > 0 && p[n - 1] == b'/' {
        n -= 1;
    }
    let s = &p[..n];
    match s.iter().rposition(|&c| c == b'/') {
        Some(i) => &s[i + 1..],
        None => s,
    }
}

/* dir + "/" + name, with the slash left out if dir already ends in one. */
fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut v = dir.to_vec();
    if v.last() != Some(&b'/') {
        v.push(b'/');
    }
    v.extend_from_slice(name);
    v
}

/* The destination path of a source copied into a directory target. A source that ends in ".." is
   a directory named by its parent link, as one that ends in "." is by itself: its contents go
   into the target. Taken at its word ("target/..") the files would land beside the target. */
fn dest_in(target: &[u8], src: &[u8]) -> Vec<u8> {
    let base = last_component(src);
    if base.is_empty() {
        target.to_vec()
    } else if base == b".." {
        join(target, b".")
    } else {
        join(target, base)
    }
}

/* The names in a directory, except "." and "..", sorted. */
fn read_names(path: &[u8]) -> std::io::Result<Vec<Vec<u8>>> {
    let c = posix::cstr(path);
    let mut out: Vec<Vec<u8>> = posix::sys::read_dir(&c)?.into_iter().filter(|n| n != b"." && n != b"..").collect();
    out.sort();
    Ok(out)
}

fn read_link(path: &[u8]) -> std::io::Result<Vec<u8>> {
    let c = posix::cstr(path);
    let mut size = 256usize;
    loop {
        let mut buf = vec![0u8; size];
        let n = posix::sys::readlink(&c, &mut buf);
        if n < 0 {
            return Err(last_err());
        }
        if (n as usize) < size {
            buf.truncate(n as usize);
            return Ok(buf);
        }
        size *= 4;
    }
}

fn timespecs(st: &libc::stat) -> [libc::timespec; 2] {
    [
        libc::timespec { tv_sec: st.st_atime, tv_nsec: st.st_atime_nsec as _ },
        libc::timespec { tv_sec: st.st_mtime, tv_nsec: st.st_mtime_nsec as _ },
    ]
}

/* The real path of an existing file, as bytes. */
fn real_path(p: &[u8]) -> Option<Vec<u8>> {
    posix::sys::realpath(&posix::cstr(p))
}

/* Is dest (not yet created) the directory src itself or somewhere below it? */
fn inside(src: &[u8], dest: &[u8]) -> bool {
    let Some(s) = real_path(src) else { return false };
    let base = last_component(dest);
    let parent_part: &[u8] = {
        let stripped_len = {
            let mut n = dest.len();
            while n > 0 && dest[n - 1] == b'/' {
                n -= 1;
            }
            n
        };
        match dest[..stripped_len].iter().rposition(|&c| c == b'/') {
            Some(0) => b"/",
            Some(i) => &dest[..i],
            None => b".",
        }
    };
    let Some(p) = real_path(parent_part) else { return false };
    let mut d = p;
    if !base.is_empty() {
        d = join(&d, base);
    }
    let mut prefix = s.clone();
    if prefix.last() != Some(&b'/') {
        prefix.push(b'/');
    }
    d == s || d.starts_with(&prefix) || s == b"/"
}

/* the question asked of the user: true means yes */
type Ask<'a> = Box<dyn FnMut(&[u8]) -> bool + 'a>;

struct Cp<'a> {
    prog: Vec<u8>,
    o: Opts,
    umask: u32,
    status: i32,
    ask: Ask<'a>,
    /* destination directories being filled: a source directory that is one of them is being copied into itself */
    active: Vec<Id>,
    /* source directories on the way down: one met again is a symbolic link cycle (-L) */
    walking: Vec<Id>,
    buf: Vec<u8>,
    /* the unit tests collect diagnostics here instead of writing them */
    log: Option<Vec<String>>,
}

impl<'a> Cp<'a> {
    fn new(prog: &[u8], o: Opts, umask: u32, ask: Ask<'a>) -> Self {
        Cp { prog: prog.to_vec(), o, umask, status: 0, ask, active: Vec::new(), walking: Vec::new(), buf: vec![0u8; BLOCK], log: None }
    }

    fn err(&mut self, path: &[u8], msg: &str) {
        self.status = 1;
        match self.log.as_mut() {
            Some(l) => l.push(format!("{}: {}", String::from_utf8_lossy(path), msg)),
            None => posix::warn_path(&self.prog, path, msg),
        }
    }

    fn err_io(&mut self, path: &[u8], e: &std::io::Error) {
        self.err(path, &posix::strerror(e));
    }

    /* A diagnostic about two files, e.g. "a and b are identical (not copied)". */
    fn err_two(&mut self, a: &[u8], b: &[u8], msg: &str) {
        let mut p = a.to_vec();
        p.extend_from_slice(b" and ");
        p.extend_from_slice(b);
        self.err(&p, msg);
    }

    fn confirm(&mut self, dest: &[u8]) -> bool {
        let mut p = Vec::new();
        p.extend_from_slice(&self.prog);
        p.extend_from_slice(b": overwrite '");
        p.extend_from_slice(dest);
        p.extend_from_slice(b"'? ");
        (self.ask)(&p)
    }

    /* One source file (an operand, or something found in a hierarchy) to one destination path. */
    fn copy_one(&mut self, src: &[u8], dest: &[u8], operand: bool) {
        let follow = if operand { self.o.follow_operand() } else { self.o.follow_traversed() };
        let st = match stat_path(src, follow) {
            Ok(s) => s,
            Err(e) => {
                self.err_io(src, &e);
                return;
            }
        };
        match kind(&st) {
            libc::S_IFDIR => {
                if self.o.recursive {
                    self.copy_dir(src, dest, &st, operand);
                } else {
                    self.err(src, "is a directory (not copied)");
                }
            }
            libc::S_IFLNK => self.copy_link(src, dest, &st),
            libc::S_IFREG => self.copy_regular(src, dest, &st, !follow),
            /* FIFOs, devices and sockets: recreated with -R, read as data without it */
            _ if self.o.recursive => self.copy_special(src, dest, &st),
            _ => self.copy_regular(src, dest, &st, !follow),
        }
    }

    /* Step 3 for a file whose contents are copied. */
    fn copy_regular(&mut self, src: &[u8], dest: &[u8], st: &libc::stat, nofollow: bool) {
        let cd = posix::cstr(dest);
        /* does the destination exist? open() follows a link, so this does too */
        let exists = match stat_path(dest, true) {
            Ok(d) => {
                if id_of(&d) == id_of(st) {
                    self.err_two(src, dest, "are identical (not copied)");
                    return;
                }
                if is_dir(&d) {
                    self.err(dest, "cannot overwrite a directory with a non-directory");
                    return;
                }
                true
            }
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => false,
            /* a link loop cannot be opened, which is what -f is for: it is removed and made again */
            Err(e) if e.raw_os_error() == Some(libc::ELOOP) && self.o.force && matches!(stat_path(dest, false), Ok(l) if kind(&l) == libc::S_IFLNK) => true,
            Err(e) => {
                self.err_io(dest, &e);
                return;
            }
        };
        if exists && self.o.interactive && !self.confirm(dest) {
            return;
        }
        /* the source is opened first: an unreadable source must not cost an existing destination its contents */
        let cs = posix::cstr(src);
        let mut sflags = libc::O_RDONLY | libc::O_NOCTTY | libc::O_CLOEXEC;
        if nofollow {
            sflags |= libc::O_NOFOLLOW;
        }
        let sfd = posix::sys::open(&cs, sflags, 0);
        if sfd < 0 {
            let e = last_err();
            self.err_io(src, &e);
            return;
        }
        /* the file as it is now: its times before our own read touches them */
        let sst = fstat(sfd).unwrap_or(*st);
        let dfd = match self.open_dest(&cd, dest, exists, sst.st_mode & 0o777) {
            Some(fd) => fd,
            None => {
                posix::sys::close(sfd);
                return;
            }
        };
        let mut ok = true;
        match self.copy_data(sfd, dfd) {
            Ok(()) => {}
            Err((is_read, e)) => {
                self.err_io(if is_read { src } else { dest }, &e);
                ok = false;
            }
        }
        if ok && self.o.preserve {
            self.preserve_fd(dfd, dest, &sst);
        }
        posix::sys::close(sfd);
        if posix::sys::close(dfd) < 0 {
            let e = last_err();
            self.err_io(dest, &e);
        }
    }

    /* Steps 3a.ii to 3c: a descriptor to write the copy to, or None (reported). */
    fn open_dest(&mut self, cd: &CStr, dest: &[u8], exists: bool, mode: libc::mode_t) -> Option<RawFd> {
        let base = libc::O_WRONLY | libc::O_NOCTTY | libc::O_CLOEXEC;
        if exists {
            let fd = posix::sys::open(cd, base | libc::O_TRUNC, 0);
            if fd >= 0 {
                return Some(fd);
            }
            let e = last_err();
            if !self.o.force {
                self.err_io(dest, &e);
                return None;
            }
            /* -f: remove it and make a new one */
            if posix::sys::unlink(cd) < 0 {
                self.err_io(dest, &e);
                return None;
            }
        }
        /* a dangling link as destination is written through, as open() does; anything else is made anew */
        let through = matches!(stat_path(dest, false), Ok(l) if kind(&l) == libc::S_IFLNK);
        let flags = if through { base | libc::O_CREAT | libc::O_TRUNC } else { base | libc::O_CREAT | libc::O_EXCL };
        let fd = posix::sys::open(cd, flags, mode as libc::c_uint);
        if fd < 0 {
            let e = last_err();
            self.err_io(dest, &e);
            return None;
        }
        Some(fd)
    }

    /* The loop of step 3d. Err: (was it a read error, the error). */
    fn copy_data(&mut self, sfd: RawFd, dfd: RawFd) -> Result<(), (bool, std::io::Error)> {
        loop {
            let n = posix::sys::read(sfd, &mut self.buf);
            if n < 0 {
                let e = last_err();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err((true, e));
            }
            if n == 0 {
                return Ok(());
            }
            let mut off = 0usize;
            while off < n as usize {
                let w = posix::sys::write(dfd, &self.buf[off..n as usize]);
                if w < 0 {
                    let e = last_err();
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err((false, e));
                }
                /* nothing written and no error: give up rather than spin */
                if w == 0 {
                    return Err((false, std::io::Error::from(std::io::ErrorKind::WriteZero)));
                }
                off += w as usize;
            }
        }
    }

    /*
     * -p for a file that is open: owner and group, then the permission bits
     * (the set-ID bits go if the owner or group could not be given), then
     * the times, last, so that nothing after them can change the modification time.
     */
    fn preserve_fd(&mut self, dfd: RawFd, dest: &[u8], sst: &libc::stat) {
        let owner_ok = posix::sys::fchown(dfd, sst.st_uid, sst.st_gid) == 0;
        let mut mode = sst.st_mode & 0o7777;
        if !owner_ok {
            mode &= !(libc::S_ISUID | libc::S_ISGID);
        }
        if posix::sys::fchmod(dfd, mode) < 0 {
            let e = last_err();
            self.err_io(dest, &e);
        }
        let ts = timespecs(sst);
        if posix::sys::futimens(dfd, &ts) < 0 {
            let e = last_err();
            self.err_io(dest, &e);
        }
    }

    /* The same for something named by a path: a directory, a FIFO, a device. */
    fn preserve_path(&mut self, dest: &[u8], sst: &libc::stat) {
        let cd = posix::cstr(dest);
        let owner_ok = posix::sys::chown(&cd, sst.st_uid, sst.st_gid) == 0;
        let mut mode = sst.st_mode & 0o7777;
        if !owner_ok {
            mode &= !(libc::S_ISUID | libc::S_ISGID);
        }
        if posix::sys::chmod(&cd, mode) < 0 {
            let e = last_err();
            self.err_io(dest, &e);
        }
        let ts = timespecs(sst);
        if posix::sys::utimensat(libc::AT_FDCWD, &cd, &ts, 0) < 0 {
            let e = last_err();
            self.err_io(dest, &e);
        }
    }

    /* Step 2: a directory and everything below it. */
    fn copy_dir(&mut self, src: &[u8], dest: &[u8], st: &libc::stat, operand: bool) {
        if self.active.contains(&id_of(st)) {
            self.err(src, "cannot copy a directory into itself");
            return;
        }
        /* only a symbolic link can lead back to a directory above: nothing is made for it */
        if self.walking.contains(&id_of(st)) {
            self.err(src, "directory causes a cycle");
            return;
        }
        let cd = posix::cstr(dest);
        let created;
        let dst = match stat_path(dest, true) {
            Ok(d) => {
                if id_of(&d) == id_of(st) {
                    self.err_two(src, dest, "are identical (not copied)");
                    return;
                }
                if !is_dir(&d) {
                    self.err(dest, "cannot overwrite a non-directory with a directory");
                    return;
                }
                created = false;
                d
            }
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => {
                if operand && inside(src, dest) {
                    self.err_two(src, dest, "a directory cannot be copied into itself");
                    return;
                }
                /* the owner can always enter it and write to it while it is being filled */
                let mode = ((st.st_mode & 0o777) | 0o700) as libc::mode_t;
                if posix::sys::mkdir(&cd, mode) < 0 {
                    let e = last_err();
                    self.err_io(dest, &e);
                    return;
                }
                created = true;
                match stat_path(dest, true) {
                    Ok(d) => d,
                    Err(e) => {
                        self.err_io(dest, &e);
                        return;
                    }
                }
            }
            Err(e) => {
                self.err_io(dest, &e);
                return;
            }
        };
        self.active.push(id_of(&dst));
        self.walking.push(id_of(st));
        match read_names(src) {
            Ok(names) => {
                for n in names {
                    self.copy_one(&join(src, &n), &join(dest, &n), false);
                }
            }
            Err(e) => self.err_io(src, &e),
        }
        self.walking.pop();
        self.active.pop();
        /* step 2g: the final permissions, now that nothing more is added. -p is about every source
           file and "the corresponding destination file": the owner, the permissions and the times
           go to a directory that was already there as well, as GNU and BSD do. */
        if self.o.preserve {
            self.preserve_path(dest, st);
        } else if created {
            let mode = (st.st_mode & 0o777) & !self.umask as libc::mode_t;
            if posix::sys::chmod(&cd, mode) < 0 {
                let e = last_err();
                self.err_io(dest, &e);
            }
        }
    }

    /* Make room for something that is not a regular file: the destination must go first. */
    fn clear_dest(&mut self, src: &[u8], dest: &[u8], st: &libc::stat) -> bool {
        match stat_path(dest, false) {
            Ok(d) => {
                if id_of(&d) == id_of(st) {
                    self.err_two(src, dest, "are identical (not copied)");
                    return false;
                }
                if is_dir(&d) {
                    self.err(dest, "cannot overwrite a directory with a non-directory");
                    return false;
                }
                if self.o.interactive && !self.confirm(dest) {
                    return false;
                }
                if posix::sys::unlink(&posix::cstr(dest)) < 0 {
                    let e = last_err();
                    self.err_io(dest, &e);
                    return false;
                }
                true
            }
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => true,
            Err(e) => {
                self.err_io(dest, &e);
                false
            }
        }
    }

    /* Step 4c: a symbolic link copied as a link. */
    fn copy_link(&mut self, src: &[u8], dest: &[u8], st: &libc::stat) {
        let target = match read_link(src) {
            Ok(t) => t,
            Err(e) => {
                self.err_io(src, &e);
                return;
            }
        };
        /* step 1: the same file. The destination is replaced by a new link, so a source link that
           leads to the destination (or to another name of it) would cost the file its contents:
           "la -> a" with "cp -P la a" must not leave "a -> a". A destination that is a link itself is
           only replaced, which is fine, so it is looked at without following. */
        if let (Ok(through), Ok(d)) = (stat_path(src, true), stat_path(dest, false)) {
            if kind(&d) != libc::S_IFLNK && id_of(&through) == id_of(&d) {
                self.err_two(src, dest, "are identical (not copied)");
                return;
            }
        }
        if !self.clear_dest(src, dest, st) {
            return;
        }
        if posix::sys::symlink(&posix::cstr(&target), &posix::cstr(dest)) < 0 {
            let e = last_err();
            self.err_io(dest, &e);
            return;
        }
        if self.o.preserve {
            let cd = posix::cstr(dest);
            posix::sys::lchown(&cd, st.st_uid, st.st_gid);
            let ts = timespecs(st);
            if posix::sys::utimensat(libc::AT_FDCWD, &cd, &ts, libc::AT_SYMLINK_NOFOLLOW) < 0 {
                let e = last_err();
                self.err_io(dest, &e);
            }
        }
    }

    /* Step 4a and 4b: a FIFO, a device or a socket, made again with the same type. */
    fn copy_special(&mut self, src: &[u8], dest: &[u8], st: &libc::stat) {
        if !self.clear_dest(src, dest, st) {
            return;
        }
        let cd = posix::cstr(dest);
        let r = if kind(st) == libc::S_IFIFO {
            posix::sys::mkfifo(&cd, st.st_mode & 0o777)
        } else {
            posix::sys::mknod(&cd, st.st_mode & (libc::S_IFMT | 0o777), st.st_rdev)
        };
        if r < 0 {
            let e = last_err();
            self.err_io(dest, &e);
            return;
        }
        if self.o.preserve {
            self.preserve_path(dest, st);
        }
    }
}

/* a source and its destination */
type Pair = (Vec<u8>, Vec<u8>);

/*
 * What the operands ask for: the (source, destination) pairs, or why it
 * cannot be done. Nothing is copied when it cannot.
 */
fn plan(sources: &[Vec<u8>], target: &[u8], target_is_dir: bool) -> Result<Vec<Pair>, &'static str> {
    if target_is_dir {
        return Ok(sources.iter().map(|s| (s.clone(), dest_in(target, s))).collect());
    }
    if sources.len() > 1 {
        return Err("not a directory");
    }
    Ok(vec![(sources[0].clone(), target.to_vec())])
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut o = Opts { force: false, interactive: false, preserve: false, recursive: false, link: Link::Default };
    let mut g = Getopt::new(&args, b"PfipRHLr");
    for opt in g.by_ref() {
        match opt {
            Opt::Flag(b'f') => o.force = true,
            Opt::Flag(b'i') => o.interactive = true,
            Opt::Flag(b'p') => o.preserve = true,
            /* -r: removed from the standard, kept by every implementation, the same as -R here */
            Opt::Flag(b'R') | Opt::Flag(b'r') => o.recursive = true,
            Opt::Flag(b'H') => o.link = Link::H,
            Opt::Flag(b'L') => o.link = Link::L,
            Opt::Flag(b'P') => o.link = Link::P,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let operands = g.operands();
    if operands.len() < 2 {
        posix::usage(SYNOPSIS);
    }
    let (sources, target) = (&operands[..operands.len() - 1], &operands[operands.len() - 1]);
    let target_is_dir = matches!(stat_path(target, true), Ok(s) if is_dir(&s));
    let pairs = match plan(sources, target, target_is_dir) {
        Ok(p) => p,
        Err(msg) => {
            posix::warn_path(&prog, target, msg);
            std::process::exit(1);
        }
    };
    let umask = posix::sys::umask();
    let yes = posix::prompt::Yes::new();
    let ask = Box::new(move |prompt: &[u8]| -> bool { posix::prompt::ask(&yes, prompt) });
    let mut cp = Cp::new(&prog, o, umask, ask);
    for (s, d) in &pairs {
        cp.copy_one(s, d, true);
    }
    std::process::exit(cp.status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("cp-test-{}-{}", std::process::id(), n));
            std::fs::create_dir(&p).unwrap();
            Tmp(p)
        }
        fn p(&self, s: &str) -> Vec<u8> {
            self.0.join(s).as_os_str().as_bytes().to_vec()
        }
        fn path(&self, s: &str) -> PathBuf {
            self.0.join(s)
        }
        fn write(&self, s: &str, c: &str) {
            std::fs::write(self.path(s), c).unwrap();
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            fn fix(p: &Path) {
                if let Ok(m) = std::fs::symlink_metadata(p) {
                    if m.is_dir() {
                        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
                        if let Ok(rd) = std::fs::read_dir(p) {
                            for e in rd.flatten() {
                                fix(&e.path());
                            }
                        }
                    }
                }
            }
            fix(&self.0);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn opts() -> Opts {
        Opts { force: false, interactive: false, preserve: false, recursive: false, link: Link::Default }
    }

    fn current_umask() -> u32 {
        posix::sys::umask()
    }

    /* run one copy of src to dest as the operands of cp would be; answers feed the -i prompts */
    fn run(o: Opts, answers: &[bool], pairs: &[(Vec<u8>, Vec<u8>)]) -> (i32, Vec<String>, Vec<String>) {
        let prompts = std::cell::RefCell::new(Vec::new());
        let mut it = answers.iter().copied();
        let (status, log) = {
            let ask = Box::new(|p: &[u8]| {
                prompts.borrow_mut().push(String::from_utf8_lossy(p).into_owned());
                it.next().unwrap_or(false)
            });
            let mut cp = Cp::new(b"cp", o, current_umask(), ask);
            cp.log = Some(Vec::new());
            for (s, d) in pairs {
                cp.copy_one(s, d, true);
            }
            (cp.status, cp.log.take().unwrap())
        };
        (status, log, prompts.into_inner())
    }

    fn one(o: Opts, s: Vec<u8>, d: Vec<u8>) -> (i32, Vec<String>) {
        let (st, log, _) = run(o, &[], &[(s, d)]);
        (st, log)
    }

    #[test]
    fn components_and_joins() {
        assert_eq!(last_component(b"a/b"), b"b");
        assert_eq!(last_component(b"a/b/"), b"b");
        assert_eq!(last_component(b"a/b//"), b"b");
        assert_eq!(last_component(b"b"), b"b");
        assert_eq!(last_component(b"/"), b"");
        assert_eq!(last_component(b"///"), b"");
        assert_eq!(last_component(b""), b"");
        assert_eq!(last_component(b"x/."), b".");
        assert_eq!(last_component(b"x/.."), b"..");
        assert_eq!(join(b"d", b"f"), b"d/f");
        assert_eq!(join(b"d/", b"f"), b"d/f");
        assert_eq!(join(b"/", b"f"), b"/f");
        assert_eq!(dest_in(b"t", b"a/b"), b"t/b");
        assert_eq!(dest_in(b"t/", b"a/b/"), b"t/b");
        assert_eq!(dest_in(b"t", b"/"), b"t");
        assert_eq!(dest_in(b"t", b"."), b"t/.");
        assert_eq!(dest_in(b"t", b".."), b"t/.");
        assert_eq!(dest_in(b"t/", b"a/b/.."), b"t/.");
        assert_eq!(dest_in(b"t", b"a/..b"), b"t/..b");
    }

    #[test]
    fn link_policy_table() {
        let mk = |r, l| Opts { recursive: r, link: l, ..opts() };
        /* without -R every symbolic link is followed, except with -P */
        assert!(mk(false, Link::Default).follow_operand());
        assert!(mk(false, Link::H).follow_operand());
        assert!(mk(false, Link::L).follow_operand());
        assert!(!mk(false, Link::P).follow_operand());
        assert!(!mk(false, Link::Default).follow_traversed());
        /* with -R: -H and -L follow operands, -L also what is found; the default is -P */
        assert!(!mk(true, Link::Default).follow_operand());
        assert!(mk(true, Link::H).follow_operand());
        assert!(mk(true, Link::L).follow_operand());
        assert!(!mk(true, Link::P).follow_operand());
        assert!(!mk(true, Link::Default).follow_traversed());
        assert!(!mk(true, Link::H).follow_traversed());
        assert!(mk(true, Link::L).follow_traversed());
        assert!(!mk(true, Link::P).follow_traversed());
    }

    #[test]
    fn plan_forms() {
        let s = |x: &[&str]| x.iter().map(|y| y.as_bytes().to_vec()).collect::<Vec<_>>();
        /* form 1: two operands, the target not a directory */
        assert_eq!(plan(&s(&["a"]), b"b", false), Ok(vec![(b"a".to_vec(), b"b".to_vec())]));
        /* form 2: into a directory */
        assert_eq!(
            plan(&s(&["a", "x/b"]), b"d", true),
            Ok(vec![(b"a".to_vec(), b"d/a".to_vec()), (b"x/b".to_vec(), b"d/b".to_vec())])
        );
        assert_eq!(plan(&s(&["a"]), b"d/", true), Ok(vec![(b"a".to_vec(), b"d/a".to_vec())]));
        /* several sources and no directory: an error, nothing to do */
        assert!(plan(&s(&["a", "b"]), b"t", false).is_err());
    }

    #[test]
    fn copies_to_a_new_name_with_the_mode_of_the_source_less_the_umask() {
        let t = Tmp::new();
        t.write("a", "hello");
        std::fs::set_permissions(t.path("a"), std::fs::Permissions::from_mode(0o751)).unwrap();
        assert_eq!(one(opts(), t.p("a"), t.p("b")), (0, vec![]));
        assert_eq!(std::fs::read(t.path("b")).unwrap(), b"hello");
        assert_eq!(std::fs::metadata(t.path("b")).unwrap().mode() & 0o7777, 0o751 & !current_umask());
    }

    #[test]
    fn setid_bits_are_not_copied_without_p() {
        let t = Tmp::new();
        t.write("a", "x");
        std::fs::set_permissions(t.path("a"), std::fs::Permissions::from_mode(0o6755)).unwrap();
        assert_eq!(one(opts(), t.p("a"), t.p("b")).0, 0);
        assert_eq!(std::fs::metadata(t.path("b")).unwrap().mode() & 0o7000, 0);
    }

    #[test]
    fn existing_destination_keeps_its_mode_and_loses_its_contents() {
        let t = Tmp::new();
        t.write("a", "new");
        t.write("b", "old contents that are longer");
        std::fs::set_permissions(t.path("b"), std::fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(one(opts(), t.p("a"), t.p("b")).0, 0);
        assert_eq!(std::fs::read(t.path("b")).unwrap(), b"new");
        assert_eq!(std::fs::metadata(t.path("b")).unwrap().mode() & 0o777, 0o640);
    }

    #[test]
    fn empty_and_large_files_and_all_byte_values() {
        let t = Tmp::new();
        std::fs::write(t.path("empty"), b"").unwrap();
        assert_eq!(one(opts(), t.p("empty"), t.p("e2")).0, 0);
        assert_eq!(std::fs::metadata(t.path("e2")).unwrap().len(), 0);
        let big: Vec<u8> = (0..(BLOCK * 3 + 17)).map(|i| (i * 7 + i / 251) as u8).collect();
        std::fs::write(t.path("big"), &big).unwrap();
        assert_eq!(one(opts(), t.p("big"), t.p("b2")).0, 0);
        assert_eq!(std::fs::read(t.path("b2")).unwrap(), big);
        let all: Vec<u8> = (0..=255u8).collect();
        std::fs::write(t.path("all"), &all).unwrap();
        assert_eq!(one(opts(), t.p("all"), t.p("a2")).0, 0);
        assert_eq!(std::fs::read(t.path("a2")).unwrap(), all);
    }

    #[test]
    fn same_file_is_skipped_with_a_diagnostic() {
        let t = Tmp::new();
        t.write("a", "keep");
        let (st, log) = one(opts(), t.p("a"), t.p("a"));
        assert_eq!((st, log.len()), (1, 1));
        assert_eq!(std::fs::read(t.path("a")).unwrap(), b"keep");
        /* a hard link and a symbolic link to it are the same file too */
        std::fs::hard_link(t.path("a"), t.path("h")).unwrap();
        std::os::unix::fs::symlink("a", t.path("l")).unwrap();
        assert_eq!(one(opts(), t.p("a"), t.p("h")).0, 1);
        assert_eq!(one(opts(), t.p("a"), t.p("l")).0, 1);
        assert_eq!(one(opts(), t.p("l"), t.p("a")).0, 1);
        assert_eq!(std::fs::read(t.path("a")).unwrap(), b"keep");
        /* through a differently spelled path */
        std::fs::create_dir(t.path("d")).unwrap();
        let mut dotted = t.0.as_os_str().as_bytes().to_vec();
        dotted.extend_from_slice(b"/d/../a");
        assert_eq!(one(opts(), t.p("a"), dotted).0, 1);
        assert_eq!(std::fs::read(t.path("a")).unwrap(), b"keep");
    }

    #[test]
    fn a_link_copied_as_a_link_never_replaces_the_file_it_leads_to() {
        let t = Tmp::new();
        t.write("a", "keep");
        std::os::unix::fs::symlink("a", t.path("la")).unwrap();
        std::os::unix::fs::symlink("la", t.path("lla")).unwrap();
        std::os::unix::fs::symlink("a", t.path("la2")).unwrap();
        std::fs::hard_link(t.path("a"), t.path("ha")).unwrap();
        let o = || Opts { link: Link::P, ..opts() };
        let intact = |t: &Tmp| {
            assert_eq!(std::fs::read(t.path("a")).unwrap(), b"keep");
            assert!(!std::fs::symlink_metadata(t.path("a")).unwrap().file_type().is_symlink());
            assert!(!std::fs::symlink_metadata(t.path("ha")).unwrap().file_type().is_symlink());
        };
        /* the link, a chain of links, and another name of the file: all refused */
        for (src, dst) in [("la", "a"), ("lla", "a"), ("la", "ha"), ("lla", "ha")] {
            let (st, log) = one(o(), t.p(src), t.p(dst));
            assert_eq!((st, log.len()), (1, 1), "{src} {dst}");
            intact(&t);
        }
        /* with -R, which also copies links as links */
        let r = || Opts { recursive: true, ..o() };
        assert_eq!(one(r(), t.p("la"), t.p("a")).0, 1);
        intact(&t);
        /* a destination that is only another link to the file is replaced, as it always was */
        assert_eq!(one(o(), t.p("la2"), t.p("la")).0, 0);
        assert_eq!(std::fs::read_link(t.path("la")).unwrap().to_str(), Some("a"));
        intact(&t);
        /* a link that dangles has nothing to protect */
        std::os::unix::fs::symlink("nowhere", t.path("dang")).unwrap();
        assert_eq!(one(o(), t.p("dang"), t.p("copy")).0, 0);
        assert_eq!(std::fs::read_link(t.path("copy")).unwrap().to_str(), Some("nowhere"));
    }

    #[test]
    fn source_is_opened_before_the_destination_is_touched() {
        if posix::user::euid() == 0 {
            return;
        }
        let t = Tmp::new();
        t.write("src", "secret");
        t.write("dst", "precious");
        std::fs::set_permissions(t.path("src"), std::fs::Permissions::from_mode(0o0)).unwrap();
        let (st, log) = one(opts(), t.p("src"), t.p("dst"));
        assert_eq!((st, log.len()), (1, 1));
        assert_eq!(std::fs::read(t.path("dst")).unwrap(), b"precious");
    }

    #[test]
    fn missing_source_is_an_error_and_the_next_one_is_done() {
        let t = Tmp::new();
        t.write("b", "b!");
        std::fs::create_dir(t.path("d")).unwrap();
        let (st, log, _) = run(opts(), &[], &[(t.p("nosuch"), t.p("d/nosuch")), (t.p("b"), t.p("d/b"))]);
        assert_eq!((st, log.len()), (1, 1));
        assert_eq!(std::fs::read(t.path("d/b")).unwrap(), b"b!");
        assert!(!t.path("d/nosuch").exists());
    }

    #[test]
    fn interactive_asks_only_for_existing_destinations() {
        let t = Tmp::new();
        t.write("a", "A");
        t.write("b", "B");
        t.write("old", "O");
        /* a new name: no question */
        let (st, _, pr) = run(Opts { interactive: true, ..opts() }, &[], &[(t.p("a"), t.p("fresh"))]);
        assert_eq!((st, pr.len()), (0, 0));
        /* an existing one: asked, and the prompt names it */
        let (st, _, pr) = run(Opts { interactive: true, ..opts() }, &[false], &[(t.p("a"), t.p("old"))]);
        assert_eq!((st, pr.len()), (0, 1));
        assert!(pr[0].contains("/old"));
        assert_eq!(std::fs::read(t.path("old")).unwrap(), b"O");
        let (st, _, pr) = run(Opts { interactive: true, ..opts() }, &[true], &[(t.p("a"), t.p("old"))]);
        assert_eq!((st, pr.len()), (0, 1));
        assert_eq!(std::fs::read(t.path("old")).unwrap(), b"A");
        /* -i -f still asks */
        t.write("old", "O");
        let (_, _, pr) = run(Opts { interactive: true, force: true, ..opts() }, &[false], &[(t.p("a"), t.p("old"))]);
        assert_eq!(pr.len(), 1);
        assert_eq!(std::fs::read(t.path("old")).unwrap(), b"O");
    }

    #[test]
    fn without_f_an_unwritable_destination_is_an_error_with_f_it_is_replaced() {
        if posix::user::euid() == 0 {
            return;
        }
        let t = Tmp::new();
        t.write("a", "new");
        t.write("ro", "old");
        std::fs::set_permissions(t.path("ro"), std::fs::Permissions::from_mode(0o444)).unwrap();
        let (st, log) = one(opts(), t.p("a"), t.p("ro"));
        assert_eq!((st, log.len()), (1, 1));
        assert_eq!(std::fs::read(t.path("ro")).unwrap(), b"old");
        let (st, log) = one(Opts { force: true, ..opts() }, t.p("a"), t.p("ro"));
        assert_eq!((st, log.len()), (0, 0));
        assert_eq!(std::fs::read(t.path("ro")).unwrap(), b"new");
        /* the replacement is a new file with the source's mode, not the old one's */
        assert_eq!(std::fs::metadata(t.path("ro")).unwrap().mode() & 0o777, 0o644 & !current_umask() | (std::fs::metadata(t.path("a")).unwrap().mode() & 0o777 & !current_umask()));
    }

    #[test]
    fn f_replaces_a_hard_linked_destination_without_touching_the_other_link() {
        if posix::user::euid() == 0 {
            return;
        }
        let t = Tmp::new();
        t.write("a", "new");
        t.write("d1", "shared");
        std::fs::hard_link(t.path("d1"), t.path("d2")).unwrap();
        std::fs::set_permissions(t.path("d1"), std::fs::Permissions::from_mode(0o444)).unwrap();
        assert_eq!(one(Opts { force: true, ..opts() }, t.p("a"), t.p("d1")).0, 0);
        assert_eq!(std::fs::read(t.path("d1")).unwrap(), b"new");
        assert_eq!(std::fs::read(t.path("d2")).unwrap(), b"shared");
    }

    #[test]
    fn a_directory_needs_r() {
        let t = Tmp::new();
        std::fs::create_dir(t.path("d")).unwrap();
        let (st, log) = one(opts(), t.p("d"), t.p("e"));
        assert_eq!((st, log.len()), (1, 1));
        assert!(!t.path("e").exists());
    }

    fn tree(t: &Tmp) {
        std::fs::create_dir_all(t.path("src/sub/deep")).unwrap();
        t.write("src/a", "A");
        t.write("src/sub/b", "B");
        t.write("src/sub/deep/c", "C");
        std::fs::create_dir(t.path("src/empty")).unwrap();
    }

    #[test]
    fn recursive_copy_to_a_new_name_and_into_a_directory() {
        let t = Tmp::new();
        tree(&t);
        let r = Opts { recursive: true, ..opts() };
        assert_eq!(one(r, t.p("src"), t.p("copy")), (0, vec![]));
        assert_eq!(std::fs::read(t.path("copy/sub/deep/c")).unwrap(), b"C");
        assert_eq!(std::fs::read(t.path("copy/a")).unwrap(), b"A");
        assert!(t.path("copy/empty").is_dir());
        /* into an existing directory: below it, under the name of the source */
        std::fs::create_dir(t.path("into")).unwrap();
        assert_eq!(one(r, t.p("src"), dest_in(&t.p("into"), &t.p("src"))).0, 0);
        assert_eq!(std::fs::read(t.path("into/src/sub/b")).unwrap(), b"B");
        /* a source that ends in ".." puts its contents into the target, not beside it */
        std::fs::create_dir(t.path("up")).unwrap();
        let mut via = t.0.as_os_str().as_bytes().to_vec();
        via.extend_from_slice(b"/src/sub/..");
        let to = dest_in(&t.p("up"), &via);
        assert_eq!(one(r, via, to).0, 0);
        assert_eq!(std::fs::read(t.path("up/sub/deep/c")).unwrap(), b"C");
        assert_eq!(std::fs::read(t.path("up/a")).unwrap(), b"A");
        assert!(!t.path("a").exists());
        /* a second time over an existing hierarchy merges and overwrites */
        t.write("src/a", "A2");
        assert_eq!(one(r, t.p("src"), t.p("copy")).0, 0);
        assert_eq!(std::fs::read(t.path("copy/a")).unwrap(), b"A2");
    }

    #[test]
    fn directory_permissions_follow_the_source_less_the_umask_and_the_owner_can_always_write() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.path("src/ro")).unwrap();
        t.write("src/ro/f", "x");
        std::fs::set_permissions(t.path("src/ro"), std::fs::Permissions::from_mode(0o555)).unwrap();
        std::fs::set_permissions(t.path("src"), std::fs::Permissions::from_mode(0o750)).unwrap();
        let r = Opts { recursive: true, ..opts() };
        assert_eq!(one(r, t.p("src"), t.p("copy")), (0, vec![]));
        let m = |p: &str| std::fs::metadata(t.path(p)).unwrap().mode() & 0o777;
        assert_eq!(m("copy"), 0o750 & !current_umask());
        assert_eq!(m("copy/ro"), 0o555 & !current_umask());
        assert_eq!(std::fs::read(t.path("copy/ro/f")).unwrap(), b"x");
    }

    #[test]
    fn a_directory_over_a_file_and_a_file_over_a_directory() {
        let t = Tmp::new();
        tree(&t);
        t.write("plain", "p");
        let r = Opts { recursive: true, ..opts() };
        let (st, log) = one(r, t.p("src"), t.p("plain"));
        assert_eq!((st, log.len()), (1, 1));
        assert_eq!(std::fs::read(t.path("plain")).unwrap(), b"p");
        /* a subdirectory of the source that is a file in the destination: that one is skipped, the rest is copied */
        std::fs::create_dir(t.path("dst")).unwrap();
        t.write("dst/sub", "i am a file");
        let (st, log) = one(r, t.p("src"), t.p("dst"));
        assert_eq!((st, log.len()), (1, 1));
        assert_eq!(std::fs::read(t.path("dst/a")).unwrap(), b"A");
        assert_eq!(std::fs::read(t.path("dst/sub")).unwrap(), b"i am a file");
        /* and a regular file over an existing directory */
        t.write("f", "f");
        std::fs::create_dir(t.path("adir")).unwrap();
        assert_eq!(one(opts(), t.p("f"), t.p("adir")).0, 1);
    }

    #[test]
    fn a_directory_is_never_copied_into_itself() {
        let t = Tmp::new();
        tree(&t);
        let r = Opts { recursive: true, ..opts() };
        /* below itself: refused before anything is made */
        let (st, log) = one(r, t.p("src"), t.p("src/copy"));
        assert_eq!((st, log.len()), (1, 1));
        assert!(!t.path("src/copy").exists());
        let (st, _) = one(r, t.p("src"), t.p("src/sub/deep/copy"));
        assert_eq!(st, 1);
        assert!(!t.path("src/sub/deep/copy").exists());
        /* onto itself */
        let (st, log) = one(r, t.p("src"), t.p("src"));
        assert_eq!((st, log.len()), (1, 1));
        /* a prefix that is only a prefix of the name is not "inside" */
        assert_eq!(one(r, t.p("src"), t.p("src2")).0, 0);
        assert_eq!(std::fs::read(t.path("src2/a")).unwrap(), b"A");
    }

    #[test]
    fn the_destination_directory_is_found_again_if_it_appears_inside_the_source_while_copying() {
        /* the second line of defence: a source directory that is a destination being filled */
        let t = Tmp::new();
        tree(&t);
        let r = Opts { recursive: true, ..opts() };
        std::fs::create_dir(t.path("src/sub/loop")).unwrap();
        let mut cp = Cp::new(b"cp", r, current_umask(), Box::new(|_| false));
        cp.log = Some(Vec::new());
        let loop_st = stat_path(&t.p("src/sub/loop"), true).unwrap();
        cp.active.push(id_of(&loop_st));
        cp.copy_one(&t.p("src/sub/loop"), &t.p("elsewhere"), true);
        assert_eq!(cp.status, 1);
        assert!(!t.path("elsewhere").exists());
    }

    #[test]
    fn a_symbolic_link_cycle_under_l_is_reported_once_and_makes_no_nested_copies() {
        let t = Tmp::new();
        tree(&t);
        std::os::unix::fs::symlink("..", t.path("src/sub/up")).unwrap();
        let l = Opts { recursive: true, link: Link::L, ..opts() };
        let (st, log) = one(l, t.p("src"), t.p("copy"));
        assert_eq!(st, 1);
        assert_eq!(log.len(), 1);
        assert!(log[0].ends_with("src/sub/up: directory causes a cycle"), "{log:?}");
        assert!(t.path("copy/sub/deep/c").exists());
        assert!(!t.path("copy/sub/up").exists());
        /* without -L the link is copied as a link: no cycle, no error */
        let (st, _) = one(Opts { recursive: true, ..opts() }, t.p("src"), t.p("copy2"));
        assert_eq!(st, 0);
    }

    #[test]
    fn symbolic_links_as_operands() {
        let t = Tmp::new();
        t.write("f", "file");
        std::os::unix::fs::symlink("f", t.path("lf")).unwrap();
        /* plain cp reads through the link */
        assert_eq!(one(opts(), t.p("lf"), t.p("o1")).0, 0);
        assert!(std::fs::symlink_metadata(t.path("o1")).unwrap().is_file());
        /* -P without -R copies the link */
        assert_eq!(one(Opts { link: Link::P, ..opts() }, t.p("lf"), t.p("o2")).0, 0);
        assert_eq!(std::fs::read_link(t.path("o2")).unwrap(), Path::new("f"));
        /* -R: the default and -P copy the link; -H and -L read through it */
        let r = Opts { recursive: true, ..opts() };
        assert_eq!(one(r, t.p("lf"), t.p("o3")).0, 0);
        assert!(std::fs::symlink_metadata(t.path("o3")).unwrap().file_type().is_symlink());
        assert_eq!(one(Opts { link: Link::H, ..r }, t.p("lf"), t.p("o4")).0, 0);
        assert!(std::fs::symlink_metadata(t.path("o4")).unwrap().is_file());
        assert_eq!(one(Opts { link: Link::L, ..r }, t.p("lf"), t.p("o5")).0, 0);
        assert!(std::fs::symlink_metadata(t.path("o5")).unwrap().is_file());
        /* the last of -H -L -P decides, as the options are read */
        assert_eq!(one(Opts { link: Link::P, ..r }, t.p("lf"), t.p("o6")).0, 0);
        assert!(std::fs::symlink_metadata(t.path("o6")).unwrap().file_type().is_symlink());
    }

    #[test]
    fn symbolic_links_inside_a_hierarchy() {
        let t = Tmp::new();
        std::fs::create_dir(t.path("src")).unwrap();
        t.write("outside", "OUT");
        std::fs::create_dir(t.path("odir")).unwrap();
        t.write("odir/x", "X");
        std::os::unix::fs::symlink("../outside", t.path("src/lf")).unwrap();
        std::os::unix::fs::symlink("../odir", t.path("src/ld")).unwrap();
        std::os::unix::fs::symlink("nowhere", t.path("src/dang")).unwrap();
        let r = Opts { recursive: true, ..opts() };
        /* -P and -H and the default keep them as links, the dangling one included */
        for l in [Link::Default, Link::P, Link::H] {
            let d = format!("c_{l:?}");
            assert_eq!(one(Opts { link: l, ..r }, t.p("src"), t.p(&d)), (0, vec![]), "{l:?}");
            assert_eq!(std::fs::read_link(t.path(&format!("{d}/lf"))).unwrap(), Path::new("../outside"));
            assert_eq!(std::fs::read_link(t.path(&format!("{d}/dang"))).unwrap(), Path::new("nowhere"));
        }
        /* -L reads through the working ones; the dangling one is an error and the rest is copied */
        let (st, log) = one(Opts { link: Link::L, ..r }, t.p("src"), t.p("cL"));
        assert_eq!((st, log.len()), (1, 1));
        assert_eq!(std::fs::read(t.path("cL/lf")).unwrap(), b"OUT");
        assert_eq!(std::fs::read(t.path("cL/ld/x")).unwrap(), b"X");
        assert!(!t.path("cL/dang").exists());
    }

    #[test]
    fn a_link_loop_is_an_error_not_a_hang() {
        let t = Tmp::new();
        std::fs::create_dir(t.path("src")).unwrap();
        std::os::unix::fs::symlink("l2", t.path("src/l1")).unwrap();
        std::os::unix::fs::symlink("l1", t.path("src/l2")).unwrap();
        std::os::unix::fs::symlink("..", t.path("src/up")).unwrap();
        let (st, log) = one(Opts { recursive: true, link: Link::L, ..opts() }, t.p("src"), t.p("c"));
        assert_eq!(st, 1);
        assert!(!log.is_empty());
        /* as an operand */
        std::os::unix::fs::symlink("b", t.path("a")).unwrap();
        std::os::unix::fs::symlink("a", t.path("b")).unwrap();
        let (st, _) = one(opts(), t.p("a"), t.p("out"));
        assert_eq!(st, 1);
        assert!(!t.path("out").exists());
        /* a loop as the destination: an error, and with -f the loop is replaced by the copy */
        t.write("data", "D");
        let (st, _) = one(opts(), t.p("data"), t.p("a"));
        assert_eq!(st, 1);
        assert!(std::fs::symlink_metadata(t.path("a")).unwrap().file_type().is_symlink());
        let f = Opts { force: true, ..opts() };
        assert_eq!(one(f, t.p("data"), t.p("a")).0, 0);
        assert_eq!(std::fs::read(t.path("a")).unwrap(), b"D");
        assert!(std::fs::symlink_metadata(t.path("a")).unwrap().file_type().is_file());
    }

    #[test]
    fn a_dangling_link_as_destination_is_written_through() {
        let t = Tmp::new();
        t.write("a", "data");
        std::os::unix::fs::symlink("made", t.path("dang")).unwrap();
        assert_eq!(one(opts(), t.p("a"), t.p("dang")).0, 0);
        assert_eq!(std::fs::read(t.path("made")).unwrap(), b"data");
        assert!(std::fs::symlink_metadata(t.path("dang")).unwrap().file_type().is_symlink());
    }

    #[test]
    fn copying_a_link_over_an_existing_destination_replaces_it() {
        let t = Tmp::new();
        std::os::unix::fs::symlink("target", t.path("l")).unwrap();
        t.write("old", "o");
        let p = Opts { link: Link::P, ..opts() };
        assert_eq!(one(p, t.p("l"), t.p("old")).0, 0);
        assert_eq!(std::fs::read_link(t.path("old")).unwrap(), Path::new("target"));
        /* -i asks first, and a no leaves it */
        t.write("old2", "o");
        let (st, _, pr) = run(Opts { interactive: true, ..p }, &[false], &[(t.p("l"), t.p("old2"))]);
        assert_eq!((st, pr.len()), (0, 1));
        assert_eq!(std::fs::read(t.path("old2")).unwrap(), b"o");
        /* a link is not copied over a directory */
        std::fs::create_dir(t.path("dd")).unwrap();
        assert_eq!(one(p, t.p("l"), t.p("dd")).0, 1);
    }

    #[test]
    fn fifos_are_made_again_with_r_and_the_mode_of_the_source_less_the_umask() {
        let t = Tmp::new();
        let c = posix::cstr(&t.p("fifo"));
        assert_eq!(posix::sys::mkfifo(&c, 0o640), 0);
        posix::sys::chmod(&c, 0o640);
        let r = Opts { recursive: true, ..opts() };
        assert_eq!(one(r, t.p("fifo"), t.p("fifo2")), (0, vec![]));
        let m = std::fs::metadata(t.path("fifo2")).unwrap();
        assert_eq!(kind_of(&m), libc::S_IFIFO);
        assert_eq!(m.mode() & 0o777, 0o640 & !current_umask());
        /* in a hierarchy too */
        std::fs::create_dir(t.path("d")).unwrap();
        std::fs::rename(t.path("fifo"), t.path("d/ff")).unwrap();
        assert_eq!(one(r, t.p("d"), t.p("d2")).0, 0);
        assert_eq!(kind_of(&std::fs::metadata(t.path("d2/ff")).unwrap()), libc::S_IFIFO);
    }

    fn kind_of(m: &std::fs::Metadata) -> libc::mode_t {
        (m.mode() as libc::mode_t) & libc::S_IFMT
    }

    #[test]
    fn dev_null_is_read_as_data_without_r() {
        let t = Tmp::new();
        t.write("f", "something");
        assert_eq!(one(opts(), b"/dev/null".to_vec(), t.p("f")).0, 0);
        assert_eq!(std::fs::metadata(t.path("f")).unwrap().len(), 0);
        assert!(std::fs::metadata(t.path("f")).unwrap().is_file());
        /* and into /dev/null as a destination */
        t.write("g", "data");
        assert_eq!(one(opts(), t.p("g"), b"/dev/null".to_vec()).0, 0);
    }

    #[test]
    fn p_keeps_times_and_permissions_including_the_set_id_bits_when_the_owner_is_ours() {
        let t = Tmp::new();
        t.write("a", "x");
        std::fs::set_permissions(t.path("a"), std::fs::Permissions::from_mode(0o4751)).unwrap();
        let old = [libc::timespec { tv_sec: 1000, tv_nsec: 5 }, libc::timespec { tv_sec: 2000, tv_nsec: 7 }];
        let ca = posix::cstr(&t.p("a"));
        assert_eq!(posix::sys::utimensat(libc::AT_FDCWD, &ca, &old, 0), 0);
        let p = Opts { preserve: true, ..opts() };
        assert_eq!(one(p, t.p("a"), t.p("b")), (0, vec![]));
        let m = std::fs::metadata(t.path("b")).unwrap();
        assert_eq!((m.atime(), m.atime_nsec(), m.mtime(), m.mtime_nsec()), (1000, 5, 2000, 7));
        /* same owner: the set-user-ID bit survives */
        assert_eq!(m.mode() & 0o7777, 0o4751);
        /* the source's modification time is the same afterwards (its access time moves: it was read) */
        assert_eq!(std::fs::metadata(t.path("a")).unwrap().mtime(), 2000);
    }

    #[test]
    fn p_on_an_existing_destination_applies_to_it_and_without_p_the_times_are_new() {
        let t = Tmp::new();
        t.write("a", "x");
        t.write("b", "y");
        std::fs::set_permissions(t.path("b"), std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(t.path("a"), std::fs::Permissions::from_mode(0o644)).unwrap();
        let old = [libc::timespec { tv_sec: 1000, tv_nsec: 0 }, libc::timespec { tv_sec: 2000, tv_nsec: 0 }];
        let ca = posix::cstr(&t.p("a"));
        posix::sys::utimensat(libc::AT_FDCWD, &ca, &old, 0);
        assert_eq!(one(opts(), t.p("a"), t.p("b")).0, 0);
        assert!(std::fs::metadata(t.path("b")).unwrap().mtime() > 2000);
        assert_eq!(std::fs::metadata(t.path("b")).unwrap().mode() & 0o777, 0o600);
        assert_eq!(one(Opts { preserve: true, ..opts() }, t.p("a"), t.p("b")).0, 0);
        assert_eq!(std::fs::metadata(t.path("b")).unwrap().mtime(), 2000);
        assert_eq!(std::fs::metadata(t.path("b")).unwrap().mode() & 0o777, 0o644);
    }

    #[test]
    fn p_on_directories_sets_the_times_after_the_contents_are_in() {
        let t = Tmp::new();
        tree(&t);
        let old = [libc::timespec { tv_sec: 3000, tv_nsec: 0 }, libc::timespec { tv_sec: 4000, tv_nsec: 0 }];
        for d in ["src", "src/sub", "src/sub/deep", "src/empty"] {
            let c = posix::cstr(&t.p(d));
            posix::sys::utimensat(libc::AT_FDCWD, &c, &old, 0);
        }
        let r = Opts { recursive: true, preserve: true, ..opts() };
        assert_eq!(one(r, t.p("src"), t.p("copy")), (0, vec![]));
        for d in ["copy", "copy/sub", "copy/sub/deep", "copy/empty"] {
            assert_eq!(std::fs::metadata(t.path(d)).unwrap().mtime(), 4000, "{d}");
        }
        /* directories that were already there get them too: the mode and the times of the source */
        std::fs::create_dir_all(t.path("again/sub")).unwrap();
        std::fs::set_permissions(t.path("again/sub"), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(t.path("src/sub"), std::fs::Permissions::from_mode(0o750)).unwrap();
        let c = posix::cstr(&t.p("src/sub"));
        posix::sys::utimensat(libc::AT_FDCWD, &c, &old, 0);
        assert_eq!(one(r, t.p("src"), t.p("again")).0, 0);
        let m = std::fs::metadata(t.path("again/sub")).unwrap();
        assert_eq!((m.mtime(), m.mode() & 0o7777), (4000, 0o750));
        assert_eq!(std::fs::metadata(t.path("again")).unwrap().mtime(), 4000);
        /* and without -p a directory that was there keeps both */
        std::fs::create_dir_all(t.path("keep/sub")).unwrap();
        std::fs::set_permissions(t.path("keep/sub"), std::fs::Permissions::from_mode(0o700)).unwrap();
        let r = Opts { preserve: false, ..r };
        assert_eq!(one(r, t.p("src"), t.p("keep")).0, 0);
        let m = std::fs::metadata(t.path("keep/sub")).unwrap();
        assert_eq!(m.mode() & 0o7777, 0o700);
        assert_ne!(m.mtime(), 4000);
    }

    #[test]
    fn p_on_a_symbolic_link_copies_its_own_times() {
        let t = Tmp::new();
        std::os::unix::fs::symlink("target", t.path("l")).unwrap();
        let old = [libc::timespec { tv_sec: 5000, tv_nsec: 0 }, libc::timespec { tv_sec: 6000, tv_nsec: 0 }];
        let c = posix::cstr(&t.p("l"));
        assert_eq!(posix::sys::utimensat(libc::AT_FDCWD, &c, &old, libc::AT_SYMLINK_NOFOLLOW), 0);
        let p = Opts { preserve: true, link: Link::P, ..opts() };
        assert_eq!(one(p, t.p("l"), t.p("l2")), (0, vec![]));
        assert_eq!(std::fs::symlink_metadata(t.path("l2")).unwrap().mtime(), 6000);
    }

    #[test]
    fn a_source_directory_that_cannot_be_read_is_reported_and_the_rest_is_copied() {
        if posix::user::euid() == 0 {
            return;
        }
        let t = Tmp::new();
        tree(&t);
        std::fs::set_permissions(t.path("src/sub"), std::fs::Permissions::from_mode(0o0)).unwrap();
        let (st, log) = one(Opts { recursive: true, ..opts() }, t.p("src"), t.p("copy"));
        assert_eq!((st, log.len()), (1, 1));
        assert_eq!(std::fs::read(t.path("copy/a")).unwrap(), b"A");
        assert!(t.path("copy/sub").is_dir());
    }

    #[test]
    fn a_long_names_error_is_an_error_and_goes_on() {
        let t = Tmp::new();
        t.write("a", "x");
        let long = "n".repeat(300);
        let (st, log) = one(opts(), t.p("a"), t.p(&long));
        assert_eq!((st, log.len()), (1, 1));
        assert_eq!(one(opts(), t.p("a"), t.p("fine")).0, 0);
    }

    #[test]
    fn a_trailing_slash_after_a_file_is_an_error() {
        let t = Tmp::new();
        t.write("a", "x");
        t.write("b", "y");
        let mut dest = t.p("b");
        dest.push(b'/');
        let (st, log) = one(opts(), t.p("a"), dest);
        assert_eq!((st, log.len()), (1, 1));
        assert_eq!(std::fs::read(t.path("b")).unwrap(), b"y");
    }
}
