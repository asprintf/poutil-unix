/* SPDX-License-Identifier: GPL-2.0-only */
/* chmod - change the file modes. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::walk::{walk, Entry, Links, Visit};

const SYNOPSIS: &str = "chmod [-R] mode file...";

struct Chmod {
    prog: Vec<u8>,
    mode: Vec<u8>,
    umask: u32,
    status: i32,
    /* the unit tests collect diagnostics here instead of writing them */
    log: Option<Vec<String>>,
}

impl Visit for Chmod {
    fn visit(&mut self, e: &Entry) -> std::io::Result<()> {
        /* a link is never changed itself, nor what it leads to unless named as an operand */
        if e.link {
            return Ok(());
        }
        let (mask, umask) = (&self.mode, self.umask);
        let mut mode_of = |st: &libc::stat| {
            let is_dir = st.st_mode & libc::S_IFMT == libc::S_IFDIR;
            posix::mode::parse(mask, st.st_mode as u32, is_dir, umask).map(|m| m as libc::mode_t)
        };
        if e.followed {
            /* an operand link is followed */
            let Some(m) = mode_of(&e.st) else {
                return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
            };
            if posix::sys::fchmodat(e.dirfd, e.name, m) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            return Ok(());
        }
        /* a link put there since is not followed */
        posix::sys::chmod_entry(e.dirfd, e.name, &e.st, &mut mode_of)
    }

