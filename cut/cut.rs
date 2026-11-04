/* SPDX-License-Identifier: GPL-2.0-only */
/* cut - cut out selected fields of each line of a file. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::tabs::Fail;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};

const SYNOPSIS: &str = "cut -b list [-n] [file...]\n       cut -c list [file...]\n       cut -f list [-d delim] [-s] [file...]";

/* What the list counts. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Unit {
    Bytes,
    Chars,
    Fields,
}

/*
 * The list: positions counted from 1, as ranges that are sorted, do not
 * touch and do not overlap (so that every position is written once and in
 * the order of the input). An open end ("3-") is usize::MAX.
 */
#[derive(Debug, PartialEq, Eq)]
struct List(Vec<(usize, usize)>);

const OPEN: usize = usize::MAX;

/* A positive decimal number; one that is too big to count is as good as OPEN. */
fn number(s: &[u8]) -> Option<usize> {
    if s.is_empty() || !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let n = posix::parse_decimal(s).map(|n| usize::try_from(n).unwrap_or(OPEN)).unwrap_or(OPEN);
    if n == 0 {
        None
    } else {
        Some(n)
    }
}

/*
 * Elements separated by commas or blanks: N, N-M, N- or -M. None for
 * anything else: an empty element, a zero, a range that runs backwards.
 */
fn parse_list(arg: &[u8]) -> Option<List> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for el in arg.split(|&c| c == b',' || c == b' ' || c == b'\t') {
        let r = match el.iter().position(|&c| c == b'-') {
            None => {
                let n = number(el)?;
                (n, n)
            }
            Some(i) => {
                let (lo, hi) = (&el[..i], &el[i + 1..]);
                match (lo.is_empty(), hi.is_empty()) {
                    (true, true) => return None,
                    (true, false) => (1, number(hi)?),
                    (false, true) => (number(lo)?, OPEN),
                    (false, false) => {
                        let (l, h) = (number(lo)?, number(hi)?);
                        if l > h {
                            return None;
                        }
                        (l, h)
                    }
                }
            }
        };
        ranges.push(r);
    }
    Some(List(merge(ranges)))
}

/* Sort ranges and join those that overlap or touch. */
fn merge(mut v: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    v.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(v.len());
    for (lo, hi) in v {
        match out.last_mut() {
            Some(last) if lo <= last.1.saturating_add(1) => last.1 = last.1.max(hi),
            _ => out.push((lo, hi)),
        }
    }
    out
}

/*
 * Offsets in line where each character starts, and at the end the length,
 * so that character k (from 0) is line[starts[k]..starts[k + 1]].
 */
fn char_starts(line: &[u8], starts: &mut Vec<usize>) {
    starts.clear();
    let mut i = 0;
    while i < line.len() {
        starts.push(i);
        i += posix::mb::char_at(&line[i..]).0;
    }
    starts.push(line.len());
}

/* Is every byte of the line ASCII (so that bytes and characters are the same)? */
fn is_ascii(line: &[u8]) -> bool {
    line.is_ascii()
}

/* -b: the selected bytes. */
fn cut_bytes(line: &[u8], list: &List, out: &mut Vec<u8>) {
    for &(lo, hi) in &list.0 {
        if lo > line.len() {
            break;
        }
        out.extend_from_slice(&line[lo - 1..hi.min(line.len())]);
    }
}

/*
 * -b -n: the same, but a range never cuts a character in two. A start in
 * the middle of a character moves back to the start of it; an end that is
 * not the last byte of its character moves back to the end of the
 * character before; a range left with nothing is dropped.
 */
fn cut_bytes_whole(line: &[u8], list: &List, starts: &mut Vec<usize>, out: &mut Vec<u8>) {
    char_starts(line, starts);
    let len = line.len();
    /* the 0-based start of the character holding the 0-based byte i */
    let start_of = |starts: &[usize], i: usize| starts[starts.partition_point(|&s| s <= i) - 1];
    let mut ranges = Vec::new();
    for &(lo, hi) in &list.0 {
        if lo > len {
            break;
        }
        /* both ends as 1-based byte positions */
        let lo2 = start_of(starts, lo - 1) + 1;
        let hi2 = if hi >= len {
            len
        } else {
            let s = start_of(starts, hi - 1);
            let next = starts[starts.partition_point(|&x| x <= s)];
            if next == hi {
                hi
            } else {
                s
            }
        };
        if hi2 != 0 && lo2 <= hi2 {
            ranges.push((lo2, hi2));
        }
    }
    for (lo, hi) in merge(ranges) {
        out.extend_from_slice(&line[lo - 1..hi]);
    }
}

