/* SPDX-License-Identifier: GPL-2.0-only */
/* tr - translate characters. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::mb;
use posix::regex::Regex;
use posix::tabs::Fail;
use std::io::Read;
use std::collections::BTreeMap;
use std::io::{self, Write};

const SYNOPSIS: &str = "tr [-c|-C] [-s] string1 string2\n       tr -s [-c|-C] string1\n       tr -d [-c|-C] string1\n       tr -ds [-c|-C] string1 string2";

/* A character: its wide value, or RAW plus a byte that starts no character. */
type Ch = u32;
const RAW: Ch = 0x8000_0000;

/* Most characters a string can name (all of Unicode and the raw bytes). */
const CAP: u64 = 0x11_0000 + 256;

/* The character classes of string1 and string2. */
const CLASSES: [&str; 12] =
    ["alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space", "upper", "xdigit"];

/* A construct of a string operand. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Ch(Ch),
    /* both ends, and whether they are coded values (a \octal end) rather than characters */
    Range(u32, u32, bool),
    Class(&'static str),
    Equiv(Ch),
    /* [x*n]: n is 0 for "as many as it takes" */
    Repeat(Ch, u64),
}

/* Options in force. */
#[derive(Clone, Copy, Default)]
struct Opts {
    complement: bool,
    delete: bool,
    squeeze: bool,
}

/* The character of a byte that starts no character. */
fn byte_ch(b: u8, single: bool) -> Ch {
    if single {
        b as Ch
    } else {
        RAW | b as Ch
    }
}

/* The character a lone byte stands for (an octal escape, a range end). */
fn byte_value(b: u8, single: bool) -> Ch {
    match mb::char_at(&[b]) {
        (1, Some(wc)) if wc < RAW => wc,
        _ => byte_ch(b, single),
    }
}

/* The first character of s (not empty) and its length. */
fn decode(s: &[u8], single: bool) -> (usize, Ch) {
    match mb::char_at(s) {
        (n, Some(wc)) if wc < RAW => (n, wc),
        _ => (1, byte_ch(s[0], single)),
    }
}

/* Append the bytes of c; false if it has none in this locale. */
fn put(c: Ch, single: bool, out: &mut Vec<u8>) -> bool {
    if c >= RAW {
        out.push(c as u8);
        true
    } else if mb::encode(c, out) {
        true
    } else if single && c < 256 {
        out.push(c as u8);
        true
    } else {
        false
    }
}

/* Up to three octal digits at s[i..] (the first is one): the byte and the end. */
fn octal(s: &[u8], i: usize) -> Result<(u8, usize), String> {
    let mut v = 0u32;
    let mut j = i;
    while j < s.len() && j < i + 3 && (b'0'..=b'7').contains(&s[j]) {
        v = v * 8 + (s[j] - b'0') as u32;
        j += 1;
    }
    u8::try_from(v).map(|b| (b, j)).map_err(|_| "octal escape out of range".to_string())
}

/*
 * One character of a string at i, a plain one or a backslash escape: it, the
 * end of it, and the byte if it was an octal escape. With combine, octal
 * escapes that together spell a multibyte character make that one character.
 */
fn atom(s: &[u8], i: usize, combine: bool, single: bool) -> Result<(Ch, usize, Option<u8>), String> {
    if s[i] != b'\\' {
        let (n, c) = decode(&s[i..], single);
        return Ok((c, i + n, None));
    }
    let Some(&e) = s.get(i + 1) else {
        return Ok((b'\\' as Ch, i + 1, None));
    };
    let simple = |c: u8| Ok((c as Ch, i + 2, None));
    match e {
        b'0'..=b'7' => {
            let (b, j) = octal(s, i + 1)?;
            if combine && b >= 0x80 {
                /* the bytes of the escapes that follow, as far as they go */
                let (mut bytes, mut ends) = (vec![b], vec![j]);
                let mut k = j;
                while bytes.len() < 8 && s.get(k) == Some(&b'\\') && s.get(k + 1).is_some_and(|d| (b'0'..=b'7').contains(d)) {
                    let Ok((nb, nj)) = octal(s, k + 1) else { break };
                    bytes.push(nb);
                    ends.push(nj);
                    k = nj;
                }
                if let (n, Some(wc)) = mb::char_at(&bytes) {
                    if n >= 2 && wc < RAW {
                        return Ok((wc, ends[n - 1], None));
                    }
                }
            }
            Ok((byte_value(b, single), j, Some(b)))
        }
        b'\\' => simple(b'\\'),
        b'a' => simple(7),
        b'b' => simple(8),
        b'f' => simple(12),
        b'n' => simple(10),
        b'r' => simple(13),
        b't' => simple(9),
        b'v' => simple(11),
        _ => {
            /* any other character after a backslash stands for itself */
            let (n, c) = decode(&s[i + 1..], single);
            Ok((c, i + 1 + n, None))
        }
    }
}

/* Where needle first occurs in hay. */
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/* The count of a [x*n]: none is 0, a leading zero is octal. */
fn repeat_count(d: &[u8]) -> Result<u64, String> {
    if d.is_empty() {
        return Ok(0);
    }
    let radix = if d[0] == b'0' { 8 } else { 10 };
    let mut n = 0u64;
    for &c in d {
        let v = (c - b'0') as u64;
        if v >= radix {
            return Err("invalid repeat count".to_string());
        }
        n = n.saturating_mul(radix).saturating_add(v);
    }
    Ok(n)
}

