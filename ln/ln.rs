/* SPDX-License-Identifier: GPL-2.0-only */
/* ln - link files. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};

const SYNOPSIS: &str = "ln [-fs] [-L|-P] source_file target_file\n       ln [-fs] [-L|-P] source_file... target_dir";

#[derive(Clone, Copy)]
struct Opts {
    force: bool,
    symbolic: bool,
    /* -L (default) or -P; linkat() always gets the flag spelled out */
    follow: bool,
}

/* directory (device, inode) and name */
type Entry = (u64, u64, Vec<u8>);

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

fn kind(st: &libc::stat) -> libc::mode_t {
    st.st_mode & libc::S_IFMT
}

/* "" for only slashes */
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

/* "." if there is none */
fn dir_part(p: &[u8]) -> Vec<u8> {
    let mut n = p.len();
    while n > 0 && p[n - 1] == b'/' {
        n -= 1;
    }
    let s = &p[..n];
    match s.iter().rposition(|&c| c == b'/') {
        None => b".".to_vec(),
        Some(i) => {
            let mut j = i;
            while j > 0 && s[j - 1] == b'/' {
                j -= 1;
            }
            if j == 0 {
                /* "/x": the root, and "//x" keeps its two slashes */
                s[..i + 1].to_vec()
            } else {
                s[..j].to_vec()
            }
        }
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

/* The destination path of a source linked into a directory target. */
fn dest_in(target: &[u8], src: &[u8]) -> Vec<u8> {
    let base = last_component(src);
    if base.is_empty() {
        target.to_vec()
    } else {
        join(target, base)
    }
}

/* The entry a pathname names, so that two spellings compare equal. */
fn entry_of(path: &[u8]) -> Option<Entry> {
    let name = last_component(path);
    if name.is_empty() {
        return None;
    }
    let dir = stat_path(&dir_part(path), true).ok()?;
    Some((dir.st_dev as u64, dir.st_ino as u64, name.to_vec()))
}

type Pair = (Vec<u8>, Vec<u8>);

/* (source, destination) pairs */
fn plan(sources: &[Vec<u8>], target: &[u8], target_is_dir: bool) -> Result<Vec<Pair>, &'static str> {
    if target_is_dir {
        return Ok(sources.iter().map(|s| (s.clone(), dest_in(target, s))).collect());
    }
    if sources.len() > 1 {
        return Err("not a directory");
    }
    Ok(vec![(sources[0].clone(), target.to_vec())])
}

struct Ln {
    prog: Vec<u8>,
    o: Opts,
    status: i32,
    /* made by this run: never replaced by a later source */
    made: Vec<Entry>,
    /* the tests collect diagnostics here */
    log: Option<Vec<String>>,
}

impl Ln {
    fn new(prog: &[u8], o: Opts) -> Ln {
        Ln { prog: prog.to_vec(), o, status: 0, made: Vec::new(), log: None }
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

    fn err_code(&mut self, path: &[u8], code: i32) {
        self.err_io(path, &std::io::Error::from_raw_os_error(code));
    }

    /* The steps of the DESCRIPTION for one source and destination. */
    fn link_one(&mut self, src: &[u8], dest: &[u8]) {
        let cdest = posix::cstr(dest);
        let csrc = posix::cstr(src);
        /* look first, so nothing is removed for a link that cannot be made */
        let mut linked: Option<libc::stat> = None;
        let mut is_link = false;
        if !self.o.symbolic {
            let ls = match posix::sys::lstat(&csrc) {
                Ok(s) => s,
                Err(e) => {
                    self.err_io(src, &e);
                    return;
                }
            };
            is_link = kind(&ls) == libc::S_IFLNK;
            let st = if is_link && self.o.follow {
                match posix::sys::stat(&csrc) {
                    Ok(s) => s,
                    Err(e) => {
                        self.err_io(src, &e);
                        return;
                    }
                }
            } else {
                ls
            };
            /* hard links to directories are refused */
            if kind(&st) == libc::S_IFDIR {
                self.err_code(src, libc::EPERM);
                return;
            }
            linked = Some(st);
        }
        let existing = posix::sys::lstat(&cdest).ok();
        if let Some(d) = &existing {
            /* 1a */
            if !self.o.force {
                self.err_code(dest, libc::EEXIST);
                return;
            }
            /* 1b */
            let (se, de) = (entry_of(src), entry_of(dest));
            if se.is_some() && se == de {
                self.err(dest, "is the same directory entry as the source");
                return;
            }
            if let Some(l) = &linked {
                /* the only name of the file that would be linked */
                if l.st_dev == d.st_dev && l.st_ino == d.st_ino && l.st_nlink <= 1 {
                    self.err(dest, "is the same directory entry as the source");
                    return;
                }
            }
            if let Some(de) = &de {
                if self.made.contains(de) {
                    self.err(dest, "will not overwrite just-created file");
                    return;
                }
            }
            /* say so before the destination goes */
            if let Some(l) = &linked {
                if let Ok(dir) = stat_path(&dir_part(dest), true) {
                    if dir.st_dev != l.st_dev {
                        self.err_code(dest, libc::EXDEV);
                        return;
                    }
                }
            }
            /* 1c */
            if posix::sys::unlink(&cdest) < 0 {
                let e = last_err();
                self.err_io(dest, &e);
                return;
            }
        }
        /* 2 */
        let r = if self.o.symbolic {
            posix::sys::symlink(&csrc, &cdest)
        } else if is_link {
            /* 3 */
            let flags = if self.o.follow { libc::AT_SYMLINK_FOLLOW } else { 0 };
            posix::sys::linkat(libc::AT_FDCWD, &csrc, libc::AT_FDCWD, &cdest, flags)
        } else {
            /* 4 */
            posix::sys::link(&csrc, &cdest)
        };
        if r < 0 {
            let e = last_err();
            self.err_io(dest, &e);
            return;
        }
        if let Some(en) = entry_of(dest) {
            self.made.push(en);
        }
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut o = Opts { force: false, symbolic: false, follow: true };
    let mut g = Getopt::new(&args, b"fsLP");
    for opt in g.by_ref() {
        match opt {
            Opt::Flag(b'f') => o.force = true,
            Opt::Flag(b's') => o.symbolic = true,
            Opt::Flag(b'L') => o.follow = true,
            Opt::Flag(b'P') => o.follow = false,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let operands = g.operands();
    if operands.len() < 2 {
        posix::usage(SYNOPSIS);
    }
    let (sources, target) = (&operands[..operands.len() - 1], &operands[operands.len() - 1]);
    let target_is_dir = matches!(stat_path(target, true), Ok(s) if kind(&s) == libc::S_IFDIR);
    let pairs = match plan(sources, target, target_is_dir) {
        Ok(p) => p,
        Err(msg) => {
            posix::warn_path(&prog, target, msg);
            std::process::exit(1);
        }
    };
    let mut ln = Ln::new(&prog, o);
    for (s, d) in &pairs {
        ln.link_one(s, d);
    }
    std::process::exit(ln.status);
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
            let p = std::env::temp_dir().join(format!("ln-test-{}-{}", std::process::id(), n));
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
        fn ino(&self, s: &str) -> u64 {
            std::fs::metadata(self.path(s)).unwrap().ino()
        }
        fn nlink(&self, s: &str) -> u64 {
            std::fs::metadata(self.path(s)).unwrap().nlink()
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

    fn opts(force: bool, symbolic: bool, follow: bool) -> Opts {
        Opts { force, symbolic, follow }
    }

    /* the pairs the operands make, linked in order */
    fn run(o: Opts, pairs: &[(Vec<u8>, Vec<u8>)]) -> (i32, Vec<String>) {
        let mut ln = Ln::new(b"ln", o);
        ln.log = Some(Vec::new());
        for (s, d) in pairs {
            ln.link_one(s, d);
        }
        (ln.status, ln.log.take().unwrap())
    }

    fn one(o: Opts, s: Vec<u8>, d: Vec<u8>) -> (i32, Vec<String>) {
        run(o, &[(s, d)])
    }

    #[test]
    fn components_and_joins() {
        assert_eq!(last_component(b"a/b/c"), b"c");
        assert_eq!(last_component(b"a/b/c///"), b"c");
        assert_eq!(last_component(b"c"), b"c");
        assert_eq!(last_component(b"/"), b"");
        assert_eq!(last_component(b""), b"");
        assert_eq!(dir_part(b"a/b/c"), b"a/b");
        assert_eq!(dir_part(b"a/b//c//"), b"a/b");
        assert_eq!(dir_part(b"c"), b".");
        assert_eq!(dir_part(b"/c"), b"/");
        assert_eq!(dir_part(b"//c"), b"//");
        assert_eq!(dir_part(b"///c"), b"///");
        assert_eq!(join(b"d", b"f"), b"d/f");
        assert_eq!(join(b"d/", b"f"), b"d/f");
        assert_eq!(dest_in(b"d", b"x/y/z"), b"d/z");
        assert_eq!(dest_in(b"d", b"x/y/z/"), b"d/z");
        assert_eq!(dest_in(b"d", b"/"), b"d");
    }

    #[test]
    fn plan_forms() {
        let s = vec![b"a".to_vec(), b"b".to_vec()];
        assert_eq!(plan(&s, b"d", true).unwrap(), vec![(b"a".to_vec(), b"d/a".to_vec()), (b"b".to_vec(), b"d/b".to_vec())]);
        assert!(plan(&s, b"d", false).is_err());
        assert_eq!(plan(&s[..1], b"t", false).unwrap(), vec![(b"a".to_vec(), b"t".to_vec())]);
    }

    #[test]
    fn hard_link_to_a_new_name() {
        let t = Tmp::new();
        t.write("a", "data");
        let (st, log) = one(opts(false, false, true), t.p("a"), t.p("b"));
        assert_eq!((st, log.len()), (0, 0));
        assert_eq!(t.read("b"), "data");
        assert_eq!(t.ino("a"), t.ino("b"));
        assert_eq!(t.nlink("a"), 2);
    }

    #[test]
    fn symbolic_link_keeps_the_text_and_needs_no_file() {
        let t = Tmp::new();
        let (st, _) = one(opts(false, true, true), b"no/such/file".to_vec(), t.p("s"));
        assert_eq!(st, 0);
        assert_eq!(std::fs::read_link(t.path("s")).unwrap().as_os_str().as_bytes(), b"no/such/file");
        /* -L and -P are ignored with -s */
        let (st, _) = one(opts(false, true, false), b"x".to_vec(), t.p("s2"));
        assert_eq!(st, 0);
        assert_eq!(std::fs::read_link(t.path("s2")).unwrap().as_os_str().as_bytes(), b"x");
    }

    #[test]
    fn existing_destination_is_an_error_without_f_and_replaced_with_f() {
        let t = Tmp::new();
        t.write("a", "A");
        t.write("b", "B");
        let (st, log) = one(opts(false, false, true), t.p("a"), t.p("b"));
        assert_eq!(st, 1);
        assert!(log[0].contains("File exists"), "{log:?}");
        assert_eq!(t.read("b"), "B");
        let (st, log) = one(opts(true, false, true), t.p("a"), t.p("b"));
        assert_eq!((st, log.len()), (0, 0));
        assert_eq!(t.read("b"), "A");
        assert_eq!(t.ino("a"), t.ino("b"));
    }

    #[test]
    fn a_to_a_never_unlinks_a_with_or_without_f() {
        let t = Tmp::new();
        t.write("a", "A");
        for f in [false, true] {
            let (st, log) = one(opts(f, false, true), t.p("a"), t.p("a"));
            assert_eq!(st, 1, "force {f}");
            assert_eq!(log.len(), 1);
            assert_eq!(t.read("a"), "A");
        }
        /* spelled differently */
        std::fs::create_dir(t.path("d")).unwrap();
        let mut spelled = t.p("d");
        spelled.extend_from_slice(b"/../a");
        let (st, _) = one(opts(true, false, true), spelled, t.p("a"));
        assert_eq!(st, 1);
        assert_eq!(t.read("a"), "A");
        /* symbolic as well */
        let (st, _) = one(opts(true, true, true), t.p("a"), t.p("a"));
        assert_eq!(st, 1);
        assert_eq!(t.read("a"), "A");
    }

    #[test]
    fn f_does_not_destroy_the_file_a_link_would_lead_to() {
        let t = Tmp::new();
        t.write("t", "T");
        symlink("t", t.path("l")).unwrap();
        /* ln -f l t: the link is followed, so the only name of t would be removed */
        let (st, _) = one(opts(true, false, true), t.p("l"), t.p("t"));
        assert_eq!(st, 1);
        assert_eq!(t.read("t"), "T");
        /* with -P the link itself is linked and t may go: l is not t */
        let (st, _) = one(opts(true, false, false), t.p("l"), t.p("t"));
        assert_eq!(st, 0);
        assert!(std::fs::symlink_metadata(t.path("t")).unwrap().file_type().is_symlink());
    }

    #[test]
    fn f_with_a_missing_source_leaves_the_destination_alone() {
        let t = Tmp::new();
        t.write("b", "B");
        let (st, log) = one(opts(true, false, true), t.p("nosuch"), t.p("b"));
        assert_eq!(st, 1);
        assert!(log[0].contains("nosuch"), "{log:?}");
        assert_eq!(t.read("b"), "B");
        /* a dangling link followed with -L is a missing source; with -P it is a link to link */
        symlink("nowhere", t.path("dang")).unwrap();
        let (st, _) = one(opts(true, false, true), t.p("dang"), t.p("b"));
        assert_eq!(st, 1);
        assert_eq!(t.read("b"), "B");
        let (st, _) = one(opts(true, false, false), t.p("dang"), t.p("b"));
        assert_eq!(st, 0);
        assert_eq!(std::fs::read_link(t.path("b")).unwrap().as_os_str().as_bytes(), b"nowhere");
    }

    #[test]
    fn unlinkable_destination_is_reported_and_the_next_source_goes_on() {
        let t = Tmp::new();
        t.write("a", "A");
        t.write("b", "B");
        std::fs::create_dir(t.path("d")).unwrap();
        std::fs::create_dir(t.path("d/a")).unwrap();
        t.write("d/b", "old");
        let pairs = plan(&[t.p("a"), t.p("b")], &t.p("d"), true).unwrap();
        let (st, log) = run(opts(true, false, true), &pairs);
        assert_eq!(st, 1);
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("d/a"), "{log:?}");
        assert_eq!(t.read("d/b"), "B");
        assert_eq!(t.ino("b"), t.ino("d/b"));
    }

    #[test]
    fn a_destination_made_by_this_run_is_not_replaced_by_a_later_source() {
        let t = Tmp::new();
        std::fs::create_dir(t.path("x")).unwrap();
        std::fs::create_dir(t.path("y")).unwrap();
        std::fs::create_dir(t.path("d")).unwrap();
        t.write("x/f", "X");
        t.write("y/f", "Y");
        let pairs = plan(&[t.p("x/f"), t.p("y/f")], &t.p("d"), true).unwrap();
        let (st, log) = run(opts(true, false, true), &pairs);
        assert_eq!(st, 1);
        assert!(log[0].contains("just-created"), "{log:?}");
        assert_eq!(t.read("d/f"), "X");
        /* without -f it is the ordinary "File exists" */
        let t = Tmp::new();
        std::fs::create_dir(t.path("x")).unwrap();
        std::fs::create_dir(t.path("y")).unwrap();
        std::fs::create_dir(t.path("d")).unwrap();
        t.write("x/f", "X");
        t.write("y/f", "Y");
        let pairs = plan(&[t.p("x/f"), t.p("y/f")], &t.p("d"), true).unwrap();
        let (st, log) = run(opts(false, false, true), &pairs);
        assert_eq!(st, 1);
        assert!(log[0].contains("File exists"), "{log:?}");
    }

    #[test]
    fn l_links_the_target_p_links_the_link() {
        let t = Tmp::new();
        t.write("f", "F");
        symlink("f", t.path("s")).unwrap();
        let (st, _) = one(opts(false, false, true), t.p("s"), t.p("hl"));
        assert_eq!(st, 0);
        assert!(std::fs::symlink_metadata(t.path("hl")).unwrap().file_type().is_file());
        assert_eq!(t.nlink("f"), 2);
        let (st, _) = one(opts(false, false, false), t.p("s"), t.p("hp"));
        assert_eq!(st, 0);
        assert!(std::fs::symlink_metadata(t.path("hp")).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(t.path("hp")).unwrap().as_os_str().as_bytes(), b"f");
        assert_eq!(t.nlink("f"), 2);
    }

    #[test]
    fn a_hard_link_to_a_directory_is_refused() {
        let t = Tmp::new();
        std::fs::create_dir(t.path("d")).unwrap();
        let (st, log) = one(opts(false, false, true), t.p("d"), t.p("e"));
        assert_eq!(st, 1);
        assert_eq!(log.len(), 1);
        assert!(!t.path("e").exists());
        /* a symbolic link to one is fine */
        let (st, _) = one(opts(false, true, true), t.p("d"), t.p("e"));
        assert_eq!(st, 0);
        /* through a link to a directory the directory is still what is meant under -L */
        let (st, _) = one(opts(false, false, true), t.p("e"), t.p("e2"));
        assert_eq!(st, 1);
        assert!(!t.path("e2").exists());
    }

    #[test]
    fn dangling_destination_symlink_counts_as_existing() {
        let t = Tmp::new();
        t.write("a", "A");
        symlink("nowhere", t.path("b")).unwrap();
        let (st, _) = one(opts(false, false, true), t.p("a"), t.p("b"));
        assert_eq!(st, 1);
        let (st, _) = one(opts(true, false, true), t.p("a"), t.p("b"));
        assert_eq!(st, 0);
        assert_eq!(t.read("b"), "A");
    }

    #[test]
    fn missing_source_and_missing_destination_directory() {
        let t = Tmp::new();
        let (st, log) = one(opts(false, false, true), t.p("nosuch"), t.p("b"));
        assert_eq!(st, 1);
        assert!(log[0].contains("nosuch"));
        t.write("a", "A");
        let (st, log) = one(opts(false, false, true), t.p("a"), t.p("nodir/b"));
        assert_eq!(st, 1);
        assert!(log[0].contains("nodir/b"), "{log:?}");
    }

    #[test]
    fn symlink_into_a_directory_uses_the_last_component() {
        let t = Tmp::new();
        std::fs::create_dir(t.path("d")).unwrap();
        let pairs = plan(&[b"x/y/z".to_vec(), b"q".to_vec()], &t.p("d"), true).unwrap();
        let (st, _) = run(opts(false, true, true), &pairs);
        assert_eq!(st, 0);
        assert_eq!(std::fs::read_link(t.path("d/z")).unwrap().as_os_str().as_bytes(), b"x/y/z");
        assert_eq!(std::fs::read_link(t.path("d/q")).unwrap().as_os_str().as_bytes(), b"q");
    }
}
