/* SPDX-License-Identifier: GPL-2.0-only */
/* chown - change the file ownership. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::walk::{walk, Entry, Links, Visit};

const SYNOPSIS: &str = "chown [-h] owner[:group] file...\n       chown -R [-H|-L|-P] owner[:group] file...";

/* (uid_t)-1: leave unchanged */
const KEEP: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hlp {
    Default,
    H,
    L,
    P,
}

/* A name of the database wins over digits; KEEP cannot be asked for. */
fn id_of(s: &[u8], by_name: &dyn Fn(&[u8]) -> Option<u32>) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    if let Some(id) = by_name(s) {
        return (id != KEEP).then_some(id);
    }
    let n = posix::parse_decimal(s)?;
    u32::try_from(n).ok().filter(|&id| id != KEEP)
}

/* (uid, gid) of owner[:group]; gid is KEEP without a group */
fn parse_owner(
    op: &[u8],
    user: &dyn Fn(&[u8]) -> Option<u32>,
    group: &dyn Fn(&[u8]) -> Option<u32>,
) -> Result<(u32, u32), String> {
    let (u, g) = match op.iter().position(|&c| c == b':') {
        Some(i) => (&op[..i], Some(&op[i + 1..])),
        None => (op, None),
    };
    let uid = id_of(u, user).ok_or_else(|| format!("invalid user: '{}'", String::from_utf8_lossy(u)))?;
    let gid = match g {
        Some(g) => id_of(g, group).ok_or_else(|| format!("invalid group: '{}'", String::from_utf8_lossy(g)))?,
        None => KEEP,
    };
    Ok((uid, gid))
}

struct Chown {
    prog: Vec<u8>,
    uid: u32,
    gid: u32,
    status: i32,
    /* the tests collect diagnostics here */
    log: Option<Vec<String>>,
}

