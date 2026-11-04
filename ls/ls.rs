/* SPDX-License-Identifier: GPL-2.0-only */
/* ls - list directory contents. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::mb;
use std::collections::HashMap;
use std::io::Write;

const SYNOPSIS: &str = "ls [-ikqrs] [-glno] [-A|-a] [-C|-m|-x|-1] [-F|-p] [-H|-L] [-R|-d] [-S|-f|-t] [-c|-u] [file...]";

/* A file modified more than this many seconds ago (half a year) is shown with its year. */
const SIX_MONTHS: i64 = 15_778_476;

/* The format of the entries when no long format is asked for. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fmt {
    /* a terminal gets columns, anything else one entry per line */
    Default,
    One,
    Down,
    Across,
    Stream,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sort {
    Name,
    Size,
    Time,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TimeKey {
    Mtime,
    Ctime,
    Atime,
}

#[derive(Clone, Copy, Debug)]
struct Opts {
    all: bool,
    almost: bool,
    fmt: Fmt,
    long: bool,
    numeric: bool,
    no_owner: bool,
    no_group: bool,
    classify: bool,
    slash: bool,
    follow_cmd: bool,
    follow_all: bool,
    recursive: bool,
    dir_only: bool,
    sort: Sort,
    reverse: bool,
    unsorted: bool,
    timekey: TimeKey,
    seen_t: bool,
    inode: bool,
    blocks: bool,
    kilo: bool,
    quote: bool,
}

impl Opts {
    fn new() -> Opts {
        Opts {
            all: false,
            almost: false,
            fmt: Fmt::Default,
            long: false,
            numeric: false,
            no_owner: false,
            no_group: false,
            classify: false,
            slash: false,
            follow_cmd: false,
            follow_all: false,
            recursive: false,
            dir_only: false,
            sort: Sort::Name,
            reverse: false,
            unsorted: false,
            timekey: TimeKey::Mtime,
            seen_t: false,
            inode: false,
            blocks: false,
            kilo: false,
            quote: false,
        }
    }

    /*
     * Apply one option letter. Of each set of exclusive options the last one
     * given decides, as the spec says; -f ignores -r, -S and -t.
     */
    fn apply(&mut self, c: u8) -> bool {
        match c {
            /* -f shall turn on -a and may ignore -A: it does, wherever -A stands */
            b'A' if self.unsorted => {}
            b'A' => {
                self.almost = true;
                self.all = false;
            }
            b'a' => {
                self.all = true;
                self.almost = false;
            }
            b'C' => {
                self.fmt = Fmt::Down;
                self.long = false;
            }
            b'x' => {
                self.fmt = Fmt::Across;
                self.long = false;
            }
            b'm' => {
                self.fmt = Fmt::Stream;
                self.long = false;
            }
            b'1' => self.fmt = Fmt::One,
            b'l' => self.long = true,
            b'n' => {
                self.long = true;
                self.numeric = true;
            }
            b'g' => {
                self.long = true;
                self.no_owner = true;
            }
            b'o' => {
                self.long = true;
                self.no_group = true;
            }
            b'F' => {
                self.classify = true;
                self.slash = false;
            }
            b'p' => {
                self.slash = true;
                self.classify = false;
            }
            b'H' => {
                self.follow_cmd = true;
                self.follow_all = false;
            }
            b'L' => {
                self.follow_all = true;
                self.follow_cmd = false;
            }
            /* -R with -d is unspecified; as in the other implementations -d wins, in either order */
            b'R' => self.recursive = true,
            b'd' => self.dir_only = true,
            b'S' => self.sort = Sort::Size,
            b't' => {
                self.sort = Sort::Time;
                self.seen_t = true;
            }
            b'f' => {
                self.unsorted = true;
                self.all = true;
                self.almost = false;
            }
            /* -c and -u pick the time; once -t was seen they also take the sort back from -S */
            b'c' => {
                self.timekey = TimeKey::Ctime;
                if self.seen_t {
                    self.sort = Sort::Time;
                }
            }
            b'u' => {
                self.timekey = TimeKey::Atime;
                if self.seen_t {
                    self.sort = Sort::Time;
                }
            }
            b'r' => self.reverse = true,
            b'i' => self.inode = true,
            b's' => self.blocks = true,
            b'k' => self.kilo = true,
            b'q' => self.quote = true,
            _ => return false,
        }
        true
    }

    /* Is a hierarchy listed? Not when -d says directories are listed as they are. */
    fn recurse(&self) -> bool {
        self.recursive && !self.dir_only
    }

    /* Is a symbolic link operand shown as itself, not followed? Only if -d, -F or -l is given and not -H or -L. */
    fn operand_link_is_itself(&self) -> bool {
        (self.dir_only || self.classify || self.long) && !self.follow_cmd && !self.follow_all
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

fn stat_path(path: &[u8], follow: bool) -> std::io::Result<libc::stat> {
    let c = posix::cstr(path);
    if follow {
        posix::sys::stat(&c)
    } else {
        posix::sys::lstat(&c)
    }
}

fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut v = dir.to_vec();
    if v.last() != Some(&b'/') {
        v.push(b'/');
    }
    v.extend_from_slice(name);
    v
}

/* The ten characters of the file mode of the long format (without the alternate access flag). */
fn mode_string(mode: libc::mode_t) -> [u8; 10] {
    let mut s = *b"----------";
    s[0] = match mode & libc::S_IFMT {
        libc::S_IFDIR => b'd',
        libc::S_IFBLK => b'b',
        libc::S_IFCHR => b'c',
        libc::S_IFLNK => b'l',
        libc::S_IFIFO => b'p',
        libc::S_IFSOCK => b's',
        _ => b'-',
    };
    let bits = [
        (libc::S_IRUSR, 1, b'r'),
        (libc::S_IWUSR, 2, b'w'),
        (libc::S_IRGRP, 4, b'r'),
        (libc::S_IWGRP, 5, b'w'),
        (libc::S_IROTH, 7, b'r'),
        (libc::S_IWOTH, 8, b'w'),
    ];
    for (bit, i, ch) in bits {
        if mode & bit != 0 {
            s[i] = ch;
        }
    }
    let x = |bit: libc::mode_t| mode & bit != 0;
    s[3] = match (x(libc::S_IXUSR), x(libc::S_ISUID)) {
        (true, true) => b's',
        (false, true) => b'S',
        (true, false) => b'x',
        _ => b'-',
    };
    s[6] = match (x(libc::S_IXGRP), x(libc::S_ISGID)) {
        (true, true) => b's',
        (false, true) => b'S',
        (true, false) => b'x',
        _ => b'-',
    };
    s[9] = match (x(libc::S_IXOTH), x(libc::S_ISVTX)) {
        (true, true) => b't',
        (false, true) => b'T',
        (true, false) => b'x',
        _ => b'-',
    };
    s
}

/*
 * The date and time field: "%b %e %H:%M" for a file modified in the last six
 * months, "%b %e  %Y" otherwise or when the time is in the future. local
 * says whether the time zone applies (false: UTC, for the unit tests).
 */
fn time_string(t: i64, now: i64, local: bool) -> Vec<u8> {
    let recent = t <= now && t > now - SIX_MONTHS;
    let fmt = if recent { c"%b %e %H:%M" } else { c"%b %e  %Y" };
    /* a time_t that cannot hold the value gets the plain number, like a time the library rejects */
    let Some(tt) = libc::time_t::try_from(t).ok() else {
        return t.to_string().into_bytes();
    };
    let broken = if local { posix::time::localtime(tt) } else { posix::time::gmtime(tt) };
    let Some(tm) = broken else {
        return t.to_string().into_bytes();
    };
    /* both formats always produce text, so a zero from strftime means the buffer was too small */
    let mut size = 128;
    loop {
        let mut buf = vec![0u8; size];
        let n = posix::time::strftime(&mut buf, fmt, &tm);
        if n > 0 || size >= 1 << 16 {
            buf.truncate(n);
            return buf;
        }
        size *= 4;
    }
}

/*
 * The optional alternate access method flag of the long format: '+' for a
 * file with an access control list (the character the standard recommends),
 * 0 for none. Only macOS is covered; other systems say none.
 */
#[cfg(target_os = "macos")]
fn access_flag(path: &[u8], st: &libc::stat) -> u8 {
    if posix::sys::has_acl(&posix::cstr(path), kind(st) == libc::S_IFLNK) {
        b'+'
    } else {
        0
    }
}

#[cfg(not(target_os = "macos"))]
fn access_flag(_path: &[u8], _st: &libc::stat) -> u8 {
    0
}

/* The name as it is written: with -q, a character that is not printable (a tab included) is a "?". */
fn quote_name(name: &[u8], q: bool) -> Vec<u8> {
    if !q {
        return name.to_vec();
    }
    let mut out = Vec::with_capacity(name.len());
    let mut i = 0;
    while i < name.len() {
        let (len, wc) = mb::char_at(&name[i..]);
        let len = len.max(1);
        match wc {
            Some(c) if mb::is_print(c) => out.extend_from_slice(&name[i..i + len]),
            _ => out.push(b'?'),
        }
        i += len;
    }
    out
}

/* How many columns of a terminal the text takes. */
fn text_width(s: &[u8]) -> usize {
    let mut w = 0;
    let mut i = 0;
    while i < s.len() {
        let (len, wc) = mb::char_at(&s[i..]);
        let len = len.max(1);
        w += match wc {
            Some(c) => {
                let x = mb::width(c);
                if x < 0 {
                    1
                } else {
                    x as usize
                }
            }
            None => 1,
        };
        i += len;
    }
    w
}

/*
 * Multi-column layout: the rows of item indexes. Every column is as wide as
 * the widest entry and two spaces; the number of columns is what fits in
 * width. down: entries run down the columns (-C), else across the rows (-x).
 */
fn layout(widths: &[usize], width: usize, down: bool) -> (usize, Vec<Vec<usize>>) {
    let n = widths.len();
    if n == 0 {
        return (0, Vec::new());
    }
    let colw = widths.iter().copied().max().unwrap_or(0) + 2;
    let cols = (width / colw).clamp(1, n);
    let rows = n.div_ceil(cols);
    let mut out = vec![Vec::new(); rows];
    if down {
        for i in 0..n {
            out[i % rows].push(i);
        }
    } else {
        for i in 0..n {
            out[i / cols].push(i);
        }
    }
    (colw, out)
}

/*
 * Stream layout (-m): names separated by a comma and a space, a new line
 * instead of the space when the next name would not fit in width.
 */
fn stream(names: &[Vec<u8>], width: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut col = 0usize;
    for (i, n) in names.iter().enumerate() {
        let w = text_width(n);
        let last = i + 1 == names.len();
        if i > 0 {
            /* the comma went with the previous name; a space and this name (and its comma) must fit */
            if col + 1 + w + if last { 0 } else { 1 } > width {
                out.push(b'\n');
                col = 0;
            } else {
                out.push(b' ');
                col += 1;
            }
        }
        out.extend_from_slice(n);
        col += w;
        if !last {
            out.push(b',');
            col += 1;
        }
    }
    out.push(b'\n');
    out
}

/* Everything known about one entry that is going to be written. */
struct Item {
    /* as written: the operand, or the name in its directory */
    name: Vec<u8>,
    /* for system calls */
    path: Vec<u8>,
    /* the file information to use: the link itself, or what it points to */
    st: libc::stat,
    /* is the entry itself a symbolic link (whatever st describes) */
    is_link: bool,
}

struct Out {
    w: std::io::BufWriter<posix::Stdout>,
    prog: Vec<u8>,
}

impl Out {
    fn put(&mut self, b: &[u8]) {
        if let Err(e) = self.w.write_all(b) {
            posix::fail(&self.prog, &format!("write error: {}", posix::strerror(&e)));
        }
    }
    fn flush(&mut self) {
        if let Err(e) = self.w.flush() {
            posix::fail(&self.prog, &format!("write error: {}", posix::strerror(&e)));
        }
    }
}

struct Ls {
    prog: Vec<u8>,
    o: Opts,
    out: Out,
    status: i32,
    width: usize,
    now: i64,
    c_collate: bool,
    sa: Vec<u8>,
    sb: Vec<u8>,
    users: HashMap<u32, Vec<u8>>,
    groups: HashMap<u32, Vec<u8>>,
    /* is the time zone applied to the times (false: UTC, for the unit tests) */
    local: bool,
    /* nothing has been written yet (the first directory header has no blank line before it) */
    first: bool,
    /* the unit tests collect diagnostics here */
    log: Option<Vec<String>>,
}

impl Ls {
    fn err(&mut self, path: &[u8], msg: &str) {
        self.status = 1;
        match self.log.as_mut() {
            Some(l) => l.push(format!("{}: {}", String::from_utf8_lossy(path), msg)),
            None => {
                self.out.flush();
                posix::warn_path(&self.prog, path, msg);
            }
        }
    }

    fn err_io(&mut self, path: &[u8], e: &std::io::Error) {
        self.err(path, &posix::strerror(e));
    }

    fn time_of(&self, st: &libc::stat) -> (i64, i64) {
        match self.o.timekey {
            TimeKey::Mtime => (st.st_mtime as i64, st.st_mtime_nsec as i64),
            TimeKey::Ctime => (st.st_ctime as i64, st.st_ctime_nsec as i64),
            TimeKey::Atime => (st.st_atime as i64, st.st_atime_nsec as i64),
        }
    }

    fn sort_items(&mut self, items: &mut [Item]) {
        if self.o.unsorted {
            return;
        }
        let o = self.o;
        /* sort_by needs a comparator that does not borrow self mutably twice: take the scratch buffers out */
        let mut sa = std::mem::take(&mut self.sa);
        let mut sb = std::mem::take(&mut self.sb);
        let c = self.c_collate;
        let tk = o.timekey;
        let tm = |st: &libc::stat| match tk {
            TimeKey::Mtime => (st.st_mtime as i64, st.st_mtime_nsec as i64),
            TimeKey::Ctime => (st.st_ctime as i64, st.st_ctime_nsec as i64),
            TimeKey::Atime => (st.st_atime as i64, st.st_atime_nsec as i64),
        };
        items.sort_by(|a, b| {
            let names = mb::collate(&a.name, &b.name, c, &mut sa, &mut sb).then_with(|| a.name.cmp(&b.name));
            let ord = match o.sort {
                Sort::Name => names,
                Sort::Size => (b.st.st_size as i64).cmp(&(a.st.st_size as i64)).then(names),
                Sort::Time => tm(&b.st).cmp(&tm(&a.st)).then(names),
            };
            if o.reverse {
                ord.reverse()
            } else {
                ord
            }
        });
        self.sa = sa;
        self.sb = sb;
    }

    fn user_name(&mut self, uid: u32) -> Vec<u8> {
        if self.o.numeric {
            return uid.to_string().into_bytes();
        }
        if let Some(n) = self.users.get(&uid) {
            return n.clone();
        }
        let name = posix::user::user_name(uid).unwrap_or_else(|| uid.to_string().into_bytes());
        self.users.insert(uid, name.clone());
        name
    }

    fn group_name(&mut self, gid: u32) -> Vec<u8> {
        if self.o.numeric {
            return gid.to_string().into_bytes();
        }
        if let Some(n) = self.groups.get(&gid) {
            return n.clone();
        }
        let name = posix::user::group_name(gid).unwrap_or_else(|| gid.to_string().into_bytes());
        self.groups.insert(gid, name.clone());
        name
    }

    /* The suffix of -F or -p. */
    fn suffix(&self, it: &Item) -> &'static [u8] {
        let st = &it.st;
        if self.o.slash {
            return if is_dir(st) { b"/" } else { b"" };
        }
        if !self.o.classify {
            return b"";
        }
        match kind(st) {
            libc::S_IFDIR => b"/",
            libc::S_IFLNK => b"@",
            libc::S_IFIFO => b"|",
            libc::S_IFSOCK => b"=",
            libc::S_IFREG if st.st_mode & (libc::S_IXUSR | libc::S_IXGRP | libc::S_IXOTH) != 0 => b"*",
            _ => b"",
        }
    }

    /* Blocks in the unit of -s and "total": 512 bytes, or 1024 with -k (rounded up). */
    fn blocks_of(&self, st: &libc::stat) -> u64 {
        let b = st.st_blocks as u64;
        if self.o.kilo {
            b.div_ceil(2)
        } else {
            b
        }
    }

    /* The name field of a line: quoted name, suffix, and for a link in the long format " -> target". */
    fn name_field(&self, it: &Item) -> Vec<u8> {
        let mut v = quote_name(&it.name, self.o.quote);
        v.extend_from_slice(self.suffix(it));
        if self.o.long && it.is_link && kind(&it.st) == libc::S_IFLNK {
            if let Some(t) = read_link(&it.path) {
                v.extend_from_slice(b" -> ");
                v.extend_from_slice(&quote_name(&t, self.o.quote));
            }
        }
        v
    }

    /* The lines of a listing of the given items, after the header and the total. */
    fn render(&mut self, items: &[Item]) -> Vec<u8> {
        let o = self.o;
        let mut out = Vec::new();
        if items.is_empty() {
            return out;
        }
        let inode_w = if o.inode { items.iter().map(|i| (i.st.st_ino as u64).to_string().len()).max().unwrap_or(0) } else { 0 };
        let blk_w = if o.blocks { items.iter().map(|i| self.blocks_of(&i.st).to_string().len()).max().unwrap_or(0) } else { 0 };
        let prefix = |ls: &Ls, it: &Item| -> Vec<u8> {
            let mut p = Vec::new();
            if o.inode {
                p.extend_from_slice(format!("{:>w$} ", it.st.st_ino as u64, w = inode_w).as_bytes());
            }
            if o.blocks {
                p.extend_from_slice(format!("{:>w$} ", ls.blocks_of(&it.st), w = blk_w).as_bytes());
            }
            p
        };
        if o.long {
            struct Row {
                prefix: Vec<u8>,
                mode: [u8; 10],
                flag: u8,
                links: String,
                owner: Vec<u8>,
                group: Vec<u8>,
                size: String,
                date: Vec<u8>,
                name: Vec<u8>,
            }
            let mut rows = Vec::new();
            for it in items {
                let st = &it.st;
                let size = match kind(st) {
                    libc::S_IFCHR | libc::S_IFBLK => {
                        format!("{}, {}", libc::major(st.st_rdev) as u64, libc::minor(st.st_rdev) as u64)
                    }
                    _ => (st.st_size as u64).to_string(),
                };
                let t = self.time_of(st).0;
                let owner = if o.no_owner { Vec::new() } else { self.user_name(st.st_uid) };
                let group = if o.no_group { Vec::new() } else { self.group_name(st.st_gid) };
                rows.push(Row {
                    prefix: prefix(self, it),
                    mode: mode_string(st.st_mode),
                    flag: access_flag(&it.path, st),
                    links: (st.st_nlink as u64).to_string(),
                    owner,
                    group,
                    size,
                    date: time_string(t, self.now, self.local),
                    name: self.name_field(it),
                });
            }
            let w_links = rows.iter().map(|r| r.links.len()).max().unwrap_or(0);
            let w_owner = rows.iter().map(|r| text_width(&r.owner)).max().unwrap_or(0);
            let w_group = rows.iter().map(|r| text_width(&r.group)).max().unwrap_or(0);
            let w_size = rows.iter().map(|r| r.size.len()).max().unwrap_or(0);
            /* the flag follows the ten characters; a listing that has one pads the rows that have none */
            let any_flag = rows.iter().any(|r| r.flag != 0);
            for r in &rows {
                out.extend_from_slice(&r.prefix);
                out.extend_from_slice(&r.mode);
                if r.flag != 0 {
                    out.push(r.flag);
                } else if any_flag {
                    out.push(b' ');
                }
                out.push(b' ');
                out.extend_from_slice(format!("{:>w$}", r.links, w = w_links).as_bytes());
                out.push(b' ');
                if !o.no_owner {
                    out.extend_from_slice(&r.owner);
                    out.extend(std::iter::repeat_n(b' ', w_owner - text_width(&r.owner)));
                    out.push(b' ');
                }
                if !o.no_group {
                    out.extend_from_slice(&r.group);
                    out.extend(std::iter::repeat_n(b' ', w_group - text_width(&r.group)));
                    out.push(b' ');
                }
                out.extend_from_slice(format!("{:>w$}", r.size, w = w_size).as_bytes());
                out.push(b' ');
                out.extend_from_slice(&r.date);
                out.push(b' ');
                out.extend_from_slice(&r.name);
                out.push(b'\n');
            }
            return out;
        }
        let cells: Vec<Vec<u8>> = items
            .iter()
            .map(|it| {
                let mut c = prefix(self, it);
                c.extend_from_slice(&self.name_field(it));
                c
            })
            .collect();
        let fmt = if o.fmt == Fmt::Default {
            if posix::sys::isatty(1) {
                Fmt::Down
            } else {
                Fmt::One
            }
        } else {
            o.fmt
        };
        match fmt {
            Fmt::Default | Fmt::One => {
                for c in &cells {
                    out.extend_from_slice(c);
                    out.push(b'\n');
                }
            }
            Fmt::Stream => out = stream(&cells, self.width),
            Fmt::Down | Fmt::Across => {
                let widths: Vec<usize> = cells.iter().map(|c| text_width(c)).collect();
                let (colw, rows) = layout(&widths, self.width, fmt == Fmt::Down);
                for row in rows {
                    for (k, &i) in row.iter().enumerate() {
                        out.extend_from_slice(&cells[i]);
                        if k + 1 < row.len() {
                            out.extend(std::iter::repeat_n(b' ', colw - widths[i]));
                        }
                    }
                    out.push(b'\n');
                }
            }
        }
        out
    }

    /* Do the items need file information beyond their names? */
    fn needs_stat(&self) -> bool {
        let o = &self.o;
        o.long || o.inode || o.blocks || o.classify || o.slash || o.sort != Sort::Name && !o.unsorted
    }

    /* The file information for one directory entry, following links as the options say. */
    fn info(&mut self, path: &[u8]) -> std::io::Result<(libc::stat, bool)> {
        let l = stat_path(path, false)?;
        let is_link = kind(&l) == libc::S_IFLNK;
        if is_link && self.o.follow_all {
            /* -L evaluates the file the link refers to; if there is none the entry cannot be described: an error, not the link (PCTS ls 69) */
            let t = stat_path(path, true)?;
            return Ok((t, true));
        }
        Ok((l, is_link))
    }

    fn list_files(&mut self, items: &mut [Item]) {
        if items.is_empty() {
            return;
        }
        self.sort_items(items);
        let text = self.render(items);
        self.out.put(&text);
        self.first = false;
    }

    /* One directory: the header, the total, its entries, and with -R what is below it. */
    fn list_dir(&mut self, display: &[u8], path: &[u8], ancestors: &mut Vec<Id>, header: bool) {
        let names = match read_names(path) {
            Ok(n) => n,
            Err(e) => {
                if header {
                    self.print_header(display);
                }
                self.err_io(display, &e);
                return;
            }
        };
        let all = self.o.all;
        let almost = self.o.almost;
        let mut items: Vec<Item> = Vec::new();
        let need = self.needs_stat();
        for n in names {
            let hidden = n.first() == Some(&b'.');
            let dots = n == b"." || n == b"..";
            if hidden && !all && !(almost && !dots) {
                continue;
            }
            let p = join(path, &n);
            let item = if need || self.o.recurse() {
                match self.info(&p) {
                    Ok((st, l)) => Item { name: n, path: p, st, is_link: l },
                    Err(e) => {
                        self.err_io(&join(display, &n), &e);
                        continue;
                    }
                }
            } else {
                Item { name: n, path: p, st: posix::sys::zeroed(), is_link: false }
            };
            items.push(item);
        }
        self.sort_items(&mut items);
        if header {
            self.print_header(display);
        }
        if self.o.long || self.o.blocks {
            let total: u64 = items.iter().map(|i| self.blocks_of(&i.st)).sum();
            let line = format!("total {total}\n");
            self.out.put(line.as_bytes());
            self.first = false;
        }
        let text = self.render(&items);
        self.out.put(&text);
        if !items.is_empty() {
            self.first = false;
        }
        if !self.o.recurse() {
            return;
        }
        for it in &items {
            if it.name == b"." || it.name == b".." || !is_dir(&it.st) {
                continue;
            }
            let sub = join(display, &it.name);
            let id = id_of(&it.st);
            if ancestors.contains(&id) {
                self.err(&sub, "directory causes a cycle");
                continue;
            }
            ancestors.push(id);
            self.list_dir(&sub, &it.path, ancestors, true);
            ancestors.pop();
        }
    }

    fn print_header(&mut self, display: &[u8]) {
        let mut h = Vec::new();
        if !self.first {
            h.push(b'\n');
        }
        h.extend_from_slice(&quote_name(display, self.o.quote));
        h.extend_from_slice(b":\n");
        self.out.put(&h);
        self.first = false;
    }

    /* The operands: files first, then directories, each group sorted. */
    fn run(&mut self, operands: &[Vec<u8>]) {
        let o = self.o;
        let mut files: Vec<Item> = Vec::new();
        let mut dirs: Vec<Item> = Vec::new();
        for op in operands {
            let l = match stat_path(op, false) {
                Ok(s) => s,
                Err(e) => {
                    self.err_io(op, &e);
                    continue;
                }
            };
            let is_link = kind(&l) == libc::S_IFLNK;
            let mut st = l;
            let mut listed_dir = is_dir(&l);
            if is_link {
                /* -H and -L: the information is the target's; otherwise a link to a directory is listed unless -d, -F or -l */
                let explicit = o.follow_cmd || o.follow_all;
                if explicit || !o.operand_link_is_itself() {
                    match stat_path(op, true) {
                        Ok(t) => {
                            if explicit {
                                st = t;
                            }
                            listed_dir = is_dir(&t);
                        }
                        Err(e) => {
                            /* a link that leads nowhere is an error when it has to be followed; otherwise it is the link */
                            if explicit {
                                self.err_io(op, &e);
                                continue;
                            }
                        }
                    }
                }
            }
            if o.dir_only {
                listed_dir = false;
            }
            let it = Item { name: op.clone(), path: op.clone(), st, is_link };
            if listed_dir {
                dirs.push(it);
            } else {
                files.push(it);
            }
        }
        let headers = operands.len() > 1 || o.recurse();
        self.list_files(&mut files);
        self.sort_items(&mut dirs);
        for d in dirs {
            let mut anc = vec![id_of(&d.st)];
            /* the directory itself, not the link to it, identifies the ancestor for loop detection */
            if let Ok(t) = stat_path(&d.path, true) {
                anc = vec![id_of(&t)];
            }
            self.list_dir(&d.name, &d.path, &mut anc, headers);
        }
        self.out.flush();
    }
}

fn read_link(path: &[u8]) -> Option<Vec<u8>> {
    let c = posix::cstr(path);
    let mut size = 256usize;
    loop {
        let mut buf = vec![0u8; size];
        let n = posix::sys::readlink(&c, &mut buf);
        if n < 0 {
            return None;
        }
        if (n as usize) < size {
            buf.truncate(n as usize);
            return Some(buf);
        }
        size *= 4;
    }
}

/* Every name in a directory, "." and ".." included. */
fn read_names(path: &[u8]) -> std::io::Result<Vec<Vec<u8>>> {
    let c = posix::cstr(path);
    posix::sys::read_dir(&c)
}

/* The line width for -C, -x and -m: COLUMNS, else the terminal's, else 80. */
fn line_width(columns: Option<&[u8]>) -> usize {
    if let Some(c) = columns {
        if let Some(n) = posix::parse_decimal(c) {
            if n > 0 {
                return n as usize;
            }
        }
    }
    posix::sys::terminal_columns(1).unwrap_or(80)
}

fn main() {
    posix::init();
    posix::cal::null_tz_is_unset();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut o = Opts::new();
    let mut g = Getopt::new(&args, b"ACFHLRSacdfgiklmnopqrstux1");
    for opt in g.by_ref() {
        match opt {
            Opt::Flag(c) => {
                if !o.apply(c) {
                    posix::usage(SYNOPSIS);
                }
            }
            _ => posix::usage(SYNOPSIS),
        }
    }
    /* -f turns off the sorting options that came before or after it */
    if o.unsorted {
        o.reverse = false;
        o.sort = Sort::Name;
    }
    /* on a terminal, a name with unprintable characters is written with question marks anyway */
    if posix::sys::isatty(1) {
        o.quote = true;
    }
    let mut operands: Vec<Vec<u8>> = g.operands().to_vec();
    if operands.is_empty() {
        operands.push(b".".to_vec());
    }
    let columns = std::env::var_os("COLUMNS").map(std::os::unix::ffi::OsStringExt::into_vec);
    let width = line_width(columns.as_deref());
    let now = posix::time::now();
    let mut ls = Ls {
        prog: prog.clone(),
        o,
        out: Out { w: std::io::BufWriter::with_capacity(64 * 1024, posix::Stdout::new()), prog },
        status: 0,
        width,
        now,
        c_collate: mb::collation_is_c(),
        sa: Vec::new(),
        sb: Vec::new(),
        users: HashMap::new(),
        groups: HashMap::new(),
        local: true,
        first: true,
        log: None,
    };
    ls.run(&operands);
    std::process::exit(ls.status);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(s: &str) -> Opts {
        let mut o = Opts::new();
        for c in s.bytes() {
            assert!(o.apply(c), "{}", c as char);
        }
        if o.unsorted {
            o.reverse = false;
            o.sort = Sort::Name;
        }
        o
    }

    fn st_with(mode: libc::mode_t, nlink: u64, uid: u32, gid: u32, size: i64, mtime: i64) -> libc::stat {
        let mut st: libc::stat = posix::sys::zeroed();
        st.st_mode = mode;
        st.st_nlink = nlink as _;
        st.st_uid = uid;
        st.st_gid = gid;
        st.st_size = size;
        st.st_mtime = mtime as _;
        st
    }

    #[test]
    fn mode_strings() {
        let m = |x: libc::mode_t| String::from_utf8(mode_string(x).to_vec()).unwrap();
        assert_eq!(m(libc::S_IFREG | 0o644), "-rw-r--r--");
        assert_eq!(m(libc::S_IFDIR | 0o755), "drwxr-xr-x");
        assert_eq!(m(libc::S_IFREG), "----------");
        assert_eq!(m(libc::S_IFREG | 0o777), "-rwxrwxrwx");
        assert_eq!(m(libc::S_IFLNK | 0o777), "lrwxrwxrwx");
        assert_eq!(m(libc::S_IFIFO | 0o600), "prw-------");
        assert_eq!(m(libc::S_IFCHR | 0o620), "crw--w----");
        assert_eq!(m(libc::S_IFBLK | 0o640), "brw-r-----");
        assert_eq!(m(libc::S_IFSOCK | 0o755), "srwxr-xr-x");
        /* set-user-ID and set-group-ID: s with execute, S without */
        assert_eq!(m(libc::S_IFREG | 0o4755), "-rwsr-xr-x");
        assert_eq!(m(libc::S_IFREG | 0o4644), "-rwSr--r--");
        assert_eq!(m(libc::S_IFREG | 0o2755), "-rwxr-sr-x");
        assert_eq!(m(libc::S_IFREG | 0o2745), "-rwxr-Sr-x");
        assert_eq!(m(libc::S_IFREG | 0o6755), "-rwsr-sr-x");
        /* the restricted deletion flag: t with search permission for others, T without */
        assert_eq!(m(libc::S_IFDIR | 0o1777), "drwxrwxrwt");
        assert_eq!(m(libc::S_IFDIR | 0o1770), "drwxrwx--T");
        assert_eq!(m(libc::S_IFDIR | 0o1771), "drwxrwx--t");
    }

    #[test]
    fn time_in_the_posix_locale() {
        /* Nov 12 2007 10:15:30 UTC; "now" a month later: recent, with the time of day */
        let t = 1194862530;
        let s = |t, now| String::from_utf8(time_string(t, now, false)).unwrap();
        assert_eq!(s(t, t + 30 * 86400), "Nov 12 10:15");
        /* a single digit day has a leading space (%e) */
        assert_eq!(s(1183548000, 1183548000 + 86400), "Jul  4 11:20");
        /* more than six months old: the year, two spaces after the day */
        assert_eq!(s(t, t + 200 * 86400), "Nov 12  2007");
        assert_eq!(s(t, t + 182 * 86400), "Nov 12 10:15");
        /* in the future: the year */
        assert_eq!(s(t + 10, t), "Nov 12  2007");
        assert_eq!(s(t, t), "Nov 12 10:15");
        assert_eq!(s(0, 1_000_000_000), "Jan  1  1970");
    }

    #[test]
    fn quoting_of_names() {
        assert_eq!(quote_name(b"plain", true), b"plain");
        assert_eq!(quote_name(b"a\tb", true), b"a?b");
        assert_eq!(quote_name(b"a\nb", true), b"a?b");
        assert_eq!(quote_name(b"a\x01\x7fb", true), b"a??b");
        assert_eq!(quote_name(b"a\tb", false), b"a\tb");
        assert_eq!(quote_name(b"a b", true), b"a b");
        assert_eq!(quote_name(b"", true), b"");
    }

    #[test]
    fn column_layout_down_and_across() {
        let w = [3usize; 7];
        /* width 20, column 5 wide: 4 columns, 2 rows */
        let (cw, rows) = layout(&w, 20, true);
        assert_eq!(cw, 5);
        assert_eq!(rows, vec![vec![0, 2, 4, 6], vec![1, 3, 5]]);
        let (_, rows) = layout(&w, 20, false);
        assert_eq!(rows, vec![vec![0, 1, 2, 3], vec![4, 5, 6]]);
        /* too narrow for two columns: one per line */
        let (_, rows) = layout(&w, 8, true);
        assert_eq!(rows.len(), 7);
        /* everything fits on one row */
        let (_, rows) = layout(&w, 80, true);
        assert_eq!(rows, vec![vec![0, 1, 2, 3, 4, 5, 6]]);
        /* the widest name sets the column width for all */
        let (cw, _) = layout(&[1, 10, 2], 80, true);
        assert_eq!(cw, 12);
        assert_eq!(layout(&[], 80, true).1.len(), 0);
        /* more columns than entries is not a gap */
        let (_, rows) = layout(&[2, 2], 80, true);
        assert_eq!(rows, vec![vec![0, 1]]);
    }

    #[test]
    fn column_layout_is_the_same_width_for_every_column() {
        /* the spec: the column width in any one directory is constant */
        let names: Vec<usize> = vec![1, 2, 3, 20, 4, 5, 6, 7, 8, 9];
        let (cw, rows) = layout(&names, 60, true);
        assert_eq!(cw, 22);
        for r in &rows {
            assert!(r.len() <= 60 / cw);
        }
    }

    #[test]
    fn stream_format() {
        let n = |x: &[&str]| x.iter().map(|s| s.as_bytes().to_vec()).collect::<Vec<_>>();
        assert_eq!(stream(&n(&["a", "b", "c"]), 80), b"a, b, c\n");
        assert_eq!(stream(&n(&["only"]), 80), b"only\n");
        assert_eq!(stream(&[], 80), b"\n");
        /* a line of exactly the width, and one that must wrap */
        assert_eq!(stream(&n(&["aaaa", "bbbb", "cccc"]), 14), b"aaaa, bbbb,\ncccc\n");
        assert_eq!(stream(&n(&["aaaa", "bbbb", "cccc"]), 80), b"aaaa, bbbb, cccc\n");
        assert_eq!(stream(&n(&["aaaa", "bbbb", "cccc"]), 16), b"aaaa, bbbb, cccc\n");
        assert_eq!(stream(&n(&["aaaa", "bbbb", "cccc"]), 15), b"aaaa, bbbb,\ncccc\n");
        /* a name longer than the line stands alone */
        assert_eq!(stream(&n(&["a", "longlonglong", "b"]), 5), b"a,\nlonglonglong,\nb\n");
    }

    #[test]
    fn option_groups_last_one_wins() {
        let a = opts("Aa");
        assert!(a.all && !a.almost);
        let a = opts("aA");
        assert!(!a.all && a.almost);
        /* -f turns on -a, and -A (before or after it) cannot turn that off again */
        for s in ["fA", "Af", "Afa", "fAA"] {
            let a = opts(s);
            assert!(a.all && !a.almost && a.unsorted, "{s}");
        }
        assert!(opts("Fp").slash && !opts("Fp").classify);
        assert!(opts("pF").classify && !opts("pF").slash);
        assert!(opts("HL").follow_all && !opts("HL").follow_cmd);
        assert!(opts("LH").follow_cmd && !opts("LH").follow_all);
        /* long output is on or off as the last of -l/-n/-g/-o and -C/-m/-x says */
        assert!(opts("Cl").long);
        assert!(!opts("lC").long);
        assert!(!opts("lm").long);
        assert!(!opts("lx").long);
        assert!(opts("xl").long);
        assert!(opts("ml").long);
        assert!(opts("1l").long);
        assert!(opts("l1").long && opts("l1").fmt == Fmt::One);
        assert!(opts("gC").no_owner && !opts("gC").long);
        assert!(opts("n").numeric && opts("n").long);
        assert!(opts("o").no_group && opts("g").no_owner);
        /* -R with -d: -d wins, whichever comes last */
        assert!(!opts("Rd").recurse() && !opts("dR").recurse());
        assert!(opts("R").recurse() && !opts("d").recurse());
    }

    #[test]
    fn sort_option_rules() {
        assert_eq!(opts("St").sort, Sort::Time);
        assert_eq!(opts("tS").sort, Sort::Size);
        assert_eq!(opts("t").sort, Sort::Time);
        /* -c and -u pick the timestamp; they decide the sort only once -t was given */
        assert_eq!(opts("lcutS").sort, Sort::Size);
        assert_eq!(opts("lcutS").timekey, TimeKey::Atime);
        assert_eq!(opts("luctS").timekey, TimeKey::Ctime);
        assert_eq!(opts("luctS").sort, Sort::Size);
        assert_eq!(opts("Su").sort, Sort::Size);
        assert_eq!(opts("Su").timekey, TimeKey::Atime);
        assert_eq!(opts("tSu").sort, Sort::Time);
        assert_eq!(opts("tSc").timekey, TimeKey::Ctime);
        assert_eq!(opts("cu").timekey, TimeKey::Atime);
        assert_eq!(opts("uc").timekey, TimeKey::Ctime);
        /* -f ignores -r, -S and -t, wherever they are */
        let f = opts("ftSr");
        assert!(f.unsorted && f.all && !f.reverse && f.sort == Sort::Name);
        let f = opts("rStf");
        assert!(f.unsorted && !f.reverse && f.sort == Sort::Name);
    }

    #[test]
    fn operand_link_rule() {
        assert!(!opts("").operand_link_is_itself());
        assert!(opts("d").operand_link_is_itself());
        assert!(opts("F").operand_link_is_itself());
        assert!(opts("l").operand_link_is_itself());
        assert!(!opts("dH").operand_link_is_itself());
        assert!(!opts("lL").operand_link_is_itself());
        assert!(!opts("p").operand_link_is_itself());
        assert!(!opts("R").operand_link_is_itself());
    }

    #[test]
    fn unknown_option_letters_are_refused() {
        let mut o = Opts::new();
        for c in b"bBeEhjJKMNOPQTUvVwWXyYzZ0234567890" {
            assert!(!o.apply(*c), "{}", *c as char);
        }
    }

    fn ls_for_tests(o: Opts) -> Ls {
        Ls {
            prog: b"ls".to_vec(),
            o,
            out: Out { w: std::io::BufWriter::new(posix::Stdout::new()), prog: b"ls".to_vec() },
            status: 0,
            width: 80,
            now: 1183548000 + 86400,
            c_collate: true,
            sa: Vec::new(),
            sb: Vec::new(),
            users: HashMap::new(),
            groups: HashMap::new(),
            local: false,
            first: true,
            log: Some(Vec::new()),
        }
    }

    fn item(name: &str, st: libc::stat) -> Item {
        Item { name: name.as_bytes().to_vec(), path: name.as_bytes().to_vec(), st, is_link: false }
    }

    /* squeeze runs of blanks, as the spec's text of the example has */
    fn squeeze(b: &[u8]) -> String {
        String::from_utf8_lossy(b).split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /* EXAMPLES: ls -laRF a, in the POSIX locale (the first directory shown) */
    #[test]
    fn example_ls_la_r_f() {
        let mut ls = ls_for_tests(opts("laRF"));
        ls.users.insert(501, b"fox".to_vec());
        ls.groups.insert(20, b"prog".to_vec());
        let t = 1183548000 - 86400; /* well within six months of "now" */
        let mut items = vec![
            item(".", st_with(libc::S_IFDIR | 0o755, 3, 501, 20, 64, t)),
            item("..", st_with(libc::S_IFDIR | 0o777, 4, 501, 20, 3264, t)),
            item("b", st_with(libc::S_IFDIR | 0o755, 2, 501, 20, 48, t)),
            item("foo", st_with(libc::S_IFREG | 0o744, 1, 501, 20, 572, t)),
        ];
        ls.sort_items(&mut items);
        let text = ls.render(&items);
        let lines: Vec<String> = String::from_utf8_lossy(&text).lines().map(|l| squeeze(l.as_bytes())).collect();
        /* the date is the day before "now": Jul 3 */
        assert_eq!(
            lines,
            vec![
                "drwxr-xr-x 3 fox prog 64 Jul 3 11:20 ./",
                "drwxrwxrwx 4 fox prog 3264 Jul 3 11:20 ../",
                "drwxr-xr-x 2 fox prog 48 Jul 3 11:20 b/",
                "-rwxr--r-- 1 fox prog 572 Jul 3 11:20 foo*",
            ]
        );
    }

    #[test]
    fn long_format_alignment_and_variants() {
        let t = 1183548000 - 86400;
        let mk = |o: &str| {
            let mut ls = ls_for_tests(opts(o));
            ls.users.insert(1, b"al".to_vec());
            ls.users.insert(1000, b"longer".to_vec());
            ls.groups.insert(5, b"g".to_vec());
            let items = vec![
                item("a", st_with(libc::S_IFREG | 0o644, 1, 1, 5, 7, t)),
                item("b", st_with(libc::S_IFREG | 0o600, 12, 1000, 5, 123456, t)),
            ];
            String::from_utf8(ls.render(&items)).unwrap()
        };
        assert_eq!(
            mk("l"),
            "-rw-r--r--  1 al     g      7 Jul  3 11:20 a\n-rw------- 12 longer g 123456 Jul  3 11:20 b\n"
        );
        /* -g leaves out the owner, -o the group, -n shows the numbers */
        assert!(!mk("g").contains("al") && mk("g").contains(" g "));
        assert!(!mk("o").contains(" g ") && mk("o").contains(" al "));
        assert!(mk("n").contains(" 1 ") && mk("n").contains("1000"));
        assert_eq!(squeeze(mk("go").lines().next().unwrap().as_bytes()), "-rw-r--r-- 1 7 Jul 3 11:20 a");
    }

    #[test]
    fn inode_and_block_prefixes() {
        let t = 1183548000 - 86400;
        let mut st = st_with(libc::S_IFREG | 0o644, 1, 1, 5, 7, t);
        st.st_ino = 42;
        st.st_blocks = 8;
        let mut ls = ls_for_tests(opts("1is"));
        let items = vec![item("x", st)];
        assert_eq!(String::from_utf8(ls.render(&items)).unwrap(), "42 8 x\n");
        let mut ls = ls_for_tests(opts("1isk"));
        assert_eq!(String::from_utf8(ls.render(&items)).unwrap(), "42 4 x\n");
        let mut ls = ls_for_tests(opts("ls"));
        ls.users.insert(1, b"u".to_vec());
        ls.groups.insert(5, b"g".to_vec());
        assert!(String::from_utf8(ls.render(&items)).unwrap().starts_with("8 -rw-r--r--"));
        let mut ls = ls_for_tests(opts("li"));
        ls.users.insert(1, b"u".to_vec());
        ls.groups.insert(5, b"g".to_vec());
        assert!(String::from_utf8(ls.render(&items)).unwrap().starts_with("42 -rw-r--r--"));
        /* an odd number of 512 byte blocks rounds up to a whole kilobyte */
        let mut odd = st;
        odd.st_blocks = 3;
        let ls = ls_for_tests(opts("sk"));
        assert_eq!(ls.blocks_of(&odd), 2);
    }

    #[test]
    fn suffixes() {
        let mk = |o: &str, mode: libc::mode_t| {
            let ls = ls_for_tests(opts(o));
            String::from_utf8(ls.suffix(&item("x", st_with(mode, 1, 0, 0, 0, 0))).to_vec()).unwrap()
        };
        assert_eq!(mk("F", libc::S_IFDIR | 0o755), "/");
        assert_eq!(mk("F", libc::S_IFREG | 0o755), "*");
        assert_eq!(mk("F", libc::S_IFREG | 0o644), "");
        assert_eq!(mk("F", libc::S_IFREG | 0o010), "*");
        assert_eq!(mk("F", libc::S_IFIFO | 0o644), "|");
        assert_eq!(mk("F", libc::S_IFLNK | 0o777), "@");
        assert_eq!(mk("F", libc::S_IFSOCK | 0o777), "=");
        assert_eq!(mk("p", libc::S_IFDIR | 0o755), "/");
        assert_eq!(mk("p", libc::S_IFREG | 0o755), "");
        assert_eq!(mk("p", libc::S_IFIFO | 0o755), "");
        assert_eq!(mk("", libc::S_IFDIR | 0o755), "");
        assert_eq!(mk("pF", libc::S_IFREG | 0o755), "*");
        assert_eq!(mk("Fp", libc::S_IFREG | 0o755), "");
    }

    #[test]
    fn sorting_rules() {
        let t = 1000;
        let mk = |name: &str, size: i64, m: i64| {
            let mut st = st_with(libc::S_IFREG | 0o644, 1, 0, 0, size, m);
            st.st_mtime_nsec = 0;
            item(name, st)
        };
        let names = |ls: &mut Ls, o: &str| {
            let mut items = vec![mk("b", 5, t), mk("a", 5, t + 10), mk("c", 99, t), mk("B", 1, t + 5), mk("d", 5, t)];
            *ls = ls_for_tests(opts(o));
            ls.sort_items(&mut items);
            items.iter().map(|i| String::from_utf8(i.name.clone()).unwrap()).collect::<Vec<_>>().join("")
        };
        let mut ls = ls_for_tests(opts(""));
        /* the C locale: byte order, upper case first */
        assert_eq!(names(&mut ls, ""), "Babcd");
        assert_eq!(names(&mut ls, "r"), "dcbaB");
        /* by size, biggest first, equal sizes by name */
        assert_eq!(names(&mut ls, "S"), "cabdB");
        assert_eq!(names(&mut ls, "Sr"), "Bdbac");
        /* by time, newest first, equal times by name */
        assert_eq!(names(&mut ls, "t"), "aBbcd");
        assert_eq!(names(&mut ls, "tr"), "dcbBa");
        /* -f: the order the entries came in */
        assert_eq!(names(&mut ls, "f"), "bacBd");
    }

    #[test]
    fn wide_characters_count_by_columns() {
        assert_eq!(text_width(b"abc"), 3);
        assert_eq!(text_width(b""), 0);
        assert_eq!(text_width(&[0xff, b'a']), 2);
    }

    #[test]
    fn width_from_columns_variable() {
        assert_eq!(line_width(Some(b"40")), 40);
        assert_eq!(line_width(Some(b"132")), 132);
        /* invalid or zero: the terminal's width or 80 */
        let d = line_width(None);
        assert_eq!(line_width(Some(b"abc")), d);
        assert_eq!(line_width(Some(b"0")), d);
        assert_eq!(line_width(Some(b"-5")), d);
        assert_eq!(line_width(Some(b"")), d);
    }
}
