/* SPDX-License-Identifier: GPL-2.0-only */
/* join - relational database operator. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use std::io::{self, BufRead, BufReader, Write};

const SYNOPSIS: &str =
    "join [-a file_number|-v file_number] [-e string] [-o list] [-t char] [-1 field] [-2 field] file1 file2";
const BLOCK: usize = 65536;

/* One element of the -o list. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    /* 0: the join field */
    Join,
    /* file_number.field: the file (0 or 1) and the field (counted from 0) */
    Field(usize, usize),
}

#[derive(Debug, Default)]
struct Opts {
    /* -a 1, -a 2: also write the lines of that file that have no partner */
    unpaired: [bool; 2],
    /* -v 1, -v 2: write only the lines of that file that have no partner */
    only: [bool; 2],
    /* -e: what stands for an empty field of the -o list */
    empty: Vec<u8>,
    /* -o */
    list: Option<Vec<Item>>,
    /* -t: the one character that separates fields (None: runs of <blank>) */
    sep: Option<Vec<u8>>,
    /* -1, -2: the join field of each file, counted from 0 */
    key: [usize; 2],
}

enum Fail {
    /* which input (1 or 2) could not be read */
    Read(u8, io::Error),
    Write(io::Error),
}

/*
 * A decimal integer of one or more digits, at least 1, counted from 0. A
 * number too big for a usize is kept as the biggest one: no line has that
 * many fields, so the field is simply not there.
 */
fn number(s: &[u8]) -> Option<usize> {
    if s.is_empty() || !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut n: usize = 0;
    for &d in s {
        n = n.saturating_mul(10).saturating_add(usize::from(d - b'0'));
    }
    n.checked_sub(1)
}

/* A file_number: 1 or 2, as 0 or 1. */
fn file_number(s: &[u8]) -> Option<usize> {
    match number(s) {
        Some(n @ 0..=1) => Some(n),
        _ => None,
    }
}

/*
 * The -o list: elements are separated either by commas or by <blank>
 * characters, not by a mixture; an element is 0 or file_number.field.
 */
fn parse_list(s: &[u8]) -> Option<Vec<Item>> {
    let mut words: Vec<&[u8]> = Vec::new();
    if s.contains(&b',') {
        words.extend(s.split(|&c| c == b','));
    } else {
        let mut i = 0;
        let mut start = None;
        while i < s.len() {
            let (n, blank) = posix::mb::blank_at(&s[i..]);
            match (blank, start) {
                (true, Some(b)) => {
                    words.push(&s[b..i]);
                    start = None;
                }
                (false, None) => start = Some(i),
                _ => {}
            }
            i += n;
        }
        if let Some(b) = start {
            words.push(&s[b..]);
        }
    }
    if words.is_empty() {
        return None;
    }
    let mut list = Vec::new();
    for w in words {
        if w == b"0" {
            list.push(Item::Join);
            continue;
        }
        let dot = w.iter().position(|&c| c == b'.')?;
        let f = file_number(&w[..dot])?;
        let n = number(&w[dot + 1..])?;
        list.push(Item::Field(f, n));
    }
    Some(list)
}

/* The -t option-argument must be exactly one character. */
fn one_char(s: &[u8]) -> bool {
    !s.is_empty() && posix::mb::char_at(s).0 == s.len()
}

/* The options and the two operands; None is a usage error. */
fn parse(args: &[Vec<u8>]) -> Option<(Opts, Vec<Vec<u8>>)> {
    let mut o = Opts::default();
    let mut g = Getopt::new(args, b"a:v:e:o:t:1:2:");
    for opt in g.by_ref() {
        match opt {
            Opt::Arg(b'a', v) => o.unpaired[file_number(&v)?] = true,
            Opt::Arg(b'v', v) => o.only[file_number(&v)?] = true,
            Opt::Arg(b'e', v) => o.empty = v,
            Opt::Arg(b'o', v) => o.list.get_or_insert_with(Vec::new).extend(parse_list(&v)?),
            Opt::Arg(b't', v) => {
                if !one_char(&v) {
                    return None;
                }
                o.sep = Some(v);
            }
            Opt::Arg(b'1', v) => o.key[0] = number(&v)?,
            Opt::Arg(b'2', v) => o.key[1] = number(&v)?,
            _ => return None,
        }
    }
    /* the synopsis offers -a or -v, not both */
    if (o.unpaired[0] || o.unpaired[1]) && (o.only[0] || o.only[1]) {
        return None;
    }
    let files = g.operands();
    if files.len() != 2 {
        return None;
    }
    Some((o, files.to_vec()))
}

