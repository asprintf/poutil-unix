/* SPDX-License-Identifier: GPL-2.0-only */
/* rm - remove directory entries. */

use posix::getopt::{Getopt, Opt};
use std::ffi::CStr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

const SYNOPSIS: &str = "rm [-iRr] file...\n       rm -f [-iRr] [file...]";

/*
 * Parent directories are kept open for this many levels; deeper ones are
 * closed and found again through ".." when the walk comes back up. The
 * descent therefore never needs a descriptor per level.
 */
const KEEP: usize = 16;

#[derive(Clone, Copy)]
struct Opts {
    force: bool,
    interactive: bool,
    recursive: bool,
}

/* (device, inode): the identity of a file. */
type Id = (u64, u64);

fn id_of(st: &libc::stat) -> Id {
    (st.st_dev as u64, st.st_ino as u64)
}

fn is_dir(st: &libc::stat) -> bool {
    st.st_mode & libc::S_IFMT == libc::S_IFDIR
}

fn is_link(st: &libc::stat) -> bool {
    st.st_mode & libc::S_IFMT == libc::S_IFLNK
}

/* Drop trailing slashes, but keep a pathname of only slashes as one. */
fn strip_slashes(p: &[u8]) -> &[u8] {
    let mut n = p.len();
    while n > 1 && p[n - 1] == b'/' {
        n -= 1;
    }
    &p[..n]
}

/* The final pathname component: the basename of p, without trailing slashes. */
fn last_component(p: &[u8]) -> &[u8] {
    let s = strip_slashes(p);
    match s.iter().rposition(|&c| c == b'/') {
        Some(i) if i + 1 < s.len() => &s[i + 1..],
        Some(_) => s,
        None => s,
    }
}

/*
 * The condition of the RATIONALE: prompt unless -f, when -i was given or
 * when the file is not writable and standard input is a terminal.
 */
fn need_prompt(o: &Opts, writable: bool, tty: bool) -> bool {
    !o.force && (o.interactive || (!writable && tty))
}

fn last_err() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn lstat_at(dirfd: RawFd, name: &CStr) -> std::io::Result<libc::stat> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatat(dirfd, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } < 0 {
        return Err(last_err());
    }
    Ok(st)
}

fn fstat(fd: RawFd) -> std::io::Result<libc::stat> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } < 0 {
        return Err(last_err());
    }
    Ok(st)
}

/* Open a directory without following a symbolic link in the last component. */
fn open_dir_at(dirfd: RawFd, name: &CStr) -> std::io::Result<OwnedFd> {
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::openat(dirfd, name.as_ptr(), flags) };
    if fd < 0 {
        return Err(last_err());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn writable_at(dirfd: RawFd, name: &CStr) -> bool {
    unsafe { libc::faccessat(dirfd, name.as_ptr(), libc::W_OK, libc::AT_EACCESS) == 0 }
}

fn unlink_at(dirfd: RawFd, name: &CStr, flags: libc::c_int) -> std::io::Result<()> {
    if unsafe { libc::unlinkat(dirfd, name.as_ptr(), flags) } < 0 {
        return Err(last_err());
    }
    Ok(())
}

/* Every name in an open directory except "." and "..", read before anything is removed. */
fn read_names(fd: &OwnedFd) -> std::io::Result<Vec<Vec<u8>>> {
    let d = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if d < 0 {
        return Err(last_err());
    }
    let dir = unsafe { libc::fdopendir(d) };
    if dir.is_null() {
        let e = last_err();
        unsafe { libc::close(d) };
        return Err(e);
    }
    let mut out = Vec::new();
    let r = loop {
        posix::clear_errno();
        let ent = unsafe { libc::readdir(dir) };
        if ent.is_null() {
            let e = last_err();
            break if e.raw_os_error() == Some(0) { Ok(()) } else { Err(e) };
        }
        let name = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) }.to_bytes();
        if name != b"." && name != b".." {
            out.push(name.to_vec());
        }
    };
    unsafe { libc::closedir(dir) };
    r.map(|_| out)
}

