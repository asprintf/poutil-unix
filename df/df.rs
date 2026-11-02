/* SPDX-License-Identifier: GPL-2.0-only */
/* df - report free disk space. */

use posix::getopt::{Getopt, Opt};
use std::io::Write;

const SYNOPSIS: &str = "df [-k] [-P|-t] [file...]";

#[derive(Debug, Default)]
struct Opts {
    /* -k: 1024-byte units instead of 512 */
    k: bool,
    /* -P: the portable format */
    p: bool,
    /* -t: total allocated space too (the usual columns already have it) */
    t: bool,
}

/* What the system says about one file system, in its own units. */
#[derive(Clone, Debug)]
struct Fs {
    /* the file system name and the directory it is mounted on */
    name: Vec<u8>,
    root: Vec<u8>,
    /* the size of a block, and the counts of blocks: all, free, free for an unprivileged user */
    bsize: u64,
    blocks: u64,
    bfree: u64,
    bavail: u64,
    /*
     * the bytes the files of this file system take, when the system says: on
     * APFS the blocks and the free blocks are those of the whole container,
     * which holds other volumes too
     */
    spaceused: Option<u64>,
}

/* One line of output, with the space figures already in 512- or 1024-byte units. */
#[derive(Debug, PartialEq, Eq)]
struct Row {
    name: Vec<u8>,
    root: Vec<u8>,
    total: i128,
    used: i128,
    free: i128,
}

/* a / b rounded up, for b > 0. */
fn ceil_div(a: i128, b: i128) -> i128 {
    -((-a).div_euclid(b))
}

/* A count of file system blocks as units of 512 or 1024 bytes, rounded up to the next higher unit. */
fn in_units(count: i128, bsize: u64, unit: u64) -> i128 {
    ceil_div(count * i128::from(bsize), i128::from(unit))
}

/*
 * The percentage used: used / (used + free) as a percentage, a fraction
 * rounded up. It is more than 100 when free is below zero. A file system
 * with nothing to divide is as full as it can be.
 */
fn percent(used: i128, free: i128) -> i128 {
    let whole = used + free;
    if whole <= 0 {
        return 100;
    }
    ceil_div(100 * used.max(0), whole)
}

/*
 * <space free> may be less than zero: the kernel reports the blocks an
 * unprivileged user can have as an unsigned count that wraps then.
 */
fn signed(n: u64) -> i128 {
    i128::from(n as i64)
}

fn row(fs: &Fs, unit: u64) -> Row {
    /* <space used> is the space allocated to the files of this file system, not the container's */
    let used = match fs.spaceused {
        Some(bytes) => ceil_div(i128::from(bytes), i128::from(unit)),
        None => in_units(i128::from(fs.blocks.saturating_sub(fs.bfree)), fs.bsize, unit),
    };
    let free = in_units(signed(fs.bavail), fs.bsize, unit);
    /*
     * <total space> should include <space used> and <space free>. Each is
     * rounded up by itself, so when the block size is not a multiple of the
     * unit their sum can pass the rounded total by a unit or two: then the
     * total is that sum.
     */
    let total = in_units(i128::from(fs.blocks), fs.bsize, unit).max(used + free);
    Row {
        name: fs.name.clone(),
        root: fs.root.clone(),
        total,
        used,
        free,
    }
}

/*
 * The cells of each row on one line, one blank between them, exactly as the
 * format of the standard has it. The standard lets the spacing be adjusted
 * for orderly columns; it does not ask for that, and a plain blank is what
 * a script can split on.
 */
fn lines(cells: &[Vec<Vec<u8>>]) -> Vec<u8> {
    let mut out = Vec::new();
    for r in cells {
        for (c, s) in r.iter().enumerate() {
            if c > 0 {
                out.push(b' ');
            }
            out.extend_from_slice(s);
        }
        out.push(b'\n');
    }
    out
}

fn text(n: i128) -> Vec<u8> {
    n.to_string().into_bytes()
}