/* -c: the selected characters. */
fn cut_chars(line: &[u8], list: &List, starts: &mut Vec<usize>, out: &mut Vec<u8>) {
    if is_ascii(line) {
        return cut_bytes(line, list, out);
    }
    char_starts(line, starts);
    let n = starts.len() - 1;
    for &(lo, hi) in &list.0 {
        if lo > n {
            break;
        }
        out.extend_from_slice(&line[starts[lo - 1]..starts[hi.min(n)]]);
    }
}

/*
 * -f: the selected fields, joined by the delimiter. A line with no
 * delimiter is passed on whole (None: it is suppressed by -s).
 */
fn cut_fields(line: &[u8], list: &List, delim: &[u8], suppress: bool, out: &mut Vec<u8>) -> bool {
    /* where the fields end: the offsets of the delimiter characters */
    let mut cuts: Vec<usize> = Vec::new();
    if delim.len() == 1 && delim[0] < 0x80 && is_ascii(line) {
        let mut at = 0;
        while let Some(p) = posix::find_byte(delim[0], &line[at..]) {
            cuts.push(at + p);
            at += p + 1;
        }
    } else {
        let mut i = 0;
        while i < line.len() {
            let n = posix::mb::char_at(&line[i..]).0;
            if &line[i..i + n] == delim {
                cuts.push(i);
            }
            i += n;
        }
    }
    if cuts.is_empty() {
        if suppress {
            return false;
        }
        out.extend_from_slice(line);
        return true;
    }
    /* field k (from 1) is line[begin..end] */
    let mut first = true;
    let mut ri = 0;
    for k in 1..=cuts.len() + 1 {
        while ri < list.0.len() && list.0[ri].1 < k {
            ri += 1;
        }
        if ri == list.0.len() {
            break;
        }
        if list.0[ri].0 > k {
            continue;
        }
        let begin = if k == 1 { 0 } else { cuts[k - 2] + delim.len() };
        let end = if k == cuts.len() + 1 { line.len() } else { cuts[k - 1] };
        if !first {
            out.extend_from_slice(delim);
        }
        first = false;
        out.extend_from_slice(&line[begin..end]);
    }
    true
}

struct Job<'a> {
    unit: Unit,
    list: &'a List,
    whole: bool,
    delim: &'a [u8],
    suppress: bool,
}

/*
 * Every line of r, cut. A last line without a newline gets one. Output
 * that is waiting is flushed before a read that has to wait for input.
 */