/* A [:class:], [=equiv=] or [x*n] at i (a '['), or None if the bracket is only a character. */
fn bracket(s: &[u8], i: usize, single: bool) -> Result<Option<(Item, usize)>, String> {
    match s.get(i + 1) {
        Some(b':') => {
            let Some(p) = find(&s[i + 2..], b":]") else { return Ok(None) };
            let name = &s[i + 2..i + 2 + p];
            match CLASSES.iter().find(|c| c.as_bytes() == name) {
                Some(c) => Ok(Some((Item::Class(c), i + 2 + p + 2))),
                None => Err(format!("invalid character class `{}'", String::from_utf8_lossy(name))),
            }
        }
        Some(b'=') => {
            let Some(p) = find(&s[i + 2..], b"=]") else { return Ok(None) };
            let body = &s[i + 2..i + 2 + p];
            if !body.is_empty() {
                if let Ok((c, end, _)) = atom(body, 0, true, single) {
                    if end == body.len() {
                        return Ok(Some((Item::Equiv(c), i + 2 + p + 2)));
                    }
                }
            }
            Err(format!("invalid equivalence class `{}'", String::from_utf8_lossy(body)))
        }
        Some(_) => {
            let Ok((c, j, _)) = atom(s, i + 1, true, single) else { return Ok(None) };
            if s.get(j) != Some(&b'*') {
                return Ok(None);
            }
            let mut k = j + 1;
            while s.get(k).is_some_and(u8::is_ascii_digit) {
                k += 1;
            }
            if s.get(k) != Some(&b']') {
                return Ok(None);
            }
            Ok(Some((Item::Repeat(c, repeat_count(&s[j + 1..k])?), k + 1)))
        }
        None => Ok(None),
    }
}

/* A range from two ends. */
fn range(a: (Ch, Option<u8>), b: (Ch, Option<u8>)) -> Result<Item, String> {
    /* an octal end, or a byte that is no character, makes a range of coded values */
    let coded = a.1.is_some() || b.1.is_some() || a.0 >= RAW || b.0 >= RAW;
    let value = |e: (Ch, Option<u8>)| match e {
        (_, Some(byte)) => byte as u32,
        (c, None) if c >= RAW => c & 0xff,
        (c, None) => c,
    };
    let (lo, hi) = (value(a), value(b));
    if lo > hi {
        return Err("range endpoints are in reverse collating sequence order".to_string());
    }
    Ok(Item::Range(lo, hi, coded))
}

/* The constructs of a string operand. */
fn parse(s: &[u8], single: bool) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'[' {
            if let Some((item, next)) = bracket(s, i, single)? {
                items.push(item);
                i = next;
                continue;
            }
        }
        let (a, next, a_byte) = atom(s, i, false, single)?;
        if next + 1 < s.len() && s[next] == b'-' {
            let (b, end, b_byte) = atom(s, next + 1, false, single)?;
            items.push(range((a, a_byte), (b, b_byte))?);
            i = end;
            continue;
        }
        let (a, next, _) = atom(s, i, true, single)?;
        items.push(Item::Ch(a));
        i = next;
    }
    Ok(items)
}

/* Every character of the locale in ascending order, then the raw bytes. */
fn universe(single: bool) -> Box<dyn Iterator<Item = Ch>> {
    if single {
        Box::new(0..256)
    } else {
        Box::new((0..=0x10_FFFFu32).filter(|&c| mb::is_valid(c)).chain((0..256).map(|b| RAW | b)))
    }
}

/* The characters of a class in ascending order. */
fn class_members(name: &str, single: bool) -> Vec<Ch> {
    let test = mb::class_test(name.as_bytes()).unwrap_or(|_| false);
    let top = if single { 255 } else { 0x10_FFFF };
    (0..=top).filter(|&c| !(0xD800..0xE000).contains(&c) && test(c)).collect()
}

/* Every letter and digit of the locale as text, and where each starts: what an equivalence class is looked for in. */
struct Candidates {
    text: Vec<u8>,
    starts: Vec<(u32, Ch)>,
}

fn candidates(single: bool) -> Candidates {
    let mut cs = Candidates { text: Vec::new(), starts: Vec::new() };
    let top = if single { 255 } else { 0x10_FFFF };
    for c in 1..=top {
        if (0xD800..0xE000).contains(&c) || !mb::is_alnum(c) {
            continue;
        }
        let at = cs.text.len();
        if put(c, single, &mut cs.text) {
            cs.starts.push((at as u32, c));
        } else {
            cs.text.truncate(at);
        }
    }
    cs.text.push(0);
    cs
}

/*
 * The characters of the equivalence class of c: those that the regular expression [[=c=]] of the C library
 * matches (it knows LC_COLLATE). In the C locale a class has its one character.
 */