/*
 * The fields of a line as (start, end) offsets. By default they are
 * separated by runs of <blank> and leading blanks are ignored; with -t every
 * appearance of the character counts, so "a::b" has an empty second field
 * and a line without the character is a single field.
 */
fn split(text: &[u8], sep: Option<&[u8]>, out: &mut Vec<(usize, usize)>) {
    out.clear();
    let mut i = 0;
    match sep {
        None => loop {
            while i < text.len() {
                let (n, blank) = posix::mb::blank_at(&text[i..]);
                if !blank {
                    break;
                }
                i += n;
            }
            if i >= text.len() {
                break;
            }
            let start = i;
            while i < text.len() {
                let (n, blank) = posix::mb::blank_at(&text[i..]);
                if blank {
                    break;
                }
                i += n;
            }
            out.push((start, i));
        },
        Some(sep) => {
            let mut start = 0;
            while i < text.len() {
                let n = posix::mb::char_at(&text[i..]).0;
                if &text[i..i + n] == sep {
                    out.push((start, i));
                    start = i + n;
                }
                i += n;
            }
            out.push((start, text.len()));
        }
    }
}

struct Line {
    text: Vec<u8>,
    fields: Vec<(usize, usize)>,
    /* the join field; empty if the line is too short to have one */
    key: (usize, usize),
}

impl Line {
    fn key(&self) -> &[u8] {
        &self.text[self.key.0..self.key.1]
    }

    /* Field n (from 0), if the line has it. */
    fn field(&self, n: usize) -> Option<&[u8]> {
        self.fields.get(n).map(|&(a, b)| &self.text[a..b])
    }
}

/*
 * The order of two join fields: the collating sequence of the locale. Fields
 * that collate equally are the same field, identical or not (the spec's
 * APPLICATION USAGE).
 */
struct Order {
    c_locale: bool,
    sa: Vec<u8>,
    sb: Vec<u8>,
}

impl Order {
    fn new() -> Order {
        Order { c_locale: posix::mb::collation_is_c(), sa: Vec::new(), sb: Vec::new() }
    }

    fn cmp(&mut self, a: &[u8], b: &[u8]) -> std::cmp::Ordering {
        if a == b {
            return std::cmp::Ordering::Equal;
        }
        posix::mb::collate(a, b, self.c_locale, &mut self.sa, &mut self.sb)
    }
}

/* One input: its lines are read a group at a time, a group being the lines with equal join fields. */
struct Side<R> {
    r: R,
    which: u8,
    key: usize,
    ahead: Option<Line>,
}

impl<R: BufRead> Side<R> {
    fn new(r: R, which: u8, key: usize) -> Side<R> {
        Side { r, which, key, ahead: None }
    }

    /* The next line, without its newline; a last line with none is a line too. */
    fn line(&mut self, sep: Option<&[u8]>) -> Result<Option<Line>, Fail> {
        let mut text = Vec::new();
        if self.r.read_until(b'\n', &mut text).map_err(|e| Fail::Read(self.which, e))? == 0 {
            return Ok(None);
        }
        if text.last() == Some(&b'\n') {
            text.pop();
        }
        let mut fields = Vec::new();
        split(&text, sep, &mut fields);
        let key = fields.get(self.key).copied().unwrap_or((0, 0));
        Ok(Some(Line { text, fields, key }))
    }