fn cut_stream<R: Read, W: Write>(r: &mut BufReader<R>, j: &Job, out: &mut W) -> Result<(), Fail> {
    let mut line = Vec::new();
    let mut res = Vec::new();
    let mut starts = Vec::new();
    loop {
        if r.buffer().is_empty() {
            out.write_all(&res).map_err(Fail::Write)?;
            res.clear();
            out.flush().map_err(Fail::Write)?;
        }
        line.clear();
        if r.read_until(b'\n', &mut line).map_err(Fail::Read)? == 0 {
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let keep = match j.unit {
            Unit::Bytes if j.whole => {
                cut_bytes_whole(&line, j.list, &mut starts, &mut res);
                true
            }
            Unit::Bytes => {
                cut_bytes(&line, j.list, &mut res);
                true
            }
            Unit::Chars => {
                cut_chars(&line, j.list, &mut starts, &mut res);
                true
            }
            Unit::Fields => cut_fields(&line, j.list, j.delim, j.suppress, &mut res),
        };
        if keep {
            res.push(b'\n');
        }
        if res.len() >= 32768 {
            out.write_all(&res).map_err(Fail::Write)?;
            res.clear();
        }
    }
    out.write_all(&res).map_err(Fail::Write)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut unit: Option<Unit> = None;
    let mut list_arg: Option<Vec<u8>> = None;
    let (mut whole, mut suppress) = (false, false);
    let mut delim: Option<Vec<u8>> = None;
    let mut g = Getopt::new(&args, b"b:c:f:d:ns");
    for o in &mut g {
        match o {
            Opt::Arg(c @ (b'b' | b'c' | b'f'), v) => {
                if unit.is_some() {
                    posix::usage(SYNOPSIS);
                }
                unit = Some(match c {
                    b'b' => Unit::Bytes,
                    b'c' => Unit::Chars,
                    _ => Unit::Fields,
                });
                list_arg = Some(v);
            }
            Opt::Arg(b'd', v) => delim = Some(v),
            Opt::Flag(b'n') => whole = true,
            Opt::Flag(b's') => suppress = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let (Some(unit), Some(list_arg)) = (unit, list_arg) else {
        posix::usage(SYNOPSIS);
    };
    if (whole && unit != Unit::Bytes) || ((delim.is_some() || suppress) && unit != Unit::Fields) {
        posix::usage(SYNOPSIS);
    }
    let Some(list) = parse_list(&list_arg) else {
        posix::fail(&prog, "invalid list");
    };
    let delim = match delim {
        None => b"\t".to_vec(),
        Some(d) => {
            if d.is_empty() || posix::mb::char_at(&d).0 != d.len() {
                posix::fail(&prog, "the delimiter must be a single character");
            }
            d
        }
    };
    let job = Job { unit, list: &list, whole, delim: &delim, suppress };
    let files = g.operands();
    let names: Vec<&[u8]> = if files.is_empty() { vec![b"-".as_slice()] } else { files.iter().map(|f| f.as_slice()).collect() };
    let mut out = BufWriter::with_capacity(65536, posix::Stdout::new());
    let mut status = 0;
    for n in names {
        let res = match posix::open_input(n) {
            Ok(h) => cut_stream(&mut BufReader::with_capacity(65536, h), &job, &mut out),
            Err(e) => Err(Fail::Read(e)),
        };
        match res {
            Ok(()) => {}
            /* a write that fails (a closed pipe with SIGPIPE ignored, a full disk) ends cut */
            Err(Fail::Write(e)) => posix::fail(&prog, &format!("write error: {}", posix::strerror(&e))),
            /* a read error ends this file only */
            Err(Fail::Read(e)) => {
                let _ = out.flush();
                if n == b"-" && files.is_empty() {
                    posix::warn(&prog, &format!("standard input: {}", posix::strerror(&e)));
                } else {
                    posix::warn_path(&prog, n, &posix::strerror(&e));
                }
                status = 1;
            }
        }
    }
    if let Err(e) = out.flush() {
        posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /* the locale is process-wide: tests that change it hold this */
    static LOCALE: Mutex<()> = Mutex::new(());

    fn list(s: &str) -> List {
        parse_list(s.as_bytes()).unwrap()
    }

    fn b(l: &str, line: &str) -> String {
        let mut out = Vec::new();
        cut_bytes(line.as_bytes(), &list(l), &mut out);
        String::from_utf8(out).unwrap()
    }

    fn f(l: &str, delim: &str, s: bool, line: &str) -> Option<String> {
        let mut out = Vec::new();
        cut_fields(line.as_bytes(), &list(l), delim.as_bytes(), s, &mut out).then(|| String::from_utf8(out).unwrap())
    }

    #[test]
    fn list_syntax() {
        assert_eq!(list("1,3-5,7-").0, vec![(1, 1), (3, 5), (7, OPEN)]);
        assert_eq!(list("-3").0, vec![(1, 3)]);
        assert_eq!(list("3,1").0, vec![(1, 1), (3, 3)]);
        assert_eq!(list("1 3\t5").0, vec![(1, 1), (3, 3), (5, 5)]);
        /* overlapping and touching ranges become one */
        assert_eq!(list("1-3,2-5,6,9-").0, vec![(1, 6), (9, OPEN)]);
        assert_eq!(list("5-,2").0, vec![(2, 2), (5, OPEN)]);
        for bad in ["", ",", "1,", ",1", "1,,2", "0", "0-3", "3-1", "-", "1-2-3", "a", "1-a", "-0", " ", "1 ", "+1", "1.5"] {
            assert_eq!(parse_list(bad.as_bytes()), None, "{:?}", bad);
        }
        /* too big to count: as good as no end */
        assert_eq!(list("99999999999999999999999").0, vec![(OPEN, OPEN)]);
    }

    #[test]
    fn bytes() {
        assert_eq!(b("1,3", "abcdef"), "ac");
        assert_eq!(b("3,1", "abcdef"), "ac");
        assert_eq!(b("2-4", "abcdef"), "bcd");
        assert_eq!(b("-2", "abcdef"), "ab");
        assert_eq!(b("5-", "abcdef"), "ef");
        assert_eq!(b("1-3,2-5", "abcdefg"), "abcde");
        assert_eq!(b("9", "abc"), "");
        assert_eq!(b("2-9", "abc"), "bc");
        assert_eq!(b("1", ""), "");
    }

    #[test]
    fn fields() {
        assert_eq!(f("2", ":", false, "a:b:c").unwrap(), "b");
        assert_eq!(f("1,3", ":", false, "a:b:c").unwrap(), "a:c");
        assert_eq!(f("2-", ":", false, "a:b:c").unwrap(), "b:c");
        assert_eq!(f("-2", ":", false, "a:b:c").unwrap(), "a:b");
        assert_eq!(f("9", ":", false, "a:b:c").unwrap(), "");
        /* empty fields count */
        assert_eq!(f("2", ":", false, "a::c").unwrap(), "");
        assert_eq!(f("1-3", ":", false, "::").unwrap(), "::");
        assert_eq!(f("3", ":", false, ":x:").unwrap(), "");
        /* no delimiter: the whole line, or nothing with -s */
        assert_eq!(f("2", ":", false, "abc").unwrap(), "abc");
        assert_eq!(f("2", ":", true, "abc"), None);
        assert!(f("1", ":", true, "").is_none());
        assert_eq!(f("1", ":", true, "a:").unwrap(), "a");
    }

    #[test]
    fn whole_characters_in_utf8() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        if !posix::locale::try_set("en_US.UTF-8") {
            return;
        }
        /* bytes: e-acute 1-2, a 3, euro sign 4-6 */
        let line = "\u{e9}a\u{20ac}";
        let n = |l: &str| {
            let (mut out, mut st) = (Vec::new(), Vec::new());
            cut_bytes_whole(line.as_bytes(), &list(l), &mut st, &mut out);
            String::from_utf8(out).unwrap()
        };
        let c = |l: &str| {
            let (mut out, mut st) = (Vec::new(), Vec::new());
            cut_chars(line.as_bytes(), &list(l), &mut st, &mut out);
            String::from_utf8(out).unwrap()
        };
        assert_eq!(n("1"), "");
        assert_eq!(n("2"), "\u{e9}");
        assert_eq!(n("1-2"), "\u{e9}");
        assert_eq!(n("2-3"), "\u{e9}a");
        assert_eq!(n("3-4"), "a");
        assert_eq!(n("4-"), "\u{20ac}");
        assert_eq!(n("5-"), "\u{20ac}");
        assert_eq!(n("1-5"), "\u{e9}a");
        assert_eq!(n("3"), "a");
        assert_eq!(n("6"), "\u{20ac}");
        assert_eq!(n("9-"), "");
        assert_eq!(n("1,2"), "\u{e9}");
        assert_eq!(c("1"), "\u{e9}");
        assert_eq!(c("2-3"), "a\u{20ac}");
        assert_eq!(c("3,1"), "\u{e9}\u{20ac}");
        assert_eq!(c("4-"), "");
        /* a delimiter that is a character of more than one byte */
        let mut out = Vec::new();
        assert!(cut_fields("a\u{20ac}b\u{20ac}c".as_bytes(), &list("2"), "\u{20ac}".as_bytes(), false, &mut out));
        assert_eq!(out, b"b");
        posix::locale::set_c();
    }
}
