/* SPDX-License-Identifier: GPL-2.0-only */
/* du - estimate file space usage. */

use posix::getopt::{Getopt, Opt};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::Metadata;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

const SYNOPSIS: &str = "du [-a|-s] [-kx] [-H|-L] [file...]";
const BLOCK: usize = 65536;

/* Which symbolic links are followed. */
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Follow {
    /* none: a link is counted as the link */
    #[default]
    Never,
    /* -H: the links given as operands */
    Operands,
    /* -L: those too that are met in the hierarchy */
    All,
}

#[derive(Debug, Default)]
struct Opts {
    /* -a: every file, not only the directories */
    a: bool,
    /* -s: only the total of each operand */
    s: bool,
    /* -k: units of 1024 bytes instead of 512 */
    k: bool,
    /* -x: only files on the device of the operand */
    x: bool,
    follow: Follow,
}

/* The options and the operands; None is a usage error. */
fn parse(args: &[Vec<u8>]) -> Option<(Opts, Vec<Vec<u8>>)> {
    let mut o = Opts::default();
    let mut g = Getopt::new(args, b"aHkLsx");
    for opt in g.by_ref() {
        match opt {
            Opt::Flag(b'a') => o.a = true,
            Opt::Flag(b's') => o.s = true,
            Opt::Flag(b'k') => o.k = true,
            Opt::Flag(b'x') => o.x = true,
            /* "the last option specified shall determine the behavior" */
            Opt::Flag(b'H') => o.follow = Follow::Operands,
            Opt::Flag(b'L') => o.follow = Follow::All,
            _ => return None,
        }
    }
    /* the synopsis is [-a|-s] */
    if o.a && o.s {
        return None;
    }
    Some((o, g.operands().to_vec()))
}

fn as_path(p: &[u8]) -> &Path {
    Path::new(OsStr::from_bytes(p))
}

/* The pathname of a name in a directory: no second slash after one that ends the directory's. */
fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut p = dir.to_vec();
    if p.last() != Some(&b'/') {
        p.push(b'/');
    }
    p.extend_from_slice(name);
    p
}

/* what is done with a diagnostic about a pathname */
type Warn<'a> = Box<dyn FnMut(&[u8], &str) + 'a>;

struct Du<'a, W: Write> {
    o: &'a Opts,
    out: W,
    /* a diagnostic for a pathname */
    warn: Warn<'a>,
    status: i32,
    /* the (device, inode) of the files with more than one link met under the operand being done */
    seen: HashSet<(u64, u64)>,
    /* the directories being walked, to see a cycle that links make */
    walking: Vec<(u64, u64)>,
    /* the device of the operand, for -x */
    root_dev: u64,
}

impl<'a, W: Write> Du<'a, W> {
    fn new(o: &'a Opts, out: W, warn: Warn<'a>) -> Self {
        Du { o, out, warn, status: 0, seen: HashSet::new(), walking: Vec::new(), root_dev: 0 }
    }

    fn fail(&mut self, path: &[u8], e: &io::Error) {
        (self.warn)(path, &posix::strerror(e));
        self.status = 1;
    }

    /* One line: the size in 512- or 1024-byte units, rounded up, a blank, the pathname. */
    fn line(&mut self, blocks: u64, path: &[u8]) -> io::Result<()> {
        let size = if self.o.k { blocks.div_ceil(2) } else { blocks };
        let mut l = size.to_string().into_bytes();
        l.push(b' ');
        l.extend_from_slice(path);
        l.push(b'\n');
        self.out.write_all(&l)
    }