fn open_and_read(dirfd: RawFd, name: &CStr) -> std::io::Result<(OwnedFd, Vec<Vec<u8>>)> {
    let fd = open_dir_at(dirfd, name)?;
    let names = read_names(&fd)?;
    Ok((fd, names))
}

/* One directory of the hierarchy being walked. */
struct Frame {
    /* open while this directory is the current one or among the KEEP shallowest */
    fd: Option<OwnedFd>,
    id: Id,
    entries: Vec<Vec<u8>>,
    next: usize,
    /* the name in its parent (unused for the operand itself) */
    name: Vec<u8>,
    /* length of the display path of this directory */
    plen: usize,
    /* some entry is still there: cancelled, or it could not be removed */
    left: bool,
    /* the directory had no entries: one prompt covers steps 2b and 2d */
    was_empty: bool,
}

struct Rm<'a> {
    prog: Vec<u8>,
    opts: Opts,
    tty: bool,
    status: i32,
    ask: Box<dyn FnMut(&[u8]) -> bool + 'a>,
    root: Option<Id>,
    /* the unit tests collect diagnostics here instead of writing them */
    log: Option<Vec<String>>,
}

impl<'a> Rm<'a> {
    fn new(prog: &[u8], opts: Opts, tty: bool, ask: Box<dyn FnMut(&[u8]) -> bool + 'a>) -> Self {
        let root = lstat_at(libc::AT_FDCWD, c"/").ok().map(|s| id_of(&s));
        Rm { prog: prog.to_vec(), opts, tty, status: 0, ask, root, log: None }
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

    /*
     * With -f an entry that does not exist is no error. A name below
     * something that is not a directory does not exist either.
     */
    fn ignorable(&self, e: &std::io::Error) -> bool {
        self.opts.force && matches!(e.raw_os_error(), Some(libc::ENOENT) | Some(libc::ENOTDIR))
    }

    fn confirm(&mut self, what: &str, path: &[u8]) -> bool {
        let mut p = Vec::new();
        p.extend_from_slice(&self.prog);
        p.extend_from_slice(format!(": {what} '").as_bytes());
        p.extend_from_slice(path);
        p.extend_from_slice(b"'? ");
        (self.ask)(&p)
    }

    /* Step 3 and step 4 for something that is not a directory; true if it is gone. */
    fn remove_file(&mut self, dirfd: RawFd, name: &CStr, display: &[u8], st: &libc::stat) -> bool {
        /* a symbolic link has no permissions of its own to consult */
        let writable = is_link(st) || writable_at(dirfd, name);
        if need_prompt(&self.opts, writable, self.tty) && !self.confirm("remove", display) {
            return false;
        }
        match unlink_at(dirfd, name, 0) {
            Ok(()) => true,
            Err(e) if self.ignorable(&e) => true,
            Err(e) => {
                self.err_io(display, &e);
                false
            }
        }
    }

    /* The operand as given: steps 1 to 4 of the DESCRIPTION. */
    fn operand(&mut self, p: &[u8]) {
        let base = last_component(p);
        if base == b"." || base == b".." {
            self.err(p, "refusing to remove '.' or '..'");
            return;
        }
        /* however it is spelled, a pathname of only slashes is the root */
        if strip_slashes(p) == b"/" {
            self.err(p, "refusing to remove the root directory");
            return;
        }
        let cp = posix::cstr(p);
        let st = match lstat_at(libc::AT_FDCWD, &cp) {
            Ok(s) => s,
            Err(e) => {
                if !self.ignorable(&e) {
                    self.err_io(p, &e);
                }
                return;
            }
        };
        /* a trailing slash makes the lookup follow a link, so this sees the real target */
        if self.root == Some(id_of(&st)) {
            self.err(p, "refusing to remove the root directory");
            return;
        }
        /* never walk a directory without being able to tell whether it is the root */
        if is_dir(&st) && self.root.is_none() {
            self.err(p, "cannot tell whether this is the root directory");
            return;
        }
        if !is_dir(&st) {
            self.remove_file(libc::AT_FDCWD, &cp, p, &st);
            return;
        }
        if !self.opts.recursive {
            self.err(p, "is a directory");
            return;
        }
        /* "link/" names the directory the link points to; it is not walked through the link */
        let stripped = strip_slashes(p);
        if stripped.len() != p.len() {
            if let Ok(l) = lstat_at(libc::AT_FDCWD, &posix::cstr(stripped)) {
                if is_link(&l) {
                    self.err(p, "Not a directory");
                    return;
                }
            }
        }
        self.tree(p, &cp, &st);
    }

    /*
     * Open a directory for the walk and settle step 2b. None: it was skipped
     * (an error is reported already, or the answer was no).
     */
    fn enter(&mut self, dirfd: RawFd, name: &CStr, display: &[u8], st: &libc::stat, stack: &mut [Frame]) -> Option<Frame> {
        let writable = writable_at(dirfd, name);
        let mut got = open_and_read(dirfd, name);
        if matches!(&got, Err(e) if matches!(e.raw_os_error(), Some(libc::EMFILE) | Some(libc::ENFILE))) {
            /* out of descriptors: let go of the parents, they are found again through ".." */
            let n = stack.len();
            for f in stack.iter_mut().take(n.saturating_sub(1)) {
                f.fd = None;
            }
            got = open_and_read(dirfd, name);
        }
        let (fd, entries) = match got {
            Ok(v) => v,
            Err(e) => {
                if !self.ignorable(&e) {
                    self.err_io(display, &e);
                }
                return None;
            }
        };
        match fstat(fd.as_raw_fd()) {
            Ok(s) if id_of(&s) == id_of(st) => {}
            _ => {
                self.err(display, "directory changed during removal");
                return None;
            }
        }
        let was_empty = entries.is_empty();
        if need_prompt(&self.opts, writable, self.tty) {
            let what = if was_empty { "remove directory" } else { "examine files in directory" };
            if !self.confirm(what, display) {
                return None;
            }
        }
        Some(Frame { fd: Some(fd), id: id_of(st), entries, next: 0, name: Vec::new(), plen: display.len(), left: false, was_empty })
    }

    /*
     * Step 2 for a directory operand, with -R or -r. The hierarchy is walked
     * with an explicit stack and descriptor-relative calls, so that neither
     * its depth nor the length of a path matters, and a link met on the way
     * is removed, never followed.
     */
    fn tree(&mut self, display: &[u8], cp: &CStr, st: &libc::stat) {
        let mut stack: Vec<Frame> = Vec::new();
        let Some(top) = self.enter(libc::AT_FDCWD, cp, display, st, &mut stack) else { return };
        stack.push(top);
        let mut path = display.to_vec();
        loop {
            let ti = stack.len() - 1;
            if stack[ti].next < stack[ti].entries.len() {
                let k = stack[ti].next;
                let name = std::mem::take(&mut stack[ti].entries[k]);
                stack[ti].next += 1;
                let pfd = match stack[ti].fd.as_ref() {
                    Some(f) => f.as_raw_fd(),
                    None => return,
                };
                path.truncate(stack[ti].plen);
                if path.last() != Some(&b'/') {
                    path.push(b'/');
                }
                path.extend_from_slice(&name);
                let cname = posix::cstr(&name);
                match lstat_at(pfd, &cname) {
                    Err(e) => {
                        if !self.ignorable(&e) {
                            self.err_io(&path, &e);
                            stack[ti].left = true;
                        }
                    }
                    Ok(cst) if is_dir(&cst) => match self.enter(pfd, &cname, &path, &cst, &mut stack) {
                        Some(mut f) => {
                            f.name = name;
                            f.plen = path.len();
                            /* the parents beyond KEEP levels are closed to save descriptors */
                            if stack.len() >= KEEP {
                                stack[ti].fd = None;
                            }
                            stack.push(f);
                        }
                        None => stack[ti].left = true,
                    },
                    Ok(cst) => {
                        if !self.remove_file(pfd, &cname, &path, &cst) {
                            stack[ti].left = true;
                        }
                    }
                }
                continue;
            }

            /* every entry has been dealt with: step 2d, then step 4 for the directory itself */
            let frame = stack.pop().unwrap();
            path.truncate(frame.plen);
            /*
             * Step 2d asks even when an entry was left behind (PCTS 13 expects
             * the prompt); then the directory is only removed if nothing is
             * left in it, without a second "not empty" diagnostic on top of
             * the reason it is not empty.
             */
            let cancelled = self.opts.interactive && !frame.was_empty && !self.confirm("remove directory", &path);
            let keep = cancelled || frame.left;
            if stack.is_empty() {
                if !keep {
                    drop(frame.fd);
                    match unlink_at(libc::AT_FDCWD, cp, libc::AT_REMOVEDIR) {
                        Ok(()) => {}
                        Err(e) if self.ignorable(&e) => {}
                        Err(e) => self.err_io(&path, &e),
                    }
                }
                return;
            }
            if !self.climb(&mut stack, &frame) {
                return;
            }
            let pi = stack.len() - 1;
            if keep {
                stack[pi].left = true;
                continue;
            }
            let pfd = stack[pi].fd.as_ref().unwrap().as_raw_fd();
            drop(frame.fd);
            match unlink_at(pfd, &posix::cstr(&frame.name), libc::AT_REMOVEDIR) {
                Ok(()) => {}
                Err(e) if self.ignorable(&e) => {}
                Err(e) => {
                    self.err_io(&path, &e);
                    stack[pi].left = true;
                }
            }
        }
    }

    /*
     * Make sure the parent of a finished directory is open: if it was closed
     * to save descriptors, reopen it through ".." and check it is the same
     * directory. false: the walk cannot go on (reported).
     */
    fn climb(&mut self, stack: &mut [Frame], child: &Frame) -> bool {
        let pi = stack.len() - 1;
        if stack[pi].fd.is_some() {
            return true;
        }
        let up = child.fd.as_ref().map(|f| open_dir_at(f.as_raw_fd(), c".."));
        let fd = match up {
            Some(Ok(fd)) => fd,
            Some(Err(e)) => {
                let shown = parent_display(&stack[pi]);
                self.err(&shown, &format!("cannot return to the directory: {}", posix::strerror(&e)));
                return false;
            }
            None => return false,
        };
        match fstat(fd.as_raw_fd()) {
            Ok(s) if id_of(&s) == stack[pi].id => {}
            _ => {
                let shown = parent_display(&stack[pi]);
                self.err(&shown, "directory changed during removal");
                return false;
            }
        }
        stack[pi].fd = Some(fd);
        true
    }
}

/* A name for diagnostics about a directory whose full path is no longer at hand. */
fn parent_display(f: &Frame) -> Vec<u8> {
    if f.name.is_empty() {
        b".".to_vec()
    } else {
        f.name.clone()
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut o = Opts { force: false, interactive: false, recursive: false };
    let mut g = Getopt::new(&args, b"fiRr");
    while let Some(opt) = g.next() {
        match opt {
            /* of -f and -i the last one given decides */
            Opt::Flag(b'f') => {
                o.force = true;
                o.interactive = false;
            }
            Opt::Flag(b'i') => {
                o.interactive = true;
                o.force = false;
            }
            Opt::Flag(b'R') | Opt::Flag(b'r') => o.recursive = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let operands = g.operands();
    if operands.is_empty() {
        if o.force {
            std::process::exit(0);
        }
        posix::usage(SYNOPSIS);
    }
    let tty = unsafe { libc::isatty(0) } == 1;
    let yes = posix::prompt::Yes::new();
    let ask = Box::new(move |prompt: &[u8]| -> bool { posix::prompt::ask(&yes, prompt) });
    let mut rm = Rm::new(&prog, o, tty, ask);
    for p in operands {
        rm.operand(p);
    }
    std::process::exit(rm.status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    /* a fresh scratch directory that removes itself at the end */
    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("rm-test-{}-{}", std::process::id(), n));
            std::fs::create_dir(&p).unwrap();
            Tmp(p)
        }
        fn p(&self, s: &str) -> Vec<u8> {
            self.0.join(s).as_os_str().as_bytes().to_vec()
        }
        fn path(&self, s: &str) -> PathBuf {
            self.0.join(s)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            fn fix(p: &Path) {
                use std::os::unix::fs::PermissionsExt;
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

    fn opts(f: bool, i: bool, r: bool) -> Opts {
        Opts { force: f, interactive: i, recursive: r }
    }

    /* a remover that answers the prompts from a list (then "no") and records them */
    fn run(o: Opts, tty: bool, answers: &[bool], ops: &[Vec<u8>]) -> (i32, Vec<String>, Vec<String>) {
        let prompts = std::cell::RefCell::new(Vec::new());
        let mut it = answers.to_vec().into_iter();
        let (status, log) = {
            let ask = Box::new(|p: &[u8]| {
                prompts.borrow_mut().push(String::from_utf8_lossy(p).into_owned());
                it.next().unwrap_or(false)
            });
            let mut rm = Rm::new(b"rm", o, tty, ask);
            rm.log = Some(Vec::new());
            for p in ops {
                rm.operand(p);
            }
            (rm.status, rm.log.take().unwrap())
        };
        (status, log, prompts.into_inner())
    }

    #[test]
    fn last_component_basics() {
        assert_eq!(last_component(b"a/b"), b"b");
        assert_eq!(last_component(b"a/b/"), b"b");
        assert_eq!(last_component(b"a/b//"), b"b");
        assert_eq!(last_component(b"b"), b"b");
        assert_eq!(last_component(b"a/."), b".");
        assert_eq!(last_component(b"a/./"), b".");
        assert_eq!(last_component(b"a/.."), b"..");
        assert_eq!(last_component(b"a/../"), b"..");
        assert_eq!(last_component(b"/"), b"/");
        assert_eq!(last_component(b"///"), b"/");
        assert_eq!(last_component(b""), b"");
        assert_eq!(last_component(b"/a"), b"a");
        assert_eq!(last_component(b"..."), b"...");
        assert_eq!(last_component(b".a"), b".a");
    }

    #[test]
    fn prompt_rule() {
        /* the RATIONALE: (NOT f) AND ((not_writable AND tty) OR i) */
        for f in [false, true] {
            for i in [false, true] {
                for w in [false, true] {
                    for t in [false, true] {
                        let want = !f && ((!w && t) || i);
                        assert_eq!(need_prompt(&opts(f, i, false), w, t), want);
                    }
                }
            }
        }
    }

    #[test]
    fn plain_file_and_missing() {
        let t = Tmp::new();
        std::fs::write(t.path("f"), b"x").unwrap();
        let (st, log, pr) = run(opts(false, false, false), false, &[], &[t.p("f")]);
        assert_eq!((st, log.len(), pr.len()), (0, 0, 0));
        assert!(!t.path("f").exists());
        let (st, log, _) = run(opts(false, false, false), false, &[], &[t.p("f")]);
        assert_eq!((st, log.len()), (1, 1));
        let (st, log, _) = run(opts(true, false, false), false, &[], &[t.p("f")]);
        assert_eq!((st, log.len()), (0, 0));
    }

    #[test]
    fn force_ignores_enotdir_but_not_other_errors() {
        let t = Tmp::new();
        std::fs::write(t.path("f"), b"x").unwrap();
        let (st, log, _) = run(opts(true, false, false), false, &[], &[t.p("f/x")]);
        assert_eq!((st, log.len()), (0, 0));
        let (st, log, _) = run(opts(false, false, false), false, &[], &[t.p("f/x")]);
        assert_eq!((st, log.len()), (1, 1));
        /* a symbolic link loop is an error even with -f */
        std::os::unix::fs::symlink("l2", t.path("l1")).unwrap();
        std::os::unix::fs::symlink("l1", t.path("l2")).unwrap();
        let (st, log, _) = run(opts(true, false, false), false, &[], &[t.p("l1/x")]);
        assert_eq!((st, log.len()), (1, 1));
    }

    #[test]
    fn directory_needs_r() {
        let t = Tmp::new();
        std::fs::create_dir(t.path("d")).unwrap();
        let (st, log, _) = run(opts(false, false, false), false, &[], &[t.p("d")]);
        assert_eq!((st, log.len()), (1, 1));
        assert!(t.path("d").exists());
        /* -f does not make it work either */
        let (st, _, _) = run(opts(true, false, false), false, &[], &[t.p("d")]);
        assert_eq!(st, 1);
        assert!(t.path("d").exists());
        let (st, log, _) = run(opts(false, false, true), false, &[], &[t.p("d")]);
        assert_eq!((st, log.len()), (0, 0));
        assert!(!t.path("d").exists());
    }

    #[test]
    fn dot_dotdot_and_root_are_refused_and_the_rest_goes_on() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.path("d/s")).unwrap();
        std::fs::write(t.path("flag"), b"").unwrap();
        for bad in ["d/s/..", "d/s/../", "d/.", "d/./", "d/s/..//"] {
            let (st, log, _) = run(opts(false, false, true), false, &[], &[t.p(bad), t.p("flag")]);
            assert_eq!((st, log.len()), (1, 1), "{bad}");
            assert!(!t.path("flag").exists(), "{bad}");
            std::fs::write(t.path("flag"), b"").unwrap();
        }
        assert!(t.path("d/s").exists());
        /* "." as the whole operand, and the root in all its spellings */
        for bad in [&b"."[..], b"..", b"/", b"//", b"///", b"/."] {
            let (st, log, pr) = run(opts(false, false, true), false, &[], &[bad.to_vec()]);
            assert_eq!((st, log.len(), pr.len()), (1, 1, 0));
        }
        /* a link to the root, named with a trailing slash, resolves to the root */
        std::os::unix::fs::symlink("/", t.path("lnk")).unwrap();
        let (st, log, _) = run(opts(true, false, true), false, &[], &[t.p("lnk/"), t.p("flag")]);
        assert_eq!((st, log.len()), (1, 1));
        assert!(!t.path("flag").exists());
        /* without the slash it is just a link and is removed */
        let (st, _, _) = run(opts(false, false, true), false, &[], &[t.p("lnk")]);
        assert_eq!(st, 0);
        assert!(std::fs::symlink_metadata(t.path("lnk")).is_err());
    }

    #[test]
    fn hierarchy_is_removed_and_links_are_not_followed() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.path("keep/inner")).unwrap();
        std::fs::write(t.path("keep/inner/precious"), b"x").unwrap();
        std::fs::create_dir_all(t.path("d/a/b")).unwrap();
        std::fs::write(t.path("d/a/b/f"), b"x").unwrap();
        std::fs::write(t.path("d/g"), b"x").unwrap();
        std::os::unix::fs::symlink(t.path("keep"), t.path("d/a/tolink")).unwrap();
        std::os::unix::fs::symlink("nowhere", t.path("d/dangling")).unwrap();
        let (st, log, pr) = run(opts(false, false, true), false, &[], &[t.p("d")]);
        assert_eq!((st, log.len(), pr.len()), (0, 0, 0));
        assert!(!t.path("d").exists());
        assert!(t.path("keep/inner/precious").exists());
    }

    #[test]
    fn trailing_slash_link_to_a_directory_is_refused() {
        let t = Tmp::new();
        std::fs::create_dir(t.path("d")).unwrap();
        std::fs::write(t.path("d/f"), b"x").unwrap();
        std::os::unix::fs::symlink("d", t.path("lnk")).unwrap();
        let (st, log, _) = run(opts(false, false, true), false, &[], &[t.p("lnk/")]);
        assert_eq!((st, log.len()), (1, 1));
        assert!(t.path("d/f").exists());
        assert!(std::fs::symlink_metadata(t.path("lnk")).is_ok());
        /* the real directory with a trailing slash is fine */
        let (st, _, _) = run(opts(false, false, true), false, &[], &[t.p("d/")]);
        assert_eq!(st, 0);
        assert!(!t.path("d").exists());
    }

    #[test]
    fn interactive_file_prompts_and_answers() {
        let t = Tmp::new();
        std::fs::write(t.path("a"), b"").unwrap();
        std::fs::write(t.path("b"), b"").unwrap();
        let (st, log, pr) = run(opts(false, true, false), false, &[false, true], &[t.p("a"), t.p("b")]);
        assert_eq!((st, log.len(), pr.len()), (0, 0, 2));
        assert!(pr[0].contains("/a") && pr[1].contains("/b"));
        assert!(t.path("a").exists() && !t.path("b").exists());
    }

    #[test]
    fn readonly_file_prompts_only_on_a_terminal() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let t = Tmp::new();
        std::fs::write(t.path("ro"), b"").unwrap();
        std::fs::set_permissions(t.path("ro"), std::fs::Permissions::from_mode(0o444)).unwrap();
        let (st, _, pr) = run(opts(false, false, false), false, &[], &[t.p("ro")]);
        assert_eq!((st, pr.len()), (0, 0));
        std::fs::write(t.path("ro"), b"").unwrap();
        std::fs::set_permissions(t.path("ro"), std::fs::Permissions::from_mode(0o444)).unwrap();
        let (st, _, pr) = run(opts(false, false, false), true, &[false], &[t.p("ro")]);
        assert_eq!((st, pr.len()), (0, 1));
        assert!(t.path("ro").exists());
        let (_, _, pr) = run(opts(false, false, false), true, &[true], &[t.p("ro")]);
        assert_eq!(pr.len(), 1);
        assert!(!t.path("ro").exists());
        /* -f never asks */
        std::fs::write(t.path("ro"), b"").unwrap();
        std::fs::set_permissions(t.path("ro"), std::fs::Permissions::from_mode(0o444)).unwrap();
        let (st, _, pr) = run(opts(true, false, false), true, &[], &[t.p("ro")]);
        assert_eq!((st, pr.len()), (0, 0));
        assert!(!t.path("ro").exists());
    }

