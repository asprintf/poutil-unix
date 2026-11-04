/* SPDX-License-Identifier: GPL-2.0-only */
/* fold - filter for folding lines. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::tabs::{col_width, each_event, Ev, Fail};
use std::io::{self, BufWriter, Write};

const SYNOPSIS: &str = "fold [-bs] [-w width] [file...]";
const DEFAULT_WIDTH: usize = 80;

#[derive(Clone, Copy)]
struct Opts {
    /* count bytes, not column positions, and treat no character specially */
    bytes: bool,
    /* break after the last blank of a segment, if it has one */
    spaces: bool,
    width: usize,
}

/*
 * The state of the line being folded. The segment waiting to be written is
 * kept as it came, so that a break at a blank can hand its end to the next
 * segment; without -s nothing is kept back, the characters are written as
 * they arrive.
 */
struct Folder {
    o: Opts,
    /* column position after the segment; 0 is the start of a line */
    col: usize,
    seg: Vec<u8>,
    /* the length of seg up to and including each of its blanks, in order */
    blanks: Vec<usize>,
}

/* What one character does to the column position. */
fn advance(o: &Opts, col: usize, bytes: &[u8], wc: Option<u32>) -> usize {
    if o.bytes {
        return col + bytes.len();
    }
    match wc {
        Some(0x0d) => 0,
        Some(0x08) => col.saturating_sub(1),
        /* the tab stops are at every eighth column */
        Some(0x09) => (col / 8 + 1) * 8,
        _ => col + col_width(wc),
    }
}

/* Does the character add to the width (so that it can overflow a line)? */
fn is_plain(o: &Opts, wc: Option<u32>) -> bool {
    o.bytes || !matches!(wc, Some(0x0d) | Some(0x08))
}

impl Folder {
    fn new(o: Opts) -> Self {
        Folder { o, col: 0, seg: Vec::new(), blanks: Vec::new() }
    }

    /* The column position after a run of characters that starts a line. */
    fn measure(&self, bytes: &[u8]) -> usize {
        let mut col = 0;
        let mut i = 0;
        while i < bytes.len() {
            let (n, wc) = posix::mb::char_at(&bytes[i..]);
            col = advance(&self.o, col, &bytes[i..i + n], wc);
            i += n;
        }
        col
    }

    /* End the line here: what is waiting, then (if asked for) a newline. */
    fn end_segment<W: Write>(&mut self, newline: bool, out: &mut W) -> io::Result<()> {
        out.write_all(&self.seg)?;
        if newline {
            out.write_all(b"\n")?;
        }
        self.seg.clear();
        self.blanks.clear();
        self.col = 0;
        Ok(())
    }

    /*
     * May a newline go right after the blank that ends seg[..end]? Not when
     * a <carriage-return> or a <backspace> follows it: no newline immediately
     * before either of them (not with -b, where they are bytes like the rest).
     */
    fn may_break_at(&self, end: usize) -> bool {
        if self.o.bytes || end >= self.seg.len() {
            return true;
        }
        let (_, wc) = posix::mb::char_at(&self.seg[end..]);
        !matches!(wc, Some(0x0d) | Some(0x08))
    }

    /*
     * Put a newline in before the character that does not fit: after the last
     * blank that may take one with -s, else here. A blank that may not (it
     * stands before a return or a backspace) is passed over for an earlier one.
     */
    fn break_line<W: Write>(&mut self, out: &mut W) -> io::Result<()> {
        let pick = if self.o.spaces { self.blanks.iter().rposition(|&e| self.may_break_at(e)) } else { None };
        match pick {
            Some(i) => {
                let end = self.blanks[i];
                out.write_all(&self.seg[..end])?;
                out.write_all(b"\n")?;
                self.seg.drain(..end);
                /* the blanks after the break stay, counted from the new start */
                self.blanks = self.blanks[i + 1..].iter().map(|b| b - end).collect();
                self.col = self.measure(&self.seg);
                Ok(())
            }
            None => self.end_segment(true, out),
        }
    }

