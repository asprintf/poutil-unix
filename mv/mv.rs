/* SPDX-License-Identifier: GPL-2.0-only */
/* mv - move files. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::walk::{walk, Entry, Links, Visit};
use std::os::fd::RawFd;

const SYNOPSIS: &str = "mv [-if] source_file target_file\n       mv [-if] source_file... target_dir";

type Ask = Box<dyn FnMut(&[u8]) -> bool>;
type Pairs = Vec<(Vec<u8>, Vec<u8>)>;

const BLOCK: usize = 128 * 1024;

/* A hierarchy deeper than this is not duplicated: each level is a stack frame. */
const DEPTH_MAX: usize = 1000;

fn last_err() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn kind(st: &libc::stat) -> libc::mode_t {
    st.st_mode & libc::S_IFMT
}

fn is_dir(st: &libc::stat) -> bool {
    kind(st) == libc::S_IFDIR
}

fn lstat(path: &[u8]) -> std::io::Result<libc::stat> {
    posix::sys::lstat(&posix::cstr(path))
}

fn same_file(a: &libc::stat, b: &libc::stat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino
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

/* Removes a hierarchy, the contents of a directory first. */
struct Remove {
    prog: Vec<u8>,
    failed: bool,
}

impl Visit for Remove {
    fn visit(&mut self, e: &Entry) -> std::io::Result<()> {
        let flags = if e.is_dir() { libc::AT_REMOVEDIR } else { 0 };
        if posix::sys::unlinkat(e.dirfd, e.name, flags) < 0 {
            return Err(last_err());
        }
        Ok(())
    }

    fn report(&mut self, path: &[u8], e: &std::io::Error) {
        self.failed = true;
        posix::warn_path(&self.prog, path, &posix::strerror(e));
    }

    fn after_contents(&self) -> bool {
        true
    }
}

struct Mv {
    prog: Vec<u8>,
    force: bool,
    interactive: bool,
    /* is the standard input a terminal? */
    tty: bool,
    status: i32,
    ask: Ask,
    buf: Vec<u8>,
    /* the unit tests collect diagnostics here instead of writing them */
    log: Option<Vec<String>>,
}

impl Mv {
    fn new(prog: &[u8], force: bool, interactive: bool, tty: bool, ask: Ask) -> Mv {
        Mv { prog: prog.to_vec(), force, interactive, tty, status: 0, ask, buf: vec![0u8; BLOCK], log: None }
    }

    fn say(&mut self, msg: String) {
        match self.log.as_mut() {
            Some(l) => l.push(msg),
            None => posix::warn(&self.prog, &msg),
        }
    }

    /* A diagnostic that makes the exit status non-zero. */
    fn err(&mut self, path: &[u8], msg: &str) {
        self.status = 1;
        self.say(format!("{}: {}", String::from_utf8_lossy(path), msg));
    }

    fn err_io(&mut self, path: &[u8], e: &std::io::Error) {
        self.err(path, &posix::strerror(e));
    }

    /* A diagnostic that leaves the status alone: a file characteristic that could not be duplicated. */
    fn note_io(&mut self, path: &[u8], e: &std::io::Error) {
        self.say(format!("{}: {}", String::from_utf8_lossy(path), posix::strerror(e)));
    }

    fn confirm(&mut self, dest: &[u8]) -> bool {
        let mut p = Vec::new();
        p.extend_from_slice(&self.prog);
        p.extend_from_slice(b": overwrite '");
        p.extend_from_slice(dest);
        p.extend_from_slice(b"'? ");
        (self.ask)(&p)
    }

    /* Steps 1 to 7 for one source_file and its destination path. */
    fn move_one(&mut self, src: &[u8], dest: &[u8]) {
        let ss = match lstat(src) {
            Ok(s) => s,
            Err(e) => {
                self.err_io(src, &e);
                return;
            }
        };
        let ds = lstat(dest).ok();
        if let Some(d) = &ds {
            let unwritable = posix::sys::access(&posix::cstr(dest), libc::W_OK) != 0;
            if !self.force && (self.interactive || (unwritable && self.tty)) && !self.confirm(dest) {
                return;
            }
            /* the same file under two names: nothing is moved, and it is said (the standard leaves the choice) */
            if same_file(d, &ss) {
                let mut p = src.to_vec();
                p.extend_from_slice(b" and ");
                p.extend_from_slice(dest);
                self.err(&p, "are identical");
                return;
            }
        }
        if posix::sys::rename(&posix::cstr(src), &posix::cstr(dest)) == 0 {
            return;
        }
        let e = last_err();
        if e.raw_os_error() != Some(libc::EXDEV) {
            let msg = format!("rename {} to {}: {}", String::from_utf8_lossy(src), String::from_utf8_lossy(dest), posix::strerror(&e));
            self.status = 1;
            self.say(msg);
            return;
        }
        self.move_by_copy(src, dest, &ss, ds.as_ref());
    }

    /* Steps 4 to 7, for a destination on another file system. */
    fn move_by_copy(&mut self, src: &[u8], dest: &[u8], ss: &libc::stat, ds: Option<&libc::stat>) {
        if let Some(d) = ds {
            if is_dir(d) != is_dir(ss) {
                let msg = if is_dir(d) { "cannot overwrite a directory with a non-directory" } else { "cannot overwrite a non-directory with a directory" };
                self.err(dest, msg);
                return;
            }
            let cd = posix::cstr(dest);
            let r = if is_dir(d) { posix::sys::unlinkat(libc::AT_FDCWD, &cd, libc::AT_REMOVEDIR) } else { posix::sys::unlink(&cd) };
            if r < 0 {
                let e = last_err();
                self.err_io(dest, &e);
                return;
            }
        }
        if !self.duplicate(src, dest, ss, 0) {
            return;
        }
        let mut rm = Remove { prog: self.prog.clone(), failed: false };
        walk(src, true, Links::Never, &mut rm);
        if rm.failed {
            self.status = 1;
        }
    }

    /* Step 6 for one file of the hierarchy; false if it could not be duplicated (reported). */
    fn duplicate(&mut self, src: &[u8], dest: &[u8], st: &libc::stat, depth: usize) -> bool {
        match kind(st) {
            libc::S_IFDIR => self.duplicate_dir(src, dest, st, depth),
            libc::S_IFREG => self.duplicate_regular(src, dest, st),
            libc::S_IFLNK => {
                let target = match read_link(src) {
                    Ok(t) => t,
                    Err(e) => {
                        self.err_io(src, &e);
                        return false;
                    }
                };
                if posix::sys::symlink(&posix::cstr(&target), &posix::cstr(dest)) < 0 {
                    let e = last_err();
                    self.err_io(dest, &e);
                    return false;
                }
                self.characteristics_of_link(dest, st);
                true
            }
            libc::S_IFIFO => {
                if posix::sys::mkfifo(&posix::cstr(dest), 0o600) < 0 {
                    let e = last_err();
                    self.err_io(dest, &e);
                    return false;
                }
                self.characteristics(dest, st);
                true
            }
            libc::S_IFCHR | libc::S_IFBLK => {
                if posix::sys::mknod(&posix::cstr(dest), kind(st) | 0o600, st.st_rdev) < 0 {
                    let e = last_err();
                    self.err_io(dest, &e);
                    return false;
                }
                self.characteristics(dest, st);
                true
            }
            _ => {
                self.err(src, "cannot be duplicated: not a file type that can be made again");
                false
            }
        }
    }

    fn duplicate_dir(&mut self, src: &[u8], dest: &[u8], st: &libc::stat, depth: usize) -> bool {
        if depth >= DEPTH_MAX {
            self.err(src, "file hierarchy too deep");
            return false;
        }
        /* owner-only until the contents are in, the real mode goes on last */
        if posix::sys::mkdir(&posix::cstr(dest), 0o700) < 0 {
            let e = last_err();
            self.err_io(dest, &e);
            return false;
        }
        let names = match posix::sys::read_dir(&posix::cstr(src)) {
            Ok(n) => n,
            Err(e) => {
                self.err_io(src, &e);
                return false;
            }
        };
        let mut ok = true;
        for n in names.iter().filter(|n| *n != b"." && *n != b"..") {
            let (s, d) = (join(src, n), join(dest, n));
            match lstat(&s) {
                Ok(cst) => ok &= self.duplicate(&s, &d, &cst, depth + 1),
                Err(e) => {
                    self.err_io(&s, &e);
                    ok = false;
                }
            }
        }
        self.characteristics(dest, st);
        ok
    }

    fn duplicate_regular(&mut self, src: &[u8], dest: &[u8], st: &libc::stat) -> bool {
        let sfd = posix::sys::open(&posix::cstr(src), libc::O_RDONLY | libc::O_NOCTTY | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0);
        if sfd < 0 {
            let e = last_err();
            self.err_io(src, &e);
            return false;
        }
        let dfd = posix::sys::open(&posix::cstr(dest), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOCTTY | libc::O_CLOEXEC, 0o600);
        if dfd < 0 {
            let e = last_err();
            self.err_io(dest, &e);
            posix::sys::close(sfd);
            return false;
        }
        let mut ok = true;
        if let Err((is_read, e)) = self.copy_data(sfd, dfd) {
            self.err_io(if is_read { src } else { dest }, &e);
            ok = false;
        }
        if ok {
            self.characteristics_fd(dfd, dest, st);
        }
        posix::sys::close(sfd);
        if posix::sys::close(dfd) < 0 && ok {
            let e = last_err();
            self.err_io(dest, &e);
            ok = false;
        }
        ok
    }

    /* Err: (was it a read error, the error). */
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
                if w == 0 {
                    return Err((false, std::io::Error::from(std::io::ErrorKind::WriteZero)));
                }
                off += w as usize;
            }
        }
    }

    /* The set-ID bits are not duplicated if the owner or group could not be. */
    fn mode_for(st: &libc::stat, owner_ok: bool) -> libc::mode_t {
        let mode = st.st_mode & 0o7777;
        if owner_ok { mode } else { mode & !(libc::S_ISUID | libc::S_ISGID) }
    }

    /* Owner, group, mode, then the times last. A failure is said, the status stays. */
    fn characteristics_fd(&mut self, dfd: RawFd, dest: &[u8], st: &libc::stat) {
        let owner_ok = posix::sys::fchown(dfd, st.st_uid, st.st_gid) == 0;
        if posix::sys::fchmod(dfd, Self::mode_for(st, owner_ok)) < 0 {
            let e = last_err();
            self.note_io(dest, &e);
        }
        if posix::sys::futimens(dfd, &timespecs(st)) < 0 {
            let e = last_err();
            self.note_io(dest, &e);
        }
    }

    fn characteristics(&mut self, dest: &[u8], st: &libc::stat) {
        let cd = posix::cstr(dest);
        let owner_ok = posix::sys::chown(&cd, st.st_uid, st.st_gid) == 0;
        if posix::sys::chmod(&cd, Self::mode_for(st, owner_ok)) < 0 {
            let e = last_err();
            self.note_io(dest, &e);
        }
        if posix::sys::utimensat(libc::AT_FDCWD, &cd, &timespecs(st), 0) < 0 {
            let e = last_err();
            self.note_io(dest, &e);
        }
    }

    /* A symbolic link has no mode of its own; an owner that cannot be given is no error, as for a file. */
    fn characteristics_of_link(&mut self, dest: &[u8], st: &libc::stat) {
        let cd = posix::cstr(dest);
        if posix::sys::lchown(&cd, st.st_uid, st.st_gid) < 0 {
            let e = last_err();
            if e.raw_os_error() != Some(libc::EPERM) {
                self.note_io(dest, &e);
            }
        }
        if posix::sys::utimensat(libc::AT_FDCWD, &cd, &timespecs(st), libc::AT_SYMLINK_NOFOLLOW) < 0 {
            let e = last_err();
            self.note_io(dest, &e);
        }
    }
}

