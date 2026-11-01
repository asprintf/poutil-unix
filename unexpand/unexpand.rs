/* SPDX-License-Identifier: GPL-2.0-only */
/* unexpand - convert spaces to tabs. */

use posix::getopt::{Getopt, Opt};
use posix::tabs::{self, Ev, Fail, Stops};
use std::io::{self, BufWriter, Write};

const SYNOPSIS: &str = "unexpand [-a|-t tablist] [file...]";

/*
 * The state of the conversion. A run is a row of <blank> characters (space
 * and tab); it is held back until the character after it is known, then it
 * is either written as it came or written again with the fewest characters
 * that fill the same columns: as many tabs as fit, then spaces.
 */
struct Unexpand<'a> {
    stops: &'a Stops,
    /* convert runs anywhere in the line (-a, or any -t), not only at its start */
    all: bool,
    /* the column the next character lands in */
    col: usize,
    /* nothing but blanks so far in this line */
    leading: bool,
    /* the blanks of the current run, as read, the column it began in, and how many */
    run: Vec<u8>,
    run_col: usize,
    run_len: usize,
}

impl<'a> Unexpand<'a> {
    fn new(stops: &'a Stops, all: bool) -> Self {
        Unexpand { stops, all, col: 0, leading: true, run: Vec::new(), run_col: 0, run_len: 0 }
    }

    /*
     * Write out the current run. It is converted if it is at the start of
     * the line, or anywhere with -a or -t and then only if it is two blanks
     * or more (a lone space before a tab stop stays a space).
     */
    fn flush<W: Write>(&mut self, w: &mut W) -> io::Result<()> {
        if self.run_len == 0 {
            return Ok(());
        }
        let wanted = if self.leading { true } else { self.all && self.run_len >= 2 };
        if wanted {
            let mut at = self.run_col;
            while let Some(n) = self.stops.next(at) {
                if n > self.col {
                    break;
                }
                w.write_all(b"\t")?;
                at = n;
            }
            tabs::write_spaces(w, self.col - at)?;
        } else {
            w.write_all(&self.run)?;
        }
        self.run.clear();
        self.run_len = 0;
        Ok(())
    }