    fn char<W: Write>(&mut self, bytes: &[u8], wc: Option<u32>, out: &mut W) -> io::Result<()> {
        if wc == Some(0x0a) && !self.o.bytes || (self.o.bytes && bytes == b"\n") {
            self.seg.extend_from_slice(bytes);
            return self.end_segment(false, out);
        }
        while is_plain(&self.o, wc)
            && self.col > 0
            && advance(&self.o, self.col, bytes, wc) > self.o.width
        {
            self.break_line(out)?;
        }
        self.col = advance(&self.o, self.col, bytes, wc);
        if self.o.spaces {
            self.seg.extend_from_slice(bytes);
            if wc.is_some_and(posix::mb::is_blank) {
                self.blanks.push(self.seg.len());
            }
        } else {
            out.write_all(bytes)?;
        }
        Ok(())
    }

    /* The end of an input: a last line without a newline stays without. */
    fn finish<W: Write>(&mut self, out: &mut W) -> io::Result<()> {
        self.end_segment(false, out)
    }
}

fn fold_file<R: io::Read, W: Write>(r: &mut R, o: Opts, out: &mut W) -> Result<(), Fail> {
    let mut f = Folder::new(o);
    each_event(r, |ev| match ev {
        Ev::Char(b, wc) => f.char(b, wc, out),
        Ev::Idle => out.flush(),
    })?;
    f.finish(out).map_err(Fail::Write)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut o = Opts { bytes: false, spaces: false, width: DEFAULT_WIDTH };
    let mut g = Getopt::new(&args, b"bsw:");
    for opt in &mut g {
        match opt {
            Opt::Flag(b'b') => o.bytes = true,
            Opt::Flag(b's') => o.spaces = true,
            Opt::Arg(b'w', v) => match posix::parse_decimal(&v) {
                Some(n) if n > 0 => o.width = usize::try_from(n).unwrap_or(usize::MAX),
                _ => posix::fail(&prog, "illegal width value"),
            },
            _ => posix::usage(SYNOPSIS),
        }
    }
    let files = g.operands();
    let mut out = BufWriter::with_capacity(65536, posix::Stdout::new());
    let mut status = 0;
    let mut one = |name: Option<&[u8]>, status: &mut i32| {
        let res = match name {
            None => fold_file(&mut *posix::borrow_fd(0), o, &mut out),
            Some(n) => match posix::open_input(n) {
                Ok(mut h) => fold_file(&mut h, o, &mut out),
                Err(e) => Err(Fail::Read(e)),
            },
        };
        match res {
            Ok(()) => {}
            Err(Fail::Write(e)) => {
                posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
            }
            Err(Fail::Read(e)) => {
                match name {
                    Some(n) => posix::warn_path(&prog, n, &posix::strerror(&e)),
                    None => posix::warn(&prog, &format!("standard input: {}", posix::strerror(&e))),
                }
                *status = 1;
            }
        }
    };
    if files.is_empty() {
        one(None, &mut status);
    }
    for f in files {
        one(Some(f), &mut status);
    }
    if let Err(e) = out.flush() {
        posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(flags: &str, width: usize, input: &str) -> String {
        let o = Opts { bytes: flags.contains('b'), spaces: flags.contains('s'), width };
        let mut out = Vec::new();
        let mut r = io::Cursor::new(input.as_bytes().to_vec());
        fold_file(&mut r, o, &mut out).ok().unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn plain_folding() {
        assert_eq!(run("", 4, "abcdefghij\n"), "abcd\nefgh\nij\n");
        assert_eq!(run("", 4, "abcd\n"), "abcd\n");
        assert_eq!(run("", 4, "abcde\n"), "abcd\ne\n");
        assert_eq!(run("", 4, "ab\ncd\n"), "ab\ncd\n");
        assert_eq!(run("", 1, "abc\n"), "a\nb\nc\n");
    }

    #[test]
    fn a_last_line_without_newline_stays_without() {
        assert_eq!(run("", 4, "abcdef"), "abcd\nef");
        assert_eq!(run("", 4, "abcd"), "abcd");
        assert_eq!(run("", 4, ""), "");
    }

    #[test]
    fn empty_lines_stay() {
        assert_eq!(run("", 4, "\n\nab\n\n"), "\n\nab\n\n");
    }

    #[test]
    fn tab_goes_to_the_next_stop() {
        /* a tab at column 0 reaches column 8 */
        assert_eq!(run("", 8, "\tab\n"), "\t\nab\n");
        assert_eq!(run("", 9, "\tab\n"), "\ta\nb\n");
        assert_eq!(run("", 10, "ab\tcd\n"), "ab\tcd\n");
        assert_eq!(run("", 8, "abcdefg\tx\n"), "abcdefg\t\nx\n");
        assert_eq!(run("", 9, "abcdefg\tx\n"), "abcdefg\tx\n");
        /* with -b a tab is one byte */
        assert_eq!(run("b", 4, "\tabcd\n"), "\tabc\nd\n");
    }

    #[test]
    fn backspace_and_return() {
        /* a backspace takes one off the count, never below zero */
        assert_eq!(run("", 3, "ab\x08cde\n"), "ab\x08cd\ne\n");
        assert_eq!(run("", 3, "\x08\x08abc\n"), "\x08\x08abc\n");
        /* a return starts the count again, and no newline goes before it */
        assert_eq!(run("", 3, "abc\rdef\n"), "abc\rdef\n");
        assert_eq!(run("", 3, "abcd\rd\n"), "abc\nd\rd\n");
        /* with -b they are bytes like the others */
        assert_eq!(run("b", 3, "abc\rdef\n"), "abc\n\rde\nf\n");
    }

    #[test]
    fn break_after_the_last_blank() {
        assert_eq!(run("s", 10, "hello world foo\n"), "hello \nworld foo\n");
        assert_eq!(run("s", 10, "hello world\n"), "hello \nworld\n");
        assert_eq!(run("s", 5, "ab cd ef\n"), "ab \ncd ef\n");
        /* no blank in the segment: break at the width */
        assert_eq!(run("s", 4, "abcdefgh\n"), "abcd\nefgh\n");
        assert_eq!(run("s", 4, "ab cdefgh\n"), "ab \ncdef\ngh\n");
        /* the part after the blank starts the next segment */
        assert_eq!(run("s", 6, "ab cde fgh\n"), "ab \ncde \nfgh\n");
    }

    #[test]
    fn break_after_blank_then_tab_columns_start_again() {
        /* "b" after the break starts at column 0, so the tab reaches 8 and fits in 9 */
        assert_eq!(run("s", 9, "aaaa b\tc\n"), "aaaa b\tc\n");
        assert_eq!(run("s", 9, "aaaaaaaa bb\tc\n"), "aaaaaaaa \nbb\tc\n");
    }

    /* -b counts bytes; this test runs in the C locale, so every byte is a character (see regress.sh for UTF-8) */
    #[test]
    fn bytes_mode_counts_each_byte() {
        assert_eq!(run("b", 2, "abcd\n"), "ab\ncd\n");
    }

    /* DESCRIPTION: no newline immediately before a carriage-return or a backspace, -s included */
    #[test]
    fn s_does_not_break_before_return_or_backspace() {
        /* the blank stands before the return: no blank may be used, so the line breaks at the width */
        assert_eq!(run("s", 4, "abc \rdefghi\n"), "abc \rdefg\nhi\n");
        /* the same before a backspace */
        assert_eq!(run("s", 4, "abc \x08defghi\n"), "abc \x08d\nefgh\ni\n");
        /* an earlier blank that may take the newline is used instead */
        assert_eq!(run("s", 4, "a b \rcdefg\n"), "a \nb \rcdef\ng\n");
        assert_eq!(run("s", 4, "a b \x08cdefg\n"), "a \nb \x08cde\nfg\n");
        /* a blank after the return, or one followed by something else, is as good as ever */
        assert_eq!(run("s", 4, "ab\r cdefg\n"), "ab\r \ncdef\ng\n");
        assert_eq!(run("s", 4, "ab cdefgh\n"), "ab \ncdef\ngh\n");
    }

    #[test]
    fn s_with_b_treats_return_and_backspace_as_bytes() {
        assert_eq!(run("bs", 4, "abc \rdefg\n"), "abc \n\rdef\ng\n");
    }
}