fn equiv_members(c: Ch, single: bool, cache: &mut Option<Candidates>) -> Vec<Ch> {
    let mut name = Vec::new();
    if c >= RAW || c == 0 || mb::collation_is_c() || !put(c, single, &mut name) || name.iter().any(|b| matches!(b, b'=' | b']' | 0)) {
        return vec![c];
    }
    let pat = [b"[[=".as_slice(), &name, b"=]]"].concat();
    let Some(re) = Regex::new(&posix::cstr(&pat), 0) else { return vec![c] };
    let cs = cache.get_or_insert_with(|| candidates(single));
    let mut found = vec![c];
    let mut from = 0;
    while let Some((so, eo)) = re.find_at(&cs.text, from) {
        if let Ok(i) = cs.starts.binary_search_by_key(&((from + so) as u32), |p| p.0) {
            found.push(cs.starts[i].1);
        }
        from += eo.max(so + 1);
        if from >= cs.text.len() {
            break;
        }
    }
    found.sort_unstable();
    found.dedup();
    found
}

/* The characters of a range in ascending order. */
fn range_members(lo: u32, hi: u32, coded: bool, single: bool) -> Vec<Ch> {
    if coded {
        (lo..=hi.min(255)).map(|b| byte_value(b as u8, single)).collect()
    } else {
        (lo..=hi).filter(|&c| mb::is_valid(c) || (single && c < 256)).collect()
    }
}

/* What string1 names: the characters in order, and where each class starts. */
struct First {
    chars: Vec<Ch>,
    /* (first position, length, class) */
    marks: Vec<(usize, usize, &'static str)>,
}

fn expand1(items: &[Item], single: bool, cache: &mut Option<Candidates>) -> Result<First, String> {
    let mut f = First { chars: Vec::new(), marks: Vec::new() };
    for it in items {
        match *it {
            Item::Ch(c) => f.chars.push(c),
            Item::Range(lo, hi, coded) => f.chars.extend(range_members(lo, hi, coded, single)),
            Item::Class(name) => {
                let m = class_members(name, single);
                f.marks.push((f.chars.len(), m.len(), name));
                f.chars.extend(m);
            }
            Item::Equiv(c) => f.chars.extend(equiv_members(c, single, cache)),
            Item::Repeat(c, n) => {
                if n == 0 || n > CAP {
                    return Err("invalid repeat count in string1".to_string());
                }
                f.chars.extend(std::iter::repeat_n(c, n as usize));
            }
        }
    }
    Ok(f)
}

/* A character of string2 repeated n times; n is 0 for "as many as it takes". */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    ch: Ch,
    n: u64,
}

/*
 * What string2 names. With first (a translation) a lower or upper class is
 * the case conversion of the class of string1 at the same position, and any
 * other class is an error; without it (-d -s) every class is a set.
 */
fn expand2(items: &[Item], single: bool, first: Option<&First>, cache: &mut Option<Candidates>) -> Result<Vec<Run>, String> {
    let mut runs: Vec<Run> = Vec::new();
    let mut pos: u64 = 0;
    let one = |runs: &mut Vec<Run>, pos: &mut u64, c: Ch| {
        runs.push(Run { ch: c, n: 1 });
        *pos += 1;
    };
    for it in items {
        match *it {
            Item::Ch(c) => one(&mut runs, &mut pos, c),
            Item::Range(lo, hi, coded) => {
                for c in range_members(lo, hi, coded, single) {
                    one(&mut runs, &mut pos, c);
                }
            }
            Item::Class(name) => match first {
                None => {
                    for c in class_members(name, single) {
                        one(&mut runs, &mut pos, c);
                    }
                }
                Some(f) => {
                    let (from, conv): (&str, fn(u32) -> u32) = match name {
                        "upper" => ("lower", mb::to_upper),
                        "lower" => ("upper", mb::to_lower),
                        _ => ("", |c| c),
                    };
                    let mark = f.marks.iter().find(|m| m.0 as u64 == pos && m.2 == from && !from.is_empty());
                    let Some(&(start, len, _)) = mark else {
                        return Err(format!("[:{name}:] in string2 needs the matching class in string1"));
                    };
                    for k in start..start + len {
                        one(&mut runs, &mut pos, conv(f.chars[k]));
                    }
                }
            },
            Item::Equiv(c) => {
                if first.is_some() {
                    return Err("an equivalence class is not allowed in string2".to_string());
                }
                for m in equiv_members(c, single, cache) {
                    one(&mut runs, &mut pos, m);
                }
            }
            Item::Repeat(c, n) => {
                runs.push(Run { ch: c, n });
                pos = pos.saturating_add(n);
            }
        }
    }
    Ok(runs)
}

/* A set of characters, or all the others. */
struct Set {
    low: [bool; 256],
    high: Vec<Ch>,
    not: bool,
}

impl Set {
    fn new(mut chars: Vec<Ch>, not: bool) -> Set {
        let mut low = [false; 256];
        chars.retain(|&c| {
            if c < 256 {
                low[c as usize] = true;
                false
            } else {
                true
            }
        });
        chars.sort_unstable();
        chars.dedup();
        Set { low, high: chars, not }
    }

    fn has(&self, c: Ch) -> bool {
        let inside = if c < 256 { self.low[c as usize] } else { self.high.binary_search(&c).is_ok() };
        inside != self.not
    }
}

/* What a character becomes. */
struct Map {
    low: [Option<Ch>; 256],
    high: Vec<(Ch, Ch)>,
    /* with -c: the characters outside this set become the character */
    rest: Option<(Set, Ch)>,
}