    /*
     * The space of one file in 512-byte blocks, and of everything below it
     * if it is a directory, written as the options ask. A file that is not
     * counted (another device with -x, a second name of a file counted
     * already, one that cannot be examined) is 0 and not written.
     */
    fn entry(&mut self, path: &[u8], top: bool) -> io::Result<u64> {
        let follow = match self.o.follow {
            Follow::Never => false,
            Follow::Operands => top,
            Follow::All => true,
        };
        let md: io::Result<Metadata> = if follow { std::fs::metadata(as_path(path)) } else { std::fs::symlink_metadata(as_path(path)) };
        let md = match md {
            Ok(m) => m,
            Err(e) => {
                self.fail(path, &e);
                return Ok(0);
            }
        };
        if top {
            self.root_dev = md.dev();
        } else if self.o.x && md.dev() != self.root_dev {
            return Ok(0);
        }
        let id = (md.dev(), md.ino());
        if md.is_dir() && self.walking.contains(&id) {
            (self.warn)(path, "directory causes a cycle");
            self.status = 1;
            return Ok(0);
        }
        /*
         * A file with more than one link is counted and written for one name
         * only. One with a single link can be met twice only through a
         * symbolic link that is followed, and the standard leaves it open
         * whether that is once or each time: once, so that the same space is
         * not added twice.
         */
        if (md.nlink() > 1 || self.o.follow != Follow::Never) && !self.seen.insert(id) {
            return Ok(0);
        }
        let mut total = md.blocks();
        if md.is_dir() {
            self.walking.push(id);
            let mut names: Vec<Vec<u8>> = Vec::new();
            match std::fs::read_dir(as_path(path)) {
                Ok(rd) => {
                    for e in rd {
                        match e {
                            Ok(e) => names.push(e.file_name().as_bytes().to_vec()),
                            Err(e) => self.fail(path, &e),
                        }
                    }
                }
                Err(e) => self.fail(path, &e),
            }
            names.sort();
            for n in names {
                total += self.entry(&join(path, &n), false)?;
            }
            self.walking.pop();
            if top || !self.o.s {
                self.line(total, path)?;
            }
        } else if top || self.o.a {
            /* the -a option does not decide whether a file given as an operand is written */
            self.line(total, path)?;
        }
        Ok(total)
    }

    fn operand(&mut self, path: &[u8]) -> io::Result<()> {
        self.seen.clear();
        self.walking.clear();
        self.entry(path, true).map(|_| ())
    }
}

