/* SPDX-License-Identifier: GPL-2.0-only */
/* sort - sort, merge, or sequence check text files. */

use posix::getopt::{Getopt, Opt};
use posix::mb;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;

const SYNOPSIS: &str = "sort [-m] [-o output] [-bdfinru] [-t char] [-k keydef]... [file...]\n       sort [-c|-C] [-bdfinru] [-t char] [-k keydef] [file]";
const BLOCK: usize = 65536;

/* The ordering options of a key. */
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Flags {
    d: bool,
    f: bool,
    i: bool,
    n: bool,
    r: bool,
}

/*
 * A sort key: where it starts and ends in a line, and how it is compared.
 * Field numbers count from 1. A character number is the position inside
 * the field; at the start 0 means the first character (as 1), at the end 0
 * means the last character of the field. `own` is set when the key carries
 * modifiers; the options given for the whole command then do not apply.
 */
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    f1: usize,
    c1: usize,
    b1: bool,
    end: Option<(usize, usize)>,
    b2: bool,
    flags: Flags,
    own: bool,
}

/* A decimal number of any size; a huge one saturates. */
fn number(arg: &[u8], i: &mut usize) -> Option<usize> {
    let start = *i;
    let mut n: usize = 0;
    while let Some(&c) = arg.get(*i) {
        if !c.is_ascii_digit() {
            break;
        }
        n = n.saturating_mul(10).saturating_add((c - b'0') as usize);
        *i += 1;
    }
    if *i == start { None } else { Some(n) }
}

/* The modifiers b d f i n r after a field_start or field_end; returns whether b was one. */
fn modifiers(arg: &[u8], i: &mut usize, flags: &mut Flags) -> bool {
    let mut b = false;
    while let Some(&c) = arg.get(*i) {
        match c {
            b'b' => b = true,
            b'd' => flags.d = true,
            b'f' => flags.f = true,
            b'i' => flags.i = true,
            b'n' => flags.n = true,
            b'r' => flags.r = true,
            _ => break,
        }
        *i += 1;
    }
    b
}

/* field_start[type][,field_end[type]] */
fn parse_keydef(arg: &[u8]) -> Option<Key> {
    let mut i = 0;
    let mut flags = Flags::default();
    let f1 = number(arg, &mut i).filter(|&n| n > 0)?;
    let mut c1 = 0;
    if arg.get(i) == Some(&b'.') {
        i += 1;
        c1 = number(arg, &mut i).filter(|&n| n > 0)?;
    }
    let before = i;
    let b1 = modifiers(arg, &mut i, &mut flags);
    let mut own = i > before;
    let mut end = None;
    let mut b2 = false;
    if arg.get(i) == Some(&b',') {
        i += 1;
        let f2 = number(arg, &mut i).filter(|&n| n > 0)?;
        let mut c2 = 0;
        if arg.get(i) == Some(&b'.') {
            i += 1;
            c2 = number(arg, &mut i)?;
        }
        let before = i;
        b2 = modifiers(arg, &mut i, &mut flags);
        own |= i > before;
        end = Some((f2, c2));
    }
    if i != arg.len() {
        return None;
    }
    Some(Key { f1, c1, b1, end, b2, flags, own })
}

/* How a command line asks for the work to be done. */
#[derive(Default)]
struct Opts {
    check: bool,
    quiet: bool,
    merge: bool,
    unique: bool,
    output: Option<Vec<u8>>,
    sep: Option<Vec<u8>>,
    keys: Vec<Key>,
    global: Flags,
    global_b: bool,
    files: Vec<Vec<u8>>,
}

/* None: a usage error. */
fn parse_args(args: &[Vec<u8>]) -> Option<Opts> {
    let mut o = Opts::default();
    let mut g = Getopt::new(args, b"cCmo:bdfinrut:k:");
    for opt in g.by_ref() {
        match opt {
            Opt::Flag(b'c') => o.check = true,
            Opt::Flag(b'C') => {
                o.check = true;
                o.quiet = true;
            }
            Opt::Flag(b'm') => o.merge = true,
            Opt::Flag(b'u') => o.unique = true,
            Opt::Flag(b'b') => o.global_b = true,
            Opt::Flag(b'd') => o.global.d = true,
            Opt::Flag(b'f') => o.global.f = true,
            Opt::Flag(b'i') => o.global.i = true,
            Opt::Flag(b'n') => o.global.n = true,
            Opt::Flag(b'r') => o.global.r = true,
            Opt::Arg(b'o', a) => o.output = Some(a),
            Opt::Arg(b't', a) => {
                /* exactly one character */
                if a.is_empty() || mb::char_at(&a).0 != a.len() {
                    return None;
                }
                o.sep = Some(a);
            }
            Opt::Arg(b'k', a) => o.keys.push(parse_keydef(&a)?),
            _ => return None,
        }
    }
    o.files = g.operands().to_vec();
    if o.check && (o.merge || o.output.is_some() || o.files.len() > 1) {
        return None;
    }
    if o.keys.is_empty() {
        o.keys.push(Key { f1: 1, c1: 0, b1: false, end: None, b2: false, flags: Flags::default(), own: false });
    }
    /*
     * What was given for the whole command goes to the keys that have no
     * modifiers (the default key, the whole line, is one).
     */
    let (global, gb) = (o.global, o.global_b);
    for k in o.keys.iter_mut().filter(|k| !k.own) {
        k.flags = global;
        k.b1 = gb;
        k.b2 = gb;
    }
    Some(o)
}

