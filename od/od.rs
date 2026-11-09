/* SPDX-License-Identifier: GPL-2.0-only */
/* od - dump files in various formats. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::ldouble;
use posix::mb;

const SYNOPSIS: &str = "od [-v] [-A address_base] [-j skip] [-N count] [-t type_string]... [file...]\n       od [-bcdosx] [file] [[+]offset[.][b]]";

/* the bytes read at once, and the least input block */
const CHUNK: usize = 64 * 1024;
const BLOCK_MIN: usize = 16;
/* the bytes that follow a block and may belong to its last character */
const LOOKAHEAD: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Named,
    Char,
    Octal,
    Hex,
    Unsigned,
    Signed,
    Float,
}

/* one output type: what it shows and how many bytes each item takes */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Spec {
    kind: Kind,
    size: usize,
}

impl Spec {
    /* The columns of an item (without the blank before it). */
    fn width(&self) -> usize {
        let s = self.size;
        match self.kind {
            Kind::Named | Kind::Char => 3,
            Kind::Octal => (8 * s).div_ceil(3),
            Kind::Hex => 2 * s,
            Kind::Unsigned => [3, 5, 10, 20][s.trailing_zeros() as usize],
            Kind::Signed => [4, 6, 11, 20][s.trailing_zeros() as usize],
            Kind::Float if s == 4 => 15,
            Kind::Float if s == 8 => 24,
            Kind::Float => ldouble::WIDTH,
        }
    }
}

/* the size of C's int and long, which d o u x take without a number */
const INT: usize = std::mem::size_of::<libc::c_int>();
const LONG: usize = std::mem::size_of::<libc::c_long>();

/* Add the specifications of one -t option-argument. */
fn parse_types(s: &[u8], out: &mut Vec<Spec>) -> Result<(), String> {
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        i += 1;
        let (kind, is_float) = match c {
            b'a' => {
                out.push(Spec { kind: Kind::Named, size: 1 });
                continue;
            }
            b'c' => {
                out.push(Spec { kind: Kind::Char, size: 1 });
                continue;
            }
            b'd' => (Kind::Signed, false),
            b'o' => (Kind::Octal, false),
            b'u' => (Kind::Unsigned, false),
            b'x' => (Kind::Hex, false),
            b'f' => (Kind::Float, true),
            _ => return Err(format!("invalid character '{}' in type string '{}'", String::from_utf8_lossy(&[c]), String::from_utf8_lossy(s))),
        };
        let start = i;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        let size = if i > start {
            let n = posix::parse_decimal(&s[start..i]).filter(|&n| n <= 64).ok_or_else(|| format!("invalid type size '{}'", String::from_utf8_lossy(&s[start..i])))?;
            n as usize
        } else if i < s.len() && is_float && matches!(s[i], b'F' | b'D' | b'L') {
            i += 1;
            match s[i - 1] {
                b'F' => 4,
                b'D' => 8,
                _ => ldouble::SIZE,
            }
        } else if i < s.len() && !is_float && matches!(s[i], b'C' | b'S' | b'I' | b'L') {
            i += 1;
            match s[i - 1] {
                b'C' => 1,
                b'S' => std::mem::size_of::<libc::c_short>(),
                b'I' => INT,
                _ => LONG,
            }
        } else if is_float {
            8
        } else {
            INT
        };
        let ok = if is_float { size == 4 || size == 8 || size == ldouble::SIZE } else { matches!(size, 1 | 2 | 4 | 8) };
        if !ok {
            return Err(format!("invalid type size '{size}' for '{}'", c as char));
        }
        out.push(Spec { kind, size });
    }
    Ok(())
}

/*
 * A count for -j and -N: decimal, octal with a leading 0, hexadecimal with 0x
 * or 0X. With `suffix` a b, k or m may follow a decimal or an octal number
 * (512, 1024, 1048576); after hexadecimal digits a b is a digit.
 */