    /* so that a mode taking away read or search does not stop the walk */
    fn after_contents(&self) -> bool {
        true
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

/* Is it a mode operand at all? */
fn valid(mode: &[u8]) -> bool {
    posix::mode::parse(mode, 0, false, 0).is_some()
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut recursive = false;
    let mut g = Getopt::new(&args, b"R");
    for o in g.by_ref() {
        match o {
            Opt::Flag(b'R') => recursive = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let operands = g.operands();
    if operands.len() < 2 {
        posix::usage(SYNOPSIS);
    }
    let mode = &operands[0];
    if !valid(mode) {
        posix::warn_path(&prog, mode, "invalid mode");
        std::process::exit(1);
    }
    let mut c = Chmod { prog, mode: mode.clone(), umask: posix::sys::umask(), status: 0, log: None };
    for f in &operands[1..] {
        walk(f, recursive, Links::Operands, &mut c);
    }
    std::process::exit(c.status);
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
            let p = std::env::temp_dir().join(format!("chmod-test-{}-{}", std::process::id(), n));
            std::fs::create_dir(&p).unwrap();
            Tmp(p)
        }
        fn p(&self, s: &str) -> Vec<u8> {
            self.0.join(s).as_os_str().as_bytes().to_vec()
        }
        fn path(&self, s: &str) -> PathBuf {
            self.0.join(s)
        }
        fn file(&self, s: &str, mode: u32) {
            std::fs::write(self.path(s), "").unwrap();
            self.set(s, mode);
        }
        fn dir(&self, s: &str, mode: u32) {
            std::fs::create_dir(self.path(s)).unwrap();
            self.set(s, mode);
        }
        fn set(&self, s: &str, mode: u32) {
            std::fs::set_permissions(self.path(s), std::fs::Permissions::from_mode(mode)).unwrap();
        }
        fn mode(&self, s: &str) -> u32 {
            std::fs::metadata(self.path(s)).unwrap().mode() & 0o7777
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

    fn run(mode: &str, recursive: bool, files: &[Vec<u8>], umask: u32) -> (i32, Vec<String>) {
        let mut c = Chmod { prog: b"chmod".to_vec(), mode: mode.as_bytes().to_vec(), umask, status: 0, log: Some(Vec::new()) };
        for f in files {
            walk(f, recursive, Links::Operands, &mut c);
        }
        (c.status, c.log.take().unwrap())
    }

    #[test]
    fn a_link_put_in_place_of_a_file_is_not_followed() {
        let t = Tmp::new();
        t.file("outside", 0o644);
        for start in [0o644, 0o000] {
            t.file("a", start);
            let seen = std::fs::symlink_metadata(t.path("a")).unwrap();
            std::fs::remove_file(t.path("a")).unwrap();
            symlink("outside", t.path("a")).unwrap();
            /* what the walk saw is a regular file; the name is a link by the time it is changed */
            let mut st: libc::stat = posix::sys::zeroed();
            st.st_mode = libc::S_IFREG | (seen.mode() as libc::mode_t & 0o7777);
            let name = posix::cstr(&t.p("a"));
            let r = posix::sys::chmod_entry(libc::AT_FDCWD, &name, &st, &mut |_| Some(0o600));
            assert!(r.is_err(), "start {start:o}");
            assert_eq!(t.mode("outside"), 0o644);
            std::fs::remove_file(t.path("a")).unwrap();
        }
    }

    #[test]
    fn a_file_with_no_permission_at_all_is_still_changed() {
        let t = Tmp::new();
        t.file("f", 0o000);
        let (st, log) = run("600", false, &[t.p("f")], 0o022);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
        assert_eq!(t.mode("f"), 0o600);
    }

    #[test]
    fn mode_operand_validity() {
        for m in ["0", "755", "7777", "u+x", "a=rwx,go-w", "+", "=", "u=g", "g+X", "+t", "u+s,o+t", "u+u+r", "-w", "a+r-x=u"] {
            assert!(valid(m.as_bytes()), "{m}");
        }
        for m in ["", "8", "10000", "u", "x+u", "+rwxz", "u+rx,", ",u+x", "ug", "u+ro", "u=ru", "u+rg", "u+ug", "o+su", "u+Xg"] {
            assert!(!valid(m.as_bytes()), "{m}");
        }
    }

    #[test]
    fn an_octal_mode_sets_every_bit() {
        let t = Tmp::new();
        t.file("f", 0o777);
        assert_eq!(run("640", false, &[t.p("f")], 0o022).0, 0);
        assert_eq!(t.mode("f"), 0o640);
        assert_eq!(run("0", false, &[t.p("f")], 0o022).0, 0);
        assert_eq!(t.mode("f"), 0);
        assert_eq!(run("4755", false, &[t.p("f")], 0o022).0, 0);
        assert_eq!(t.mode("f"), 0o4755);
        /* a mode without the set-id bits clears them */
        assert_eq!(run("644", false, &[t.p("f")], 0o022).0, 0);
        assert_eq!(t.mode("f"), 0o644);
    }

    #[test]
    fn symbolic_modes_are_relative_to_each_file() {
        let t = Tmp::new();
        t.file("a", 0o600);
        t.file("b", 0o644);
        assert_eq!(run("g+r,o-r", false, &[t.p("a"), t.p("b")], 0o022).0, 0);
        assert_eq!(t.mode("a"), 0o640);
        assert_eq!(t.mode("b"), 0o640);
        /* no who: the umask is left out */
        t.set("a", 0o000);
        assert_eq!(run("+rw", false, &[t.p("a")], 0o027).0, 0);
        assert_eq!(t.mode("a"), 0o640);
    }

    #[test]
    fn capital_x_depends_on_each_file() {
        let t = Tmp::new();
        t.file("noexec", 0o644);
        t.file("exec", 0o744);
        t.dir("d", 0o644);
        assert_eq!(run("a+X", false, &[t.p("noexec"), t.p("exec"), t.p("d")], 0o022).0, 0);
        assert_eq!(t.mode("noexec"), 0o644);
        assert_eq!(t.mode("exec"), 0o755);
        assert_eq!(t.mode("d"), 0o755);
    }

    #[test]
    fn recursive_changes_the_directory_and_everything_below_it() {
        let t = Tmp::new();
        t.dir("d", 0o755);
        t.dir("d/s", 0o755);
        t.file("d/a", 0o644);
        t.file("d/s/b", 0o644);
        assert_eq!(run("go-rwx", true, &[t.p("d")], 0o022), (0, vec![]));
        for f in ["d", "d/s", "d/a", "d/s/b"] {
            assert_eq!(t.mode(f) & 0o077, 0, "{f}");
        }
        /* without -R only the operand */
        assert_eq!(run("a+rx", false, &[t.p("d")], 0o022).0, 0);
        assert_eq!(t.mode("d"), 0o755);
        assert_eq!(t.mode("d/a"), 0o600);
    }

    #[test]
    fn a_mode_that_shuts_a_directory_does_not_keep_the_walk_out_of_it() {
        let t = Tmp::new();
        t.dir("d", 0o755);
        t.dir("d/s", 0o755);
        t.file("d/a", 0o644);
        t.file("d/s/b", 0o644);
        for m in ["a-x", "a=", "u-rwx", "000"] {
            let (st, log) = run(m, true, &[t.p("d")], 0o022);
            assert_eq!((st, log.len()), (0, 0), "{m}: {log:?}");
            t.set("d", 0o755);
            t.set("d/s", 0o755);
        }
        let (st, _) = run("a=", true, &[t.p("d")], 0o022);
        assert_eq!(st, 0);
        assert_eq!(t.mode("d"), 0);
        t.set("d", 0o755);
        assert_eq!(t.mode("d/a"), 0);
        assert_eq!(t.mode("d/s"), 0);
        t.set("d/s", 0o755);
        assert_eq!(t.mode("d/s/b"), 0);
    }

    #[test]
    fn a_directory_that_cannot_be_entered_is_made_enterable_before_it_is_read() {
        let t = Tmp::new();
        t.dir("d", 0o700);
        t.file("d/f", 0o600);
        t.set("d", 0o000);
        let (st, log) = run("u+rwx", true, &[t.p("d")], 0o022);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
        assert_eq!(t.mode("d"), 0o700);
        assert_eq!(t.mode("d/f"), 0o700);
    }

    #[test]
    fn symbolic_links_inside_a_hierarchy_are_left_alone_and_an_operand_link_is_followed() {
        let t = Tmp::new();
        t.dir("d", 0o755);
        t.file("outside", 0o644);
        symlink("../outside", t.path("d/l")).unwrap();
        symlink("nowhere", t.path("d/dang")).unwrap();
        let (st, log) = run("go=", true, &[t.p("d")], 0o022);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
        assert_eq!(t.mode("outside"), 0o644);
        assert_eq!(t.mode("d"), 0o700);
        /* and a mode that shuts the directory is applied last */
        let (st, log) = run("a=", true, &[t.p("d")], 0o022);
        assert_eq!((st, log.len()), (0, 0), "{log:?}");
        assert_eq!(t.mode("d"), 0);
        /* a link named as an operand: the file it leads to, and the directory it leads to is walked */
        t.set("d", 0o755);
        symlink("d", t.path("top")).unwrap();
        t.file("d/in", 0o644);
        let (st, _) = run("u=rwx,go=", true, &[t.p("top")], 0o022);
        assert_eq!(st, 0);
        assert_eq!(t.mode("d"), 0o700);
        assert_eq!(t.mode("d/in"), 0o700);
        assert_eq!(t.mode("outside"), 0o644);
        symlink("outside", t.path("fl")).unwrap();
        assert_eq!(run("600", false, &[t.p("fl")], 0o022).0, 0);
        assert_eq!(t.mode("outside"), 0o600);
        assert!(std::fs::symlink_metadata(t.path("fl")).unwrap().file_type().is_symlink());
    }

    #[test]
    fn a_missing_file_is_an_error_and_the_rest_are_done_in_order() {
        let t = Tmp::new();
        t.file("a", 0o666);
        t.file("c", 0o666);
        let (st, log) = run("600", false, &[t.p("a"), t.p("nosuch"), t.p("c")], 0o022);
        assert_eq!(st, 1);
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("nosuch"));
        assert_eq!(t.mode("a"), 0o600);
        assert_eq!(t.mode("c"), 0o600);
        /* a dangling link operand is an error as well */
        symlink("nowhere", t.path("dl")).unwrap();
        assert_eq!(run("600", false, &[t.p("dl")], 0o022).0, 1);
    }

    #[test]
    fn the_status_change_time_moves() {
        let t = Tmp::new();
        t.file("f", 0o644);
        let before = std::fs::metadata(t.path("f")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        /* the same mode again: the file is still marked for update */
        assert_eq!(run("644", false, &[t.p("f")], 0o022).0, 0);
        let after = std::fs::metadata(t.path("f")).unwrap();
        assert!((after.ctime(), after.ctime_nsec()) > (before.ctime(), before.ctime_nsec()));
    }

    #[test]
    fn spec_examples_of_the_grammar() {
        let t = Tmp::new();
        t.file("f", 0o000);
        /* u=rwx,g=u gives the group what the user has (permcopy) */
        assert_eq!(run("u=rwx,g=u,o=", false, &[t.p("f")], 0o022).0, 0);
        assert_eq!(t.mode("f"), 0o770);
        /* a clause with several actions */
        assert_eq!(run("a=r+w-x", false, &[t.p("f")], 0o022).0, 0);
        assert_eq!(t.mode("f"), 0o666);
        /* o with s changes nothing (and is not an error) */
        t.set("f", 0o755);
        assert_eq!(run("o+s", false, &[t.p("f")], 0o022).0, 0);
        assert_eq!(t.mode("f"), 0o755);
    }
}