/* Numeric conventions of LC_NUMERIC for -n. */
#[derive(Clone, Default)]
struct NumLocale {
    point: Vec<u8>,
    thousands: Vec<u8>,
}

fn num_locale() -> NumLocale {
    unsafe {
        let lc = libc::localeconv();
        if lc.is_null() {
            return NumLocale { point: b".".to_vec(), thousands: Vec::new() };
        }
        let get = |p: *const libc::c_char| if p.is_null() { Vec::new() } else { std::ffi::CStr::from_ptr(p).to_bytes().to_vec() };
        let mut point = get((*lc).decimal_point);
        if point.is_empty() {
            point = b".".to_vec();
        }
        NumLocale { point, thousands: get((*lc).thousands_sep) }
    }
}

/* Everything a comparison needs to know. */
struct Ctx {
    keys: Vec<Key>,
    sep: Option<Vec<u8>>,
    unique: bool,
    /* -r for the whole command: it reverses the last resort comparison too */
    reverse: bool,
    c_locale: bool,
    num: NumLocale,
    /* one key, the whole line, no ordering option but -r (not even -b): compare lines directly */
    plain: bool,
}

impl Ctx {
    fn new(o: &Opts) -> Ctx {
        let k = &o.keys[0];
        let plain = o.keys.len() == 1 && k.f1 == 1 && k.c1 == 0 && k.end.is_none() && !k.b1 && !k.flags.d && !k.flags.f && !k.flags.i && !k.flags.n;
        Ctx {
            keys: o.keys.clone(),
            sep: o.sep.clone(),
            unique: o.unique,
            reverse: o.global.r,
            c_locale: mb::collation_is_c(),
            num: num_locale(),
            plain,
        }
    }
}

/* Bytes of leading <blank>s. */
fn blank_prefix(s: &[u8]) -> usize {
    let mut i = 0;
    while i < s.len() {
        let (n, blank) = mb::blank_at(&s[i..]);
        if !blank {
            break;
        }
        i += n;
    }
    i
}

/* Offset of the first separator character sp in line at or after from. */
fn find_sep(line: &[u8], from: usize, sp: &[u8]) -> Option<usize> {
    if sp.len() == 1 && sp[0] < 0x80 {
        return posix::find_byte(sp[0], &line[from..]).map(|i| from + i);
    }
    let mut i = from;
    while i < line.len() {
        if line[i..].starts_with(sp) {
            return Some(i);
        }
        i += mb::char_at(&line[i..]).0;
    }
    None
}

/*
 * The bytes of field n (from 1) of a line, as (start, end). Without -t a
 * field is its separator (a run of blanks that follows a non-blank) and
 * the non-blanks after it, so the first field also holds any leading
 * blanks of the line. With -t the separators are not part of the fields.
 * A field the line does not reach is empty and at the end of the line.
 */
fn field_bounds(line: &[u8], n: usize, sep: &Option<Vec<u8>>) -> (usize, usize) {
    let len = line.len();
    match sep {
        Some(sp) => {
            let mut start = 0;
            for _ in 1..n {
                match find_sep(line, start, sp) {
                    Some(p) => start = p + sp.len(),
                    None => return (len, len),
                }
            }
            (start, find_sep(line, start, sp).unwrap_or(len))
        }
        None => {
            let mut p = 0;
            for k in 1..=n {
                let start = p;
                let mut q = start + blank_prefix(&line[start..]);
                while q < len {
                    let (l, blank) = mb::blank_at(&line[q..]);
                    if blank {
                        break;
                    }
                    q += l;
                }
                if k == n {
                    return (start, q);
                }
                p = q;
                if p >= len {
                    return (len, len);
                }
            }
            (len, len)
        }
    }
}

/* The part of a line a key covers, as (start, end); empty when it ends before it starts. */
fn key_range(line: &[u8], k: &Key, sep: &Option<Vec<u8>>) -> (usize, usize) {
    let len = line.len();
    let (fs, fe) = field_bounds(line, k.f1, sep);
    let mut s = fs;
    if k.b1 {
        s += blank_prefix(&line[s..fe]);
    }
    if k.c1 > 1 {
        s += mb::skip_chars(&line[s..fe], k.c1 - 1);
    }
    let e = match k.end {
        None => len,
        Some((f2, c2)) => {
            let (gs, ge) = field_bounds(line, f2, sep);
            if c2 == 0 {
                ge
            } else {
                let mut b = gs;
                if k.b2 {
                    b += blank_prefix(&line[b..ge]);
                }
                b + mb::skip_chars(&line[b..ge], c2)
            }
        }
    };
    if s >= e {
        let s = s.min(len);
        (s, s)
    } else {
        (s, e)
    }
}