fn parse_count(s: &[u8], suffix: bool) -> Option<u64> {
    if s.len() > 2 && s[0] == b'0' && (s[1] == b'x' || s[1] == b'X') {
        return s[2..].iter().try_fold(0u64, |n, &c| n.checked_mul(16)?.checked_add((c as char).to_digit(16)? as u64));
    }
    let (digits, mult) = match s.last() {
        Some(b'b') if suffix => (&s[..s.len() - 1], 512u64),
        Some(b'k') if suffix => (&s[..s.len() - 1], 1024),
        Some(b'm') if suffix => (&s[..s.len() - 1], 1024 * 1024),
        _ => (s, 1),
    };
    if digits.is_empty() || !digits.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let radix = if digits.len() > 1 && digits[0] == b'0' { 8 } else { 10 };
    let n = digits.iter().try_fold(0u64, |n, &c| {
        let d = (c - b'0') as u64;
        if d >= radix {
            return None;
        }
        n.checked_mul(radix)?.checked_add(d)
    })?;
    n.checked_mul(mult)
}

/* The offset operand: [+]offset[.][b], octal unless a point follows, in units of 512 with a b. */
fn parse_offset(s: &[u8]) -> Option<u64> {
    let s = s.strip_prefix(b"+").unwrap_or(s);
    let (s, blocks) = match s.strip_suffix(b"b") {
        Some(r) => (r, true),
        None => (s, false),
    };
    let (digits, radix) = match s.strip_suffix(b".") {
        Some(r) => (r, 10u64),
        None => (s, 8),
    };
    if digits.is_empty() {
        return None;
    }
    let n = digits.iter().try_fold(0u64, |n, &c| {
        let d = (c as char).to_digit(10)? as u64;
        if d >= radix {
            return None;
        }
        n.checked_mul(radix)?.checked_add(d)
    })?;
    if blocks { n.checked_mul(512) } else { Some(n) }
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

/* The bytes of an input block: a multiple of the least common multiple of the item sizes, at least BLOCK_MIN. */
fn block_size(types: &[Spec]) -> usize {
    let l = types.iter().fold(1, |l, t| l / gcd(l, t.size) * t.size);
    let mut b = l;
    while b < BLOCK_MIN {
        b += l;
    }
    b
}

const NAMES: [&str; 33] = [
    "nul", "soh", "stx", "etx", "eot", "enq", "ack", "bel", "bs", "ht", "nl", "vt", "ff", "cr", "so", "si", "dle", "dc1", "dc2", "dc3", "dc4", "nak", "syn",
    "etb", "can", "em", "sub", "esc", "fs", "gs", "rs", "us", "sp",
];

/* The cell of a byte for -t a: its name, or itself, in three columns. */
fn named(b: u8) -> Vec<u8> {
    let b = b & 0x7f;
    let s: String = match b {
        0..=0x20 => NAMES[b as usize].to_string(),
        0x7f => "del".to_string(),
        _ => (b as char).to_string(),
    };
    pad(s.as_bytes(), s.len())
}

/* text, right-aligned in three columns, given the columns it takes */
fn pad(text: &[u8], cols: usize) -> Vec<u8> {
    let mut v = vec![b' '; 3usize.saturating_sub(cols)];
    v.extend_from_slice(text);
    v
}

/* The cells of the characters of the input for -t c: a character that is cut by the end of a block goes on in the next. */
struct Chars {
    carry: std::collections::VecDeque<Vec<u8>>,
}

impl Chars {
    /* The cells of the first n bytes of data, which goes on past n with bytes of the same input. */
    fn cells(&mut self, data: &[u8], n: usize) -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = Vec::with_capacity(n);
        while out.len() < n {
            if let Some(c) = self.carry.pop_front() {
                out.push(c);
                continue;
            }
            let rest = &data[out.len()..];
            let mut cs = char_cells(rest);
            while out.len() < n && !cs.is_empty() {
                out.push(cs.remove(0));
            }
            self.carry.extend(cs);
        }
        out
    }
}

