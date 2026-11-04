/* SPDX-License-Identifier: GPL-2.0-only */
/* comm - select or reject lines common to two files. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use std::cmp::Ordering;
use std::io::{self, BufRead, BufReader, Write};

const SYNOPSIS: &str = "comm [-123] file1 file2";
const BLOCK: usize = 65536;

#[derive(Clone, Copy, Default)]
struct Opts {
    /* -1, -2, -3: suppress the column of lines only in file1, only in file2, in both */
    hide1: bool,
    hide2: bool,
    hide3: bool,
}

enum Fail {
    /* which input (1 or 2) could not be read */
    Read(u8, io::Error),
    Write(io::Error),
}

/*
 * The order of two lines: the collating sequence of the locale, then, for
 * lines that collate equally without being identical, the bytes. Such lines
 * are different lines (the spec's "should"), and an input sorted by sort(1)
 * is sorted this way.
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

    fn cmp(&mut self, a: &[u8], b: &[u8]) -> Ordering {
        if a == b {
            return Ordering::Equal;
        }
        posix::mb::collate(a, b, self.c_locale, &mut self.sa, &mut self.sb).then_with(|| a.cmp(b))
    }
}

/* Read the next line of r into line, without its newline. False at the end of the input. */
fn next_line<R: BufRead>(r: &mut R, line: &mut Vec<u8>, which: u8) -> Result<bool, Fail> {
    line.clear();
    if r.read_until(b'\n', line).map_err(|e| Fail::Read(which, e))? == 0 {
        return Ok(false);
    }
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    Ok(true)
}

/* One output line: the lead, the text, a newline. */
fn put<W: Write>(w: &mut W, lead: &[u8], line: &[u8]) -> Result<(), Fail> {
    w.write_all(lead).and_then(|_| w.write_all(line)).and_then(|_| w.write_all(b"\n")).map_err(Fail::Write)
}

/*
 * Merge the two sorted inputs. Only the current line of each is kept, so
 * memory does not depend on the input. A last line without a newline is a
 * line like the others.
 */