    fn group(&mut self, sep: Option<&[u8]>, ord: &mut Order) -> Result<Vec<Line>, Fail> {
        let first = match self.ahead.take() {
            Some(l) => l,
            None => match self.line(sep)? {
                Some(l) => l,
                None => return Ok(Vec::new()),
            },
        };
        let mut group = vec![first];
        while let Some(l) = self.line(sep)? {
            if ord.cmp(group[0].key(), l.key()).is_eq() {
                group.push(l);
            } else {
                self.ahead = Some(l);
                break;
            }
        }
        Ok(group)
    }
}

/*
 * One output line for a pair of lines, or for a line without a partner (the
 * other is None). With -o only the listed fields are written, a missing or
 * empty one as the -e string. Otherwise: the join field, the other fields
 * of file 1, the other fields of file 2. Fields are separated by the -t
 * character or a space.
 */
fn emit<W: Write>(w: &mut W, o: &Opts, l1: Option<&Line>, l2: Option<&Line>) -> Result<(), Fail> {
    let sep: &[u8] = o.sep.as_deref().unwrap_or(b" ");
    let mut out: Vec<u8> = Vec::new();
    let key = l1.or(l2).map_or(&b""[..], Line::key);
    match &o.list {
        Some(list) => {
            for (i, item) in list.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(sep);
                }
                let text = match *item {
                    Item::Join => Some(key),
                    Item::Field(f, n) => [l1, l2][f].and_then(|l| l.field(n)),
                };
                match text {
                    Some(t) if !t.is_empty() => out.extend_from_slice(t),
                    _ => out.extend_from_slice(&o.empty),
                }
            }
        }
        None => {
            out.extend_from_slice(key);
            for (l, skip) in [(l1, o.key[0]), (l2, o.key[1])] {
                if let Some(l) = l {
                    for (i, &(a, b)) in l.fields.iter().enumerate() {
                        if i != skip {
                            out.extend_from_slice(sep);
                            out.extend_from_slice(&l.text[a..b]);
                        }
                    }
                }
            }
        }
    }
    out.push(b'\n');
    w.write_all(&out).map_err(Fail::Write)
}

/*
 * Merge the two inputs, both in the collating sequence of their join fields.
 * Lines whose fields collate equally are paired, every line of file 1 with
 * every line of file 2, in the order read. A line with no partner is written
 * for -a and -v; the paired lines are not written with -v.
 */
fn join<A: BufRead, B: BufRead, W: Write>(a: A, b: B, w: &mut W, o: &Opts) -> Result<(), Fail> {
    let sep = o.sep.as_deref();
    let mut s1 = Side::new(a, 1, o.key[0]);
    let mut s2 = Side::new(b, 2, o.key[1]);
    let mut ord = Order::new();
    let show_pairs = !(o.only[0] || o.only[1]);
    let show1 = o.unpaired[0] || o.only[0];
    let show2 = o.unpaired[1] || o.only[1];
    let mut g1 = s1.group(sep, &mut ord)?;
    let mut g2 = s2.group(sep, &mut ord)?;
    while !g1.is_empty() || !g2.is_empty() {
        let c = if g1.is_empty() {
            std::cmp::Ordering::Greater
        } else if g2.is_empty() {
            std::cmp::Ordering::Less
        } else {
            ord.cmp(g1[0].key(), g2[0].key())
        };
        match c {
            std::cmp::Ordering::Less => {
                if show1 {
                    for l in &g1 {
                        emit(w, o, Some(l), None)?;
                    }
                }
                g1 = s1.group(sep, &mut ord)?;
            }
            std::cmp::Ordering::Greater => {
                if show2 {
                    for l in &g2 {
                        emit(w, o, None, Some(l))?;
                    }
                }
                g2 = s2.group(sep, &mut ord)?;
            }
            std::cmp::Ordering::Equal => {
                if show_pairs {
                    for l1 in &g1 {
                        for l2 in &g2 {
                            emit(w, o, Some(l1), Some(l2))?;
                        }
                    }
                }
                g1 = s1.group(sep, &mut ord)?;
                g2 = s2.group(sep, &mut ord)?;
            }
        }
    }
    w.flush().map_err(Fail::Write)
}