/* The cells for the first character of s (not empty), one for each byte of it. */
fn char_cells(s: &[u8]) -> Vec<Vec<u8>> {
    let octal = |bytes: &[u8]| -> Vec<Vec<u8>> { bytes.iter().map(|b| format!("{b:03o}").into_bytes()).collect() };
    let (len, wc) = mb::char_at(s);
    let Some(wc) = wc else { return octal(&s[..1]) };
    if len == 1 {
        let esc: Option<&[u8]> = match s[0] {
            0 => Some(b"\\0"),
            7 => Some(b"\\a"),
            8 => Some(b"\\b"),
            9 => Some(b"\\t"),
            10 => Some(b"\\n"),
            11 => Some(b"\\v"),
            12 => Some(b"\\f"),
            13 => Some(b"\\r"),
            b'\\' => Some(b"\\"),
            _ => None,
        };
        if let Some(e) = esc {
            return vec![pad(e, e.len())];
        }
    }
    if !mb::is_print(wc) {
        return octal(&s[..len]);
    }
    let cols = mb::width(wc).max(1) as usize;
    let mut cells = vec![pad(&s[..len], cols)];
    for _ in 1..len {
        cells.push(b" **".to_vec());
    }
    cells
}

/* the integer of an item of `size` bytes, zero-extended, in the byte order of memory */
fn unsigned_of(b: &[u8], size: usize) -> u64 {
    let mut a = [0u8; 8];
    a[..b.len()].copy_from_slice(b);
    match size {
        1 => a[0] as u64,
        2 => u16::from_ne_bytes([a[0], a[1]]) as u64,
        4 => u32::from_ne_bytes([a[0], a[1], a[2], a[3]]) as u64,
        _ => u64::from_ne_bytes(a),
    }
}

fn signed_of(v: u64, size: usize) -> i64 {
    match size {
        1 => v as u8 as i8 as i64,
        2 => v as u16 as i16 as i64,
        4 => v as u32 as i32 as i64,
        _ => v as i64,
    }
}

fn real(v: f64, prec: usize) -> String {
    let fmt = std::ffi::CString::new(format!("%.{prec}g")).unwrap();
    let mut buf = [0u8; 64];
    let n = posix::cfmt::format_f64(&mut buf, &fmt, v);
    String::from_utf8_lossy(&buf[..n.clamp(0, 63) as usize]).into_owned()
}

/*
 * The fewest digits, from the digits a float or double always keeps (all of them
 * for a denormal number: from one), that read back as the same number.
 */
fn shortest(v: f64, single: bool) -> String {
    let (keep, most, least) = if single { (6, 9, f32::MIN_POSITIVE as f64) } else { (15, 17, f64::MIN_POSITIVE) };
    let from = if v.abs() < least { 1 } else { keep };
    for p in from..most {
        let s = real(v, p);
        let Ok(c) = std::ffi::CString::new(s.clone()) else { continue };
        let same = if single { posix::cfmt::parse_f32(&c) == v as f32 } else { posix::cfmt::parse_f64(&c).0 == v };
        if same {
            return s;
        }
    }
    real(v, most)
}

/* The text of one item for the numeric kinds; b holds the bytes that exist, the rest of the item is null. */
fn number(t: &Spec, b: &[u8]) -> String {
    let w = t.width();
    let mut full = [0u8; 16];
    full[..b.len()].copy_from_slice(b);
    match t.kind {
        Kind::Float if t.size == 4 => shortest(f32::from_ne_bytes([full[0], full[1], full[2], full[3]]) as f64, true),
        Kind::Float if t.size == 8 => {
            let mut a = [0u8; 8];
            a.copy_from_slice(&full[..8]);
            shortest(f64::from_ne_bytes(a), false)
        }
        Kind::Float => ldouble::format(&full[..ldouble::SIZE]),
        Kind::Octal => format!("{:0w$o}", unsigned_of(b, t.size)),
        Kind::Hex => format!("{:0w$x}", unsigned_of(b, t.size)),
        Kind::Unsigned => unsigned_of(b, t.size).to_string(),
        Kind::Signed => signed_of(unsigned_of(b, t.size), t.size).to_string(),
        Kind::Named | Kind::Char => unreachable!(),
    }
}

