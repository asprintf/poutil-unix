/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Walk a file hierarchy relative to open directory descriptors (chmod -R,
 * chown -R). A directory is visited before its contents, or after them if the
 * visitor says so (then first, if it cannot be opened).
 */

use crate::sys;
use std::ffi::CStr;
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

/* Each level holds a descriptor and a stack frame. */
const MAX_DEPTH: usize = 2000;

struct Walker<'v, V: Visit> {
    recursive: bool,
    links: Links,
    max_depth: usize,
    /* (device, inode) of the directories above */
    above: Vec<(u64, u64)>,
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

impl<V: Visit> Walker<'_, V> {
    fn entry(&mut self, dirfd: RawFd, name: &CStr, path: &[u8], operand: bool, depth: usize) {
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
                    self.children(&e, &fd, names, depth);
                    self.visit(&e);
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
        if self.above.contains(&id) {
            return Err(Opened::Report(io::Error::other("directory causes a cycle")));
        }
        if depth >= self.max_depth {
            return Err(Opened::Report(io::Error::other("file hierarchy too deep")));
        }
        let mut flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;
        if !e.followed {
            flags |= libc::O_NOFOLLOW;
        }
        let fd = match sys::openat_owned(e.dirfd, e.name, flags, 0) {
            Ok(fd) => fd,
            Err(err) if matches!(err.raw_os_error(), Some(libc::EACCES) | Some(libc::EPERM)) => return Err(Opened::Retry),
            Err(err) => return Err(Opened::Report(err)),
        };
        match sys::fstat(fd.as_raw_fd()) {
            Ok(s) if id_of(&s) == id => {}
            Ok(_) => return Err(Opened::Report(io::Error::other("directory changed during the walk"))),
            Err(err) => return Err(Opened::Report(err)),
        }
        let mut names = match sys::read_dir_fd(fd.as_raw_fd()) {
            Ok(n) => n,
            Err(err) => return Err(Opened::Report(err)),
        };
        names.retain(|n| n != b"." && n != b"..");
        /* a fixed order */
        names.sort();
        Ok((fd, names))
    }

    fn descend(&mut self, e: &Entry, depth: usize) {
        match self.open(e, depth) {
            Ok((fd, names)) => self.children(e, &fd, names, depth),
            Err(Opened::Report(err)) => self.v.report(e.path, &err),
            Err(Opened::Retry) => {
                let err = sys::openat_owned(e.dirfd, e.name, libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC, 0).err();
                self.v.report(e.path, &err.unwrap_or_else(|| io::Error::from_raw_os_error(libc::EACCES)));
            }
        }
    }

    fn children(&mut self, e: &Entry, fd: &OwnedFd, names: Vec<Vec<u8>>, depth: usize) {
        self.above.push(id_of(&e.st));
        for n in &names {
            let cname = crate::cstr(n);
            let p = join(e.path, n);
            self.entry(fd.as_raw_fd(), &cname, &p, false, depth + 1);
        }
        self.above.pop();
    }
}

enum Opened {
    /* no permission: the visit may give it */
    Retry,
    Report(io::Error),
}

/* Visit the operand and, if recursive, everything below it. */
pub fn walk<V: Visit>(operand: &[u8], recursive: bool, links: Links, v: &mut V) {
    walk_limited(operand, recursive, links, MAX_DEPTH, v)
}

fn walk_limited<V: Visit>(operand: &[u8], recursive: bool, links: Links, max_depth: usize, v: &mut V) {
    let mut w = Walker { recursive, links, max_depth, above: Vec::new(), v };
    let c = crate::cstr(operand);
    w.entry(libc::AT_FDCWD, &c, operand, true, 0);
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
        walk_limited(&t.p(""), true, Links::Never, 3, &mut r);
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
        walk_limited(&t.p(""), true, Links::Never, 3, &mut r);
        /* depth 0 is the operand: its entries are 1, a/b/c is 3, and c cannot be entered */
        assert_eq!(r.seen, vec!["D", "Da", "Da/b", "Da/b/c"]);
        assert_eq!(r.errs, vec!["a/b/c: file hierarchy too deep"]);
    }
}
