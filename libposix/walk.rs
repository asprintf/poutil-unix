/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Walk a file hierarchy relative to open directory descriptors (chmod -R,
 * chown -R). A directory is visited before its contents, or after them if the
 * visitor says so (then first, if it cannot be opened).
 */

use crate::sys;
use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};

/* Which symbolic links are followed. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Links {
    /* none: every symbolic link is an entry of its own */
    Never,
    /* those named as operands (not those found in a directory) */
    Operands,
    /* all of them */
    All,
}

/* One file of the hierarchy. */
pub struct Entry<'a> {
    /* name is relative to dirfd (AT_FDCWD for an operand) */
    pub dirfd: RawFd,
    pub name: &'a CStr,
    /* for diagnostics */
    pub path: &'a [u8],
    /* of the link itself if `link`, else of the file */
    pub st: libc::stat,
    /* a symbolic link that is not followed */
    pub link: bool,
    /* a symbolic link that was followed */
    pub followed: bool,
    pub operand: bool,
}

impl Entry<'_> {
    pub fn is_dir(&self) -> bool {
        !self.link && self.st.st_mode & libc::S_IFMT == libc::S_IFDIR
    }

    /* flag for the *at() calls */
    pub fn at_flags(&self) -> libc::c_int {
        if self.followed {
            0
        } else {
            libc::AT_SYMLINK_NOFOLLOW
        }
    }
}

pub trait Visit {
    /* An error is reported, then the walk goes on. */
    fn visit(&mut self, e: &Entry) -> io::Result<()>;
    /* Write the diagnostic. */
    fn report(&mut self, path: &[u8], e: &io::Error);
    /* Visit a directory after its contents? */
    fn after_contents(&self) -> bool {
        false
    }
}

/* Each level is a stack frame; only the deepest MAX_OPEN (fewer when out of descriptors) keep a descriptor. */
const MAX_DEPTH: usize = 2000;
const MAX_OPEN: usize = 32;

/* A directory being read: how to open it again, and its descriptor while it has one. */
struct Level {
    /* relative to the level above (AT_FDCWD for the operand) */
    name: CString,
    path: Vec<u8>,
    followed: bool,
    id: (u64, u64),
    fd: Option<OwnedFd>,
}

struct Walker<'v, V: Visit> {
    recursive: bool,
    links: Links,
    max_depth: usize,
    max_open: usize,
    /* the directories above, the deepest last; those from open_from on have a descriptor */
    stack: Vec<Level>,
    open_from: usize,
    v: &'v mut V,
}

fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut v = dir.to_vec();
    if v.last() != Some(&b'/') {
        v.push(b'/');
    }
    v.extend_from_slice(name);
    v
}

fn id_of(st: &libc::stat) -> (u64, u64) {
    (st.st_dev as u64, st.st_ino as u64)
}

fn dir_flags(followed: bool) -> libc::c_int {
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;
    if followed {
        flags
    } else {
        flags | libc::O_NOFOLLOW
    }
}

fn too_many(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::EMFILE) | Some(libc::ENFILE))
}

fn changed() -> io::Error {
    io::Error::other("directory changed during the walk")
}