/* Buffers that comparisons reuse, so that they allocate nothing. */
#[derive(Default)]
struct Scratch {
    a: Vec<u8>,
    b: Vec<u8>,
    ca: Vec<u8>,
    cb: Vec<u8>,
    ia: Vec<u8>,
    fa: Vec<u8>,
    ib: Vec<u8>,
    fb: Vec<u8>,
}

/* The key text after -d, -i and -f. */
fn transform(src: &[u8], f: Flags, out: &mut Vec<u8>) {
    out.clear();
    let mut i = 0;
    while i < src.len() {
        let (l, wc) = mb::char_at(&src[i..]);
        let piece = &src[i..i + l];
        i += l;
        match wc {
            /* a byte that is no character is neither alphanumeric nor printable */
            None => {
                if !f.d && !f.i {
                    out.extend_from_slice(piece);
                }
            }
            Some(w) => {
                if f.d && !(mb::is_blank(w) || mb::is_alnum(w)) {
                    continue;
                }
                if f.i && !mb::is_print(w) {
                    continue;
                }
                if f.f {
                    let u = mb::to_upper(w);
                    if u != w && mb::encode(u, out) {
                        continue;
                    }
                }
                out.extend_from_slice(piece);
            }
        }
    }
}

/*
 * The initial number of s: blanks, a minus sign, digits (thousands
 * separators between them are ignored), the radix character and more
 * digits. Returns whether it is negative; the digits without leading zeros
 * go to int and the fraction digits without trailing zeros to frac. Zero
 * has no sign.
 */
fn parse_number(s: &[u8], lc: &NumLocale, int: &mut Vec<u8>, frac: &mut Vec<u8>) -> bool {
    int.clear();
    frac.clear();
    let mut i = blank_prefix(s);
    let neg = s.get(i) == Some(&b'-');
    if neg {
        i += 1;
    }
    loop {
        match s.get(i) {
            Some(&c) if c.is_ascii_digit() => {
                if !(int.is_empty() && c == b'0') {
                    int.push(c);
                }
                i += 1;
            }
            Some(_) if !lc.thousands.is_empty() && s[i..].starts_with(&lc.thousands) && s.get(i + lc.thousands.len()).is_some_and(|c| c.is_ascii_digit()) => {
                i += lc.thousands.len();
            }
            _ => break,
        }
    }
    if s[i.min(s.len())..].starts_with(&lc.point) {
        i += lc.point.len();
        while let Some(&c) = s.get(i) {
            if !c.is_ascii_digit() {
                break;
            }
            frac.push(c);
            i += 1;
        }
        while frac.last() == Some(&b'0') {
            frac.pop();
        }
    }
    neg && !(int.is_empty() && frac.is_empty())
}

fn compare_numbers(a: &[u8], b: &[u8], lc: &NumLocale, sc: &mut Scratch) -> Ordering {
    let na = parse_number(a, lc, &mut sc.ia, &mut sc.fa);
    let nb = parse_number(b, lc, &mut sc.ib, &mut sc.fb);
    match (na, nb) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    let mag = sc.ia.len().cmp(&sc.ib.len()).then_with(|| sc.ia.cmp(&sc.ib)).then_with(|| sc.fa.cmp(&sc.fb));
    if na { mag.reverse() } else { mag }
}

/* Compare the texts of one key. */
fn compare_key(ctx: &Ctx, k: &Key, a: &[u8], b: &[u8], sc: &mut Scratch) -> Ordering {
    let f = k.flags;
    let o = if f.n {
        compare_numbers(a, b, &ctx.num, sc)
    } else if f.d || f.f || f.i {
        transform(a, f, &mut sc.a);
        transform(b, f, &mut sc.b);
        mb::collate(&sc.a, &sc.b, ctx.c_locale, &mut sc.ca, &mut sc.cb)
    } else {
        mb::collate(a, b, ctx.c_locale, &mut sc.ca, &mut sc.cb)
    };
    if f.r { o.reverse() } else { o }
}

/* All input: the text of the lines, where they are, and where each key is. */
#[derive(Default)]
struct Data {
    buf: Vec<u8>,
    /* start and end (without the newline) of every line */
    lines: Vec<(usize, usize)>,
    /* the keys of every line, `keys.len()` of them in a row */
    ranges: Vec<(usize, usize)>,
    /*
     * For each numeric key of every line (laid out like ranges), the number
     * when it is an integer of at most 18 digits: those compare as i64
     * without parsing the text again. Empty if no key is numeric.
     */
    nums: Vec<Option<i64>>,
    /* for each file, the number of lines up to its end */
    ends: Vec<usize>,
    /* where the lines of the file being added begin in buf */
    from: usize,
    /* some line has a NUL inside */
    has_nul: bool,
}

impl Data {
    /* The bytes of one more file have been appended to buf: split them into lines. */
    fn finish_file(&mut self) {
        if self.buf.len() > self.from && self.buf.last() != Some(&b'\n') {
            self.buf.push(b'\n');
        }
        if posix::find_byte(0, &self.buf[self.from..]).is_some() {
            self.has_nul = true;
        }
        let mut pos = self.from;
        while pos < self.buf.len() {
            let nl = match posix::find_byte(b'\n', &self.buf[pos..]) {
                Some(i) => pos + i,
                None => self.buf.len(),
            };
            self.lines.push((pos, nl));
            pos = nl + 1;
        }
        self.ends.push(self.lines.len());
        self.from = self.buf.len();
    }