/* Do the operands in the order given (none is the current directory); the exit status. */
fn du<W: Write>(o: &Opts, operands: &[Vec<u8>], out: W, warn: Warn<'_>) -> io::Result<i32> {
    let mut d = Du::new(o, out, warn);
    if operands.is_empty() {
        d.operand(b".")?;
    }
    for p in operands {
        d.operand(p)?;
    }
    d.out.flush()?;
    Ok(d.status)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let Some((o, files)) = parse(&args) else {
        posix::usage(SYNOPSIS);
    };
    let w = io::BufWriter::with_capacity(BLOCK, posix::Stdout::new());
    let p = prog.clone();
    let status = match du(&o, &files, w, Box::new(move |path, msg| posix::warn_path(&p, path, msg))) {
        Ok(s) => s,
        Err(e) => posix::fail(&prog, &format!("write error: {}", posix::strerror(&e))),
    };
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /* a directory of our own under the temporary one, removed at the end */
    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            static N: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!("du-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            std::fs::create_dir(&p).unwrap();
            Tmp(p)
        }

        fn p(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }

        fn s(&self, rel: &str) -> String {
            self.p(rel).to_str().unwrap().to_string()
        }

        fn mkdir(&self, rel: &str) {
            std::fs::create_dir_all(self.p(rel)).unwrap();
        }

        fn file(&self, rel: &str, len: usize) {
            std::fs::write(self.p(rel), vec![b'x'; len]).unwrap();
        }

        /* the 512-byte blocks the system says a file has, without following a link */
        fn blocks(&self, rel: &str) -> u64 {
            std::fs::symlink_metadata(self.p(rel)).unwrap().blocks()
        }
    }

    /* make every directory below p readable again, so that it can be removed */
    fn unlock(p: &Path) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    let _ = std::fs::set_permissions(e.path(), std::fs::Permissions::from_mode(0o755));
                    unlock(&e.path());
                }
            }
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            unlock(&self.0);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Out {
        text: String,
        warnings: Vec<String>,
        status: i32,
    }

    impl Out {
        /* the lines with the directory of the test replaced by "T" and the size left out */
        fn names(&self, t: &Tmp) -> Vec<String> {
            self.text.lines().map(|l| l.split_once(' ').unwrap().1.replace(&t.s(""), "T/").replace(&t.s(""), "T")).collect()
        }

        fn sizes(&self) -> Vec<u64> {
            self.text.lines().map(|l| l.split_once(' ').unwrap().0.parse().unwrap()).collect()
        }
    }

    fn run(flags: &str, ops: &[&str]) -> Out {
        let mut args = vec![b"du".to_vec()];
        if !flags.is_empty() {
            for f in flags.split(' ') {
                args.push(f.as_bytes().to_vec());
            }
        }
        args.extend(ops.iter().map(|s| s.as_bytes().to_vec()));
        let (o, files) = parse(&args).expect("usage error");
        let mut buf = Vec::new();
        let warnings = RefCell::new(Vec::new());
        let status = du(&o, &files, &mut buf, Box::new(|p, m| warnings.borrow_mut().push(format!("{}: {}", String::from_utf8_lossy(p), m)))).unwrap();
        Out { text: String::from_utf8(buf).unwrap(), warnings: warnings.into_inner(), status }
    }

    fn usage_error(flags: &str) -> bool {
        let mut args = vec![b"du".to_vec()];
        args.extend(flags.split(' ').map(|s| s.as_bytes().to_vec()));
        parse(&args).is_none()
    }

    #[test]
    fn a_file_operand_is_written_with_its_blocks_and_name() {
        let t = Tmp::new();
        t.file("f", 5000);
        let f = t.s("f");
        let r = run("", &[&f]);
        assert_eq!(r.text, format!("{} {}\n", t.blocks("f"), f));
        assert_eq!(r.status, 0);
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn the_format_is_a_size_one_blank_and_the_pathname() {
        let t = Tmp::new();
        t.file("a b", 10);
        let f = t.s("a b");
        let r = run("", &[&f]);
        assert_eq!(r.text, format!("{} {}\n", t.blocks("a b"), f));
        assert!(!r.text.contains('\t'));
    }

    #[test]
    fn a_directory_is_its_files_and_subdirectories_and_itself() {
        let t = Tmp::new();
        t.mkdir("d/sub");
        t.file("d/f1", 3000);
        t.file("d/sub/f2", 9000);
        let d = t.s("d");
        let r = run("", &[&d]);
        let sub = t.blocks("d/sub") + t.blocks("d/sub/f2");
        let top = t.blocks("d") + t.blocks("d/f1") + sub;
        assert_eq!(r.sizes(), [sub, top]);
        assert_eq!(r.names(&t), ["T/d/sub", "T/d"]);
    }

    #[test]
    fn the_subdirectory_is_written_before_the_directory_that_holds_it() {
        let t = Tmp::new();
        t.mkdir("d/a/b");
        t.mkdir("d/c");
        let d = t.s("d");
        let r = run("", &[&d]);
        assert_eq!(r.names(&t), ["T/d/a/b", "T/d/a", "T/d/c", "T/d"]);
    }

    #[test]
    fn a_empty_directory_is_its_own_space() {
        let t = Tmp::new();
        t.mkdir("e");
        let e = t.s("e");
        assert_eq!(run("", &[&e]).sizes(), [t.blocks("e")]);
    }

    #[test]
    fn a_adds_the_files_that_are_not_directories() {
        let t = Tmp::new();
        t.mkdir("d/sub");
        t.file("d/f1", 3000);
        t.file("d/sub/f2", 9000);
        let d = t.s("d");
        let r = run("-a", &[&d]);
        assert_eq!(r.names(&t), ["T/d/f1", "T/d/sub/f2", "T/d/sub", "T/d"]);
        assert_eq!(r.sizes()[0], t.blocks("d/f1"));
        assert_eq!(r.sizes()[1], t.blocks("d/sub/f2"));
    }

    #[test]
    fn a_does_not_decide_whether_a_file_operand_is_written() {
        let t = Tmp::new();
        t.file("f", 100);
        let f = t.s("f");
        assert_eq!(run("", &[&f]).text, run("-a", &[&f]).text);
        assert_eq!(run("-s", &[&f]).text, run("-a", &[&f]).text);
        assert_eq!(run("", &[&f]).text.lines().count(), 1);
    }

    #[test]
    fn s_writes_the_total_of_each_operand_only() {
        let t = Tmp::new();
        t.mkdir("d/sub");
        t.mkdir("e");
        t.file("d/sub/f", 9000);
        t.file("f", 100);
        let (d, e, f) = (t.s("d"), t.s("e"), t.s("f"));
        let r = run("-s", &[&d, &e, &f]);
        assert_eq!(r.names(&t), ["T/d", "T/e", "T/f"]);
        assert_eq!(r.sizes()[0], run("", &[&d]).sizes().last().copied().unwrap());
    }

    #[test]
    fn a_and_s_exclude_each_other() {
        assert!(usage_error("-a -s"));
        assert!(usage_error("-s -a"));
        assert!(usage_error("-as"));
        assert!(usage_error("-sa"));
        assert!(!usage_error("-s -s"));
        assert!(!usage_error("-a -a"));
    }

    #[test]
    fn k_is_1024_byte_units_rounded_up() {
        let t = Tmp::new();
        t.mkdir("d");
        t.file("d/f1", 3000);
        t.file("d/f2", 20000);
        let d = t.s("d");
        let k = run("-k", &[&d]).sizes()[0];
        let b = run("", &[&d]).sizes()[0];
        assert_eq!(k, b.div_ceil(2));
        /* the total is rounded, not each file: three blocks are 1.5 KB, written as 2 */
        assert_eq!(3u64.div_ceil(2), 2);
        let both = run("-a -k", &[&d]);
        assert_eq!(both.sizes()[0], t.blocks("d/f1").div_ceil(2));
    }

    #[test]
    fn a_symbolic_link_is_counted_as_the_link() {
        let t = Tmp::new();
        t.mkdir("d/sub");
        t.file("d/sub/f", 20000);
        symlink("sub", t.p("d/lnd")).unwrap();
        symlink("sub/f", t.p("d/lnf")).unwrap();
        let d = t.s("d");
        let r = run("-a", &[&d]);
        assert_eq!(r.names(&t), ["T/d/lnd", "T/d/lnf", "T/d/sub/f", "T/d/sub", "T/d"]);
        assert_eq!(r.sizes()[0], t.blocks("d/lnd"));
        assert_eq!(r.sizes()[1], t.blocks("d/lnf"));
    }

    #[test]
    fn a_link_as_an_operand_is_the_link_unless_h_or_l() {
        let t = Tmp::new();
        t.mkdir("d/sub");
        t.file("d/sub/f", 20000);
        symlink("d", t.p("lnd")).unwrap();
        let l = t.s("lnd");
        let r = run("", &[&l]);
        assert_eq!(r.names(&t), ["T/lnd"]);
        assert_eq!(r.sizes(), [t.blocks("lnd")]);
        /* -H and -L count what the link refers to, under the name given */
        for flags in ["-H", "-L"] {
            let r = run(flags, &[&l]);
            assert_eq!(r.names(&t), ["T/lnd/sub", "T/lnd"], "{flags}");
            assert_eq!(*r.sizes().last().unwrap(), run("", &[&t.s("d")]).sizes().last().copied().unwrap(), "{flags}");
        }
    }

    #[test]
    fn h_does_not_follow_a_link_met_in_the_hierarchy_but_l_does() {
        let t = Tmp::new();
        t.mkdir("d");
        t.mkdir("other/sub");
        t.file("other/sub/f", 20000);
        symlink("../other", t.p("d/lno")).unwrap();
        let d = t.s("d");
        let h = run("-H", &[&d]);
        assert_eq!(h.names(&t), ["T/d"]);
        assert_eq!(h.sizes(), [t.blocks("d") + t.blocks("d/lno")]);
        let l = run("-L", &[&d]);
        assert_eq!(l.names(&t), ["T/d/lno/sub", "T/d/lno", "T/d"]);
    }

    #[test]
    fn the_last_of_h_and_l_decides() {
        let t = Tmp::new();
        t.mkdir("d");
        t.file("d/f", 20000);
        symlink("f", t.p("d/lf")).unwrap();
        symlink("d", t.p("lnd")).unwrap();
        let l = t.s("lnd");
        /* -H follows the operand only: the link inside is the link; -L follows it too */
        let h = run("-H -a", &[&l]);
        let a = run("-L -a", &[&l]);
        assert_ne!(h.text, a.text);
        assert_eq!(run("-L -H -a", &[&l]).text, h.text);
        assert_eq!(run("-H -L -a", &[&l]).text, a.text);
        assert_eq!(run("-LH -a", &[&l]).text, h.text);
        assert_eq!(run("-HL -a", &[&l]).text, a.text);
        assert_eq!(run("-H -L -H -a", &[&l]).text, h.text);
        /* the link inside, as the link and as the file it refers to */
        assert_eq!(h.sizes()[1], t.blocks("d/lf"));
        assert_eq!(a.sizes()[1], t.blocks("d/f"));
    }

    #[test]
    fn a_file_with_two_links_is_counted_and_written_once() {
        let t = Tmp::new();
        t.mkdir("d");
        t.file("d/one", 20000);
        std::fs::hard_link(t.p("d/one"), t.p("d/two")).unwrap();
        let d = t.s("d");
        let r = run("-a", &[&d]);
        assert_eq!(r.text.lines().count(), 2);
        assert_eq!(r.sizes()[1], t.blocks("d") + t.blocks("d/one"));
        /* the entry that is written is the first one in the order the names are visited */
        assert_eq!(r.names(&t), ["T/d/one", "T/d"]);
    }

    #[test]
    fn hard_links_in_two_directories_are_counted_for_one() {
        let t = Tmp::new();
        t.mkdir("d/a");
        t.mkdir("d/b");
        t.file("d/a/f", 20000);
        std::fs::hard_link(t.p("d/a/f"), t.p("d/b/g")).unwrap();
        let d = t.s("d");
        let r = run("", &[&d]);
        let a = t.blocks("d/a") + t.blocks("d/a/f");
        let b = t.blocks("d/b");
        assert_eq!(r.sizes(), [a, b, t.blocks("d") + a + b]);
    }

    #[test]
    fn a_file_with_one_link_met_twice_through_links_is_counted_once() {
        /* implementation-defined for a file with one link; once, so the same space is not added twice */
        let t = Tmp::new();
        t.mkdir("d");
        t.file("target", 20000);
        symlink("../target", t.p("d/l1")).unwrap();
        symlink("../target", t.p("d/l2")).unwrap();
        let d = t.s("d");
        let r = run("-aL", &[&d]);
        assert_eq!(r.names(&t), ["T/d/l1", "T/d"]);
        assert_eq!(r.sizes(), [t.blocks("target"), t.blocks("d") + t.blocks("target")]);
        /* without a followed link each name is the link, and both are counted */
        let plain = run("-a", &[&d]);
        assert_eq!(plain.names(&t), ["T/d/l1", "T/d/l2", "T/d"]);
    }

    #[test]
    fn the_link_count_is_per_operand() {
        let t = Tmp::new();
        t.file("one", 20000);
        std::fs::hard_link(t.p("one"), t.p("two")).unwrap();
        let (one, two) = (t.s("one"), t.s("two"));
        /* every file operand is written */
        assert_eq!(run("", &[&one, &two]).text.lines().count(), 2);
        assert_eq!(run("", &[&one, &one]).text.lines().count(), 2);
    }

    #[test]
    fn a_cycle_made_by_links_is_an_error_and_not_endless() {
        let t = Tmp::new();
        t.mkdir("d/sub");
        symlink("..", t.p("d/sub/up")).unwrap();
        symlink("..", t.p("d/sub/up2")).unwrap();
        let d = t.s("d");
        let r = run("-L", &[&d]);
        assert_eq!(r.status, 1);
        assert!(r.warnings.iter().any(|w| w.contains("cycle")), "{:?}", r.warnings);
        assert!(r.text.lines().count() >= 2);
        assert_eq!(run("", &[&d]).status, 0);
    }

    #[test]
    fn a_directory_met_again_by_a_link_is_counted_once() {
        let t = Tmp::new();
        t.mkdir("d/a");
        t.mkdir("d/b");
        t.file("d/a/f", 20000);
        symlink("../a", t.p("d/b/to_a")).unwrap();
        if std::fs::metadata(t.p("d/a")).unwrap().nlink() <= 1 {
            eprintln!("skipped: this file system gives directories one link");
            return;
        }
        let d = t.s("d");
        let l = run("-L", &[&d]);
        /* a is counted under its own name, which is visited first; the link adds nothing */
        let a = t.blocks("d/a") + t.blocks("d/a/f");
        let b = t.blocks("d/b");
        assert_eq!(l.names(&t), ["T/d/a", "T/d/b", "T/d"]);
        assert_eq!(l.sizes(), [a, b, t.blocks("d") + a + b]);
        assert_eq!(l.status, 0);
    }

    #[test]
    fn an_operand_that_cannot_be_examined_is_an_error_and_the_others_go_on() {
        let t = Tmp::new();
        t.file("f", 100);
        let (f, nosuch) = (t.s("f"), t.s("nosuch"));
        let r = run("", &[&nosuch, &f, &nosuch]);
        assert_eq!(r.status, 1);
        assert_eq!(r.warnings.len(), 2);
        assert!(r.warnings[0].contains("nosuch") && r.warnings[0].contains("No such file"));
        assert_eq!(r.names(&t), ["T/f"]);
    }

    #[test]
    fn a_directory_that_cannot_be_read_is_an_error_and_is_still_written() {
        let t = Tmp::new();
        t.mkdir("d/locked");
        t.mkdir("d/ok");
        t.file("d/locked/f", 100);
        std::fs::set_permissions(t.p("d/locked"), std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(t.p("d/locked")).is_ok() {
            eprintln!("skipped: running as a user that reads everything");
            return;
        }
        let d = t.s("d");
        let r = run("", &[&d]);
        assert_eq!(r.status, 1);
        assert!(r.warnings.iter().any(|w| w.contains("locked") && w.contains("Permission denied")), "{:?}", r.warnings);
        assert_eq!(r.names(&t), ["T/d/locked", "T/d/ok", "T/d"]);
        assert_eq!(r.sizes()[0], t.blocks("d/locked"));
    }

    #[test]
    fn a_dangling_link_is_an_error_only_when_it_is_followed() {
        let t = Tmp::new();
        t.mkdir("d");
        symlink("nosuch", t.p("d/dang")).unwrap();
        let d = t.s("d");
        assert_eq!(run("", &[&d]).status, 0);
        assert_eq!(run("-H", &[&d]).status, 0);
        let r = run("-L", &[&d]);
        assert_eq!(r.status, 1);
        assert!(r.warnings[0].contains("dang"));
        assert_eq!(r.names(&t), ["T/d"]);
        let dang = t.s("d/dang");
        assert_eq!(run("", &[&dang]).status, 0);
        assert_eq!(run("-H", &[&dang]).status, 1);
    }

    #[test]
    fn a_loop_of_links_in_the_path_is_an_error() {
        let t = Tmp::new();
        symlink("b", t.p("a")).unwrap();
        symlink("a", t.p("b")).unwrap();
        let p = t.s("a/x");
        let r = run("", &[&p]);
        assert_eq!(r.status, 1);
        assert_eq!(r.text, "");
        let r = run("-H", &[&t.s("a")]);
        assert_eq!(r.status, 1);
    }

    #[test]
    fn no_operand_is_the_current_directory() {
        /* the operand "." is written as "." and its entries as "./name" */
        let mut buf = Vec::new();
        let (o, _) = parse(&[b"du".to_vec()]).unwrap();
        let t = Tmp::new();
        t.file("f", 10);
        let cwd = std::env::current_dir().unwrap();
        let st = {
            std::env::set_current_dir(&t.0).unwrap();
            let r = du(&o, &[], &mut buf, Box::new(|_, _| {}));
            std::env::set_current_dir(cwd).unwrap();
            r.unwrap()
        };
        assert_eq!(st, 0);
        assert_eq!(String::from_utf8(buf).unwrap(), format!("{} .\n", t.blocks("") + t.blocks("f")));
    }

    #[test]
    fn a_trailing_slash_does_not_make_two() {
        assert_eq!(join(b"/", b"usr"), b"/usr");
        assert_eq!(join(b"a/", b"b"), b"a/b");
        assert_eq!(join(b"a", b"b"), b"a/b");
        assert_eq!(join(b".", b"b"), b"./b");
        let t = Tmp::new();
        t.mkdir("d");
        t.file("d/f", 10);
        let d = format!("{}/", t.s("d"));
        let r = run("-a", &[&d]);
        assert_eq!(r.names(&t), ["T/d/f", "T/d/"]);
    }

    #[test]
    fn names_are_bytes_and_come_out_in_byte_order() {
        let t = Tmp::new();
        t.mkdir("d");
        /* b and B are two names only where the file system tells letters apart */
        t.file("probe", 1);
        if t.p("PROBE").exists() {
            eprintln!("skipped: the temporary directory ignores case (set TMPDIR to one that does not)");
            return;
        }
        for n in ["b", "B", "a", "_", "10", "9"] {
            t.file(&format!("d/{n}"), 1);
        }
        let d = t.s("d");
        let r = run("-a", &[&d]);
        assert_eq!(r.names(&t), ["T/d/10", "T/d/9", "T/d/B", "T/d/_", "T/d/a", "T/d/b", "T/d"]);
    }

    #[test]
    fn options_in_any_order_and_grouped() {
        let (o, _) = parse(&[b"du".to_vec(), b"-kx".to_vec(), b"-a".to_vec()]).unwrap();
        assert!(o.k && o.x && o.a && !o.s);
        let (o, _) = parse(&[b"du".to_vec(), b"-a".to_vec(), b"-x".to_vec(), b"-k".to_vec()]).unwrap();
        assert!(o.k && o.x && o.a);
        let (o, f) = parse(&[b"du".to_vec(), b"-s".to_vec(), b"--".to_vec(), b"-a".to_vec()]).unwrap();
        assert!(o.s && !o.a);
        assert_eq!(f, [b"-a".to_vec()]);
        for bad in ["-b", "-d", "-h", "-P", "-c", "-m", "-g", "-i", "-l", "-n"] {
            assert!(usage_error(bad), "{bad}");
        }
    }

    #[test]
    fn an_argument_that_starts_with_a_dash_after_an_operand_is_an_operand() {
        let (o, f) = parse(&[b"du".to_vec(), b"d".to_vec(), b"-s".to_vec()]).unwrap();
        assert!(!o.s);
        assert_eq!(f, [b"d".to_vec(), b"-s".to_vec()]);
    }

    #[test]
    fn x_with_the_device_of_the_operand_changes_nothing_inside_one_device() {
        let t = Tmp::new();
        t.mkdir("d/sub");
        t.file("d/sub/f", 1000);
        let d = t.s("d");
        assert_eq!(run("-x", &[&d]).text, run("", &[&d]).text);
        assert_eq!(run("-xa", &[&d]).text, run("-a", &[&d]).text);
    }

    #[test]
    fn a_write_error_is_reported() {
        struct Bad;
        impl Write for Bad {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from_raw_os_error(libc::ENOSPC))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let t = Tmp::new();
        t.file("f", 10);
        let (o, _) = parse(&[b"du".to_vec()]).unwrap();
        assert!(du(&o, &[t.s("f").into_bytes()], Bad, Box::new(|_, _| {})).is_err());
    }
}