fn pct(n: i128) -> Vec<u8> {
    format!("{n}%").into_bytes()
}

/*
 * The output, for every combination of options: the header "Filesystem
 * 512-blocks Used Available Capacity Mounted on" (1024-blocks with -k) and
 * a line "%s %d %d %d %d%% %s\n" for each file system, as the standard
 * describes it for -P, one blank between the fields. The standard leaves
 * the output without -P open; this has the file system name, the available
 * space and, as -t asks, the total space. Nothing at all when there are no
 * rows.
 */
fn render(rows: &[Row], o: &Opts) -> Vec<u8> {
    if rows.is_empty() {
        return Vec::new();
    }
    let unit = if o.k { "1024-blocks" } else { "512-blocks" };
    let head = ["Filesystem", unit, "Used", "Available", "Capacity", "Mounted on"];
    let mut cells: Vec<Vec<Vec<u8>>> = vec![head.iter().map(|s| s.as_bytes().to_vec()).collect()];
    for r in rows {
        cells.push(vec![r.name.clone(), text(r.total), text(r.used), text(r.free), pct(percent(r.used, r.free)), r.root.clone()]);
    }
    lines(&cells)
}

/* The options and the operands; None is a usage error. */
fn parse(args: &[Vec<u8>]) -> Option<(Opts, Vec<Vec<u8>>)> {
    let mut o = Opts::default();
    let mut g = Getopt::new(args, b"kPt");
    for opt in g.by_ref() {
        match opt {
            Opt::Flag(b'k') => o.k = true,
            Opt::Flag(b'P') => o.p = true,
            Opt::Flag(b't') => o.t = true,
            _ => return None,
        }
    }
    /* the synopsis is [-k] [-P|-t] */
    if o.p && o.t {
        return None;
    }
    Some((o, g.operands().to_vec()))
}

/* The bytes of a NUL-terminated name in a fixed array. */
fn name_bytes(a: &[libc::c_char]) -> Vec<u8> {
    a.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect()
}

fn from_statfs(s: &libc::statfs) -> Fs {
    Fs {
        name: name_bytes(&s.f_mntfromname),
        root: name_bytes(&s.f_mntonname),
        bsize: u64::from(s.f_bsize),
        blocks: s.f_blocks,
        bfree: s.f_bfree,
        bavail: s.f_bavail,
        spaceused: None,
    }
}

/* What getattrlist gives for ATTR_VOL_SPACEUSED: a length, then the bytes. */
#[repr(C, packed(4))]
struct SpaceUsed {
    len: u32,
    bytes: i64,
}

/* The bytes used by the files of the file system mounted on root, if it can say. */
fn space_used(root: &[u8]) -> Option<u64> {
    let p = posix::cstr(root);
    let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
    al.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    al.volattr = libc::ATTR_VOL_INFO | libc::ATTR_VOL_SPACEUSED;
    let mut b = SpaceUsed { len: 0, bytes: -1 };
    let r = unsafe {
        libc::getattrlist(p.as_ptr(), &mut al as *mut libc::attrlist as *mut libc::c_void, &mut b as *mut SpaceUsed as *mut libc::c_void, std::mem::size_of::<SpaceUsed>(), 0)
    };
    let (len, bytes) = (b.len, b.bytes);
    if r != 0 || (len as usize) < std::mem::size_of::<SpaceUsed>() || bytes < 0 {
        return None;
    }
    Some(bytes as u64)
}

/* A file system from statfs: with the figure per volume where the system has one. */
fn with_used(s: &libc::statfs) -> Fs {
    let mut fs = from_statfs(s);
    fs.spaceused = space_used(&fs.root);
    fs
}

/* The file system a file is on. */
fn fs_of(path: &[u8]) -> std::io::Result<Fs> {
    let p = posix::cstr(path);
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(p.as_ptr(), &mut s) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(with_used(&s))
}