    fn line(&self, i: usize) -> &[u8] {
        let (s, e) = self.lines[i];
        &self.buf[s..e]
    }

    /*
     * Work out where the keys are, and the numeric ones. Then the newline
     * after every line becomes a NUL, so that a line is a C string for
     * strcoll (the output adds its own newlines).
     */
    fn find_keys(&mut self, ctx: &Ctx) {
        if !ctx.plain {
            self.ranges.reserve(self.lines.len() * ctx.keys.len());
            for &(s, e) in &self.lines {
                let line = &self.buf[s..e];
                for k in &ctx.keys {
                    self.ranges.push(key_range(line, k, &ctx.sep));
                }
            }
        }
        if ctx.keys.iter().any(|k| k.flags.n) {
            let mut sc = Scratch::default();
            self.nums.reserve(self.ranges.len());
            for (n, &(s, e)) in self.ranges.iter().enumerate() {
                let k = &ctx.keys[n % ctx.keys.len()];
                let line = self.lines[n / ctx.keys.len()].0;
                self.nums.push(if k.flags.n { small_number(&self.buf[line + s..line + e], &ctx.num, &mut sc) } else { None });
            }
        }
        if !self.has_nul {
            for &(_, e) in &self.lines {
                self.buf[e] = 0;
            }
        }
    }
}

/* The value of a number of at most 18 digits and no fraction. */
fn small_number(s: &[u8], lc: &NumLocale, sc: &mut Scratch) -> Option<i64> {
    let neg = parse_number(s, lc, &mut sc.ia, &mut sc.fa);
    if sc.ia.len() > 18 || !sc.fa.is_empty() {
        return None;
    }
    let v = sc.ia.iter().fold(0i64, |v, &c| v * 10 + (c - b'0') as i64);
    Some(if neg { -v } else { v })
}

/*
 * Compare two lines by their keys; with -u that is all, otherwise lines
 * with equal keys are compared as whole lines, by collation and then byte
 * by byte, with -r still in effect.
 */
fn compare_lines(ctx: &Ctx, d: &Data, i: usize, j: usize, sc: &mut Scratch) -> Ordering {
    let (a, b) = (d.line(i), d.line(j));
    if ctx.plain {
        let mut o = if ctx.c_locale {
            a.cmp(b)
        } else if !d.has_nul {
            /* both lines end in a NUL, see find_keys */
            unsafe { libc::strcoll(a.as_ptr() as *const libc::c_char, b.as_ptr() as *const libc::c_char) }.cmp(&0)
        } else {
            mb::collate(a, b, false, &mut sc.ca, &mut sc.cb)
        };
        if o == Ordering::Equal && !ctx.unique {
            o = a.cmp(b);
        }
        return if ctx.keys[0].flags.r { o.reverse() } else { o };
    }
    let nk = ctx.keys.len();
    for (n, k) in ctx.keys.iter().enumerate() {
        let (s1, e1) = d.ranges[i * nk + n];
        let (s2, e2) = d.ranges[j * nk + n];
        let o = match (k.flags.n, d.nums.get(i * nk + n).copied().flatten(), d.nums.get(j * nk + n).copied().flatten()) {
            (true, Some(x), Some(y)) => {
                let o = x.cmp(&y);
                if k.flags.r { o.reverse() } else { o }
            }
            _ => compare_key(ctx, k, &a[s1..e1], &b[s2..e2], sc),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    if ctx.unique {
        return Ordering::Equal;
    }
    let o = mb::collate(a, b, ctx.c_locale, &mut sc.ca, &mut sc.cb).then_with(|| a.cmp(b));
    if ctx.reverse { o.reverse() } else { o }
}

/* The lines of all files, in order. */
fn sorted(ctx: &Ctx, d: &Data) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..d.lines.len()).collect();
    let sc = RefCell::new(Scratch::default());
    /*
     * Lines whose keys are equal keep their input order (with -u the first
     * one is kept), which the index as the last tie-break gives an unstable
     * sort. Without -u the whole line breaks every tie already.
     */
    idx.sort_unstable_by(|&i, &j| compare_lines(ctx, d, i, j, &mut sc.borrow_mut()).then(i.cmp(&j)));
    idx
}

/* The lines of the files, each presorted, merged. Equal lines keep the order of the files. */
fn merged(ctx: &Ctx, d: &Data) -> Vec<usize> {
    let nf = d.ends.len();
    let mut head: Vec<usize> = (0..nf).map(|f| if f == 0 { 0 } else { d.ends[f - 1] }).collect();
    let mut out = Vec::with_capacity(d.lines.len());
    let mut sc = Scratch::default();
    loop {
        let mut best: Option<usize> = None;
        for f in 0..nf {
            if head[f] >= d.ends[f] {
                continue;
            }
            best = match best {
                Some(b) if compare_lines(ctx, d, head[f], head[b], &mut sc) != Ordering::Less => Some(b),
                _ => Some(f),
            };
        }
        let Some(f) = best else { break };
        out.push(head[f]);
        head[f] += 1;
    }
    out
}