impl Map {
    fn get(&self, c: Ch) -> Ch {
        let found = if c < 256 {
            self.low[c as usize]
        } else {
            self.high.binary_search_by_key(&c, |p| p.0).ok().map(|i| self.high[i].1)
        };
        match (found, &self.rest) {
            (Some(t), _) => t,
            (None, Some((set, t))) if !set.has(c) => *t,
            _ => c,
        }
    }

    fn from_pairs(pairs: BTreeMap<Ch, Ch>, rest: Option<(Set, Ch)>) -> Map {
        let mut m = Map { low: [None; 256], high: Vec::new(), rest };
        for (from, to) in pairs {
            if from < 256 {
                m.low[from as usize] = Some(to);
            } else {
                m.high.push((from, to));
            }
        }
        m
    }
}

/* The first n characters of string2, the last one repeated if there are fewer. */
fn materialize(runs: &[Run], n: usize) -> Result<Vec<Ch>, String> {
    if runs.iter().filter(|r| r.n == 0).count() > 1 {
        return Err("only one [c*] may appear in string2".to_string());
    }
    let fixed = runs.iter().fold(0u64, |a, r| a.saturating_add(r.n));
    let fill = (n as u64).saturating_sub(fixed);
    let mut e: Vec<Ch> = Vec::with_capacity(n);
    for r in runs {
        let k = (if r.n == 0 { fill } else { r.n }).min((n - e.len()) as u64);
        e.extend(std::iter::repeat_n(r.ch, k as usize));
    }
    if e.len() < n {
        let last = *e.last().ok_or("string2 is empty")?;
        e.resize(n, last);
    }
    Ok(e)
}

/* The translation of -c: the complement of string1 against string2. */
fn complement_map(first: &[Ch], runs: &[Run], single: bool) -> Result<Map, String> {
    let fill = runs.iter().position(|r| r.n == 0);
    let (lead, tail) = match fill {
        Some(k) if k + 1 != runs.len() => return Err("string2 may not go on after [c*] with -c".to_string()),
        Some(k) => (&runs[..k], runs[k].ch),
        None => (runs, runs.last().ok_or("string2 is empty")?.ch),
    };
    let mut prefix: Vec<Ch> = Vec::new();
    for r in lead {
        let k = r.n.min(CAP - prefix.len() as u64);
        prefix.extend(std::iter::repeat_n(r.ch, k as usize));
    }
    let ex = Set::new(first.to_vec(), false);
    let mut pairs = BTreeMap::new();
    let mut next = 0;
    for c in universe(single) {
        if next == prefix.len() {
            break;
        }
        if !ex.has(c) {
            pairs.insert(c, prefix[next]);
            next += 1;
        }
    }
    Ok(Map::from_pairs(pairs, Some((ex, tail))))
}

/* Everything tr does to the characters. */
struct Job {
    single: bool,
    delete: Option<Set>,
    map: Option<Map>,
    squeeze: Option<Set>,
    /* the last character written, for squeezing */
    prev: Option<Ch>,
    /* the bytes of a translated character */
    buf: Vec<u8>,
    /* what an ASCII byte becomes: DELETE, SLOW (see step) or the byte */
    ascii: [u16; 128],
    /* the ASCII bytes that squeeze */
    squeezes: [bool; 128],
}

const DELETE: u16 = 0x100;
const SLOW: u16 = 0x200;

impl Job {
    fn new(o: Opts, s1: &[u8], s2: Option<&[u8]>) -> Result<Job, String> {
        let single = mb::is_single_byte();
        let mut cache = None;
        let first = expand1(&parse(s1, single)?, single, &mut cache)?;
        let mut job = Job { single, delete: None, map: None, squeeze: None, prev: None, buf: Vec::with_capacity(16), ascii: [SLOW; 128], squeezes: [false; 128] };
        let set_of = |runs: &[Run]| Set::new(runs.iter().map(|r| r.ch).collect(), false);
        match (o.delete, s2) {
            (true, None) => job.delete = Some(Set::new(first.chars, o.complement)),
            (true, Some(s2)) => {
                /* -d -s: delete string1, squeeze string2 */
                let runs = expand2(&parse(s2, single)?, single, None, &mut cache)?;
                job.squeeze = Some(set_of(&runs));
                job.delete = Some(Set::new(first.chars, o.complement));
            }
            (false, None) => job.squeeze = Some(Set::new(first.chars, o.complement)),
            (false, Some(s2)) => {
                /* with -c no class of string2 can match one of string1 */
                let none = First { chars: Vec::new(), marks: Vec::new() };
                let runs = expand2(&parse(s2, single)?, single, Some(if o.complement { &none } else { &first }), &mut cache)?;
                job.map = Some(if o.complement {
                    complement_map(&first.chars, &runs, single)?
                } else {
                    let e = materialize(&runs, first.chars.len())?;
                    Map::from_pairs(first.chars.iter().copied().zip(e).collect(), None)
                });
                if o.squeeze {
                    job.squeeze = Some(set_of(&runs));
                }
            }
        }
        job.build_ascii();
        Ok(job)
    }

    /* Fill the table of ASCII bytes, which are characters of one byte in every locale in use. */
    fn build_ascii(&mut self) {
        if !mb::ascii_is_identity() {
            return;
        }
        for c in 0..128u32 {
            let i = c as usize;
            self.squeezes[i] = self.squeeze.as_ref().is_some_and(|s| s.has(c));
            self.ascii[i] = if self.delete.as_ref().is_some_and(|d| d.has(c)) {
                DELETE
            } else {
                let t = self.map.as_ref().map_or(c, |m| m.get(c));
                if t < 128 {
                    t as u16
                } else {
                    SLOW
                }
            };
        }
    }

