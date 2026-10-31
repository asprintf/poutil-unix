/* SPDX-License-Identifier: GPL-2.0-only */
/* uniq - report or filter out repeated lines in a file. */

use posix::getopt::{Getopt, Opt};
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

const SYNOPSIS: &str = "uniq [-c|-d|-u] [-f fields] [-s chars] [input_file [output_file]]";
const BLOCK: usize = 65536;

#[derive(Clone, Copy, Default)]
struct Opts {
    /* -c: precede each line by its count */
    count: bool,
    /* -d: only lines that are repeated */
    dups: bool,
    /* -u: only lines that are not repeated */
    uniq: bool,
    /* -f: fields to ignore */
    fields: usize,
    /* -s: characters to ignore after the fields */
    chars: usize,
}

enum Fail {
    Read(io::Error),
    Write(io::Error),
}

/*
 * The part of a line that is compared: what is left after the first `fields`
 * fields (each blanks, then non-blanks) and then `chars` characters.
 */
fn key<'a>(line: &'a [u8], o: &Opts) -> &'a [u8] {
    if o.fields == 0 && o.chars == 0 {
        return line;
    }
    let mut i = 0;
    for _ in 0..o.fields {
        while i < line.len() {
            let (n, blank) = posix::mb::blank_at(&line[i..]);
            if !blank {
                break;
            }
            i += n;
        }
        while i < line.len() {
            let (n, blank) = posix::mb::blank_at(&line[i..]);
            if blank {
                break;
            }
            i += n;
        }
    }
    i += posix::mb::skip_chars(&line[i..], o.chars);
    &line[i..]
}

/* The group of equal lines just ended: write its line unless an option hides it. */
fn group<W: Write>(w: &mut W, o: &Opts, line: &[u8], n: u64) -> io::Result<()> {
    if (o.dups && n < 2) || (o.uniq && n > 1) {
        return Ok(());
    }
    if o.count {
        write!(w, "{n} ")?;
    }
    w.write_all(line)?;
    w.write_all(b"\n")
}

/*
 * Compare adjacent lines of r and write the result to w. Only the current
 * group's first line is kept, so memory does not depend on the input. A last
 * line without a newline is a line like the others.
 */
fn uniq<R: BufRead, W: Write>(r: &mut R, w: &mut W, o: &Opts) -> Result<(), Fail> {
    let mut prev: Vec<u8> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut n: u64 = 0;
    loop {
        cur.clear();
        if r.read_until(b'\n', &mut cur).map_err(Fail::Read)? == 0 {
            break;
        }
        if cur.last() == Some(&b'\n') {
            cur.pop();
        }
        if n > 0 && key(&prev, o) == key(&cur, o) {
            n += 1;
            continue;
        }
        if n > 0 {
            group(w, o, &prev, n).map_err(Fail::Write)?;
        }
        std::mem::swap(&mut prev, &mut cur);
        n = 1;
    }
    if n > 0 {
        group(w, o, &prev, n).map_err(Fail::Write)?;
    }
    w.flush().map_err(Fail::Write)
}