impl Visit for Chown {
    fn visit(&mut self, e: &Entry) -> std::io::Result<()> {
        if posix::sys::fchownat(e.dirfd, e.name, self.uid, self.gid, e.at_flags()) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn report(&mut self, path: &[u8], e: &std::io::Error) {
        self.status = 1;
        let msg = posix::strerror(e);
        match self.log.as_mut() {
            Some(l) => l.push(format!("{}: {}", String::from_utf8_lossy(path), msg)),
            None => posix::warn_path(&self.prog, path, &msg),
        }
    }
}

/* Which links the walk follows. */
fn links_for(recursive: bool, no_follow: bool, hlp: Hlp) -> Links {
    if no_follow {
        return Links::Never;
    }
    if !recursive {
        return Links::Operands;
    }
    match hlp {
        Hlp::H => Links::Operands,
        Hlp::L => Links::All,
        Hlp::P | Hlp::Default => Links::Never,
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let (mut recursive, mut no_follow, mut hlp) = (false, false, Hlp::Default);
    let mut g = Getopt::new(&args, b"hRHLP");
    for o in g.by_ref() {
        match o {
            Opt::Flag(b'h') => no_follow = true,
            Opt::Flag(b'R') => recursive = true,
            Opt::Flag(b'H') => hlp = Hlp::H,
            Opt::Flag(b'L') => hlp = Hlp::L,
            Opt::Flag(b'P') => hlp = Hlp::P,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let operands = g.operands();
    if operands.len() < 2 {
        posix::usage(SYNOPSIS);
    }
    let (uid, gid) = match parse_owner(
        &operands[0],
        &|n| posix::user::user_by_name(n).map(|u| u.uid),
        &posix::user::group_by_name,
    ) {
        Ok(v) => v,
        Err(msg) => posix::fail(&prog, &msg),
    };
    let links = links_for(recursive, no_follow, hlp);
    let mut c = Chown { prog, uid, gid, status: 0, log: None };
    for f in &operands[1..] {
        walk(f, recursive, links, &mut c);
    }
    std::process::exit(c.status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{symlink, MetadataExt};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("chown-test-{}-{}", std::process::id(), n));
            std::fs::create_dir(&p).unwrap();
            Tmp(p)
        }
        fn p(&self, s: &str) -> Vec<u8> {
            self.0.join(s).as_os_str().as_bytes().to_vec()
        }
        fn path(&self, s: &str) -> PathBuf {
            self.0.join(s)
        }
        fn file(&self, s: &str) {
            std::fs::write(self.path(s), "").unwrap();
        }
        fn gid(&self, s: &str) -> u32 {
            std::fs::symlink_metadata(self.path(s)).unwrap().gid()
        }
        fn uid(&self, s: &str) -> u32 {
            std::fs::symlink_metadata(self.path(s)).unwrap().uid()
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(uid: u32, gid: u32, recursive: bool, links: Links, files: &[Vec<u8>]) -> (i32, Vec<String>) {
        let mut c = Chown { prog: b"chown".to_vec(), uid, gid, status: 0, log: Some(Vec::new()) };
        for f in files {
            walk(f, recursive, links, &mut c);
        }
        (c.status, c.log.take().unwrap())
    }

    /* A group of this process other than the one that new files get, if there is one. */
    fn other_group(t: &Tmp) -> Option<u32> {
        t.file("probe");
        let own = t.gid("probe");
        let mut all = posix::user::process_groups();
        all.push(posix::user::egid());
        all.into_iter().find(|&g| g != own)
    }

    fn by(table: &'static [(&'static str, u32)]) -> impl Fn(&[u8]) -> Option<u32> {
        move |n: &[u8]| table.iter().find(|(k, _)| k.as_bytes() == n).map(|(_, v)| *v)
    }

    #[test]
    fn owner_operand_forms() {
        let users = by(&[("root", 0), ("mario", 501), ("7", 9), ("max", u32::MAX)]);
        let groups = by(&[("wheel", 0), ("staff", 20)]);
        let p = |s: &str| parse_owner(s.as_bytes(), &users, &groups);
        assert_eq!(p("root"), Ok((0, KEEP)));
        assert_eq!(p("mario:staff"), Ok((501, 20)));
        assert_eq!(p("501"), Ok((501, KEEP)));
        assert_eq!(p("root:20"), Ok((0, 20)));
        assert_eq!(p("12:34"), Ok((12, 34)));
        /* a number that is also a name is the name */
        assert_eq!(p("7"), Ok((9, KEEP)));
        assert_eq!(p("0:0"), Ok((0, 0)));
        assert_eq!(p("4294967294"), Ok((u32::MAX - 1, KEEP)));
        /* nothing can ask for "unchanged", by number or by name */
        assert!(p("4294967295").is_err());
        assert!(p("max").is_err());
        assert!(p("root:4294967295").is_err());
        assert!(p("18446744073709551616").is_err());
        assert!(p("99999999999999999999999").is_err());
        for bad in ["", ":", ":staff", "root:", "nobody", "root:nogroup", "-1", "+1", "1.5", "root:staff:x", "0x10", " 1", "1 "] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
        assert_eq!(p("nobody").unwrap_err(), "invalid user: 'nobody'");
        assert_eq!(p("root:nogroup").unwrap_err(), "invalid group: 'nogroup'");
    }

    #[test]
    fn links_for_the_options() {
        assert_eq!(links_for(false, false, Hlp::Default), Links::Operands);
        assert_eq!(links_for(false, true, Hlp::Default), Links::Never);
        assert_eq!(links_for(true, false, Hlp::Default), Links::Never);
        assert_eq!(links_for(true, false, Hlp::P), Links::Never);
        assert_eq!(links_for(true, false, Hlp::H), Links::Operands);
        assert_eq!(links_for(true, false, Hlp::L), Links::All);
        /* -h with -R: no link is followed, whatever -H or -L say */
        assert_eq!(links_for(true, true, Hlp::L), Links::Never);
    }

    #[test]
    fn changing_to_the_owner_it_already_has_works_and_the_group_follows_the_operand() {
        let t = Tmp::new();
        t.file("f");
        let me = posix::user::euid();
        let (st, log) = run(me, KEEP, false, Links::Operands, &[t.p("f")]);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
        assert_eq!(t.uid("f"), me);
        if let Some(g) = other_group(&t) {
            let before = t.uid("f");
            let (st, log) = run(KEEP, g, false, Links::Operands, &[t.p("f")]);
            assert_eq!((st, log.len()), (0, 0), "{log:?}");
            assert_eq!(t.gid("f"), g);
            assert_eq!(t.uid("f"), before);
            /* owner and group in one go */
            let (st, _) = run(me, t.gid("probe"), false, Links::Operands, &[t.p("f")]);
            assert_eq!(st, 0);
            assert_eq!(t.gid("f"), t.gid("probe"));
        }
    }

    #[test]
    fn a_change_that_is_not_permitted_is_reported_and_the_next_file_goes_on() {
        if posix::user::euid() == 0 {
            return;
        }
        let t = Tmp::new();
        t.file("a");
        t.file("b");
        /* root is another user: not allowed to a plain user, on any system */
        let (st, log) = run(0, KEEP, false, Links::Operands, &[t.p("a"), t.p("nosuch"), t.p("b")]);
        assert_eq!(st, 1);
        assert_eq!(log.len(), 3, "{log:?}");
        assert!(log[1].contains("nosuch"));
        assert_eq!(t.uid("a"), posix::user::euid());
    }

    #[test]
    fn recursive_physical_changes_links_themselves_and_does_not_enter_them() {
        let t = Tmp::new();
        let Some(g) = other_group(&t) else { return };
        std::fs::create_dir(t.path("d")).unwrap();
        std::fs::create_dir(t.path("d/s")).unwrap();
        t.file("d/a");
        t.file("d/s/b");
        std::fs::create_dir(t.path("out")).unwrap();
        t.file("out/o");
        symlink("../out", t.path("d/tolink")).unwrap();
        symlink("a", t.path("d/tofile")).unwrap();
        let own = t.gid("probe");
        let (st, log) = run(KEEP, g, true, Links::Never, &[t.p("d")]);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
        for f in ["d", "d/s", "d/a", "d/s/b", "d/tolink", "d/tofile"] {
            assert_eq!(t.gid(f), g, "{f}");
        }
        /* what the links lead to is untouched, though they name it */
        assert_eq!(t.gid("out"), own);
        assert_eq!(t.gid("out/o"), own);
    }

    #[test]
    fn recursive_with_l_follows_every_link_and_h_only_the_operand() {
        let t = Tmp::new();
        let Some(g) = other_group(&t) else { return };
        let own = t.gid("probe");
        std::fs::create_dir(t.path("d")).unwrap();
        t.file("d/a");
        std::fs::create_dir(t.path("out")).unwrap();
        t.file("out/o");
        symlink("../out", t.path("d/tolink")).unwrap();
        symlink("d", t.path("top")).unwrap();
        /* -H: the operand link is followed, the one inside is changed itself */
        let (st, log) = run(KEEP, g, true, Links::Operands, &[t.p("top")]);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
        assert_eq!(t.gid("d"), g);
        assert_eq!(t.gid("top"), own);
        assert_eq!(t.gid("d/a"), g);
        assert_eq!(t.gid("d/tolink"), g);
        assert_eq!(t.gid("out"), own);
        assert_eq!(t.gid("out/o"), own);
        /* -P on the operand link: only the link */
        let (st, _) = run(KEEP, own, true, Links::Never, &[t.p("d")]);
        assert_eq!(st, 0);
        let (st, _) = run(KEEP, g, true, Links::Never, &[t.p("top")]);
        assert_eq!(st, 0);
        assert_eq!(t.gid("top"), g);
        assert_eq!(t.gid("d"), own);
        /* -L: all of it, and what the inner link leads to */
        let (st, log) = run(KEEP, g, true, Links::All, &[t.p("d")]);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
        assert_eq!(t.gid("out"), g);
        assert_eq!(t.gid("out/o"), g);
    }

    #[test]
    fn h_option_changes_the_link_not_the_file() {
        let t = Tmp::new();
        let Some(g) = other_group(&t) else { return };
        let own = t.gid("probe");
        t.file("f");
        symlink("f", t.path("l")).unwrap();
        let (st, _) = run(KEEP, g, false, links_for(false, true, Hlp::Default), &[t.p("l")]);
        assert_eq!(st, 0);
        assert_eq!(t.gid("l"), g);
        assert_eq!(t.gid("f"), own);
        /* without -h the file */
        let (st, _) = run(KEEP, g, false, links_for(false, false, Hlp::Default), &[t.p("l")]);
        assert_eq!(st, 0);
        assert_eq!(t.gid("f"), g);
    }

    #[test]
    fn a_dangling_link_is_an_error_unless_the_link_itself_is_meant() {
        let t = Tmp::new();
        symlink("nowhere", t.path("dl")).unwrap();
        let me = posix::user::euid();
        let (st, _) = run(me, KEEP, false, Links::Operands, &[t.p("dl")]);
        assert_eq!(st, 1);
        let (st, log) = run(me, KEEP, false, Links::Never, &[t.p("dl")]);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
    }
}