    /*
     * One character of the input. Spaces and tabs join the run, unless they
     * are past the last stop of a list: there nothing is converted, a tab
     * counts as one column (as expand writes it) and everything goes out as
     * it is. <backspace> is copied and moves the column back, not before
     * the first; <newline> ends the line; the rest moves the column by its
     * width.
     */
    fn char<W: Write>(&mut self, c: &[u8], wc: Option<u32>, w: &mut W) -> io::Result<()> {
        match wc {
            Some(b @ (0x20 | 0x09)) => {
                if self.stops.last().is_some_and(|l| self.col >= l) {
                    self.flush(w)?;
                    w.write_all(c)?;
                    self.col = self.col.saturating_add(1);
                    return Ok(());
                }
                if self.run_len == 0 {
                    self.run_col = self.col;
                }
                self.run.extend_from_slice(c);
                self.run_len += 1;
                self.col = if b == 0x09 {
                    /* below the last stop of a list, so there is a next one */
                    self.stops.next(self.col).unwrap_or(self.col.saturating_add(1))
                } else {
                    self.col.saturating_add(1)
                };
            }
            Some(0x0a) => {
                self.flush(w)?;
                w.write_all(c)?;
                self.col = 0;
                self.leading = true;
            }
            _ => {
                self.flush(w)?;
                w.write_all(c)?;
                self.leading = false;
                self.col = if wc == Some(0x08) {
                    self.col.saturating_sub(1)
                } else {
                    self.col.saturating_add(tabs::col_width(wc))
                };
            }
        }
        Ok(())
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut stops = tabs::DEFAULT_STOPS;
    let mut all = false;
    let mut tflag = false;
    let mut g = Getopt::new(&args, b"at:");
    while let Some(o) = g.next() {
        match o {
            Opt::Flag(b'a') => all = true,
            /* a repeated -t replaces the one before */
            Opt::Arg(b't', v) => match Stops::parse(&v) {
                Ok(s) => {
                    stops = s;
                    tflag = true;
                }
                Err(e) => posix::fail(&prog, e.message()),
            },
            _ => posix::usage(SYNOPSIS),
        }
    }
    /* with -t, -a is ignored and every run is converted */
    let all = all || tflag;
    let mut files: Vec<&[u8]> = g.operands().iter().map(|v| v.as_slice()).collect();
    if files.is_empty() {
        files.push(b"-");
    }
    let mut out = BufWriter::with_capacity(65536, posix::Stdout::new());
    let mut un = Unexpand::new(&stops, all);
    let mut status = 0;
    for f in files {
        let mut input = match posix::open_input(f) {
            Ok(h) => h,
            Err(e) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                status = 1;
                continue;
            }
        };
        let res = tabs::each_event(&mut input, |ev| match ev {
            Ev::Char(c, wc) => un.char(c, wc, &mut out),
            Ev::Idle => out.flush(),
        });
        match res {
            Ok(()) => {}
            Err(Fail::Read(e)) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                status = 1;
            }
            Err(Fail::Write(e)) => {
                posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
            }
        }
    }
    /* a last line without its newline may end in a run */
    let done = un.flush(&mut out).and_then(|_| out.flush());
    if let Err(e) = done {
        posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv_in(input: &[u8], stops: &Stops, all: bool, piece: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut un = Unexpand::new(stops, all);
        for p in input.chunks(piece.max(1)) {
            tabs::each_char(&mut &p[..], |c, wc| un.char(c, wc, &mut out)).ok().unwrap();
        }
        un.flush(&mut out).unwrap();
        out
    }

    /* default behaviour: leading blanks only */
    fn lead(input: &str) -> String {
        String::from_utf8(conv_in(input.as_bytes(), &tabs::DEFAULT_STOPS, false, usize::MAX)).unwrap()
    }

    /* -a */
    fn all(input: &str) -> String {
        String::from_utf8(conv_in(input.as_bytes(), &tabs::DEFAULT_STOPS, true, usize::MAX)).unwrap()
    }

    /* -t: the argument as on the command line */
    fn with_t(t: &str, input: &str) -> String {
        let s = Stops::parse(t.as_bytes()).unwrap();
        String::from_utf8(conv_in(input.as_bytes(), &s, true, usize::MAX)).unwrap()
    }

    #[test]
    fn leading_blanks_become_the_most_tabs_then_spaces() {
        assert_eq!(lead("        x\n"), "\tx\n");
        assert_eq!(lead("          x\n"), "\t  x\n");
        assert_eq!(lead("                x\n"), "\t\tx\n");
        assert_eq!(lead("                 x\n"), "\t\t x\n");
        assert_eq!(lead("   x\n"), "   x\n");
        assert_eq!(lead("       x\n"), "       x\n");
        assert_eq!(lead(" x\n"), " x\n");
    }

    #[test]
    fn only_the_leading_blanks_without_a_or_t() {
        assert_eq!(lead("a       b\n"), "a       b\n");
        assert_eq!(lead("        a        b\n"), "\ta        b\n");
        assert_eq!(lead("        \n"), "\t\n");
        assert_eq!(lead("       \n"), "       \n");
    }

    #[test]
    fn tabs_in_the_input_are_blanks_too() {
        assert_eq!(lead("\t  x\n"), "\t  x\n");
        assert_eq!(lead("  \t  x\n"), "\t  x\n");
        assert_eq!(lead("  \tx\n"), "\tx\n");
        assert_eq!(lead("\t\t\tx\n"), "\t\t\tx\n");
        assert_eq!(lead(" \t \t x\n"), "\t\t x\n");
        assert_eq!(lead("\t        x\n"), "\t\tx\n");
    }

    #[test]
    fn a_converts_runs_of_two_or_more_before_a_stop() {
        assert_eq!(all("a       b\n"), "a\tb\n");
        assert_eq!(all("a      b\n"), "a      b\n");
        assert_eq!(all("abcdefg  b\n"), "abcdefg\t b\n");
        assert_eq!(all("abcdefg b\n"), "abcdefg b\n");
        assert_eq!(all("abcdefgh  \n"), "abcdefgh  \n");
        assert_eq!(all("abcdef  g\n"), "abcdef\tg\n");
        assert_eq!(all("a             b\n"), "a\t      b\n");
        assert_eq!(all("a b\n"), "a b\n");
    }

    #[test]
    fn a_converts_tabs_and_spaces_mixed() {
        assert_eq!(all("a\t\t  b\n"), "a\t\t  b\n");
        assert_eq!(all("a \t b\n"), "a\t b\n");
        assert_eq!(all("    a\tb\n"), "    a\tb\n");
    }

    #[test]
    fn a_lone_leading_blank_is_converted_when_it_fills_a_stop() {
        assert_eq!(with_t("1", " x\n"), "\tx\n");
        assert_eq!(lead(" x\n"), " x\n");
    }

    #[test]
    fn t_with_a_number_converts_everywhere() {
        assert_eq!(with_t("4", "a       b\n"), "a\t\tb\n");
        assert_eq!(with_t("4", "abc  d\n"), "abc\t d\n");
        assert_eq!(with_t("4", "abc d\n"), "abc d\n");
        assert_eq!(with_t("8", "a       b\n"), "a\tb\n");
        assert_eq!(with_t("2", "a b  c\n"), "a b\t c\n");
        assert_eq!(with_t("3", "         ABC         DEF\n"), "\t\t\tABC\t\t\tDEF\n");
    }

    #[test]
    fn t_with_a_list_stops_at_the_last_stop() {
        /* PCTS 815 and 819 */
        assert_eq!(with_t("9,12,21", "         ABC         DEF\n"), "\tABC\tDEF\n");
        assert_eq!(with_t("3,6,9", "         ABC         DEF\n"), "\t\t\tABC         DEF\n");
        assert_eq!(with_t("3,6,9,12,15,18,21,24", "         ABC         DEF\n"), "\t\t\tABC\t\t\tDEF\n");
        /* a run that crosses the last stop is converted up to it */
        assert_eq!(with_t("4,8", "a             b\n"), "a\t\t      b\n");
        assert_eq!(with_t("4,8", "a     b     c\n"), "a\t  b     c\n");
        /* a tab past the last stop is kept, as one column */
        assert_eq!(with_t("4,8", "a\tb\tc\td\n"), "a\tb\tc\td\n");
        assert_eq!(with_t("4,8", "a\tb\tc\t        d\n"), "a\tb\tc\t        d\n");
    }

    #[test]
    fn backspace_is_copied_and_moves_the_column_back() {
        /* PCTS 817: 10 spaces, <backspace>, hello, 6 spaces, with -t 10 */
        assert_eq!(with_t("10", "          \x08hello      \n"), "\t\x08hello\t\n");
        /* PCTS 818: never below the first column */
        let s = "\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08hello\x08\x08\x08\x08\x08\x08\x08\x08\x08\n";
        assert_eq!(lead(s), s);
        assert_eq!(all("a\x08b       c\n"), "a\x08b\tc\n");
        assert_eq!(lead("        \x08 \n"), "\t\x08 \n");
        assert_eq!(lead("  \x08\x08\x08\x08\x08  x\n"), "  \x08\x08\x08\x08\x08  x\n");
        assert_eq!(all("ab\x08\x08\x08\x08      x\n"), "ab\x08\x08\x08\x08      x\n");
        /* after backspaces the column is what counts */
        assert_eq!(all("abc\x08\x08\x08     x\n"), "abc\x08\x08\x08     x\n");
    }

    #[test]
    fn a_blank_run_ends_at_any_other_character() {
        assert_eq!(all("a  \x01  b\n"), "a  \x01  b\n");
        assert_eq!(all("       \x00 x\n"), "       \x00 x\n");
        assert_eq!(all("a      \r b\n"), "a      \r b\n");
        /* control characters have no width: the stop is where it was */
        assert_eq!(all("a\x01       b\n"), "a\x01\tb\n");
    }

    #[test]
    fn line_ends_and_the_last_line() {
        assert_eq!(lead("        \n        \n"), "\t\n\t\n");
        assert_eq!(lead("        x"), "\tx");
        assert_eq!(lead("        "), "\t");
        assert_eq!(lead("       "), "       ");
        assert_eq!(lead(""), "");
        assert_eq!(lead("\n"), "\n");
        assert_eq!(all("abcdefg  "), "abcdefg\t ");
        /* the count starts again on each line */
        assert_eq!(all("abc\n  x  y\n"), "abc\n  x  y\n");
    }

    #[test]
    fn the_input_is_never_changed_apart_from_blanks() {
        let data: Vec<u8> = (0..=255u8).filter(|&b| b != b' ' && b != b'\t').collect();
        assert_eq!(conv_in(&data, &tabs::DEFAULT_STOPS, true, usize::MAX), data);
        assert_eq!(conv_in(&data, &tabs::DEFAULT_STOPS, false, usize::MAX), data);
    }

    #[test]
    fn the_result_does_not_depend_on_how_the_input_is_cut() {
        let s = Stops::List(vec![5, 9, 20]);
        let input: Vec<u8> = b"  \t   a   bc    \t\x08d   \n        \t\t\txyz       q\r\n \x00  \t  "
            .iter()
            .cycle()
            .take(5000)
            .copied()
            .collect();
        let whole = conv_in(&input, &s, true, usize::MAX);
        for piece in [1, 2, 3, 7, 64] {
            assert_eq!(conv_in(&input, &s, true, piece), whole, "{piece}");
        }
    }

    #[test]
    fn a_huge_spacing_does_not_wrap() {
        let s = Stops::Every(usize::MAX / 2);
        let out = conv_in(b"        x\n", &s, true, usize::MAX);
        assert_eq!(out, b"        x\n");
    }

    #[test]
    fn converting_then_expanding_gives_the_text_back() {
        /* the columns of every character stay where they were */
        let s = Stops::Every(8);
        let input = b"        a   b       c\n  \t d    e         f\n";
        let out = conv_in(input, &s, true, usize::MAX);
        assert!(out.len() < input.len());
        let back = expand_ref(&out, 8);
        assert_eq!(back, expand_ref(input, 8));
    }

    /* a plain expand for the check above */
    fn expand_ref(input: &[u8], n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut col = 0;
        for &b in input {
            match b {
                b'\t' => {
                    let k = n - col % n;
                    out.extend(std::iter::repeat(b' ').take(k));
                    col += k;
                }
                b'\n' => {
                    out.push(b);
                    col = 0;
                }
                _ => {
                    out.push(b);
                    col += 1;
                }
            }
        }
        out
    }

    #[test]
    fn a_write_error_is_passed_on() {
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from_raw_os_error(libc::ENOSPC))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let s = tabs::DEFAULT_STOPS;
        let mut un = Unexpand::new(&s, true);
        assert!(un.char(b"a", Some(97), &mut Full).is_err());
        let mut un = Unexpand::new(&s, true);
        for _ in 0..8 {
            assert!(un.char(b" ", Some(32), &mut Full).is_ok());
        }
        assert!(un.flush(&mut Full).is_err());
    }
}