fn comm<A: BufRead, B: BufRead, W: Write>(a: &mut A, b: &mut B, w: &mut W, o: &Opts) -> Result<(), Fail> {
    /* the lead of a line only in file2 and of a line in both: a tab for each column before it that is shown */
    let lead2: &[u8] = if o.hide1 { b"" } else { b"\t" };
    let lead3: &[u8] = match (o.hide1, o.hide2) {
        (false, false) => b"\t\t",
        (true, true) => b"",
        _ => b"\t",
    };
    let mut order = Order::new();
    let mut la: Vec<u8> = Vec::new();
    let mut lb: Vec<u8> = Vec::new();
    let mut ha = next_line(a, &mut la, 1)?;
    let mut hb = next_line(b, &mut lb, 2)?;
    while ha || hb {
        let ord = match (ha, hb) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => order.cmp(&la, &lb),
        };
        match ord {
            Ordering::Less => {
                if !o.hide1 {
                    put(w, b"", &la)?;
                }
                ha = next_line(a, &mut la, 1)?;
            }
            Ordering::Greater => {
                if !o.hide2 {
                    put(w, lead2, &lb)?;
                }
                hb = next_line(b, &mut lb, 2)?;
            }
            Ordering::Equal => {
                if !o.hide3 {
                    put(w, lead3, &la)?;
                }
                ha = next_line(a, &mut la, 1)?;
                hb = next_line(b, &mut lb, 2)?;
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
    let mut o = Opts::default();
    let mut g = Getopt::new(&args, b"123");
    for opt in g.by_ref() {
        match opt {
            Opt::Flag(b'1') => o.hide1 = true,
            Opt::Flag(b'2') => o.hide2 = true,
            Opt::Flag(b'3') => o.hide3 = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let files = g.operands();
    if files.len() != 2 {
        posix::usage(SYNOPSIS);
    }
    let mut a = open(&prog, &files[0]);
    let mut b = open(&prog, &files[1]);
    let mut w = io::BufWriter::with_capacity(BLOCK, posix::Stdout::new());
    match comm(&mut a, &mut b, &mut w, &o) {
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

    fn run_bytes(flags: &str, a: &[u8], b: &[u8]) -> Vec<u8> {
        let o = Opts { hide1: flags.contains('1'), hide2: flags.contains('2'), hide3: flags.contains('3') };
        let mut out = Vec::new();
        let mut ra = io::Cursor::new(a);
        let mut rb = io::Cursor::new(b);
        if comm(&mut ra, &mut rb, &mut out, &o).is_err() {
            panic!("comm failed");
        }
        out
    }

    fn run(flags: &str, a: &str, b: &str) -> String {
        String::from_utf8_lossy(&run_bytes(flags, a.as_bytes(), b.as_bytes())).into_owned()
    }

    #[test]
    fn three_columns() {
        assert_eq!(run("", "a\nb\nd\n", "b\nc\nd\ne\n"), "a\n\t\tb\n\tc\n\t\td\n\te\n");
    }

    #[test]
    fn suppressing_columns_moves_the_others_left() {
        let (a, b) = ("a\nb\nd\n", "b\nc\nd\ne\n");
        assert_eq!(run("1", a, b), "\tb\nc\n\td\ne\n");
        assert_eq!(run("2", a, b), "a\n\tb\n\td\n");
        assert_eq!(run("3", a, b), "a\n\tc\n\te\n");
        assert_eq!(run("12", a, b), "b\nd\n");
        assert_eq!(run("13", a, b), "c\ne\n");
        assert_eq!(run("23", a, b), "a\n");
        assert_eq!(run("123", a, b), "");
    }

    #[test]
    fn empty_inputs() {
        assert_eq!(run("", "", ""), "");
        assert_eq!(run("", "a\n", ""), "a\n");
        assert_eq!(run("", "", "a\n"), "\ta\n");
        assert_eq!(run("23", "", "a\n"), "");
    }

    #[test]
    fn repeated_lines_pair_off_one_for_one() {
        assert_eq!(run("", "a\na\na\n", "a\n"), "\t\ta\na\na\n");
        assert_eq!(run("", "a\n", "a\na\n"), "\t\ta\n\ta\n");
    }

    #[test]
    fn the_last_line_needs_no_newline() {
        assert_eq!(run("", "a\nb", "b\nc"), "a\n\t\tb\n\tc\n");
        assert_eq!(run("", "a", "a"), "\t\ta\n");
    }

    #[test]
    fn empty_lines_are_lines() {
        assert_eq!(run("", "\n\na\n", "\na\n"), "\t\t\n\n\t\ta\n");
    }

    #[test]
    fn nul_and_high_bytes_are_plain_characters() {
        assert_eq!(run_bytes("", b"a\0b\n\xff\n", b"a\0b\n\xff\n"), b"\t\ta\0b\n\t\t\xff\n");
        assert_eq!(run_bytes("", b"a\0b\n", b"a\0c\n"), b"a\0b\n\ta\0c\n");
    }

    #[test]
    fn unsorted_input_is_merged_as_it_comes() {
        /* unspecified by the standard; it must at least finish and lose no line */
        let out = run("", "b\na\n", "a\nb\n");
        assert_eq!(out.matches('\n').count(), 3);
    }

    #[test]
    fn collation_order_decides_who_is_first() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        /* in the C locale 'B' < 'a'; the lines are in that order */
        assert_eq!(run("", "B\n", "a\n"), "B\n\ta\n");
        assert_eq!(run("", "a\n", "B\n"), "\tB\na\n");
    }

    #[test]
    fn a_long_line_spans_many_blocks() {
        let long = "x".repeat(300_000);
        let a = format!("{long}\n");
        let b = format!("{long}\n{long}y\n");
        let mut out = Vec::new();
        let mut ra = BufReader::with_capacity(1000, io::Cursor::new(a.as_bytes()));
        let mut rb = BufReader::with_capacity(1000, io::Cursor::new(b.as_bytes()));
        assert!(comm(&mut ra, &mut rb, &mut out, &Opts::default()).is_ok());
        assert_eq!(out.len(), 2 + 300_001 + 1 + 300_002);
    }

    #[test]
    fn many_lines_in_one_go() {
        let mut a = String::new();
        let mut b = String::new();
        for i in 0..100_000 {
            a.push_str(&format!("{:06}\n", i * 2));
            b.push_str(&format!("{:06}\n", i * 3));
        }
        let out = run("12", &a, &b);
        /* the common ones: multiples of 6 below 200000 */
        assert_eq!(out.lines().count(), 33_334);
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
        let mut ra = io::Cursor::new(&b"a\n"[..]);
        let mut rb = BufReader::new(Bad);
        match comm(&mut ra, &mut rb, &mut out, &Opts::default()) {
            Err(Fail::Read(2, _)) => {}
            _ => panic!("expected a read error on input 2"),
        }
    }
}