impl<V: Visit> Walker<'_, V> {
    /* The directory the next entry is in (-1 if it was lost). */
    fn dirfd(&self) -> RawFd {
        match self.stack.last() {
            None => libc::AT_FDCWD,
            Some(l) => l.fd.as_ref().map_or(-1, |f| f.as_raw_fd()),
        }
    }

    fn entry(&mut self, name: &CStr, path: &[u8], operand: bool, depth: usize) {
        let dirfd = self.dirfd();
        let lst = match sys::fstatat(dirfd, name, libc::AT_SYMLINK_NOFOLLOW) {
            Ok(s) => s,
            Err(e) => {
                self.v.report(path, &e);
                return;
            }
        };
        let mut st = lst;
        let (mut link, mut followed) = (false, false);
        if lst.st_mode & libc::S_IFMT == libc::S_IFLNK {
            let follow = match self.links {
                Links::Never => false,
                Links::Operands => operand,
                Links::All => true,
            };
            if follow {
                match sys::fstatat(dirfd, name, 0) {
                    Ok(s) => {
                        st = s;
                        followed = true;
                    }
                    Err(e) => {
                        self.v.report(path, &e);
                        return;
                    }
                }
            } else {
                link = true;
            }
        }
        let e = Entry { dirfd, name, path, st, link, followed, operand };
        if !(self.recursive && e.is_dir()) {
            self.visit(&e);
            return;
        }
        if self.v.after_contents() {
            match self.open(&e, depth) {
                Ok((fd, names)) => {
                    /* the descriptor of the directory above may have been opened again */
                    if self.children(&e, fd, names, depth) {
                        let dirfd = self.dirfd();
                        self.visit(&Entry { dirfd, ..e });
                    }
                }
                /* not entered, but still visited */
                Err(Opened::Report(err)) => {
                    self.v.report(e.path, &err);
                    self.visit(&e);
                }
                /* cannot be read as it is: visit it first, then try again */
                Err(Opened::Retry) => {
                    self.visit(&e);
                    self.descend(&e, depth);
                }
            }
        } else {
            self.visit(&e);
            self.descend(&e, depth);
        }
    }

    fn visit(&mut self, e: &Entry) {
        if let Err(err) = self.v.visit(e) {
            self.v.report(e.path, &err);
        }
    }

    /* Open a directory, check it is the one that was seen, and read its names. */
    fn open(&mut self, e: &Entry, depth: usize) -> Result<(OwnedFd, Vec<Vec<u8>>), Opened> {
        let id = id_of(&e.st);
        if self.stack.iter().any(|l| l.id == id) {
            return Err(Opened::Report(io::Error::other("directory causes a cycle")));
        }
        if depth >= self.max_depth {
            return Err(Opened::Report(io::Error::other("file hierarchy too deep")));
        }
        let fd = loop {
            match sys::openat_owned(e.dirfd, e.name, dir_flags(e.followed), 0) {
                Ok(fd) => break fd,
                Err(err) if too_many(&err) && self.shed() => continue,
                Err(err) if matches!(err.raw_os_error(), Some(libc::EACCES) | Some(libc::EPERM)) => return Err(Opened::Retry),
                Err(err) => return Err(Opened::Report(err)),
            }
        };
        match sys::fstat(fd.as_raw_fd()) {
            Ok(s) if id_of(&s) == id => {}
            Ok(_) => return Err(Opened::Report(changed())),
            Err(err) => return Err(Opened::Report(err)),
        }
        let mut names = loop {
            match sys::read_dir_fd(fd.as_raw_fd()) {
                Ok(n) => break n,
                Err(err) if too_many(&err) && self.shed() => continue,
                Err(err) => return Err(Opened::Report(err)),
            }
        };
        names.retain(|n| n != b"." && n != b"..");
        /* a fixed order */
        names.sort();
        Ok((fd, names))
    }

    fn descend(&mut self, e: &Entry, depth: usize) {
        match self.open(e, depth) {
            Ok((fd, names)) => {
                self.children(e, fd, names, depth);
            }
            Err(Opened::Report(err)) => self.v.report(e.path, &err),
            Err(Opened::Retry) => {
                let err = sys::openat_owned(e.dirfd, e.name, libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC, 0).err();
                self.v.report(e.path, &err.unwrap_or_else(|| io::Error::from_raw_os_error(libc::EACCES)));
            }
        }
    }

    /* Walk the names of e; false if the directory above e could not be opened again. */
    fn children(&mut self, e: &Entry, fd: OwnedFd, names: Vec<Vec<u8>>, depth: usize) -> bool {
        self.push(Level { name: e.name.to_owned(), path: e.path.to_vec(), followed: e.followed, id: id_of(&e.st), fd: Some(fd) });
        for n in &names {
            if self.dirfd() < 0 {
                break;
            }
            let cname = crate::cstr(n);
            let p = join(e.path, n);
            self.entry(&cname, &p, false, depth + 1);
        }
        self.pop()
    }

    /* Enter a level; past max_open, the shallowest open one gives up its descriptor. */
    fn push(&mut self, l: Level) {
        self.stack.push(l);
        if self.stack.len() - self.open_from > self.max_open {
            self.stack[self.open_from].fd = None;
            self.open_from += 1;
        }
    }

    /* Out of descriptors: the shallowest open level other than the deepest gives up its own, for good. */
    fn shed(&mut self) -> bool {
        if self.stack.len() - self.open_from < 2 {
            return false;
        }
        self.stack[self.open_from].fd = None;
        self.open_from += 1;
        self.max_open = self.stack.len() - self.open_from;
        true
    }

    /* Leave a level, opening the one above again if it gave up its descriptor. */
    fn pop(&mut self) -> bool {
        let child = self.stack.pop().expect("a level to leave");
        let n = self.stack.len();
        if n == 0 {
            self.open_from = 0;
            return true;
        }
        if self.stack[n - 1].fd.is_some() {
            return true;
        }
        match self.reopen(child.fd) {
            Ok(fd) => {
                self.stack[n - 1].fd = Some(fd);
                self.open_from = n - 1;
                true
            }
            Err(err) => {
                self.v.report(&self.stack[n - 1].path, &err);
                self.open_from = n;
                false
            }
        }
    }

    /* The deepest level again: ".." of the child, else every level from the operand down. */
    fn reopen(&self, child: Option<OwnedFd>) -> io::Result<OwnedFd> {
        let want = self.stack.last().expect("a level").id;
        if let Some(c) = child {
            if let Ok(fd) = sys::openat_owned(c.as_raw_fd(), c"..", dir_flags(false), 0) {
                if sys::fstat(fd.as_raw_fd()).is_ok_and(|s| id_of(&s) == want) {
                    return Ok(fd);
                }
            }
        }
        /* entered through a followed link, or moved */
        let mut fd: Option<OwnedFd> = None;
        for l in &self.stack {
            let base = fd.as_ref().map_or(libc::AT_FDCWD, |f| f.as_raw_fd());
            let next = sys::openat_owned(base, &l.name, dir_flags(l.followed), 0)?;
            if id_of(&sys::fstat(next.as_raw_fd())?) != l.id {
                return Err(changed());
            }
            fd = Some(next);
        }
        fd.ok_or_else(changed)
    }
}