/* A number option-argument: decimal digits only (a huge value just means "all"). */
fn number(arg: &[u8]) -> usize {
    match posix::parse_decimal(arg) {
        Some(n) => usize::try_from(n).unwrap_or(usize::MAX),
        None => posix::usage(SYNOPSIS),
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut o = Opts::default();
    let mut g = Getopt::new(&args, b"cduf:s:");
    while let Some(opt) = g.next() {
        match opt {
            Opt::Flag(b'c') => o.count = true,
            Opt::Flag(b'd') => o.dups = true,
            Opt::Flag(b'u') => o.uniq = true,
            Opt::Arg(b'f', a) => o.fields = number(&a),
            Opt::Arg(b's', a) => o.chars = number(&a),
            _ => posix::usage(SYNOPSIS),
        }
    }
    /* -d and -u cancel each other out; -c goes with either */
    if o.dups && o.uniq {
        posix::usage(SYNOPSIS);
    }
    let files = g.operands();
    if files.len() > 2 {
        posix::usage(SYNOPSIS);
    }
    let in_name: &[u8] = files.first().map(|v| v.as_slice()).unwrap_or(b"-");
    let input = match posix::open_input(in_name) {
        Ok(h) => h,
        Err(e) => {
            posix::warn_path(&prog, in_name, &posix::strerror(&e));
            std::process::exit(1);
        }
    };
    let mut r = BufReader::with_capacity(BLOCK, input);
    let res = match files.get(1).filter(|f| f.as_slice() != b"-") {
        None => {
            let mut w = io::BufWriter::with_capacity(BLOCK, posix::Stdout::new());
            uniq(&mut r, &mut w, &o)
        }
        Some(out_name) => {
            /*
             * Open without truncating, refuse to destroy the input, then
             * truncate: a failed open or a bad input leaves the file alone.
             */
            let path = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(out_name);
            let f = match OpenOptions::new().write(true).create(true).mode(0o666).open(path) {
                Ok(f) => f,
                Err(e) => {
                    posix::warn_path(&prog, out_name, &posix::strerror(&e));
                    std::process::exit(1);
                }
            };
            if let (Ok(a), Ok(b)) = (r.get_ref().metadata(), f.metadata()) {
                if a.is_file() && a.dev() == b.dev() && a.ino() == b.ino() {
                    posix::warn_path(&prog, out_name, "input file is output file");
                    std::process::exit(1);
                }
                if b.is_file() {
                    let _ = f.set_len(0);
                }
            }
            let mut w = io::BufWriter::with_capacity(BLOCK, f);
            uniq(&mut r, &mut w, &o)
        }
    };
    match res {
        Ok(()) => {}
        Err(Fail::Read(e)) => posix::fail(&prog, &format!("{}: {}", String::from_utf8_lossy(in_name), posix::strerror(&e))),
        Err(Fail::Write(e)) => posix::fail(&prog, &format!("write error: {}", posix::strerror(&e))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /* the locale is process-wide: tests that change it hold this */
    static LOCALE: Mutex<()> = Mutex::new(());

    fn run_with(o: Opts, input: &[u8]) -> String {
        let mut out = Vec::new();
        let mut r = io::Cursor::new(input);
        if uniq(&mut r, &mut out, &o).is_err() {
            panic!("uniq failed");
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn run(flags: &str, input: &str) -> String {
        let o = Opts { count: flags.contains('c'), dups: flags.contains('d'), uniq: flags.contains('u'), ..Opts::default() };
        run_with(o, input.as_bytes())
    }

    fn skip(fields: usize, chars: usize, input: &str) -> String {
        run_with(Opts { fields, chars, ..Opts::default() }, input.as_bytes())
    }

    /* the data of the spec's examples, flushed left */
    const SPEC: &str = "#01 foo0 bar0 foo1 bar1\n#02 bar0 foo1 bar1 foo1\n#03 foo0 bar0 foo1 bar1\n#04\n#05 foo0 bar0 foo1 bar1\n#06 foo0 bar0 foo1 bar1\n#07 bar0 foo1 bar1 foo0\n";

    #[test]
    fn collapses_adjacent_repeats_only() {
        assert_eq!(run("", "a\na\nb\na\n"), "a\nb\na\n");
        assert_eq!(run("", ""), "");
        assert_eq!(run("", "a\n"), "a\n");
        assert_eq!(run("", "\n\n\n"), "\n");
    }

    #[test]
    fn counts() {
        assert_eq!(run("c", "a\na\nb\na\n"), "2 a\n1 b\n1 a\n");
        assert_eq!(run("c", "x\n"), "1 x\n");
    }

    #[test]
    fn repeated_and_unrepeated() {
        assert_eq!(run("d", "a\na\nb\nc\nc\nc\n"), "a\nc\n");
        assert_eq!(run("u", "a\na\nb\nc\nc\nc\n"), "b\n");
        assert_eq!(run("cd", "a\na\nb\nc\nc\nc\n"), "2 a\n3 c\n");
        assert_eq!(run("cu", "a\na\nb\n"), "1 b\n");
    }

    #[test]
    fn the_last_line_needs_no_newline() {
        assert_eq!(run("", "a\na"), "a\n");
        assert_eq!(run("", "a\nb"), "a\nb\n");
        assert_eq!(run("c", "a\na"), "2 a\n");
    }

    #[test]
    fn the_examples_of_the_spec() {
        assert_eq!(
            run_with(Opts { count: true, fields: 1, ..Opts::default() }, SPEC.as_bytes()),
            "1 #01 foo0 bar0 foo1 bar1\n1 #02 bar0 foo1 bar1 foo1\n1 #03 foo0 bar0 foo1 bar1\n1 #04\n2 #05 foo0 bar0 foo1 bar1\n1 #07 bar0 foo1 bar1 foo0\n"
        );
        assert_eq!(run_with(Opts { dups: true, fields: 1, ..Opts::default() }, SPEC.as_bytes()), "#05 foo0 bar0 foo1 bar1\n");
        assert_eq!(
            run_with(Opts { uniq: true, fields: 1, ..Opts::default() }, SPEC.as_bytes()),
            "#01 foo0 bar0 foo1 bar1\n#02 bar0 foo1 bar1 foo1\n#03 foo0 bar0 foo1 bar1\n#04\n#07 bar0 foo1 bar1 foo0\n"
        );
        assert_eq!(run_with(Opts { dups: true, chars: 2, ..Opts::default() }, SPEC.as_bytes()), "");
    }

    #[test]
    fn fields_are_blanks_then_non_blanks() {
        assert_eq!(skip(1, 0, "a x\nb x\nc y\n"), "a x\nc y\n");
        /* leading blanks belong to the first field */
        assert_eq!(skip(1, 0, "  a x\n b x\n"), "  a x\n");
        assert_eq!(skip(2, 0, "a b x\nc d x\n"), "a b x\n");
        assert_eq!(skip(1, 0, "a\tx\nb\t\tx\n"), "a\tx\nb\t\tx\n");
        /* more fields than the line has: the null string */
        assert_eq!(skip(5, 0, "a b\nc\n\n"), "a b\n");
        assert_eq!(skip(1, 0, "a\nb\n"), "a\n");
    }

    #[test]
    fn chars_come_after_the_fields() {
        assert_eq!(skip(0, 1, "xa\nya\nzb\n"), "xa\nzb\n");
        assert_eq!(skip(1, 1, "a xb\nc xb\nd zc\n"), "a xb\nd zc\n");
        assert_eq!(skip(0, 9, "ab\ncd\n"), "ab\n");
        /* the blank that starts the next field counts as a character */
        assert_eq!(skip(1, 1, "a xb\nc  b\n"), "a xb\nc  b\n");
    }

    #[test]
    fn nul_and_high_bytes_are_plain_characters() {
        assert_eq!(run("", "a\0b\na\0b\na\0c\n"), "a\0b\na\0c\n");
        assert_eq!(run("c", "\u{ff}\n\u{ff}\n").len(), 5);
        let mut out = Vec::new();
        let mut r = io::Cursor::new(&b"\xff\xfe\n\xff\xfe\n\xff\n"[..]);
        assert!(uniq(&mut r, &mut out, &Opts::default()).is_ok());
        assert_eq!(out, b"\xff\xfe\n\xff\n");
    }

    #[test]
    fn utf8_characters_for_s() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let loc = std::ffi::CString::new("en_US.UTF-8").unwrap();
        if unsafe { libc::setlocale(libc::LC_ALL, loc.as_ptr()) }.is_null() {
            eprintln!("skipped: no locale en_US.UTF-8");
            return;
        }
        /* one character is two bytes: -s 1 must not cut inside it */
        assert_eq!(skip(0, 1, "\u{e9}a\n\u{e8}a\n\u{e8}b\n"), "\u{e9}a\n\u{e8}b\n");
        /* U+2003 EM SPACE is a blank, so it ends a field */
        assert_eq!(skip(1, 0, "a\u{2003}x\nb\u{2003}x\n"), "a\u{2003}x\n");
        unsafe { libc::setlocale(libc::LC_ALL, c"C".as_ptr()) };
    }

    #[test]
    fn a_long_line_spans_many_blocks() {
        let long = "x".repeat(300_000);
        let input = format!("{long}\n{long}\n{long}y\n");
        let mut out = Vec::new();
        let mut r = BufReader::with_capacity(1000, io::Cursor::new(input.as_bytes()));
        assert!(uniq(&mut r, &mut out, &Opts { count: true, ..Opts::default() }).is_ok());
        assert_eq!(out.len(), 300_003 + 300_004);
        assert!(out.starts_with(b"2 xxx"));
    }

    #[test]
    fn many_lines_in_one_go() {
        let mut s = String::new();
        for i in 0..100_000 {
            s.push_str(&format!("{}\n", i / 3));
        }
        let o = run("c", &s);
        assert!(o.starts_with("3 0\n3 1\n"));
        assert_eq!(o.lines().count(), 33_334);
    }
}