/* The lines of one input block, without the offset: one for each type. */
fn render(types: &[Spec], chars: &mut Chars, data: &[u8], n: usize) -> Vec<Vec<u8>> {
    let mut lines = Vec::with_capacity(types.len());
    let char_cells = if types.iter().any(|t| t.kind == Kind::Char) { chars.cells(data, n) } else { Vec::new() };
    for t in types {
        let mut line = Vec::new();
        let mut put = |cell: &[u8]| {
            line.push(b' ');
            line.extend_from_slice(cell);
        };
        match t.kind {
            Kind::Named => data[..n].iter().for_each(|&b| put(&named(b))),
            Kind::Char => char_cells.iter().for_each(|c| put(c)),
            _ => {
                let mut i = 0;
                while i < n {
                    let end = (i + t.size).min(n);
                    let s = number(t, &data[i..end]);
                    let mut cell = vec![b' '; t.width().saturating_sub(s.len())];
                    cell.extend_from_slice(s.as_bytes());
                    put(&cell);
                    i += t.size;
                }
            }
        }
        lines.push(line);
    }
    lines
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Radix {
    Octal,
    Decimal,
    Hex,
    None,
}

impl Radix {
    fn text(self, off: u64) -> String {
        match self {
            Radix::Octal => format!("{off:07o}"),
            Radix::Decimal => format!("{off:07}"),
            Radix::Hex => format!("{off:06x}"),
            Radix::None => String::new(),
        }
    }
}

/* The concatenation of the input files, read as far as it is asked for. */
struct Input {
    prog: Vec<u8>,
    names: Vec<Vec<u8>>,
    next: usize,
    cur: Option<posix::Input>,
    buf: Vec<u8>,
    pos: usize,
    /* bytes that may still be read in (-N), None for all */
    left: Option<u64>,
    ended: bool,
    failed: bool,
    /* read ahead in chunks; off while skipping, so that no more than the skipped bytes are read */
    greedy: bool,
    /* has any file been read (an empty one too)? */
    opened: bool,
}

impl Input {
    fn new(prog: &[u8], names: Vec<Vec<u8>>) -> Input {
        Input { prog: prog.to_vec(), names, next: 0, cur: None, buf: Vec::new(), pos: 0, left: None, ended: false, failed: false, greedy: true, opened: false }
    }

    fn data(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    fn consume(&mut self, n: usize) {
        self.pos += n;
        if self.pos >= CHUNK {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
    }

    /* Read until `want` bytes are there or the input is at its end; a read asks for no more than is needed. */
    fn fill(&mut self, want: usize) {
        while self.data().len() < want && !self.ended {
            let mut room = (want - self.data().len()).max(1);
            if self.left.is_none() && self.greedy {
                room = room.max(CHUNK);
            }
            if let Some(l) = self.left {
                if l == 0 {
                    self.ended = true;
                    return;
                }
                room = room.min(l.min(usize::MAX as u64) as usize);
            }
            let Some(f) = self.cur.as_mut() else {
                if !self.open_next() {
                    self.ended = true;
                }
                continue;
            };
            let old = self.buf.len();
            self.buf.resize(old + room, 0);
            match posix::read_block(f, &mut self.buf[old..]) {
                Ok(0) => {
                    self.buf.truncate(old);
                    self.cur = None;
                    self.opened = true;
                }
                Ok(n) => {
                    self.opened = true;
                    self.buf.truncate(old + n);
                    if let Some(l) = self.left.as_mut() {
                        *l -= n as u64;
                    }
                }
                Err(e) => {
                    self.buf.truncate(old);
                    let name = self.names[self.next - 1].clone();
                    posix::warn_path(&self.prog, &name, &posix::strerror(&e));
                    self.failed = true;
                    self.cur = None;
                }
            }
        }
    }

    /* The next file, reported and passed over if it cannot be opened; false if there is none. */
    fn open_next(&mut self) -> bool {
        while self.next < self.names.len() {
            let name = self.names[self.next].clone();
            self.next += 1;
            match posix::open_input(&name) {
                Ok(f) => {
                    self.cur = Some(f);
                    return true;
                }
                Err(e) => {
                    posix::warn_path(&self.prog, &name, &posix::strerror(&e));
                    self.failed = true;
                }
            }
        }
        false
    }

    /* Pass over n bytes; false if the input is shorter. Exactly n bytes are read. */
    fn skip(&mut self, mut n: u64) -> bool {
        self.greedy = false;
        while n > 0 {
            let step = n.min(CHUNK as u64) as usize;
            self.fill(step);
            let got = self.data().len().min(step);
            if got == 0 {
                return false;
            }
            self.consume(got);
            n -= got as u64;
        }
        self.greedy = true;
        true
    }
}

struct Out {
    prog: Vec<u8>,
    buf: Vec<u8>,
}

impl Out {
    fn put(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
        if self.buf.len() >= CHUNK {
            self.flush();
        }
    }

    fn flush(&mut self) {
        posix::write_stdout(&self.prog, &self.buf);
        self.buf.clear();
    }
}

struct Opts {
    types: Vec<Spec>,
    radix: Radix,
    verbose: bool,
    skip: u64,
    count: Option<u64>,
}

/* Dump the input. Returns the status. */
fn dump(prog: &[u8], o: &Opts, files: Vec<Vec<u8>>) -> i32 {
    let mut input = Input::new(prog, if files.is_empty() { vec![b"-".to_vec()] } else { files });
    if !input.skip(o.skip) {
        posix::warn(prog, "cannot skip past end of combined input");
        return 1;
    }
    input.left = o.count;
    let block = block_size(&o.types);
    let look = if o.types.iter().any(|t| t.kind == Kind::Char) { LOOKAHEAD } else { 0 };
    let mut chars = Chars { carry: std::collections::VecDeque::new() };
    let mut out = Out { prog: prog.to_vec(), buf: Vec::new() };
    let mut off = o.skip;
    let mut prev: Option<Vec<Vec<u8>>> = None;
    let mut starred = false;
    loop {
        input.fill(block + look);
        let n = input.data().len().min(block);
        if n == 0 {
            break;
        }
        let lines = render(&o.types, &mut chars, input.data(), n);
        if !o.verbose && prev.as_ref() == Some(&lines) {
            if !starred {
                out.put(b"*\n");
                starred = true;
            }
        } else {
            let blank = vec![b' '; o.radix.text(0).len()];
            for (i, l) in lines.iter().enumerate() {
                if i == 0 {
                    out.put(o.radix.text(off).as_bytes());
                } else {
                    out.put(&blank);
                }
                out.put(l);
                out.put(b"\n");
            }
            prev = Some(lines);
            starred = false;
        }
        input.consume(n);
        off += n as u64;
    }
    /* the offset after the last byte, unless no file could be read at all */
    if o.radix != Radix::None && (input.opened || !input.failed) {
        out.put(o.radix.text(off).as_bytes());
        out.put(b"\n");
    }
    out.flush();
    i32::from(input.failed)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut o = Opts { types: Vec::new(), radix: Radix::Octal, verbose: false, skip: 0, count: None };
    let mut explicit = false;
    let mut g = Getopt::new(&args, b"A:j:N:t:vbcdosx");
    for opt in g.by_ref() {
        match opt {
            Opt::Arg(b'A', a) => {
                o.radix = match a.as_slice() {
                    b"d" => Radix::Decimal,
                    b"o" => Radix::Octal,
                    b"x" => Radix::Hex,
                    b"n" => Radix::None,
                    _ => posix::fail(&prog, &format!("invalid address base '{}': it must be one of d, o, x, n", String::from_utf8_lossy(&a))),
                };
                explicit = true;
            }
            Opt::Arg(b'j', a) => {
                o.skip = parse_count(&a, true).unwrap_or_else(|| posix::fail(&prog, &format!("invalid skip '{}'", String::from_utf8_lossy(&a))));
                explicit = true;
            }
            Opt::Arg(b'N', a) => {
                o.count = Some(parse_count(&a, false).unwrap_or_else(|| posix::fail(&prog, &format!("invalid count '{}'", String::from_utf8_lossy(&a)))));
                explicit = true;
            }
            Opt::Arg(b't', a) => {
                if let Err(m) = parse_types(&a, &mut o.types) {
                    posix::fail(&prog, &m);
                }
                explicit = true;
            }
            Opt::Flag(b'v') => {
                o.verbose = true;
                explicit = true;
            }
            Opt::Flag(c) => {
                let t = match c {
                    b'b' => Spec { kind: Kind::Octal, size: 1 },
                    b'c' => Spec { kind: Kind::Char, size: 1 },
                    b'd' => Spec { kind: Kind::Unsigned, size: 2 },
                    b'o' => Spec { kind: Kind::Octal, size: 2 },
                    b's' => Spec { kind: Kind::Signed, size: 2 },
                    _ => Spec { kind: Kind::Hex, size: 2 },
                };
                o.types.push(t);
            }
            _ => posix::usage(SYNOPSIS),
        }
    }
    if o.types.is_empty() {
        o.types.push(Spec { kind: Kind::Octal, size: 2 });
    }
    let mut files: Vec<Vec<u8>> = g.operands().to_vec();
    /* the last operand is an offset: with no -A -j -N -t -v, a plus sign, or a digit when it follows a file; if it is no offset it is a file */
    if !explicit && !files.is_empty() && files.len() <= 2 {
        let last = files.last().unwrap();
        if last.first() == Some(&b'+') || (files.len() == 2 && last.first().is_some_and(u8::is_ascii_digit)) {
            if let Some(n) = parse_offset(last) {
                o.skip = n;
                files.pop();
            }
        }
    }
    std::process::exit(dump(&prog, &o, files));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(s: &str) -> Vec<Spec> {
        let mut v = Vec::new();
        parse_types(s.as_bytes(), &mut v).unwrap();
        v
    }

    fn sp(kind: Kind, size: usize) -> Spec {
        Spec { kind, size }
    }

    fn lines(t: &[Spec], data: &[u8]) -> Vec<String> {
        let mut c = Chars { carry: Default::default() };
        render(t, &mut c, data, data.len().min(block_size(t))).into_iter().map(|l| String::from_utf8_lossy(&l).into_owned()).collect()
    }

    #[test]
    fn type_strings() {
        assert_eq!(types("a"), vec![sp(Kind::Named, 1)]);
        assert_eq!(types("acdfoux").len(), 7);
        assert_eq!(types("d")[0], sp(Kind::Signed, INT));
        assert_eq!(types("f")[0], sp(Kind::Float, 8));
        assert_eq!(types("dC dS dI dL".replace(' ', "").as_str()), vec![sp(Kind::Signed, 1), sp(Kind::Signed, 2), sp(Kind::Signed, 4), sp(Kind::Signed, LONG)]);
        assert_eq!(types("o1x2u4d8"), vec![sp(Kind::Octal, 1), sp(Kind::Hex, 2), sp(Kind::Unsigned, 4), sp(Kind::Signed, 8)]);
        assert_eq!(types("fFfDf4f8"), vec![sp(Kind::Float, 4), sp(Kind::Float, 8), sp(Kind::Float, 4), sp(Kind::Float, 8)]);
        assert_eq!(types("fL")[0], sp(Kind::Float, ldouble::SIZE));
        /* the same as separate options */
        assert_eq!(types("xuofdca"), "xuofdca".chars().flat_map(|c| types(&c.to_string())).collect::<Vec<_>>());
        for bad in ["z", "d3", "d0", "d16", "f1", "f2", "a1", "c2", "dF", "fC", "d1 2", "-", "o64"] {
            let mut v = Vec::new();
            assert!(parse_types(bad.as_bytes(), &mut v).is_err(), "{bad}");
        }
    }

    #[test]
    fn counts() {
        assert_eq!(parse_count(b"10", true), Some(10));
        assert_eq!(parse_count(b"010", true), Some(8));
        assert_eq!(parse_count(b"0x10", true), Some(16));
        assert_eq!(parse_count(b"0X1f", true), Some(31));
        assert_eq!(parse_count(b"0", true), Some(0));
        assert_eq!(parse_count(b"2b", true), Some(1024));
        assert_eq!(parse_count(b"2k", true), Some(2048));
        assert_eq!(parse_count(b"1m", true), Some(1 << 20));
        assert_eq!(parse_count(b"010k", true), Some(8192));
        /* after hexadecimal digits a b is a digit */
        assert_eq!(parse_count(b"0x1b", true), Some(27));
        assert_eq!(parse_count(b"2b", false), None);
        for bad in ["", "-1", "+1", "08", "0x", "0xg", "1.5", " 1", "1 ", "b", "18446744073709551616", "k"] {
            assert_eq!(parse_count(bad.as_bytes(), true), None, "{bad:?}");
        }
        assert_eq!(parse_count(b"18446744073709551615", true), Some(u64::MAX));
    }

    #[test]
    fn offset_operands() {
        assert_eq!(parse_offset(b"10"), Some(8));
        assert_eq!(parse_offset(b"+10"), Some(8));
        assert_eq!(parse_offset(b"10."), Some(10));
        assert_eq!(parse_offset(b"+10."), Some(10));
        assert_eq!(parse_offset(b"1b"), Some(512));
        assert_eq!(parse_offset(b"10.b"), Some(5120));
        assert_eq!(parse_offset(b"+0"), Some(0));
        for bad in ["", "+", "8", "1x", "b", ".", "-1"] {
            assert_eq!(parse_offset(bad.as_bytes()), None, "{bad:?}");
        }
    }

    #[test]
    fn blocks() {
        assert_eq!(block_size(&types("o2")), 16);
        assert_eq!(block_size(&types("o1")), 16);
        assert_eq!(block_size(&types("o8")), 16);
        assert_eq!(block_size(&types("o2x4")), 16);
        assert_eq!(block_size(&types("c")), 16);
    }

    #[test]
    fn named_characters() {
        let all: Vec<u8> = (0u8..=127).collect();
        let t = types("a");
        let mut text = String::new();
        for chunk in all.chunks(16) {
            text.push_str(&lines(&t, chunk)[0]);
            text.push('\n');
        }
        /* the table of the standard, with nl for line feed; the rest as themselves */
        assert!(text.starts_with(" nul soh stx etx eot enq ack bel  bs  ht  nl  vt  ff  cr  so  si\n dle dc1 dc2 dc3 dc4 nak syn etb can  em sub esc  fs  gs  rs  us\n  sp   !   \"   #   $   %   &   '   (   )   *   +   ,   -   .   /\n"), "{text}");
        assert!(text.ends_with("   p   q   r   s   t   u   v   w   x   y   z   {   |   }   ~ del\n"), "{text}");
        /* only the low seven bits count */
        assert_eq!(lines(&t, &[0xe1, 0x80, 0xff]), vec!["   a nul del"]);
    }

    #[test]
    fn characters() {
        let t = types("c");
        assert_eq!(lines(&t, b"a\\\0\x07\x08\x0c\n\r\t\x0b\x01\x7f"), vec!["   a   \\  \\0  \\a  \\b  \\f  \\n  \\r  \\t  \\v 001 177"]);
        /* a byte that is no character of the locale: its octal value */
        assert_eq!(lines(&t, &[0xff])[0], " 377");
    }

    #[test]
    fn integers() {
        let d = 0x0102_0304_0506_0708u64.to_ne_bytes();
        assert_eq!(lines(&types("o1"), &d[..2])[0], " ".to_string() + &format!("{:03o} {:03o}", d[0], d[1]));
        assert_eq!(lines(&types("x1"), &[0xab, 0x01])[0], " ab 01");
        assert_eq!(lines(&types("u1"), &[255, 0])[0], " 255   0");
        assert_eq!(lines(&types("d1"), &[255, 128, 127])[0], "   -1 -128  127");
        assert_eq!(lines(&types("d2"), &(-5i16).to_ne_bytes())[0], "     -5");
        assert_eq!(lines(&types("u2"), &65535u16.to_ne_bytes())[0], " 65535");
        assert_eq!(lines(&types("o2"), &0o177777u16.to_ne_bytes())[0], " 177777");
        assert_eq!(lines(&types("x4"), &0xdead_beefu32.to_ne_bytes())[0], " deadbeef");
        assert_eq!(lines(&types("d4"), &i32::MIN.to_ne_bytes())[0], " -2147483648");
        assert_eq!(lines(&types("u4"), &u32::MAX.to_ne_bytes())[0], " 4294967295");
        assert_eq!(lines(&types("o4"), &u32::MAX.to_ne_bytes())[0], " 37777777777");
        assert_eq!(lines(&types("d8"), &i64::MIN.to_ne_bytes())[0], " -9223372036854775808");
        assert_eq!(lines(&types("u8"), &u64::MAX.to_ne_bytes())[0], " 18446744073709551615");
        assert_eq!(lines(&types("o8"), &u64::MAX.to_ne_bytes())[0], " 1777777777777777777777");
        assert_eq!(lines(&types("x8"), &u64::MAX.to_ne_bytes())[0], " ffffffffffffffff");
    }

    #[test]
    fn a_short_end_is_filled_with_nulls() {
        /* nine bytes with 8-byte items: the second item has one byte and seven nulls */
        let mut d = 0x0807_0605_0403_0201u64.to_ne_bytes().to_vec();
        d.push(0x09);
        let l = lines(&types("x8"), &d);
        assert_eq!(l[0], format!(" {:016x} {:016x}", u64::from_ne_bytes(d[..8].try_into().unwrap()), 9u64));
        /* three bytes with 2-byte items: two items */
        assert_eq!(lines(&types("x2"), &[1, 2, 3]).len(), 1);
        assert_eq!(lines(&types("x2"), &[1, 2, 3])[0].split(' ').count(), 3);
    }

    #[test]
    fn floats() {
        assert_eq!(lines(&types("f4"), &1.5f32.to_ne_bytes())[0], format!("{:>16}", "1.5"));
        assert_eq!(lines(&types("f8"), &0.1f64.to_ne_bytes())[0], format!("{:>25}", "0.1"));
        assert_eq!(lines(&types("f8"), &(0.1f64 + 0.2).to_ne_bytes())[0], format!("{:>25}", "0.30000000000000004"));
        assert_eq!(lines(&types("f4"), &0.1f32.to_ne_bytes())[0], format!("{:>16}", "0.1"));
        assert_eq!(lines(&types("f8"), &5e-324f64.to_ne_bytes())[0], format!("{:>25}", "5e-324"));
        assert_eq!(lines(&types("f"), &(-2.5f64).to_ne_bytes())[0], format!("{:>25}", "-2.5"));
        assert_eq!(lines(&types("f4"), &f32::INFINITY.to_ne_bytes())[0], format!("{:>16}", "inf"));
        assert_eq!(lines(&types("f8"), &f64::NAN.to_ne_bytes())[0].trim(), "nan");
        assert_eq!(real(1e22, 17), "1e+22");
        assert_eq!(shortest(16777216.0, true), "16777216");
    }

    #[test]
    fn several_types_give_a_line_each() {
        let l = lines(&types("o1x1"), &[1, 255]);
        assert_eq!(l, vec![" 001 377", " 01 ff"]);
    }

    #[test]
    fn a_character_cut_by_a_block_goes_on_in_the_next() {
        /* in the C locale the two bytes are two octal numbers, in a UTF-8 one a character and "**": two cells either way */
        let mut c = Chars { carry: Default::default() };
        let data = "\u{e9}".as_bytes();
        assert_eq!(c.cells(data, 1).len(), 1);
        assert_eq!(c.cells(&data[1..], 1).len(), 1);
        assert!(c.carry.is_empty());
    }
}