/* Drop every line whose keys equal those of the one before it that was kept. */
fn dedupe(ctx: &Ctx, d: &Data, order: Vec<usize>) -> Vec<usize> {
    let mut sc = Scratch::default();
    let mut out: Vec<usize> = Vec::with_capacity(order.len());
    for i in order {
        if let Some(&last) = out.last() {
            if compare_lines(ctx, d, last, i, &mut sc) == Ordering::Equal {
                continue;
            }
        }
        out.push(i);
    }
    out
}

/*
 * Check the order of the lines. Returns the number (from 1) of the first
 * line that is out of order, or that repeats the one before it with -u.
 */
fn disorder(ctx: &Ctx, d: &Data) -> Option<usize> {
    let mut sc = Scratch::default();
    for i in 1..d.lines.len() {
        let o = compare_lines(ctx, d, i - 1, i, &mut sc);
        if o == Ordering::Greater || (ctx.unique && o == Ordering::Equal) {
            return Some(i + 1);
        }
    }
    None
}

fn write_lines<W: Write>(w: &mut W, d: &Data, order: &[usize]) -> io::Result<()> {
    for &i in order {
        w.write_all(d.line(i))?;
        w.write_all(b"\n")?;
    }
    w.flush()
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let Some(o) = parse_args(&args) else {
        posix::usage(SYNOPSIS);
    };
    let ctx = Ctx::new(&o);
    let mut d = Data::default();
    let names: Vec<Vec<u8>> = if o.files.is_empty() { vec![b"-".to_vec()] } else { o.files.clone() };
    for name in &names {
        let res = posix::open_input(name).and_then(|mut h| {
            if let Ok(m) = h.metadata() {
                if m.is_file() {
                    d.buf.reserve(m.len() as usize + 1);
                }
            }
            h.read_to_end(&mut d.buf)
        });
        if let Err(e) = res {
            posix::warn_path(&prog, name, &posix::strerror(&e));
            std::process::exit(2);
        }
        d.finish_file();
    }
    d.find_keys(&ctx);
    if o.check {
        match disorder(&ctx, &d) {
            None => std::process::exit(0),
            Some(n) => {
                if !o.quiet {
                    let mut msg = prog.clone();
                    msg.extend_from_slice(b": ");
                    msg.extend_from_slice(&names[0]);
                    msg.extend_from_slice(format!(":{n}: disorder: ").as_bytes());
                    msg.extend_from_slice(d.line(n - 1));
                    msg.push(b'\n');
                    let _ = io::stderr().lock().write_all(&msg);
                }
                std::process::exit(1);
            }
        }
    }
    let mut order = if o.merge { merged(&ctx, &d) } else { sorted(&ctx, &d) };
    if o.unique {
        order = dedupe(&ctx, &d, order);
    }
    /* the output may be one of the inputs: it is opened only now */
    let res = match &o.output {
        None => write_lines(&mut io::BufWriter::with_capacity(BLOCK, posix::Stdout::new()), &d, &order),
        Some(name) => {
            let path = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(name);
            match OpenOptions::new().write(true).create(true).truncate(true).mode(0o666).open(path) {
                Ok(f) => write_lines(&mut io::BufWriter::with_capacity(BLOCK, f), &d, &order),
                Err(e) => {
                    posix::warn_path(&prog, name, &posix::strerror(&e));
                    std::process::exit(2);
                }
            }
        }
    };
    if let Err(e) = res {
        posix::warn(&prog, &format!("write error: {}", posix::strerror(&e)));
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /* the locale is process-wide: tests that change it hold this */
    static LOCALE: Mutex<()> = Mutex::new(());

    fn argv(a: &[&str]) -> Vec<Vec<u8>> {
        std::iter::once("sort").chain(a.iter().copied()).map(|s| s.as_bytes().to_vec()).collect()
    }

    /* the whole job on in-memory files; None for a usage error */
    fn run(a: &[&str], files: &[&str]) -> Option<String> {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        run_inner(a, files)
    }

    fn run_inner(a: &[&str], files: &[&str]) -> Option<String> {
        let o = parse_args(&argv(a))?;
        let ctx = Ctx::new(&o);
        let mut d = Data::default();
        for f in files {
            d.buf.extend_from_slice(f.as_bytes());
            d.finish_file();
        }
        d.find_keys(&ctx);
        let mut order = if o.merge { merged(&ctx, &d) } else { sorted(&ctx, &d) };
        if o.unique {
            order = dedupe(&ctx, &d, order);
        }
        let mut out = Vec::new();
        write_lines(&mut out, &d, &order).ok()?;
        Some(String::from_utf8_lossy(&out).into_owned())
    }

    fn s(a: &[&str], input: &str) -> String {
        run(a, &[input]).expect("usage")
    }

    fn check(a: &[&str], input: &str) -> Option<usize> {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let o = parse_args(&argv(a)).expect("usage");
        let ctx = Ctx::new(&o);
        let mut d = Data::default();
        d.buf.extend_from_slice(input.as_bytes());
        d.finish_file();
        d.find_keys(&ctx);
        disorder(&ctx, &d)
    }

    #[test]
    fn plain_sort_is_byte_order_in_the_c_locale() {
        assert_eq!(s(&[], "b\na\nc\n"), "a\nb\nc\n");
        assert_eq!(s(&[], "B\na\nA\nb\n"), "A\nB\na\nb\n");
        assert_eq!(s(&[], ""), "");
        assert_eq!(s(&[], "\n\na\n"), "\n\na\n");
        assert_eq!(s(&[], "a\na\nb\na\n"), "a\na\na\nb\n");
        assert_eq!(s(&[], "ab\na\nabc\n"), "a\nab\nabc\n");
    }

    #[test]
    fn an_incomplete_last_line_gets_its_newline() {
        assert_eq!(s(&[], "b\na"), "a\nb\n");
        assert_eq!(run(&[], &["b", "a"]).unwrap(), "a\nb\n");
        assert_eq!(run(&[], &["b\n", ""]).unwrap(), "b\n");
    }

    #[test]
    fn reverse_and_unique() {
        assert_eq!(s(&["-r"], "a\nc\nb\n"), "c\nb\na\n");
        assert_eq!(s(&["-u"], "b\na\nb\na\n"), "a\nb\n");
        assert_eq!(s(&["-ru"], "b\na\nb\na\n"), "b\na\n");
    }

    #[test]
    fn numeric_sort() {
        assert_eq!(s(&["-n"], "10\n9\n100\n1\n"), "1\n9\n10\n100\n");
        assert_eq!(s(&["-n"], "-1\n-10\n0\n5\n"), "-10\n-1\n0\n5\n");
        assert_eq!(s(&["-n"], "1.5\n1.25\n1.05\n1\n"), "1\n1.05\n1.25\n1.5\n");
        assert_eq!(s(&["-n"], "  3\n 2\n1\n"), "1\n 2\n  3\n");
        assert_eq!(s(&["-n"], "010\n9\n"), "9\n010\n");
        assert_eq!(s(&["-nr"], "1\n3\n2\n"), "3\n2\n1\n");
    }

    #[test]
    fn numbers_without_digits_are_zero_and_signs_on_zero_do_not_count() {
        assert_eq!(s(&["-n"], "b\n-\n1\n-1\n"), "-1\n-\nb\n1\n");
        assert_eq!(s(&["-nu"], "0\n-0\n00\n-\nx\n"), "0\n");
        assert_eq!(s(&["-n"], "-0.0\n0\n-1\n"), "-1\n-0.0\n0\n");
    }

    #[test]
    fn numbers_of_any_length_compare_exactly() {
        assert_eq!(s(&["-n"], "123456789012345678901234567891\n123456789012345678901234567890\n"), "123456789012345678901234567890\n123456789012345678901234567891\n");
        assert_eq!(s(&["-n"], "0.1234567890123456789012\n0.1234567890123456789011\n"), "0.1234567890123456789011\n0.1234567890123456789012\n");
    }

    #[test]
    fn numeric_only_looks_at_the_initial_number() {
        assert_eq!(s(&["-n"], "2 b\n10 a\n1x\n"), "1x\n2 b\n10 a\n");
        /* equal numbers fall back to the whole line */
        assert_eq!(s(&["-n"], "1 b\n1 a\n"), "1 a\n1 b\n");
        assert_eq!(s(&["-nu"], "1 b\n1 a\n"), "1 b\n");
    }

    #[test]
    fn fold_dictionary_and_ignore() {
        assert_eq!(s(&["-f"], "b\nA\na\nB\n"), "A\na\nB\nb\n");
        assert_eq!(s(&["-fu"], "b\nA\na\nB\n"), "A\nb\n");
        assert_eq!(s(&["-d"], "a-c\nab\nab-\n"), "ab\nab-\na-c\n");
        assert_eq!(s(&["-d"], "a b\na-b\nab\n"), "a b\na-b\nab\n");
        assert_eq!(s(&["-i"], "a\u{1}c\nab\n"), "ab\na\u{1}c\n");
        assert_eq!(s(&["-iu"], "a\u{1}b\nab\n"), "a\u{1}b\n");
    }

    #[test]
    fn fields_by_blanks() {
        let t = "x 3 b\ny 1 c\nz 2 a\n";
        assert_eq!(s(&["-k", "2"], t), "y 1 c\nz 2 a\nx 3 b\n");
        assert_eq!(s(&["-k", "3,3"], t), "z 2 a\nx 3 b\ny 1 c\n");
        assert_eq!(s(&["-k2,2n"], t), "y 1 c\nz 2 a\nx 3 b\n");
        assert_eq!(s(&["-k", "2,2nr"], t), "x 3 b\nz 2 a\ny 1 c\n");
        /* a field holds the blanks before it */
        assert_eq!(s(&["-k", "2"], "a  z\nb y\n"), "a  z\nb y\n");
        assert_eq!(s(&["-b", "-k", "2"], "a  z\nb y\n"), "b y\na  z\n");
        assert_eq!(s(&["-k", "2"], "a  b\nb a\n"), "a  b\nb a\n");
        assert_eq!(s(&["-k", "2"], "a   b\nb  c\n"), "a   b\nb  c\n");
        assert_eq!(s(&["-k", "2b"], "a   b\nb  a\n"), "b  a\na   b\n");
    }

    #[test]
    fn b_on_the_whole_line_ignores_leading_blanks() {
        assert_eq!(s(&["-b"], " b\na\n  c\n"), "a\n b\n  c\n");
        assert_eq!(s(&[], " b\na\n  c\n"), "  c\n b\na\n");
    }

    #[test]
    fn fields_by_a_separator() {
        let t = "b|3|x\na|10|y\nc|2|z\n";
        assert_eq!(s(&["-t", "|", "-k", "2,2n"], t), "c|2|z\nb|3|x\na|10|y\n");
        assert_eq!(s(&["-t", "|", "-k", "2,2"], t), "a|10|y\nc|2|z\nb|3|x\n");
        assert_eq!(s(&["-t", "|", "-k", "3"], t), "b|3|x\na|10|y\nc|2|z\n");
        /* every separator counts: an empty field between two */
        assert_eq!(s(&["-t", ":", "-k", "2,2"], "a::z\nb:y:\n"), "a::z\nb:y:\n");
        assert_eq!(s(&["-t", ":", "-k", "3,3"], "a::z\nb:y:\n"), "b:y:\na::z\n");
        /* a field the line does not reach is empty */
        assert_eq!(s(&["-t", ":", "-k", "3"], "a:b\nx:y:z\nc\n"), "a:b\nc\nx:y:z\n");
    }

    #[test]
    fn characters_inside_fields() {
        assert_eq!(s(&["-k", "1.2,1.2"], "ab\nba\nac\n"), "ba\nab\nac\n");
        assert_eq!(s(&["-k", "1.2"], "ab\nba\nac\n"), "ba\nab\nac\n");
        assert_eq!(s(&["-k", "1.1,1.1"], "ab\nba\nac\n"), "ab\nac\nba\n");
        assert_eq!(s(&["-k", "1.2,1.2", "-u"], "xa1\nya2\nzb3\n"), "xa1\nzb3\n");
        /* ,1.0 is the end of the field */
        assert_eq!(s(&["-um", "-k", "1.1,1.0"], "a\na\nb\n"), "a\nb\n");
        /* the second character of the second field, counted after the blanks */
        assert_eq!(s(&["-k", "2.2b,2.2b"], "x   ab\ny c\nz   aa\n"), "y c\nz   aa\nx   ab\n");
        assert_eq!(s(&["-k", "2.2,2.2"], "x ab\ny  b\n"), "y  b\nx ab\n");
    }

    #[test]
    fn several_keys_in_order() {
        let t = "b 2\na 2\nb 1\na 1\n";
        assert_eq!(s(&["-k", "2,2", "-k", "1,1"], t), "a 1\nb 1\na 2\nb 2\n");
        assert_eq!(s(&["-k", "1,1", "-k", "2,2nr"], t), "a 2\na 1\nb 2\nb 1\n");
    }

    #[test]
    fn global_options_apply_to_keys_without_modifiers() {
        let t = "a 10\nb 9\n";
        assert_eq!(s(&["-n", "-k", "2"], t), "b 9\na 10\n");
        assert_eq!(s(&["-k", "2", "-n"], t), "b 9\na 10\n");
        /* a modifier on the key turns the global ones off for it: this is r alone */
        assert_eq!(s(&["-n", "-k", "2r"], "a 10\nb 9\nc 100\n"), "b 9\nc 100\na 10\n");
        assert_eq!(s(&["-n", "-k", "2"], "a 10\nb 9\nc 100\n"), "b 9\na 10\nc 100\n");
    }

    #[test]
    fn the_whole_line_breaks_ties_unless_unique() {
        assert_eq!(s(&["-k", "1,1"], "a z\na b\n"), "a b\na z\n");
        assert_eq!(s(&["-k", "1,1", "-r"], "a z\na b\n"), "a z\na b\n");
        assert_eq!(s(&["-k", "1,1", "-u"], "a z\na b\n"), "a z\n");
        assert_eq!(s(&["-k", "1,1n", "-u"], "1 z\n01 b\n"), "1 z\n");
    }

    #[test]
    fn merge_keeps_each_file_in_order() {
        assert_eq!(run(&["-m"], &["a\nc\ne\n", "b\nd\nf\n"]).unwrap(), "a\nb\nc\nd\ne\nf\n");
        assert_eq!(run(&["-m"], &["a\nb\n"]).unwrap(), "a\nb\n");
        assert_eq!(run(&["-m"], &["", "a\n", ""]).unwrap(), "a\n");
        assert_eq!(run(&["-mu"], &["a\nb\n", "b\nc\n"]).unwrap(), "a\nb\nc\n");
        assert_eq!(run(&["-mu", "-k", "1,1"], &["a x\na y\nb z\n"]).unwrap(), "a x\nb z\n");
        assert_eq!(run(&["-mn"], &["1\n10\n", "2\n3\n"]).unwrap(), "1\n2\n3\n10\n");
        /* unsorted input is merged as it is */
        assert_eq!(run(&["-m"], &["b\na\n", "c\n"]).unwrap(), "b\na\nc\n");
    }

    #[test]
    fn several_files_sort_together() {
        assert_eq!(run(&[], &["c\na\n", "b\n"]).unwrap(), "a\nb\nc\n");
    }

    #[test]
    fn check_finds_the_first_disorder() {
        assert_eq!(check(&["-c"], "a\nb\nc\n"), None);
        assert_eq!(check(&["-c"], "a\nc\nb\n"), Some(3));
        assert_eq!(check(&["-c"], "a\na\n"), None);
        assert_eq!(check(&["-cu"], "a\na\n"), Some(2));
        assert_eq!(check(&["-cu"], "a\nb\n"), None);
        assert_eq!(check(&["-c"], ""), None);
        assert_eq!(check(&["-c"], "a\n"), None);
        assert_eq!(check(&["-cn"], "1\n2\n10\n"), None);
        assert_eq!(check(&["-c"], "1\n2\n10\n"), Some(3));
        assert_eq!(check(&["-cr"], "b\na\n"), None);
        assert_eq!(check(&["-C"], "b\na\n"), Some(2));
        /* the example of the spec: <tab> < <space> */
        assert_eq!(check(&["-c", "-k", "2"], "y\tb\nx a\n"), None);
    }

    #[test]
    fn keydefs() {
        let k = parse_keydef(b"2").unwrap();
        assert_eq!((k.f1, k.c1, k.end, k.own), (2, 0, None, false));
        let k = parse_keydef(b"2.3,4.5").unwrap();
        assert_eq!((k.f1, k.c1, k.end), (2, 3, Some((4, 5))));
        let k = parse_keydef(b"1.2b,3nr").unwrap();
        assert!(k.b1 && !k.b2 && k.own && k.flags.n && k.flags.r);
        assert_eq!(parse_keydef(b"1,2.0").unwrap().end, Some((2, 0)));
        for bad in ["", "0", "1.0", "a", "1,", "1,0", "1x", "1,2x", "1.", "-1", "1 ", ",1", "1,2,3"] {
            assert!(parse_keydef(bad.as_bytes()).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn usage_errors() {
        assert!(run(&["-x"], &[]).is_none());
        assert!(run(&["-k", "0"], &[]).is_none());
        assert!(run(&["-t", ""], &[]).is_none());
        assert!(run(&["-t", "ab"], &[]).is_none());
        assert!(parse_args(&argv(&["-c", "-m"])).is_none());
        assert!(parse_args(&argv(&["-c", "-o", "x"])).is_none());
        assert!(parse_args(&argv(&["-c", "a", "b"])).is_none());
        assert!(parse_args(&argv(&["-o"])).is_none());
        assert!(parse_args(&argv(&["-C", "a"])).is_some());
    }

    #[test]
    fn nul_and_high_bytes() {
        assert_eq!(s(&[], "a\0b\na\0a\n"), "a\0a\na\0b\n");
        assert_eq!(s(&[], "\u{ff}\na\n"), "a\n\u{ff}\n");
    }

    #[test]
    fn a_lot_of_lines() {
        let mut x: u64 = 7;
        let mut input = String::new();
        for _ in 0..50_000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            input.push_str(&format!("{}\n", (x >> 40) % 1000));
        }
        let out = s(&["-n"], &input);
        let v: Vec<u32> = out.lines().map(|l| l.parse().unwrap()).collect();
        assert!(v.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(v.len(), 50_000);
        let u = s(&["-nu"], &input);
        assert_eq!(u.lines().count(), 1000);
    }

    #[test]
    fn a_long_line() {
        let big = "x".repeat(500_000);
        let out = s(&[], &format!("{big}b\n{big}a\n"));
        assert!(out.starts_with(&format!("{big}a\n")));
    }

    #[test]
    fn utf8_fields_characters_and_classes() {
        let si = |a: &[&str], input: &str| run_inner(a, &[input]).expect("usage");
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let loc = std::ffi::CString::new("en_US.UTF-8").unwrap();
        if unsafe { libc::setlocale(libc::LC_ALL, loc.as_ptr()) }.is_null() {
            eprintln!("skipped: no locale en_US.UTF-8");
            return;
        }
        /* one character is two bytes: a key of one character must not cut it */
        assert_eq!(si(&["-k", "1.2,1.2"], "a\u{e9}z\nab\n"), "ab\na\u{e9}z\n");
        assert_eq!(si(&["-t", "\u{e9}", "-k", "2"], "a\u{e9}z\nb\u{e9}y\n"), "b\u{e9}y\na\u{e9}z\n");
        assert_eq!(si(&["-f"], "\u{e9}\nE\n\u{c9}\n").lines().count(), 3);
        assert_eq!(si(&["-fu"], "\u{e9}\n\u{c9}\n").lines().count(), 1);
        assert_eq!(si(&["-d"], "a\u{e9}\nab\n").lines().count(), 2);
        unsafe { libc::setlocale(libc::LC_ALL, c"C".as_ptr()) };
    }
}