    /*
     * Run the characters of data, all of them if eof, else up to the last
     * complete one; the bytes written go to out. Returns how many bytes of
     * data were used.
     */
    fn feed(&mut self, data: &[u8], eof: bool, out: &mut Vec<u8>) -> io::Result<usize> {
        let mut i = 0;
        while i < data.len() {
            let b = data[i];
            if b < 0x80 {
                let a = self.ascii[b as usize];
                i += 1;
                if a == DELETE {
                    continue;
                }
                if a != SLOW {
                    let t = a as u8;
                    if self.prev == Some(t as Ch) && self.squeezes[t as usize] {
                        continue;
                    }
                    self.prev = Some(t as Ch);
                    out.push(t);
                    continue;
                }
                i -= 1;
            }
            let step = if eof { Some(mb::char_at(&data[i..])) } else { mb::char_at_stream(&data[i..]) };
            let Some((n, wc)) = step else { break };
            self.step(&data[i..i + n], wc, out)?;
            i += n;
        }
        Ok(i)
    }

    /* Copy r to w through the job; a read error and a write error are told apart. */
    fn run<R: Read, W: Write>(&mut self, r: &mut R, w: &mut W) -> Result<(), Fail> {
        let mut buf = vec![0u8; 65536];
        let mut out: Vec<u8> = Vec::with_capacity(70000);
        let mut have = 0;
        loop {
            let (n, eof) = match posix::read_block(r, &mut buf[have..]) {
                Ok(0) => (0, true),
                Ok(n) => (n, false),
                Err(e) => {
                    let _ = w.write_all(&out);
                    return Err(Fail::Read(e));
                }
            };
            have += n;
            let used = self.feed(&buf[..have], eof, &mut out).map_err(Fail::Write)?;
            w.write_all(&out).map_err(Fail::Write)?;
            out.clear();
            if eof {
                return Ok(());
            }
            buf.copy_within(used..have, 0);
            have -= used;
        }
    }