    #[test]
    fn interactive_directory_prompt_sequence() {
        /* PCTS 13: the directory, each file, then the directory again */
        let t = Tmp::new();
        std::fs::create_dir(t.path("d")).unwrap();
        std::fs::write(t.path("d/a"), b"").unwrap();
        std::fs::write(t.path("d/b"), b"").unwrap();
        /* yes to the directory, no to the first file asked, yes to the second, no to the directory */
        let (st, log, pr) = run(opts(false, true, true), false, &[true, false, true, false], &[t.p("d")]);
        assert_eq!((st, log.len(), pr.len()), (0, 0, 4));
        assert!(pr[0].contains("examine files in directory"));
        assert!(pr[3].contains("remove directory"));
        let left: Vec<_> = std::fs::read_dir(t.path("d")).unwrap().flatten().collect();
        assert_eq!(left.len(), 1);
        /* yes to everything, children still there is impossible: all gone */
        let (st, _, pr) = run(opts(false, true, true), false, &[true; 8], &[t.p("d")]);
        assert_eq!((st, pr.len()), (0, 3));
        assert!(!t.path("d").exists());
    }

    #[test]
    fn empty_directory_is_asked_about_once() {
        /* PCTS 14 */
        let t = Tmp::new();
        std::fs::create_dir(t.path("e")).unwrap();
        let (st, _, pr) = run(opts(false, true, true), false, &[false], &[t.p("e")]);
        assert_eq!((st, pr.len()), (0, 1));
        assert!(t.path("e").exists());
        let (st, _, pr) = run(opts(false, true, true), false, &[true], &[t.p("e")]);
        assert_eq!((st, pr.len()), (0, 1));
        assert!(!t.path("e").exists());
    }