/* The (source, destination) pairs, or why nothing is moved. */
fn plan(sources: &[Vec<u8>], target: &[u8], target_is_dir: bool) -> Result<Pairs, &'static str> {
    if target_is_dir {
        return Ok(sources.iter().map(|s| (s.clone(), join(target, last_component(s)))).collect());
    }
    if sources.len() > 1 {
        return Err("not a directory");
    }
    /* a non-directory source with a slash after the target: an error, and nothing is processed */
    if target.last() == Some(&b'/') && matches!(lstat(&sources[0]), Ok(s) if !is_dir(&s)) {
        return Err("not a directory");
    }
    Ok(vec![(sources[0].clone(), target.to_vec())])
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let (mut force, mut interactive) = (false, false);
    let mut g = Getopt::new(&args, b"if");
    for o in g.by_ref() {
        /* the last of -f and -i decides */
        match o {
            Opt::Flag(b'f') => (force, interactive) = (true, false),
            Opt::Flag(b'i') => (force, interactive) = (false, true),
            _ => posix::usage(SYNOPSIS),
        }
    }
    let operands = g.operands();
    if operands.len() < 2 {
        posix::usage(SYNOPSIS);
    }
    let (sources, target) = (&operands[..operands.len() - 1], &operands[operands.len() - 1]);
    let target_is_dir = matches!(posix::sys::stat(&posix::cstr(target)), Ok(s) if is_dir(&s));
    let pairs = match plan(sources, target, target_is_dir) {
        Ok(p) => p,
        Err(msg) => {
            posix::warn_path(&prog, target, msg);
            std::process::exit(1);
        }
    };
    let yes = posix::prompt::Yes::new();
    let ask = Box::new(move |prompt: &[u8]| -> bool { posix::prompt::ask(&yes, prompt) });
    let mut mv = Mv::new(&prog, force, interactive, posix::sys::isatty(0), ask);
    for (s, d) in &pairs {
        mv.move_one(s, d);
    }
    std::process::exit(mv.status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("mv-test-{}-{}", std::process::id(), n));
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
        fn read(&self, s: &str) -> String {
            std::fs::read_to_string(self.path(s)).unwrap()
        }
        fn exists(&self, s: &str) -> bool {
            std::fs::symlink_metadata(self.path(s)).is_ok()
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

    fn mv(force: bool, interactive: bool, tty: bool, answer: bool) -> Mv {
        let mut m = Mv::new(b"mv", force, interactive, tty, Box::new(move |_| answer));
        m.log = Some(Vec::new());
        m
    }

    fn name(s: &[u8]) -> Vec<u8> {
        s.to_vec()
    }

    #[test]
    fn rename_a_file_and_replace_one() {
        let t = Tmp::new();
        t.write("a", "A");
        t.write("b", "B");
        let mut m = mv(false, false, false, false);
        m.move_one(&t.p("a"), &t.p("c"));
        assert_eq!((m.status, t.exists("a"), t.read("c")), (0, false, "A".into()));
        /* an existing destination is replaced without a question when there is no terminal and no -i */
        m.move_one(&t.p("c"), &t.p("b"));
        assert_eq!((m.status, t.exists("c"), t.read("b")), (0, false, "A".into()));
    }

    #[test]
    fn the_prompt_follows_the_standards_rule() {
        /* exists AND NOT -f AND ((not writable AND terminal) OR -i) */
        let t = Tmp::new();
        let go = |force, interactive, tty, answer, writable: bool| -> bool {
            t.write("s", "S");
            t.write("d", "D");
            if !writable {
                std::fs::set_permissions(t.path("d"), std::fs::Permissions::from_mode(0o444)).unwrap();
            }
            let mut m = mv(force, interactive, tty, answer);
            m.move_one(&t.p("s"), &t.p("d"));
            let moved = !t.exists("s");
            let _ = std::fs::remove_file(t.path("s"));
            let _ = std::fs::remove_file(t.path("d"));
            assert_eq!(m.status, 0);
            moved
        };
        assert!(go(false, true, false, true, true), "-i, yes");
        assert!(!go(false, true, false, false, true), "-i, no: nothing happens");
        assert!(go(false, false, false, false, true), "writable, no -i: no question");
        assert!(!go(false, false, true, false, false), "not writable, terminal: asked, no");
        assert!(go(false, false, true, true, false), "not writable, terminal: asked, yes");
        /* root may write anything: the rule cannot be seen */
        if posix::user::euid() != 0 {
            assert!(go(false, false, false, false, false), "not writable, no terminal: no question");
            assert!(go(true, false, true, false, false), "-f: no question");
        }
        assert!(go(true, false, false, false, true));
    }

    #[test]
    fn the_prompt_names_the_destination() {
        let t = Tmp::new();
        t.write("s", "S");
        t.write("d", "D");
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let s2 = seen.clone();
        let mut m = Mv::new(
            b"mv",
            false,
            true,
            false,
            Box::new(move |p| {
                s2.borrow_mut().extend_from_slice(p);
                true
            }),
        );
        let d = t.p("d");
        m.move_one(&t.p("s"), &d);
        let text = String::from_utf8(seen.borrow().clone()).unwrap();
        assert!(text.contains(&String::from_utf8(d).unwrap()), "{text}");
    }

    #[test]
    fn the_same_file_is_not_removed() {
        let t = Tmp::new();
        t.write("a", "A");
        std::fs::hard_link(t.path("a"), t.path("b")).unwrap();
        let mut m = mv(false, false, false, false);
        m.move_one(&t.p("a"), &t.p("a"));
        assert_eq!(m.status, 1);
        m.status = 0;
        m.move_one(&t.p("a"), &t.p("b"));
        assert_eq!(m.status, 1);
        assert_eq!((t.read("a"), t.read("b")), ("A".into(), "A".into()));
        assert_eq!(m.log.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn a_directory_moves_with_its_contents_and_onto_an_empty_one() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.path("d/s")).unwrap();
        t.write("d/s/f", "F");
        std::fs::create_dir(t.path("e")).unwrap();
        let mut m = mv(false, false, false, false);
        m.move_one(&t.p("d"), &t.p("e"));
        assert_eq!((m.status, t.exists("d"), t.read("e/s/f")), (0, false, "F".into()));
        /* onto a directory that has contents: rename() refuses */
        std::fs::create_dir(t.path("x")).unwrap();
        t.write("x/in", "I");
        std::fs::create_dir(t.path("y")).unwrap();
        m.move_one(&t.p("y"), &t.p("x"));
        assert_eq!(m.status, 1);
        assert!(t.exists("y") && t.exists("x/in"));
    }

    #[test]
    fn a_directory_and_a_non_directory_do_not_replace_each_other() {
        let t = Tmp::new();
        t.write("f", "F");
        std::fs::create_dir(t.path("d")).unwrap();
        let mut m = mv(false, false, false, false);
        m.move_one(&t.p("f"), &t.p("d"));
        assert_eq!(m.status, 1);
        m.status = 0;
        m.move_one(&t.p("d"), &t.p("f"));
        assert_eq!(m.status, 1);
        assert!(t.exists("f") && t.exists("d"));
    }

    #[test]
    fn a_missing_source_is_an_error_and_the_next_one_goes_on() {
        let t = Tmp::new();
        t.write("a", "A");
        std::fs::create_dir(t.path("dir")).unwrap();
        let sources = [t.p("nosuch"), t.p("a")];
        let pairs = plan(&sources, &t.p("dir"), true).unwrap();
        let mut m = mv(false, false, false, false);
        for (s, d) in &pairs {
            m.move_one(s, d);
        }
        assert_eq!(m.status, 1);
        assert_eq!(m.log.as_ref().unwrap().len(), 1);
        assert_eq!(t.read("dir/a"), "A");
    }

    #[test]
    fn plan_forms() {
        let t = Tmp::new();
        t.write("f", "");
        std::fs::create_dir(t.path("d")).unwrap();
        symlink("d", t.path("dl")).unwrap();
        let s = |v: &[&str]| -> Vec<Vec<u8>> { v.iter().map(|x| t.p(x)).collect() };
        /* into a directory, or a link to one: the last component of each source */
        let pairs = plan(&s(&["f"]), &t.p("d"), true).unwrap();
        assert_eq!(pairs, vec![(t.p("f"), join(&t.p("d"), b"f"))]);
        let mut a = t.p("f");
        a.extend_from_slice(b"//");
        let pairs = plan(&[a.clone()], b"d/", true).unwrap();
        assert_eq!(pairs[0].1, b"d/f".to_vec());
        /* more than two operands and a target that is not a directory */
        assert!(plan(&s(&["f", "f"]), &t.p("nosuch"), false).is_err());
        /* a trailing slash after a target for a non-directory source */
        let mut tf = t.p("new");
        tf.push(b'/');
        assert!(plan(&s(&["f"]), &tf, false).is_err());
        assert!(plan(&s(&["d"]), &tf, false).is_ok());
        assert!(plan(&s(&["dl"]), &tf, false).is_err());
        /* a source that does not exist is for move_one to report */
        assert!(plan(&s(&["nosuch"]), &tf, false).is_ok());
        assert_eq!(name(b"x"), b"x");
    }

    /* the steps for another file system, called directly: a test cannot make a second one */
    #[test]
    fn moving_by_copy_duplicates_the_hierarchy_and_removes_the_source() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.path("d/sub")).unwrap();
        t.write("d/a", "A\n");
        t.write("d/sub/b", "B\n");
        symlink("a", t.path("d/l")).unwrap();
        symlink("nowhere", t.path("d/dang")).unwrap();
        std::fs::set_permissions(t.path("d/a"), std::fs::Permissions::from_mode(0o640)).unwrap();
        std::fs::set_permissions(t.path("d/sub"), std::fs::Permissions::from_mode(0o750)).unwrap();
        let ts = [libc::timespec { tv_sec: 1_000_000_000, tv_nsec: 0 }, libc::timespec { tv_sec: 1_100_000_000, tv_nsec: 0 }];
        assert_eq!(posix::sys::utimensat(libc::AT_FDCWD, &posix::cstr(&t.p("d/a")), &ts, 0), 0);
        let mode_before = std::fs::metadata(t.path("d/a")).unwrap().mode();
        let ss = lstat(&t.p("d")).unwrap();
        let mut m = mv(false, false, false, false);
        m.move_by_copy(&t.p("d"), &t.p("n"), &ss, None);
        assert_eq!((m.status, m.log.as_ref().unwrap().len()), (0, 0), "{:?}", m.log);
        assert!(!t.exists("d"));
        /* the times first: reading the file would change its access time */
        let a = std::fs::metadata(t.path("n/a")).unwrap();
        assert_eq!((a.mtime(), a.atime()), (1_100_000_000, 1_000_000_000));
        assert_eq!((t.read("n/a"), t.read("n/sub/b")), ("A\n".into(), "B\n".into()));
        /* links are duplicated as links, a dangling one too */
        assert_eq!(std::fs::read_link(t.path("n/l")).unwrap(), Path::new("a"));
        assert_eq!(std::fs::read_link(t.path("n/dang")).unwrap(), Path::new("nowhere"));
        assert_eq!(a.mode(), mode_before);
        assert_eq!(std::fs::metadata(t.path("n/sub")).unwrap().mode() & 0o777, 0o750);
    }

    #[test]
    fn moving_by_copy_replaces_a_destination_of_the_same_kind_and_refuses_the_others() {
        let t = Tmp::new();
        t.write("s", "S");
        t.write("d", "D");
        std::fs::create_dir(t.path("dd")).unwrap();
        let mut m = mv(false, false, false, false);
        let (ss, ds) = (lstat(&t.p("s")).unwrap(), lstat(&t.p("d")).ok());
        m.move_by_copy(&t.p("s"), &t.p("d"), &ss, ds.as_ref());
        assert_eq!((m.status, t.exists("s"), t.read("d")), (0, false, "S".into()));
        t.write("s", "S2");
        let (ss, ds) = (lstat(&t.p("s")).unwrap(), lstat(&t.p("dd")).ok());
        m.move_by_copy(&t.p("s"), &t.p("dd"), &ss, ds.as_ref());
        assert_eq!(m.status, 1);
        assert!(t.exists("s") && t.exists("dd"));
        /* a directory onto a file */
        m.status = 0;
        std::fs::create_dir(t.path("sd")).unwrap();
        let (ss, ds) = (lstat(&t.p("sd")).unwrap(), lstat(&t.p("d")).ok());
        m.move_by_copy(&t.p("sd"), &t.p("d"), &ss, ds.as_ref());
        assert_eq!(m.status, 1);
        assert!(t.exists("sd") && t.read("d") == "S");
    }

    #[test]
    fn moving_by_copy_keeps_the_source_if_it_cannot_be_read() {
        if posix::user::euid() == 0 {
            return;
        }
        let t = Tmp::new();
        std::fs::create_dir(t.path("d")).unwrap();
        t.write("d/secret", "X");
        std::fs::set_permissions(t.path("d/secret"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let ss = lstat(&t.p("d")).unwrap();
        let mut m = mv(false, false, false, false);
        m.move_by_copy(&t.p("d"), &t.p("n"), &ss, None);
        assert_eq!(m.status, 1);
        assert!(t.exists("d/secret"), "the source stays whole");
    }

    #[test]
    fn a_fifo_is_made_again() {
        let t = Tmp::new();
        assert_eq!(posix::sys::mkfifo(&posix::cstr(&t.p("q")), 0o640), 0);
        let ss = lstat(&t.p("q")).unwrap();
        let mut m = mv(false, false, false, false);
        m.move_by_copy(&t.p("q"), &t.p("r"), &ss, None);
        assert_eq!((m.status, t.exists("q")), (0, false));
        assert_eq!(kind(&lstat(&t.p("r")).unwrap()), libc::S_IFIFO);
    }

    #[test]
    fn the_last_component() {
        assert_eq!(last_component(b"a/b"), b"b");
        assert_eq!(last_component(b"a/b///"), b"b");
        assert_eq!(last_component(b"b"), b"b");
        assert_eq!(last_component(b"///"), b"");
        assert_eq!(join(b"d/", b"x"), b"d/x");
        assert_eq!(join(b"d", b"x"), b"d/x");
    }
}