    /* One input character: its bytes and its wide value. */
    fn step<W: Write>(&mut self, bytes: &[u8], wc: Option<u32>, out: &mut W) -> io::Result<()> {
        let c = match wc {
            Some(w) if w < RAW => w,
            _ => byte_ch(bytes[0], self.single),
        };
        if self.delete.as_ref().is_some_and(|d| d.has(c)) {
            return Ok(());
        }
        let t = self.map.as_ref().map_or(c, |m| m.get(c));
        if self.prev == Some(t) && self.squeeze.as_ref().is_some_and(|s| s.has(t)) {
            return Ok(());
        }
        self.prev = Some(t);
        if t == c {
            return out.write_all(bytes);
        }
        self.buf.clear();
        if !put(t, self.single, &mut self.buf) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "a character with no encoding in this locale"));
        }
        out.write_all(&self.buf)
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut o = Opts::default();
    let mut g = Getopt::new(&args, b"cCds");
    for opt in &mut g {
        match opt {
            Opt::Flag(b'c' | b'C') => o.complement = true,
            Opt::Flag(b'd') => o.delete = true,
            Opt::Flag(b's') => o.squeeze = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let ops = g.operands();
    let ok = match (o.delete, o.squeeze) {
        (false, false) => ops.len() == 2,
        (false, true) => ops.len() == 1 || ops.len() == 2,
        (true, false) => ops.len() == 1,
        (true, true) => ops.len() == 2,
    };
    if !ok {
        posix::usage(SYNOPSIS);
    }
    let mut job = match Job::new(o, &ops[0], ops.get(1).map(|v| v.as_slice())) {
        Ok(j) => j,
        Err(m) => posix::fail(&prog, &m),
    };
    let mut input = match posix::open_input(b"-") {
        Ok(f) => f,
        Err(e) => posix::fail(&prog, &format!("standard input: {}", posix::strerror(&e))),
    };
    let mut out = posix::Stdout::new();
    match job.run(&mut input, &mut out) {
        Ok(()) => {}
        Err(Fail::Read(e)) => {
            posix::fail(&prog, &format!("standard input: {}", posix::strerror(&e)));
        }
        Err(Fail::Write(e)) if e.kind() == io::ErrorKind::InvalidData => posix::fail(&prog, &e.to_string()),
        Err(Fail::Write(e)) => posix::fail(&prog, &format!("write error: {}", posix::strerror(&e))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /* the locale is process-wide: every test holds this */
    static LOCALE: Mutex<()> = Mutex::new(());

    /* tr with the given options ("cCds") and operands on input, in the locale; None if it is missing */
    fn tr_in(locale: &str, opts: &str, ops: &[&str], input: &[u8]) -> Option<Result<Vec<u8>, String>> {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        if locale == "C" {
            posix::locale::set_c();
        } else if !posix::locale::try_set(locale) {
            return None;
        }
        let o = Opts { complement: opts.contains(['c', 'C']), delete: opts.contains('d'), squeeze: opts.contains('s') };
        let r = Job::new(o, ops[0].as_bytes(), ops.get(1).map(|s| s.as_bytes())).map(|mut job| {
            let mut out = Vec::new();
            let mut rd = input;
            job.run(&mut rd, &mut out).unwrap();
            out
        });
        posix::locale::set_c();
        Some(r)
    }

    fn tr(opts: &str, ops: &[&str], input: &str) -> String {
        String::from_utf8(tr_in("C", opts, ops, input.as_bytes()).unwrap().unwrap()).unwrap()
    }

    fn err(opts: &str, ops: &[&str]) -> bool {
        tr_in("C", opts, ops, b"").unwrap().is_err()
    }

    /* UTF-8 tests skip when the machine has no such locale */
    fn u8tr(opts: &str, ops: &[&str], input: &str) -> Option<String> {
        tr_in("en_US.UTF-8", opts, ops, input.as_bytes()).map(|r| String::from_utf8(r.unwrap()).unwrap())
    }

    /* the examples of the spec */

    #[test]
    fn example_words_one_per_line() {
        assert_eq!(tr("cs", &["[:alpha:]", "[\\n*]"], "ab, cd  e.\n"), "ab\ncd\ne\n");
    }

    #[test]
    fn example_lower_to_upper() {
        assert_eq!(tr("", &["[:lower:]", "[:upper:]"], "Hello, World 1\n"), "HELLO, WORLD 1\n");
    }

    #[test]
    fn example_equivalence_class_to_one_character() {
        assert_eq!(tr("", &["[=e=]", "[e*]"], "eel\n"), "eel\n");
        assert_eq!(tr("", &["[=e=]", "[x*]"], "eel\n"), "xxl\n");
    }

    #[test]
    fn example_digits_to_d() {
        assert_eq!(tr("", &["0123456789", "[d*]"], "a1b22\n"), "adbdd\n");
        /* the BSD padding is also what is done */
        assert_eq!(tr("", &["0123456789", "d"], "a1b22\n"), "adbdd\n");
    }

    #[test]
    fn example_strip_nul() {
        let r = tr_in("C", "d", &["\\000"], b"a\0b\0\0c").unwrap().unwrap();
        assert_eq!(r, b"abc");
    }

    /* translation */

    #[test]
    fn plain_translation() {
        assert_eq!(tr("", &["abc", "xyz"], "aabbccdd"), "xxyyzzdd");
        assert_eq!(tr("", &["a-c", "A-C"], "abcd"), "ABCd");
        assert_eq!(tr("", &["abc", "x"], "abcd"), "xxxd");
        assert_eq!(tr("", &["a", "xyz"], "abc"), "xbc");
        /* the last of a repeated character wins */
        assert_eq!(tr("", &["aa", "xy"], "ab"), "yb");
    }

    #[test]
    fn system_v_brackets_stay_characters() {
        assert_eq!(tr("", &["[a-z]", "[A-Z]"], "ab[]\n"), "AB[]\n");
    }

    #[test]
    fn repeat_constructs() {
        assert_eq!(tr("", &["abcd", "[x*2]yz"], "abcd"), "xxyz");
        assert_eq!(tr("", &["abcd", "[x*]yz"], "abcd"), "xxyz");
        assert_eq!(tr("", &["abcd", "z[x*0]"], "abcd"), "zxxx");
        assert_eq!(tr("", &["abc", "[x*010]"], "abc"), "xxx");
        assert_eq!(tr("", &["abcdefghij", "[x*012]"], "abcdefghij"), "xxxxxxxxxx");
        assert!(err("", &["abc", "[x*]y[z*]"]));
        assert!(err("", &["abc", "[x*08]"]));
    }

    #[test]
    fn escapes() {
        assert_eq!(tr("", &["\\n", "x"], "a\nb"), "axb");
        assert_eq!(tr("", &["\\t\\\\", "ab"], "\t\\"), "ab");
        assert_eq!(tr("", &["\\101", "x"], "ABA"), "xBx");
        /* the longest octal sequence of up to three digits */
        assert_eq!(tr("", &["\\1011", "xy"], "A1"), "xy");
        assert_eq!(tr("", &["\\0011", "xy"], "\x011"), "xy");
        assert_eq!(tr("", &["\\a\\b\\f\\r\\v", "12345"], "\x07\x08\x0c\r\x0b"), "12345");
        assert!(err("", &["\\400", "x"]));
        /* a backslash at the end, or before an ordinary character, is that character */
        assert_eq!(tr("", &["a\\", "xy"], "a\\"), "xy");
        assert_eq!(tr("", &["\\q", "x"], "q"), "x");
    }

    #[test]
    fn ranges() {
        assert_eq!(tr("", &["a-c", "[A*]"], "abcd"), "AAAd");
        /* a dash at the start or the end is a character */
        assert_eq!(tr("", &["-a", "xy"], "-a"), "xy");
        assert_eq!(tr("", &["a-", "xy"], "-a"), "yx");
        /* a-c, then a dash, then e */
        assert_eq!(tr("", &["a-c-e", "12345"], "abcde-"), "123d54");
        assert_eq!(tr("", &["\\000-\\037", "[.*]"], "a\x01\x1fb "), "a..b ");
        assert!(err("", &["z-a", "x"]));
    }

    #[test]
    fn classes() {
        assert_eq!(tr("", &["[:digit:]", "[#*]"], "a1b22"), "a#b##");
        assert_eq!(tr("", &["[:space:]", "[_*]"], "a b\tc\nd"), "a_b_c_d");
        assert_eq!(tr("", &["[:punct:]", "[.*]"], "a,b!"), "a.b.");
        assert_eq!(tr("", &["[:upper:]", "[:lower:]"], "ABC abc"), "abc abc");
    }

    #[test]
    fn case_conversion_needs_matching_classes() {
        assert!(err("", &["[:alpha:]", "[:upper:]"]));
        assert!(err("", &["[:lower:]", "[:lower:]"]));
        assert!(err("", &["[:lower:]", "[:alpha:]"]));
        assert!(err("", &["abc", "[:upper:]"]));
        assert!(err("c", &["[:lower:]", "[:upper:]"]));
        /* at the same position */
        assert_eq!(tr("", &["1[:lower:]", "2[:upper:]"], "1az"), "2AZ");
        assert!(err("", &["1[:lower:]", "[:upper:]"]));
        assert!(err("", &["[:lower:]1", "12[:upper:]"]));
    }

    #[test]
    fn bad_constructs() {
        assert!(err("", &["[:foo:]", "x"]));
        assert!(err("", &["[=ab=]", "x"]));
        assert!(err("", &["abc", "[=a=]"]));
        assert!(err("", &["abc", ""]));
        assert!(err("c", &["a", ""]));
        /* a bracket that is not a construct is characters */
        assert_eq!(tr("", &["[:alpha", "1234567"], "[:a"), "127");
        assert_eq!(tr("", &["[a*", "xyz"], "[a*"), "xyz");
        assert_eq!(tr("", &["[a*3b]", "abcdef"], "[a*3b]"), "abcdef");
    }

    #[test]
    fn empty_string1_changes_nothing() {
        assert_eq!(tr("", &["", "x"], "abc"), "abc");
        assert_eq!(tr("d", &[""], "abc"), "abc");
        assert_eq!(tr("s", &[""], "aabb"), "aabb");
    }

    #[test]
    fn repeat_in_string1_is_that_many() {
        assert_eq!(tr("", &["[a*3]b", "xyzw"], "aab"), "zzw");
        assert!(err("", &["[a*]", "x"]));
    }

    /* deletion and squeezing */

    #[test]
    fn delete() {
        assert_eq!(tr("d", &["abc"], "aXbYcZ"), "XYZ");
        assert_eq!(tr("d", &["[:digit:]"], "a1b2"), "ab");
        assert_eq!(tr("d", &["a-c"], "abcd"), "d");
    }

    #[test]
    fn delete_the_complement() {
        assert_eq!(tr("cd", &["a-c"], "xaybzc\n"), "abc");
        assert_eq!(tr("Cd", &["a-c\\n"], "xaybzc\n"), "abc\n");
        assert_eq!(tr("cd", &["a\\n"], "abba\n"), "aa\n");
    }

    #[test]
    fn squeeze() {
        assert_eq!(tr("s", &["a"], "aaabaa"), "aba");
        assert_eq!(tr("s", &["ab"], "aabbaabb"), "abab");
        assert_eq!(tr("s", &["[:space:]"], "a  b\t\tc"), "a b\tc");
        /* only characters of the operand */
        assert_eq!(tr("s", &["a"], "bbaa"), "bba");
        assert_eq!(tr("cs", &["a"], "aabbcc"), "aabc");
    }

    #[test]
    fn squeeze_after_translation() {
        assert_eq!(tr("s", &["ab", "xx"], "aabb"), "x");
        assert_eq!(tr("s", &["ab", "xy"], "aabb"), "xy");
        /* a repeat in the output of two different inputs */
        assert_eq!(tr("s", &["ab", "xx"], "abab"), "x");
        /* squeezing looks at string2 only */
        assert_eq!(tr("s", &["a", "x"], "aabb"), "xbb");
        assert_eq!(tr("s", &["a", "b"], "aabb"), "b");
    }

    #[test]
    fn squeeze_case_conversion_uses_the_second_characters() {
        assert_eq!(tr("s", &["[:upper:]", "[:lower:]"], "AAbbCC11"), "abc11");
        assert_eq!(tr("s", &["[:lower:]", "[:upper:]"], "aaBBcc11"), "ABC11");
    }

    #[test]
    fn delete_and_squeeze() {
        assert_eq!(tr("ds", &["[:digit:]", "[:alpha:]"], "00112233xx4455667788yy99zz"), "xyz");
        assert_eq!(tr("ds", &["b", "a"], "abaa"), "a");
        assert_eq!(tr("sd", &["b", "a"], "abaa"), "a");
        /* the characters between the deleted ones come together */
        assert_eq!(tr("ds", &["X", "a"], "aXa"), "a");
        assert_eq!(tr("ds", &["X", "b"], "aXa"), "aa");
    }

    #[test]
    fn complement_with_delete_and_squeeze() {
        /* string2 is not complemented */
        assert_eq!(tr("cds", &["a\\n", "x\\n"], "abbabba\n"), "aaa\n");
        assert_eq!(tr("cds", &["a\\n", "a\\n"], "abbabba\n"), "a\n");
    }

    #[test]
    fn complement_translation() {
        assert_eq!(tr("c", &["a", "x"], "abca"), "axxa");
        assert_eq!(tr("C", &["ab", "x"], "abcd"), "abxx");
        /* the complement in ascending order against string2 */
        assert_eq!(tr("c", &["\\000-\\140", "12"], "abc\x01 `"), "122\x01 `");
        assert_eq!(tr("cs", &["[:alpha:]", "\\n"], "ab,, cd"), "ab\ncd");
    }

    #[test]
    fn options_in_any_order_agree() {
        let a = tr("cds", &["a", "x"], "aaabbb");
        assert_eq!(a, tr("sdc", &["a", "x"], "aaabbb"));
        assert_eq!(a, tr("dcs", &["a", "x"], "aaabbb"));
    }

    #[test]
    fn nul_and_high_bytes_pass_through() {
        let r = tr_in("C", "", &["a", "b"], b"a\0\xff\x80a").unwrap().unwrap();
        assert_eq!(r, b"b\0\xff\x80b");
        let r = tr_in("C", "c", &["a", "b"], b"a\0\xff").unwrap().unwrap();
        assert_eq!(r, b"abb");
        let r = tr_in("C", "", &["\\377", "x"], b"a\xffb").unwrap().unwrap();
        assert_eq!(r, b"axb");
        let r = tr_in("C", "", &["\\200-\\377", "[x*]"], b"a\x80\xffb\x7f").unwrap().unwrap();
        assert_eq!(r, b"axxb\x7f");
    }

    /* multibyte characters */

    #[test]
    fn utf8_characters_are_one_unit() {
        let Some(r) = u8tr("", &["\u{e9}", "e"], "caf\u{e9}\n") else { return };
        assert_eq!(r, "cafe\n");
        assert_eq!(u8tr("", &["a\u{20ac}", "xy"], "a\u{20ac}\u{20ac}").unwrap(), "xyy");
        assert_eq!(u8tr("", &["abc", "\u{1f600}"], "abcd").unwrap(), "\u{1f600}\u{1f600}\u{1f600}d");
        assert_eq!(u8tr("d", &["\u{1f600}"], "a\u{1f600}b").unwrap(), "ab");
        assert_eq!(u8tr("s", &["\u{3042}"], "\u{3042}\u{3042}x").unwrap(), "\u{3042}x");
    }

    #[test]
    fn utf8_ranges_and_classes() {
        let Some(r) = u8tr("", &["\u{e0}-\u{ff}", "[?*]"], "a\u{e9}b") else { return };
        assert_eq!(r, "a?b");
        assert_eq!(u8tr("d", &["[:alpha:]"], "caf\u{e9} 1").unwrap(), " 1");
        assert_eq!(u8tr("", &["[:lower:]", "[:upper:]"], "caf\u{e9}").unwrap(), "CAF\u{c9}");
        assert_eq!(u8tr("", &["[:upper:]", "[:lower:]"], "CAF\u{c9}").unwrap(), "caf\u{e9}");
    }

    #[test]
    fn utf8_equivalence_class_takes_the_variants_of_the_locale() {
        let Some(r) = u8tr("", &["[=e=]", "x"], "e\u{e9}\u{e8}\u{ea}f") else { return };
        /* what the library's [[=e=]] says: at least the accented forms of e */
        assert_eq!(r, "xxxxf");
        assert_eq!(u8tr("d", &["[=a=]"], "a\u{e0}b").unwrap(), "b");
        /* the C locale: the character alone */
        assert_eq!(tr("", &["[=e=]", "x"], "e\u{e9}f"), "x\u{e9}f");
    }

    #[test]
    fn utf8_octal_escapes_spell_a_character() {
        let Some(r) = u8tr("", &["\\303\\251", "e"], "caf\u{e9}\n") else { return };
        assert_eq!(r, "cafe\n");
    }

    #[test]
    fn utf8_invalid_bytes_pass_through() {
        let Some(r) = tr_in("en_US.UTF-8", "", &["a", "b"], b"a\xffa\xc3") else { return };
        assert_eq!(r.unwrap(), b"b\xffb\xc3");
        let r = tr_in("en_US.UTF-8", "cd", &["a"], b"a\xffa").unwrap();
        assert_eq!(r.unwrap(), b"aa");
        let r = tr_in("en_US.UTF-8", "", &["\\377", "x"], b"a\xffa").unwrap();
        assert_eq!(r.unwrap(), b"axa");
    }

    #[test]
    fn utf8_complement() {
        let Some(r) = u8tr("c", &["a", "x"], "a\u{e9}\u{1f600}a") else { return };
        assert_eq!(r, "axxa");
        assert_eq!(u8tr("cd", &["\u{e9}"], "a\u{e9}b\u{e9}").unwrap(), "\u{e9}\u{e9}");
    }

    #[test]
    fn utf8_character_cut_by_a_read() {
        /* the euro sign in three bytes read one at a time */
        struct Slow<'a>(&'a [u8]);
        impl io::Read for Slow<'_> {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                b[0] = self.0[0];
                self.0 = &self.0[1..];
                Ok(1)
            }
        }
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        if !posix::locale::try_set("en_US.UTF-8") {
            return;
        }
        let mut job = Job::new(Opts::default(), "\u{20ac}".as_bytes(), Some(b"e")).unwrap();
        let mut out = Vec::new();
        job.run(&mut Slow("a\u{20ac}b".as_bytes()), &mut out).unwrap();
        posix::locale::set_c();
        assert_eq!(out, b"aeb");
    }
}