    #[test]
    fn readonly_empty_directory_asks_on_a_terminal() {
        /* PCTS 12 */
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let t = Tmp::new();
        std::fs::create_dir(t.path("e")).unwrap();
        std::fs::set_permissions(t.path("e"), std::fs::Permissions::from_mode(0o555)).unwrap();
        let (st, _, pr) = run(opts(false, false, true), true, &[false], &[t.p("e")]);
        assert_eq!((st, pr.len()), (0, 1));
        assert!(t.path("e").exists());
        let (_, _, pr) = run(opts(false, false, true), true, &[true], &[t.p("e")]);
        assert_eq!(pr.len(), 1);
        assert!(!t.path("e").exists());
    }

    #[test]
    fn unremovable_child_is_reported_once_and_the_rest_goes() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let t = Tmp::new();
        std::fs::create_dir_all(t.path("d/locked")).unwrap();
        std::fs::write(t.path("d/locked/x"), b"").unwrap();
        std::fs::create_dir(t.path("d/ok")).unwrap();
        std::fs::write(t.path("d/ok/y"), b"").unwrap();
        std::fs::set_permissions(t.path("d/locked"), std::fs::Permissions::from_mode(0o555)).unwrap();
        let (st, log, _) = run(opts(false, false, true), false, &[], &[t.p("d")]);
        assert_eq!(st, 1);
        /* the failed unlink only: no "not empty" for the directories above it */
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(log[0].contains("locked/x"));
        assert!(!t.path("d/ok").exists());
        assert!(t.path("d/locked/x").exists());
    }

    #[test]
    fn unreadable_directory_is_an_error_and_siblings_go() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let t = Tmp::new();
        std::fs::create_dir_all(t.path("d/B")).unwrap();
        std::fs::write(t.path("d/B/fb"), b"").unwrap();
        std::fs::create_dir_all(t.path("d/C")).unwrap();
        std::fs::write(t.path("d/f1"), b"").unwrap();
        std::fs::set_permissions(t.path("d/B"), std::fs::Permissions::from_mode(0)).unwrap();
        let (st, log, _) = run(opts(false, false, true), false, &[], &[t.p("d")]);
        assert_eq!((st, log.len()), (1, 1));
        assert!(!t.path("d/C").exists() && !t.path("d/f1").exists());
        assert!(t.path("d").exists());
    }

    /* make a chain of directories n deep without ever forming a long pathname */
    fn deep(base: &Path, name: &str, n: usize) {
        let first = posix::cstr(base.join(name).as_os_str().as_bytes());
        unsafe { assert_eq!(libc::mkdir(first.as_ptr(), 0o755), 0) };
        let mut cur = open_dir_at(libc::AT_FDCWD, &first).unwrap();
        for k in 0..n {
            let leaf = posix::cstr(b"dddddddddddddddd");
            unsafe { assert_eq!(libc::mkdirat(cur.as_raw_fd(), leaf.as_ptr(), 0o755), 0) };
            let next = open_dir_at(cur.as_raw_fd(), &leaf).unwrap();
            if k % 7 == 0 {
                let f = posix::cstr(b"file");
                let fd = unsafe { libc::openat(next.as_raw_fd(), f.as_ptr(), libc::O_CREAT | libc::O_WRONLY, 0o644) };
                assert!(fd >= 0);
                unsafe { libc::close(fd) };
            }
            cur = next;
        }
    }

    #[test]
    fn depth_beyond_path_max_and_beyond_the_kept_descriptors() {
        let t = Tmp::new();
        deep(&t.0, "deep", 600);
        let (st, log, _) = run(opts(false, false, true), false, &[], &[t.p("deep")]);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
        assert!(!t.path("deep").exists());
    }

    #[test]
    fn depth_with_prompts_climbs_correctly() {
        let t = Tmp::new();
        deep(&t.0, "deep", 40);
        let (st, log, pr) = run(opts(false, true, true), false, &[true; 400], &[t.p("deep")]);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
        assert!(!t.path("deep").exists());
        assert!(pr.len() > 80);
    }

    #[test]
    fn operands_with_odd_bytes_and_newlines() {
        let t = Tmp::new();
        /* APFS refuses names that are not valid UTF-8, so: a newline, spaces, a dash, a non-ASCII letter */
        let mut name = b"odd".to_vec();
        name.extend_from_slice("\n  -\u{e9}".as_bytes());
        let mut p = t.0.as_os_str().as_bytes().to_vec();
        p.push(b'/');
        p.extend_from_slice(&name);
        std::fs::write(std::ffi::OsStr::from_bytes(&p), b"").unwrap();
        let (st, log, _) = run(opts(false, false, true), false, &[], &[p.clone()]);
        assert_eq!((st, log.len()), (0, 0));
        assert!(std::fs::symlink_metadata(std::ffi::OsStr::from_bytes(&p)).is_err());
    }
}