fn open(prog: &[u8], name: &[u8]) -> BufReader<posix::Input> {
    match posix::open_input(name) {
        Ok(h) => BufReader::with_capacity(BLOCK, h),
        Err(e) => {
            posix::warn_path(prog, name, &posix::strerror(&e));
            std::process::exit(1);
        }
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let Some((o, files)) = parse(&args) else {
        posix::usage(SYNOPSIS);
    };
    let a = open(&prog, &files[0]);
    let b = open(&prog, &files[1]);
    let mut w = io::BufWriter::with_capacity(BLOCK, posix::Stdout::new());
    match join(a, b, &mut w, &o) {
        Ok(()) => {}
        Err(Fail::Read(n, e)) => {
            let name = String::from_utf8_lossy(&files[usize::from(n) - 1]).into_owned();
            posix::fail(&prog, &format!("{}: {}", name, posix::strerror(&e)))
        }
        Err(Fail::Write(e)) => posix::fail(&prog, &format!("write error: {}", posix::strerror(&e))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /* the locale is process-wide: tests that change it hold this */
    static LOCALE: Mutex<()> = Mutex::new(());

    fn argv(opts: &str) -> Vec<Vec<u8>> {
        let mut v = vec![b"join".to_vec()];
        v.extend(opts.split(' ').filter(|s| !s.is_empty()).map(|s| s.replace('_', " ").into_bytes()));
        v.push(b"f1".to_vec());
        v.push(b"f2".to_vec());
        v
    }

    /* the options are words split at spaces; a _ inside a word stands for a space; "TAB" is a tab */
    fn run_bytes(opts: &str, a: &[u8], b: &[u8]) -> Vec<u8> {
        let opts = opts.replace("TAB", "\t");
        let (o, _) = parse(&argv(&opts)).expect("usage error");
        let mut out = Vec::new();
        if join(io::Cursor::new(a), io::Cursor::new(b), &mut out, &o).is_err() {
            panic!("join failed");
        }
        out
    }

    fn run(opts: &str, a: &str, b: &str) -> String {
        String::from_utf8_lossy(&run_bytes(opts, a.as_bytes(), b.as_bytes())).into_owned()
    }

    fn usage_error(opts: &str) -> bool {
        parse(&argv(&opts.replace("TAB", "\t"))).is_none()
    }

    /* EXAMPLES: the phone and fax files, an outer join on tab-separated fields */
    #[test]
    fn example_phone_and_fax() {
        let phone = "Name\tPhone Number\nDon\t+1 123-456-7890\nHal\t+1 234-567-8901\nYasushi\t+2 345-678-9012\n";
        let fax = "Name\tFax Number\nDon\t+1 123-456-7899\nKeith\t+1 456-789-0122\nYasushi\t+2 345-678-9011\n";
        let want = "Name\tPhone Number\tFax Number\n\
                    Don\t+1 123-456-7890\t+1 123-456-7899\n\
                    Hal\t+1 234-567-8901\t(unknown)\n\
                    Keith\t(unknown)\t+1 456-789-0122\n\
                    Yasushi\t+2 345-678-9012\t+2 345-678-9011\n";
        assert_eq!(run("-t TAB -a 1 -a 2 -e (unknown) -o 0,1.2,2.2", phone, fax), want);
    }

    /* EXAMPLES: the same key more than once gives every combination */
    #[test]
    fn example_one_key_in_file_one_three_times() {
        assert_eq!(run("", "a x\na y\na z\n", "a p\n"), "a x p\na y p\na z p\n");
    }

    #[test]
    fn example_two_keys_each_three_times_in_file_two() {
        let want = "a b c w x\na b c y z\na b c o p\na d e w x\na d e y z\na d e o p\n";
        assert_eq!(run("", "a b c\na d e\n", "a w x\na y z\na o p\n"), want);
    }

    #[test]
    fn default_output_is_key_then_rest_of_each_file() {
        assert_eq!(run("", "k a b\n", "k c d\n"), "k a b c d\n");
        /* the join field moves to the front when it is not the first */
        assert_eq!(run("-1 2 -2 3", "x k y\n", "p q k r\n"), "k x y p q r\n");
        assert_eq!(run("-1 2", "x k\n", "k z\n"), "k x z\n");
        assert_eq!(run("-2 2", "k x\n", "z k\n"), "k x z\n");
    }

    #[test]
    fn blank_runs_are_one_separator_and_leading_blanks_are_ignored() {
        assert_eq!(run("", "  k \t a   b\n", "k\tc\n"), "k a b c\n");
    }

    #[test]
    fn only_the_join_field_in_a_line_leaves_nothing_after_it() {
        assert_eq!(run("", "k\n", "k\n"), "k\n");
        assert_eq!(run("", "k\n", "k x\n"), "k x\n");
        assert_eq!(run("", "k x\n", "k\n"), "k x\n");
    }

    #[test]
    fn unpaired_lines_with_a() {
        let (a, b) = ("1 a\n2 b\n4 d\n", "2 x\n3 y\n4 z\n");
        assert_eq!(run("", a, b), "2 b x\n4 d z\n");
        assert_eq!(run("-a 1", a, b), "1 a\n2 b x\n4 d z\n");
        assert_eq!(run("-a 2", a, b), "2 b x\n3 y\n4 d z\n");
        assert_eq!(run("-a 1 -a 2", a, b), "1 a\n2 b x\n3 y\n4 d z\n");
        assert_eq!(run("-a1 -a2", a, b), "1 a\n2 b x\n3 y\n4 d z\n");
    }

    #[test]
    fn unpaired_lines_with_v_write_nothing_else() {
        let (a, b) = ("1 a\n2 b\n4 d\n", "2 x\n3 y\n4 z\n");
        assert_eq!(run("-v 1", a, b), "1 a\n");
        assert_eq!(run("-v 2", a, b), "3 y\n");
        assert_eq!(run("-v 1 -v 2", a, b), "1 a\n3 y\n");
        assert_eq!(run("-v 1", "k a\n", "k b\n"), "");
    }

    #[test]
    fn unpaired_line_of_file_two_has_its_own_join_field_first() {
        assert_eq!(run("-a 2 -1 1 -2 2", "", "x k y\n"), "k x y\n");
        assert_eq!(run("-v 1 -1 2", "p k q\n", "z\n"), "k p q\n");
    }

    #[test]
    fn unpaired_lines_come_out_in_the_collating_sequence() {
        let out = run("-a 1 -a 2", "a 1\nc 1\ne 1\n", "b 2\nc 2\nd 2\nf 2\n");
        assert_eq!(out, "a 1\nb 2\nc 1 2\nd 2\ne 1\nf 2\n");
    }

    #[test]
    fn o_selects_fields_and_nothing_else() {
        let (a, b) = ("k a b\n", "k c d\n");
        assert_eq!(run("-o 1.2", a, b), "a\n");
        assert_eq!(run("-o 2.3,1.3", a, b), "d b\n");
        assert_eq!(run("-o 0", a, b), "k\n");
        assert_eq!(run("-o 2.2,0,1.2", "k a\n", "k b\n"), "b k a\n");
        assert_eq!(run("-o 1.1,1.1", a, b), "k k\n");
    }

    #[test]
    fn o_list_may_be_blank_separated() {
        assert_eq!(run("-o 0_1.2_2.2", "k a\n", "k b\n"), "k a b\n");
        assert_eq!(run("-o __0__2.2__", "k a\n", "k b\n"), "k b\n");
    }

    #[test]
    fn o_fields_that_are_missing_are_empty_or_the_e_string() {
        assert_eq!(run("-o 1.2,1.5,2.2", "k a\n", "k b\n"), "a  b\n");
        assert_eq!(run("-o 1.2,1.5,2.2 -e bozo", "k a\n", "k b\n"), "a bozo b\n");
        assert_eq!(run("-a 1 -o 0,1.2,2.2 -e ?", "k a\n", ""), "k a ?\n");
        assert_eq!(run("-a 2 -o 0,1.2,2.2 -e ?", "", "k b\n"), "k ? b\n");
    }

    #[test]
    fn e_replaces_empty_fields_that_are_there_too() {
        /* with -t a field can be present and empty */
        assert_eq!(run("-t : -o 1.2,1.3,2.2 -e X", "k::c\n", "k:b:\n"), "X:c:b\n");
        assert_eq!(run("-t : -o 1.2,2.3 -e X", "k::c\n", "k:b:\n"), "X:X\n");
    }

    #[test]
    fn e_does_nothing_without_o() {
        assert_eq!(run("-t : -e X", "k::c\n", "k:b:\n"), "k::c:b:\n");
        assert_eq!(run("-a 1 -e X", "k\n", ""), "k\n");
    }

    #[test]
    fn o_with_zero_uses_the_join_field_of_the_file_that_has_the_line() {
        assert_eq!(run("-a 1 -a 2 -o 0 -1 2 -2 1", "x b\n", "c y\n"), "b\nc\n");
    }

    #[test]
    fn o_works_with_unpaired_lines_in_v() {
        assert_eq!(run("-v 1 -o 1.2,2.2 -e -", "1 a\n2 b\n", "2 x\n"), "a -\n");
    }

    #[test]
    fn t_makes_every_separator_count() {
        assert_eq!(run("-t :", "a:b:c\n", "a:d:e\n"), "a:b:c:d:e\n");
        /* empty fields and blanks inside a field are fields */
        assert_eq!(run("-t :", "a::c\n", "a: x:\n"), "a::c: x:\n");
        /* a line without the character is one field */
        assert_eq!(run("-t :", "a b\n", "a b\n"), "a b\n");
        assert_eq!(run("-t :", "a b\n", "a\n"), "");
    }

    #[test]
    fn t_does_not_skip_leading_blanks_so_they_belong_to_the_key() {
        assert_eq!(run("-t :", " a:1\n", "a:2\n"), "");
        assert_eq!(run("-t :", " a:1\n", " a:2\n"), " a:1:2\n");
    }

    #[test]
    fn t_a_blank_as_the_separator_counts_each_one() {
        assert_eq!(run("-t_", "a  b\n", "a  c\n"), "a  b  c\n");
        /* two spaces make an empty field between them */
        assert_eq!(run("-t_ -o 1.2", "a  b\n", "a x\n"), "\n");
    }

    #[test]
    fn t_with_the_key_in_another_field() {
        assert_eq!(run("-t : -1 2 -2 1", "x:k:y\n", "k:z\n"), "k:x:y:z\n");
    }

    #[test]
    fn a_field_that_is_not_there_is_an_empty_key() {
        /* two empty keys are equal keys */
        assert_eq!(run("-1 3", "a b\n", "\n"), " a b\n");
        assert_eq!(run("-1 3 -a 1", "a b\n", ""), " a b\n");
        assert_eq!(run("-1 3", "a b\n", "x\n"), "");
        assert_eq!(run("-1 3 -2 2", "a b\n", "\n"), " a b\n");
        assert_eq!(run("-1 3 -2 3", "a b\n", "c d\n"), " a b c d\n");
    }

    #[test]
    fn blank_lines_are_lines_not_the_end_of_the_file() {
        /* RATIONALE: some historical joins stopped at a blank line */
        assert_eq!(run("-a 1 -a 2", "\n\nb 1\n", "\nb 2\n"), "\n\nb 1 2\n");
    }

    #[test]
    fn the_last_line_needs_no_newline() {
        assert_eq!(run("", "a 1\nb 2", "b 3\nc 4"), "b 2 3\n");
        assert_eq!(run("-a 1 -a 2", "a 1", "a 2"), "a 1 2\n");
        assert_eq!(run("-a 1", "a", ""), "a\n");
    }

    #[test]
    fn empty_inputs() {
        assert_eq!(run("", "", ""), "");
        assert_eq!(run("-a 1 -a 2", "", ""), "");
        assert_eq!(run("", "a\n", ""), "");
        assert_eq!(run("-a 2", "", "a 1\n"), "a 1\n");
    }

    #[test]
    fn nul_and_high_bytes_are_plain_characters() {
        assert_eq!(run_bytes("", b"a\0b x\n", b"a\0b y\n"), b"a\0b x y\n");
        assert_eq!(run_bytes("", b"\xff 1\n", b"\xff 2\n"), b"\xff 1 2\n");
    }

    #[test]
    fn unsorted_input_finishes_and_loses_no_pair_it_can_see() {
        /* unspecified by the standard; it must at least finish */
        let out = run("-a 1 -a 2", "b 1\na 1\n", "a 2\nb 2\n");
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn collation_decides_who_is_first() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        /* in the C locale 'B' < 'a' */
        assert_eq!(run("-a 1 -a 2", "B 1\n", "a 2\n"), "B 1\na 2\n");
        assert_eq!(run("-a 1 -a 2", "a 1\n", "B 2\n"), "B 2\na 1\n");
    }

    #[test]
    fn t_takes_a_whole_multibyte_character() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        if !posix::locale::try_set("en_US.UTF-8") {
            return;
        }
        /* the euro sign is three bytes; the same bytes inside another character do not split */
        let euro = "\u{20ac}";
        let opts = format!("-t {euro}");
        assert!(!usage_error(&opts));
        assert_eq!(run(&opts, &format!("k{euro}a\n"), &format!("k{euro}b\n")), format!("k{euro}a{euro}b\n"));
        /* two characters, or part of one, are no separator */
        assert!(usage_error(&format!("-t {euro}{euro}")));
        assert!(usage_error("-t ab"));
        posix::locale::set_c();
    }

    #[test]
    fn repeated_options_are_taken_in_order() {
        assert_eq!(run("-e a -e b -o 1.9", "k\n", "k\n"), "b\n");
        assert_eq!(run("-1 5 -1 1", "k\n", "k\n"), "k\n");
        /* a repeated -o adds to the list, in the order given */
        assert_eq!(run("-o 1.2 -o 2.2,0", "k a\n", "k b\n"), "a b k\n");
        assert_eq!(run("-t : -t ,", "a,b\n", "a,c\n"), "a,b,c\n");
    }

    #[test]
    fn options_attached_to_their_arguments() {
        assert_eq!(run("-a1 -ebozo -o1.3 -t: -11 -21", "k:a\n", "z:b\n"), "bozo\n");
        assert_eq!(run("-a1 -ebozo -o1.3 -t: -11 -21", "k:a\n", "k:b\n"), "bozo\n");
        assert_eq!(run("-a1 -ebozo -o1.3 -t: -11 -21", "k:a:c\n", "z:b\n"), "c\n");
    }

    #[test]
    fn usage_errors() {
        for bad in [
            "-a 3", "-a 0", "-a x", "-a", "-v 3", "-v", "-1 0", "-2 0", "-1 x", "-1", "-1 -1", "-2 1.5", "-o", "-o 3.1", "-o 0.1",
            "-o 1.0", "-o 1.", "-o .1", "-o 1.x", "-o ,", "-o 1.1,", "-o ,1.1", "-o 1.1,,2.1", "-o 1.1,_2.1", "-o _", "-o x",
            "-t ab", "-j 1", "-j1 1", "-x", "-a 1 -v 2", "-v 1 -a 2", "-a 1 -v 1", "-A 1", "-e",
        ] {
            assert!(usage_error(bad), "expected a usage error for {bad}");
        }
        /* an option at the end of the line with nothing after it */
        for last in ["-a", "-v", "-e", "-o", "-t", "-1", "-2"] {
            assert!(parse(&[b"join".to_vec(), last.as_bytes().to_vec()]).is_none(), "{last}");
        }
        /* the operands: exactly two */
        assert!(parse(&[b"join".to_vec(), b"f1".to_vec()]).is_none());
        assert!(parse(&[b"join".to_vec()]).is_none());
        assert!(parse(&[b"join".to_vec(), b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]).is_none());
    }

    #[test]
    fn operands_after_an_operand_are_operands() {
        /* "join f1 -t" and "join f1 --": the argument that starts with - is a file name */
        let a = |v: &[&str]| v.iter().map(|s| s.as_bytes().to_vec()).collect::<Vec<_>>();
        let (_, f) = parse(&a(&["join", "f1", "-t"])).unwrap();
        assert_eq!(f, a(&["f1", "-t"]));
        let (_, f) = parse(&a(&["join", "f1", "--"])).unwrap();
        assert_eq!(f, a(&["f1", "--"]));
        let (_, f) = parse(&a(&["join", "--", "-a", "-b"])).unwrap();
        assert_eq!(f, a(&["-a", "-b"]));
        let (_, f) = parse(&a(&["join", "-", "-"])).unwrap();
        assert_eq!(f, a(&["-", "-"]));
    }

    #[test]
    fn a_huge_field_number_is_a_field_that_is_not_there() {
        assert_eq!(run("-1 99999999999999999999999999 -a 1", "a b\n", ""), " a b\n");
        assert_eq!(run("-o 1.99999999999999999999999999 -e z", "k\n", "k\n"), "z\n");
    }

    #[test]
    fn many_equal_keys_give_all_combinations_in_order() {
        let mut a = String::new();
        let mut b = String::new();
        for i in 0..30 {
            a.push_str(&format!("k a{i}\n"));
            b.push_str(&format!("k b{i}\n"));
        }
        let out = run("", &a, &b);
        assert_eq!(out.lines().count(), 900);
        assert_eq!(out.lines().next(), Some("k a0 b0"));
        assert_eq!(out.lines().nth(1), Some("k a0 b1"));
        assert_eq!(out.lines().nth(30), Some("k a1 b0"));
        assert_eq!(out.lines().last(), Some("k a29 b29"));
    }

    #[test]
    fn many_lines_in_one_go() {
        let mut a = String::new();
        let mut b = String::new();
        for i in 0..50_000 {
            a.push_str(&format!("{:06} x\n", i * 2));
            b.push_str(&format!("{:06} y\n", i * 3));
        }
        let out = run("", &a, &b);
        /* the common keys: multiples of 6 below 100000 */
        assert_eq!(out.lines().count(), 16_667);
    }

    #[test]
    fn a_long_line_spans_many_blocks() {
        let long = "x".repeat(300_000);
        let a = format!("k {long}\n");
        let b = format!("k {long}y\n");
        let mut out = Vec::new();
        let ra = BufReader::with_capacity(1000, io::Cursor::new(a.as_bytes()));
        let rb = BufReader::with_capacity(1000, io::Cursor::new(b.as_bytes()));
        assert!(join(ra, rb, &mut out, &Opts::default()).is_ok());
        assert_eq!(out.len(), 1 + 1 + 300_000 + 1 + 300_001 + 1);
    }

    #[test]
    fn a_read_error_names_the_input() {
        struct Bad;
        impl io::Read for Bad {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from_raw_os_error(libc::EIO))
            }
        }
        let mut out = Vec::new();
        let ra = io::Cursor::new(&b"a\n"[..]);
        let rb = BufReader::new(Bad);
        match join(ra, rb, &mut out, &Opts::default()) {
            Err(Fail::Read(2, _)) => {}
            _ => panic!("expected a read error on input 2"),
        }
        let ra = BufReader::new(Bad);
        let rb = io::Cursor::new(&b"a\n"[..]);
        match join(ra, rb, &mut out, &Opts::default()) {
            Err(Fail::Read(1, _)) => {}
            _ => panic!("expected a read error on input 1"),
        }
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
        let r = join(io::Cursor::new(&b"a\n"[..]), io::Cursor::new(&b"a\n"[..]), &mut Bad, &Opts::default());
        assert!(matches!(r, Err(Fail::Write(_))));
    }
}