enum Opened {
    /* no permission: the visit may give it */
    Retry,
    Report(io::Error),
}

/* Visit the operand and, if recursive, everything below it. */
pub fn walk<V: Visit>(operand: &[u8], recursive: bool, links: Links, v: &mut V) {
    walk_limited(operand, recursive, links, MAX_DEPTH, MAX_OPEN, v)
}

fn walk_limited<V: Visit>(operand: &[u8], recursive: bool, links: Links, max_depth: usize, max_open: usize, v: &mut V) {
    let max_open = max_open.max(1);
    let mut w = Walker { recursive, links, max_depth, max_open, stack: Vec::new(), open_from: 0, v };
    let c = crate::cstr(operand);
    w.entry(&c, operand, true, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("walk-test-{}-{}", std::process::id(), n));
            std::fs::create_dir(&p).unwrap();
            Tmp(p)
        }
        fn p(&self, s: &str) -> Vec<u8> {
            self.0.join(s).as_os_str().as_bytes().to_vec()
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Rec {
        seen: Vec<String>,
        errs: Vec<String>,
        root_len: usize,
        post: bool,
    }

    impl Visit for Rec {
        fn visit(&mut self, e: &Entry) -> io::Result<()> {
            let tag = if e.link {
                "L"
            } else if e.followed {
                "F"
            } else if e.is_dir() {
                "D"
            } else {
                "f"
            };
            let p = String::from_utf8_lossy(&e.path[self.root_len..]).into_owned();
            self.seen.push(format!("{tag}{p}"));
            Ok(())
        }
        fn report(&mut self, path: &[u8], e: &io::Error) {
            let p = String::from_utf8_lossy(&path[self.root_len..]).into_owned();
            self.errs.push(format!("{p}: {}", crate::strerror(e)));
        }
        fn after_contents(&self) -> bool {
            self.post
        }
    }

    fn go(t: &Tmp, operand: &str, recursive: bool, links: Links) -> (Vec<String>, Vec<String>) {
        let root = t.p("");
        let mut r = Rec { seen: vec![], errs: vec![], root_len: root.len(), post: false };
        walk(&t.p(operand), recursive, links, &mut r);
        (r.seen, r.errs)
    }

    fn tree(t: &Tmp) {
        std::fs::create_dir_all(t.0.join("d/sub")).unwrap();
        std::fs::write(t.0.join("d/a"), "").unwrap();
        std::fs::write(t.0.join("d/sub/b"), "").unwrap();
        std::fs::create_dir(t.0.join("other")).unwrap();
        std::fs::write(t.0.join("other/o"), "").unwrap();
        symlink("../other", t.0.join("d/tolink")).unwrap();
        symlink("a", t.0.join("d/tofile")).unwrap();
        symlink("nowhere", t.0.join("d/dangling")).unwrap();
    }

    #[test]
    fn not_recursive_visits_only_the_operand() {
        let t = Tmp::new();
        tree(&t);
        assert_eq!(go(&t, "d", false, Links::Never).0, vec!["Dd"]);
        assert_eq!(go(&t, "d/a", true, Links::Never).0, vec!["fd/a"]);
    }

    #[test]
    fn physical_walk_visits_links_as_links_and_does_not_enter_them() {
        let t = Tmp::new();
        tree(&t);
        let (seen, errs) = go(&t, "d", true, Links::Never);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(seen, vec!["Dd", "fd/a", "Ld/dangling", "Dd/sub", "fd/sub/b", "Ld/tofile", "Ld/tolink"]);
    }

    #[test]
    fn operands_only_follows_the_operand() {
        let t = Tmp::new();
        tree(&t);
        symlink("d", t.0.join("top")).unwrap();
        let (seen, errs) = go(&t, "top", true, Links::Operands);
        assert!(errs.is_empty(), "{errs:?}");
        /* the operand is a followed link and a directory; the links inside stay links */
        assert_eq!(seen[0], "Ftop");
        assert!(seen.contains(&"Ltop/tolink".to_string()), "{seen:?}");
        assert!(!seen.iter().any(|s| s.contains("other")));
        /* with Never the operand link is only itself */
        assert_eq!(go(&t, "top", true, Links::Never).0, vec!["Ltop"]);
    }

    #[test]
    fn all_follows_every_link_and_reports_a_dangling_one() {
        let t = Tmp::new();
        tree(&t);
        let (seen, errs) = go(&t, "d", true, Links::All);
        assert!(seen.contains(&"Fd/tolink".to_string()), "{seen:?}");
        assert!(seen.contains(&"Fd/tolink/o".to_string()) || seen.contains(&"fd/tolink/o".to_string()), "{seen:?}");
        assert!(seen.contains(&"Fd/tofile".to_string()));
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].starts_with("d/dangling: "), "{errs:?}");
    }

    #[test]
    fn after_contents_visits_a_directory_last_unless_it_cannot_be_opened() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.0.join("d/s")).unwrap();
        std::fs::write(t.0.join("d/a"), "").unwrap();
        std::fs::write(t.0.join("d/s/b"), "").unwrap();
        let mut r = Rec { seen: vec![], errs: vec![], root_len: t.p("").len(), post: true };
        walk(&t.p("d"), true, Links::Never, &mut r);
        assert!(r.errs.is_empty());
        assert_eq!(r.seen, vec!["fd/a", "fd/s/b", "Dd/s", "Dd"]);
        /* a directory that cannot be opened is visited before the attempt to read it, and once */
        std::fs::set_permissions(t.0.join("d/s"), std::os::unix::fs::PermissionsExt::from_mode(0o0)).unwrap();
        let mut r = Rec { seen: vec![], errs: vec![], root_len: t.p("").len(), post: true };
        walk(&t.p("d"), true, Links::Never, &mut r);
        std::fs::set_permissions(t.0.join("d/s"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        if sys_euid() != 0 {
            assert_eq!(r.seen, vec!["fd/a", "Dd/s", "Dd"]);
            assert_eq!(r.errs.len(), 1, "{:?}", r.errs);
            assert!(r.errs[0].starts_with("d/s: "));
        }
    }

    #[test]
    fn after_contents_still_visits_a_directory_it_cannot_enter() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.0.join("a/b/c/d")).unwrap();
        let mut r = Rec { seen: vec![], errs: vec![], root_len: t.p("").len(), post: true };
        walk_limited(&t.p(""), true, Links::Never, 3, MAX_OPEN, &mut r);
        assert_eq!(r.seen, vec!["Da/b/c", "Da/b", "Da", "D"]);
        assert_eq!(r.errs, vec!["a/b/c: file hierarchy too deep"]);
    }

    #[test]
    fn a_link_cycle_is_reported_and_not_followed_again() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.0.join("c/in")).unwrap();
        symlink("..", t.0.join("c/in/up")).unwrap();
        let (seen, errs) = go(&t, "c", true, Links::All);
        assert!(seen.contains(&"Fc/in/up".to_string()), "{seen:?}");
        assert_eq!(errs, vec!["c/in/up: directory causes a cycle"]);
        /* nothing below the cycle was visited a second time */
        assert_eq!(seen.iter().filter(|s| s.ends_with("/in")).count(), 1);
    }

    #[test]
    fn missing_operand_and_unreadable_directory_are_reported_and_the_walk_goes_on() {
        let t = Tmp::new();
        let (seen, errs) = go(&t, "nosuch", true, Links::Never);
        assert!(seen.is_empty());
        assert_eq!(errs.len(), 1);
        std::fs::create_dir_all(t.0.join("u/locked")).unwrap();
        std::fs::write(t.0.join("u/z"), "").unwrap();
        std::fs::set_permissions(t.0.join("u/locked"), std::os::unix::fs::PermissionsExt::from_mode(0o0)).unwrap();
        let (seen, errs) = go(&t, "u", true, Links::Never);
        std::fs::set_permissions(t.0.join("u/locked"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        assert!(seen.contains(&"Du/locked".to_string()));
        assert!(seen.contains(&"fu/z".to_string()));
        /* running as root nothing is unreadable */
        if sys_euid() != 0 {
            assert_eq!(errs.len(), 1, "{errs:?}");
            assert!(errs[0].starts_with("u/locked: "));
        }
    }

    fn sys_euid() -> u32 {
        crate::user::euid()
    }

    fn go_open(t: &Tmp, operand: &str, links: Links, post: bool, max_open: usize) -> (Vec<String>, Vec<String>) {
        let mut r = Rec { seen: vec![], errs: vec![], root_len: t.p("").len(), post };
        walk_limited(&t.p(operand), true, links, MAX_DEPTH, max_open, &mut r);
        (r.seen, r.errs)
    }

    #[test]
    fn one_or_two_descriptors_walk_a_deep_tree_like_many() {
        let t = Tmp::new();
        let mut p = t.0.join("d");
        for i in 0..12 {
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join(format!("f{i}")), "").unwrap();
            p = p.join("s");
        }
        for post in [false, true] {
            for max_open in [1, 2] {
                let few = go_open(&t, "d", Links::Never, post, max_open);
                assert!(few.1.is_empty(), "{:?}", few.1);
                assert_eq!(few.0.len(), 24);
                assert_eq!(few, go_open(&t, "d", Links::Never, post, MAX_OPEN), "post {post}");
            }
        }
    }

    #[test]
    fn a_level_entered_through_a_link_is_opened_again_from_the_operand() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.0.join("real/a/b/c")).unwrap();
        std::fs::write(t.0.join("real/a/b/c/f"), "").unwrap();
        std::fs::create_dir(t.0.join("top")).unwrap();
        symlink("../real", t.0.join("top/l")).unwrap();
        std::fs::write(t.0.join("top/z"), "").unwrap();
        for post in [false, true] {
            let few = go_open(&t, "top", Links::All, post, 2);
            assert!(few.1.is_empty(), "{:?}", few.1);
            assert!(few.0.contains(&"ftop/l/a/b/c/f".to_string()) && few.0.contains(&"ftop/z".to_string()), "{:?}", few.0);
            assert_eq!(few, go_open(&t, "top", Links::All, post, MAX_OPEN), "post {post}");
        }
    }

    #[test]
    fn deep_hierarchy_does_not_need_a_long_pathname() {
        let t = Tmp::new();
        /* 50 levels of 100 bytes: more than PATH_MAX, made one directory descriptor at a time */
        let name = crate::cstr(&[b'x'; 100]);
        let mut fd = crate::sys::openat_owned(libc::AT_FDCWD, &crate::cstr(&t.p("")), libc::O_RDONLY | libc::O_DIRECTORY, 0).unwrap();
        for _ in 0..50 {
            assert_eq!(crate::sys::mkdirat(fd.as_raw_fd(), &name, 0o755), 0);
            fd = crate::sys::openat_owned(fd.as_raw_fd(), &name, libc::O_RDONLY | libc::O_DIRECTORY, 0).unwrap();
        }
        let (seen, errs) = go(&t, "", true, Links::Never);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(seen.len(), 51);
    }

    #[test]
    fn a_hierarchy_deeper_than_the_limit_is_reported_and_not_entered() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.0.join("a/b/c/d/e/f")).unwrap();
        let mut r = Rec { seen: vec![], errs: vec![], root_len: t.p("").len(), post: false };
        walk_limited(&t.p(""), true, Links::Never, 3, MAX_OPEN, &mut r);
        /* depth 0 is the operand: its entries are 1, a/b/c is 3, and c cannot be entered */
        assert_eq!(r.seen, vec!["D", "Da", "Da/b", "Da/b/c"]);
        assert_eq!(r.errs, vec!["a/b/c: file hierarchy too deep"]);
    }
}