/*
 * The figures of a mount entry brought up to date: getfsstat(MNT_NOWAIT) may
 * hand out the numbers it cached. If statfs on the mount point fails, or
 * answers for another file system (one mounted over it), the cached ones stay.
 */
fn refreshed(cached: &libc::statfs) -> libc::statfs {
    let root = posix::cstr(&name_bytes(&cached.f_mntonname));
    let mut fresh: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(root.as_ptr(), &mut fresh) } == 0 && fresh.f_mntfromname == cached.f_mntfromname {
        fresh
    } else {
        *cached
    }
}

/* Every mounted file system (MNT_NOWAIT: the list itself is not waited for). */
fn mounts() -> std::io::Result<Vec<Fs>> {
    let size = std::mem::size_of::<libc::statfs>();
    let mut cap = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if cap < 0 {
        return Err(std::io::Error::last_os_error());
    }
    loop {
        cap += 16;
        let mut buf: Vec<libc::statfs> = vec![unsafe { std::mem::zeroed() }; cap as usize];
        let n = unsafe { libc::getfsstat(buf.as_mut_ptr(), (cap as usize * size) as libc::c_int, libc::MNT_NOWAIT) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n < cap {
            return Ok(buf[..n as usize].iter().map(|c| with_used(&refreshed(c))).collect());
        }
        cap *= 2;
    }
}

/* The mounted file system whose device is the special file at path (by name, or by device number), if any. */
fn mounted_device(path: &[u8], st: &libc::stat, all: &[Fs]) -> Option<usize> {
    let kind = st.st_mode & libc::S_IFMT;
    all.iter().position(|m| {
        if m.name == path {
            return true;
        }
        if m.name.first() != Some(&b'/') {
            return false;
        }
        let name = posix::cstr(&m.name);
        let mut ms: libc::stat = unsafe { std::mem::zeroed() };
        unsafe { libc::stat(name.as_ptr(), &mut ms) == 0 && ms.st_mode & libc::S_IFMT == kind && ms.st_rdev == st.st_rdev }
    })
}

/*
 * The file system for one operand: the one a FIFO, regular file or directory
 * is on, or the one a special file is the device of. A special file that is
 * not the device of a mounted file system is an error (the standard leaves
 * other files unspecified).
 */
fn for_operand(path: &[u8], all: &mut Option<Vec<Fs>>) -> Result<Fs, String> {
    let p = posix::cstr(path);
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::stat(p.as_ptr(), &mut st) } != 0 {
        return Err(posix::strerror(&std::io::Error::last_os_error()));
    }
    let kind = st.st_mode & libc::S_IFMT;
    if kind != libc::S_IFBLK && kind != libc::S_IFCHR {
        return fs_of(path).map_err(|e| posix::strerror(&e));
    }
    if all.is_none() {
        *all = Some(mounts().map_err(|e| posix::strerror(&e))?);
    }
    let list = all.as_ref().map_or(&[][..], |v| v.as_slice());
    match mounted_device(path, &st, list) {
        Some(i) => Ok(fs_of(&list[i].root).unwrap_or_else(|_| list[i].clone())),
        None => Err("not mounted".to_string()),
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let Some((o, files)) = parse(&args) else {
        posix::usage(SYNOPSIS);
    };
    let unit = if o.k { 1024 } else { 512 };
    let mut rows = Vec::new();
    let mut status = 0;
    if files.is_empty() {
        match mounts() {
            Ok(all) => rows.extend(all.iter().map(|fs| row(fs, unit))),
            Err(e) => posix::fail(&prog, &format!("cannot list the file systems: {}", posix::strerror(&e))),
        }
    } else {
        let mut all = None;
        for f in &files {
            match for_operand(f, &mut all) {
                Ok(fs) => rows.push(row(&fs, unit)),
                Err(m) => {
                    posix::warn_path(&prog, f, &m);
                    status = 1;
                }
            }
        }
    }
    let out = render(&rows, &o);
    let mut w = posix::Stdout::new();
    if let Err(e) = w.write_all(&out).and_then(|_| w.flush()) {
        posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(bsize: u64, blocks: u64, bfree: u64, bavail: u64) -> Fs {
        Fs { name: b"/dev/x".to_vec(), root: b"/mnt".to_vec(), bsize, blocks, bfree, bavail, spaceused: None }
    }

    fn args(v: &[&str]) -> Vec<Vec<u8>> {
        std::iter::once("df").chain(v.iter().copied()).map(|s| s.as_bytes().to_vec()).collect()
    }

    fn words(out: &[u8]) -> Vec<Vec<String>> {
        String::from_utf8_lossy(out).lines().map(|l| l.split_whitespace().map(String::from).collect()).collect()
    }

    #[test]
    fn rounding_goes_up_to_the_next_unit() {
        assert_eq!(in_units(1, 1000, 512), 2);
        assert_eq!(in_units(1, 1000, 1024), 1);
        assert_eq!(in_units(1, 513, 512), 2);
        assert_eq!(in_units(1, 512, 512), 1);
        assert_eq!(in_units(1, 1, 512), 1);
        assert_eq!(in_units(0, 4096, 512), 0);
        assert_eq!(in_units(3, 4096, 512), 24);
        assert_eq!(in_units(3, 4096, 1024), 12);
        assert_eq!(in_units(1, 1025, 1024), 2);
        /* "next higher" for a figure below zero is toward zero */
        assert_eq!(in_units(-1, 1000, 512), -1);
        assert_eq!(in_units(-3, 512, 1024), -1);
    }

    #[test]
    fn the_figures_of_a_file_system() {
        /* 1000 blocks of 4096 bytes, 400 free, 300 of them for an unprivileged user */
        let r = row(&fs(4096, 1000, 400, 300), 512);
        assert_eq!((r.total, r.used, r.free), (8000, 4800, 2400));
        let r = row(&fs(4096, 1000, 400, 300), 1024);
        assert_eq!((r.total, r.used, r.free), (4000, 2400, 1200));
    }

    #[test]
    fn used_is_the_space_of_the_files_of_this_file_system_when_known() {
        /* a container of 1000 blocks of 4096 with 400 free, a volume in it that takes 5,000,000 bytes */
        let mut f = fs(4096, 1000, 400, 400);
        f.spaceused = Some(5_000_000);
        let r = row(&f, 512);
        assert_eq!(r.used, 9766);
        assert_eq!(row(&f, 1024).used, 4883);
        assert_eq!(r.free, 3200);
        /* the capacity follows that figure, not the container's 600 blocks */
        assert_eq!(percent(r.used, r.free), 76);
        f.spaceused = Some(0);
        assert_eq!(row(&f, 512).used, 0);
        f.spaceused = Some(1);
        assert_eq!(row(&f, 512).used, 1);
        f.spaceused = Some(512);
        assert_eq!(row(&f, 512).used, 1);
        f.spaceused = Some(513);
        assert_eq!(row(&f, 512).used, 2);
    }

    #[test]
    fn total_holds_used_free_and_what_is_kept_back() {
        /* <total space> includes <space used>, <space free> and the reserved space */
        for (b, bs, tf, av) in [(1000, 4096, 400, 300), (999, 1000, 333, 111), (7, 513, 3, 3), (1, 1, 0, 0), (50, 777, 10, 10)] {
            for unit in [512, 1024] {
                let r = row(&fs(bs, b, tf, av), unit);
                assert!(r.used + r.free <= r.total, "{b} {bs} {unit}");
            }
        }
        /* 7 blocks of 513 bytes: 8 + 5 + 4 would not fit, so the total is 9 */
        let r = row(&fs(513, 7, 3, 3), 512);
        assert_eq!((r.total, r.used, r.free), (9, 5, 4));
    }

    #[test]
    fn percent_used_is_rounded_up() {
        assert_eq!(percent(1, 2), 34);
        assert_eq!(percent(1, 99), 1);
        assert_eq!(percent(1, 100), 1);
        assert_eq!(percent(2, 1), 67);
        assert_eq!(percent(0, 5), 0);
        assert_eq!(percent(5, 0), 100);
        assert_eq!(percent(1, 1000), 1);
        assert_eq!(percent(50, 50), 50);
        assert_eq!(percent(4800, 2400), 67);
        assert_eq!(percent(10, 990), 1);
    }

    #[test]
    fn percent_used_may_pass_100_when_free_is_below_zero() {
        assert_eq!(percent(150, -50), 150);
        assert_eq!(percent(100, -50), 200);
        assert_eq!(percent(101, -1), 101);
    }

    #[test]
    fn percent_of_nothing_is_full() {
        assert_eq!(percent(0, 0), 100);
        assert_eq!(percent(10, -10), 100);
        assert_eq!(percent(10, -20), 100);
    }

    #[test]
    fn a_wrapped_count_of_free_blocks_is_negative() {
        let r = row(&fs(512, 1000, 5, (-10i64) as u64), 512);
        assert_eq!(r.free, -10);
        assert!(percent(r.used, r.free) > 100);
    }

    #[test]
    fn more_free_than_there_are_blocks_is_not_a_negative_used() {
        let r = row(&fs(512, 10, 20, 20), 512);
        assert_eq!(r.used, 0);
    }

    #[test]
    fn the_portable_header_for_512_and_1024() {
        let rows = [row(&fs(4096, 1000, 400, 300), 512)];
        let out = words(&render(&rows, &Opts { p: true, ..Opts::default() }));
        assert_eq!(out[0].join(" "), "Filesystem 512-blocks Used Available Capacity Mounted on");
        let out = words(&render(&rows, &Opts { p: true, k: true, ..Opts::default() }));
        assert_eq!(out[0].join(" "), "Filesystem 1024-blocks Used Available Capacity Mounted on");
    }

    #[test]
    fn the_portable_line_is_name_total_used_free_percent_root() {
        let rows = [row(&fs(4096, 1000, 400, 300), 512)];
        let out = words(&render(&rows, &Opts { p: true, ..Opts::default() }));
        assert_eq!(out[1], ["/dev/x", "8000", "4800", "2400", "67%", "/mnt"]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn k_changes_the_unit_of_the_figures_and_of_the_header() {
        let rows = [row(&fs(4096, 1000, 400, 300), 1024)];
        let out = words(&render(&rows, &Opts { p: true, k: true, ..Opts::default() }));
        assert_eq!(out[1], ["/dev/x", "4000", "2400", "1200", "67%", "/mnt"]);
    }

    #[test]
    fn the_output_is_the_same_whatever_the_options() {
        /* only -k changes it, through the unit */
        for (k, unit) in [(false, 512), (true, 1024)] {
            let rows = [row(&fs(4096, 1000, 400, 300), unit)];
            let p = render(&rows, &Opts { p: true, k, ..Opts::default() });
            assert_eq!(render(&rows, &Opts { k, ..Opts::default() }), p);
            assert_eq!(render(&rows, &Opts { t: true, k, ..Opts::default() }), p);
        }
    }

    #[test]
    fn the_portable_output_is_exactly_the_format_of_the_standard() {
        /* "%s %d %d %d %d%% %s\n", one blank between the fields, no padding, the header once */
        let a = row(&fs(4096, 1000, 400, 300), 512);
        let mut b = row(&fs(512, 123_456_789, 100, 100), 512);
        b.name = b"/dev/disk1s1".to_vec();
        b.root = b"/System/Volumes/Data".to_vec();
        let out = render(&[a, b], &Opts { p: true, ..Opts::default() });
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Filesystem 512-blocks Used Available Capacity Mounted on\n\
             /dev/x 8000 4800 2400 67% /mnt\n\
             /dev/disk1s1 123456789 123456689 100 100% /System/Volumes/Data\n"
        );
        let a = row(&fs(4096, 1000, 400, 300), 1024);
        let out = render(&[a], &Opts { p: true, k: true, ..Opts::default() });
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/x 4000 2400 1200 67% /mnt\n"
        );
    }

    #[test]
    fn a_negative_space_free_is_written_with_its_sign() {
        let r = row(&fs(512, 1000, 5, (-10i64) as u64), 512);
        let out = String::from_utf8(render(&[r], &Opts { p: true, ..Opts::default() })).unwrap();
        assert_eq!(out.lines().nth(1).unwrap(), "/dev/x 1000 995 -10 102% /mnt");
    }

    #[test]
    fn no_rows_no_output() {
        assert!(render(&[], &Opts { p: true, ..Opts::default() }).is_empty());
        assert!(render(&[], &Opts::default()).is_empty());
    }

    #[test]
    fn a_name_with_a_blank_stays_in_one_piece_on_its_line() {
        let mut r = row(&fs(4096, 1000, 400, 300), 512);
        r.name = b"map auto_home".to_vec();
        let text = String::from_utf8(render(&[r], &Opts { p: true, ..Opts::default() })).unwrap();
        assert_eq!(text.lines().nth(1).unwrap(), "map auto_home 8000 4800 2400 67% /mnt");
    }

    #[test]
    fn the_same_file_system_gives_the_same_line_whatever_file_named_it() {
        /* EXAMPLES: df -P /usr and df -P /usr/src write the same output; the row depends on the file system only */
        let f = fs(4096, 1000, 400, 300);
        let o = Opts { p: true, ..Opts::default() };
        assert_eq!(render(&[row(&f, 512)], &o), render(&[row(&f.clone(), 512)], &o));
    }

    #[test]
    fn options_may_be_grouped_in_any_order() {
        let (o, f) = parse(&args(&["-kP", "/"])).unwrap();
        assert!(o.k && o.p && !o.t);
        assert_eq!(f, [b"/".to_vec()]);
        let (o, _) = parse(&args(&["-P", "-k"])).unwrap();
        assert!(o.k && o.p);
        let (o, _) = parse(&args(&["-k", "-P"])).unwrap();
        assert!(o.k && o.p);
        let (o, _) = parse(&args(&["-tk"])).unwrap();
        assert!(o.k && o.t && !o.p);
        let (o, f) = parse(&args(&[])).unwrap();
        assert!(!o.k && !o.p && !o.t && f.is_empty());
    }

    #[test]
    fn p_and_t_exclude_each_other() {
        assert!(parse(&args(&["-P", "-t"])).is_none());
        assert!(parse(&args(&["-tP"])).is_none());
        assert!(parse(&args(&["-kPt", "/"])).is_none());
        assert!(parse(&args(&["-t", "-t"])).is_some());
        assert!(parse(&args(&["-k", "-t"])).is_some());
    }

    #[test]
    fn unknown_options_are_usage_errors() {
        for bad in ["-x", "-h", "-kx", "-a", "-b", "-i"] {
            assert!(parse(&args(&[bad])).is_none(), "{bad}");
        }
    }

    #[test]
    fn dash_dash_and_operands_that_start_with_a_dash() {
        let (_, f) = parse(&args(&["--", "-k"])).unwrap();
        assert_eq!(f, [b"-k".to_vec()]);
        /* after an operand an argument that starts with - is an operand (PCTS df 836) */
        let (o, f) = parse(&args(&["/", "-k"])).unwrap();
        assert!(!o.k);
        assert_eq!(f, [b"/".to_vec(), b"-k".to_vec()]);
        let (o, f) = parse(&args(&["-k", "--", "-P", "/"])).unwrap();
        assert!(o.k && !o.p);
        assert_eq!(f, [b"-P".to_vec(), b"/".to_vec()]);
    }
}
