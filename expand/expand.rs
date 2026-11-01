/* SPDX-License-Identifier: GPL-2.0-only */
/* expand - convert tabs to spaces. */

use posix::getopt::{Getopt, Opt};
use posix::tabs::{self, Ev, Fail, Stops};
use std::io::{self, BufWriter, Write};

const SYNOPSIS: &str = "expand [-t tablist] [file...]";

/*
 * The state of the conversion: where the tab stops are and the column the
 * next character lands in. It survives from one file to the next, so that
 * the output is the same as for the files joined together.
 */
struct Expand<'a> {
    stops: &'a Stops,
    col: usize,
}

impl Expand<'_> {
    /*
     * One character of the input. A <tab> becomes the spaces up to the next
     * stop, or one space past the last stop of a list; a <backspace> is
     * copied and moves the column back, but not before the first; a
     * <newline> starts a new line; the rest moves the column by its width.
     */
    fn char<W: Write>(&mut self, c: &[u8], wc: Option<u32>, w: &mut W) -> io::Result<()> {
        match wc {
            Some(0x09) => match self.stops.next(self.col) {
                Some(n) => {
                    tabs::write_spaces(w, n - self.col)?;
                    self.col = n;
                }
                None => {
                    w.write_all(b" ")?;
                    self.col = self.col.saturating_add(1);
                }
            },
            Some(0x08) => {
                w.write_all(c)?;
                self.col = self.col.saturating_sub(1);
            }
            Some(0x0a) => {
                w.write_all(c)?;
                self.col = 0;
            }
            _ => {
                w.write_all(c)?;
                self.col = self.col.saturating_add(tabs::col_width(wc));
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
    let mut g = Getopt::new(&args, b"t:");
    while let Some(o) = g.next() {
        match o {
            /* a repeated -t replaces the one before */
            Opt::Arg(b't', v) => match Stops::parse(&v) {
                Ok(s) => stops = s,
                Err(e) => posix::fail(&prog, e.message()),
            },
            _ => posix::usage(SYNOPSIS),
        }
    }
    let mut files: Vec<&[u8]> = g.operands().iter().map(|v| v.as_slice()).collect();
    if files.is_empty() {
        files.push(b"-");
    }
    let mut out = BufWriter::with_capacity(65536, posix::Stdout::new());
    let mut ex = Expand { stops: &stops, col: 0 };
    /*
     * "shall terminate with an error message and non-zero exit status upon
     * encountering difficulties accessing one of the file operands": what
     * was written before stays, and no later operand is looked at.
     */
    for f in files {
        let mut input = match posix::open_input(f) {
            Ok(h) => h,
            Err(e) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                let _ = out.flush();
                std::process::exit(1);
            }
        };
        let res = tabs::each_event(&mut input, |ev| match ev {
            Ev::Char(c, wc) => ex.char(c, wc, &mut out),
            Ev::Idle => out.flush(),
        });
        match res {
            Ok(()) => {}
            Err(Fail::Read(e)) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                let _ = out.flush();
                std::process::exit(1);
            }
            Err(Fail::Write(e)) => {
                posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
            }
        }
    }
    if let Err(e) = out.flush() {
        posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /* run the conversion over input, cut into pieces of the given size */
    fn run_in(input: &[u8], stops: &Stops, piece: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut ex = Expand { stops, col: 0 };
        for p in input.chunks(piece.max(1)) {
            tabs::each_char(&mut &p[..], |c, wc| ex.char(c, wc, &mut out)).ok().unwrap();
        }
        out
    }

    fn run(input: &str, stops: &Stops) -> String {
        String::from_utf8(run_in(input.as_bytes(), stops, usize::MAX)).unwrap()
    }

    fn list(l: &[usize]) -> Stops {
        Stops::List(l.to_vec())
    }

    #[test]
    fn default_stops_every_eight() {
        let s = tabs::DEFAULT_STOPS;
        assert_eq!(run("\tx", &s), "        x");
        assert_eq!(run("a\tx", &s), "a       x");
        assert_eq!(run("1234567\tx", &s), "1234567 x");
        assert_eq!(run("12345678\tx", &s), "12345678        x");
        assert_eq!(run("\t\tx", &s), "                x");
        assert_eq!(run("", &s), "");
        assert_eq!(run("no tab\n", &s), "no tab\n");
    }

    #[test]
    fn a_single_number_is_the_spacing() {
        assert_eq!(run("a\tb\n", &Stops::Every(3)), "a  b\n");
        assert_eq!(run("abc\tb\n", &Stops::Every(3)), "abc   b\n");
        assert_eq!(run("a\tb\n", &Stops::Every(1)), "a b\n");
        assert_eq!(run("\tx", &Stops::Every(1)), " x");
        assert_eq!(run("\tx", &Stops::Every(2046)), format!("{}x", " ".repeat(2046)));
    }

    #[test]
    fn a_list_sets_the_stops_and_one_space_follows_the_last() {
        let s = list(&[3, 10]);
        assert_eq!(run("\tx", &s), "   x");
        assert_eq!(run("a\tb\tc\n", &s), "a  b      c\n");
        /* past the last stop each tab is one space */
        assert_eq!(run("a\tb\tc\td\te\n", &s), "a  b      c d e\n");
        assert_eq!(run("abcdefghij\tb\n", &s), "abcdefghij b\n");
        assert_eq!(run("abcdefghi\tb\n", &s), "abcdefghi b\n");
        assert_eq!(run("\t\t\t\tx", &list(&[1, 2])), "    x");
    }

    #[test]
    fn stop_n_puts_the_next_character_in_column_n_plus_one() {
        /* XBD Column Position: columns are numbered from 1 */
        assert_eq!(run("\tx", &list(&[4, 9])), "    x");
        assert_eq!(run("abc\tx", &list(&[4, 9])), "abc x");
        assert_eq!(run("abcd\tx", &list(&[4, 9])), "abcd     x");
    }

    #[test]
    fn backspace_is_copied_and_moves_the_column_back() {
        let s = tabs::DEFAULT_STOPS;
        assert_eq!(run("ab\x08\tx", &s), "ab\x08       x");
        assert_eq!(run("abc\x08\x08\tx", &s), "abc\x08\x08       x");
        /* never before the first column */
        assert_eq!(run("\x08\x08\x08\tx", &s), "\x08\x08\x08        x");
        assert_eq!(run("a\x08\x08\x08\x08\tx", &s), "a\x08\x08\x08\x08        x");
        /* at the start of a line */
        assert_eq!(run("\x08\tx\n", &s), "\x08        x\n");
        assert_eq!(run("12345678\x08\tx", &s), "12345678\x08 x");
    }

    #[test]
    fn newline_restarts_the_count() {
        let s = tabs::DEFAULT_STOPS;
        assert_eq!(run("abc\n\tx\n", &s), "abc\n        x\n");
        assert_eq!(run("\n\n\t\n", &s), "\n\n        \n");
    }

    #[test]
    fn the_count_runs_on_from_one_file_to_the_next() {
        /* the same as the files joined: "ab" then "\tx" */
        let s = tabs::DEFAULT_STOPS;
        let mut out = Vec::new();
        let mut ex = Expand { stops: &s, col: 0 };
        for p in [&b"ab"[..], &b"\tx"[..]] {
            tabs::each_char(&mut &p[..], |c, wc| ex.char(c, wc, &mut out)).ok().unwrap();
        }
        assert_eq!(out, b"ab      x");
    }

    #[test]
    fn other_bytes_pass_through_unchanged() {
        let s = tabs::DEFAULT_STOPS;
        assert_eq!(run_in(b"\x00\x01\xff\xfe\r\x0c\x0b\x7f", &s, usize::MAX), b"\x00\x01\xff\xfe\r\x0c\x0b\x7f");
        let all: Vec<u8> = (0..=255u8).filter(|&b| b != 9).collect();
        assert_eq!(run_in(&all, &s, usize::MAX), all);
    }

    #[test]
    fn control_characters_take_no_column_and_other_bytes_one() {
        let s = tabs::DEFAULT_STOPS;
        /* the C locale: 0xff is no character, so one column */
        assert_eq!(run_in(b"\x01\tx", &s, usize::MAX), b"\x01        x");
        assert_eq!(run_in(b"\r\tx", &s, usize::MAX), b"\r        x");
        assert_eq!(run_in(b"\xff\tx", &s, usize::MAX), b"\xff       x");
        assert_eq!(run_in(b"\x00\tx", &s, usize::MAX), b"\x00        x");
    }

    #[test]
    fn the_result_does_not_depend_on_how_the_input_is_cut() {
        let s = list(&[5, 9, 20]);
        let input: Vec<u8> = b"a\tbc\t\x08d\t\n\t\t\txyz\tq\r\n\x00\t".iter().cycle().take(5000).copied().collect();
        let whole = run_in(&input, &s, usize::MAX);
        /* each_char is fed whole slices here, so cut points are file edges */
        let mut out = Vec::new();
        let mut ex = Expand { stops: &s, col: 0 };
        let mut r = &input[..];
        tabs::each_char(&mut r, |c, wc| ex.char(c, wc, &mut out)).ok().unwrap();
        assert_eq!(out, whole);
        let mut out = Vec::new();
        let mut ex = Expand { stops: &s, col: 0 };
        for p in input.chunks(7) {
            tabs::each_char(&mut &p[..], |c, wc| ex.char(c, wc, &mut out)).ok().unwrap();
        }
        assert_eq!(out, whole);
    }

    #[test]
    fn a_huge_stop_does_not_wrap() {
        let s = Stops::Every(usize::MAX / 2);
        let mut ex = Expand { stops: &s, col: usize::MAX - 3 };
        let mut out = Vec::new();
        assert!(ex.char(b"\t", Some(9), &mut out).is_ok());
        /* the stop is 2 * (MAX / 2), two columns on */
        assert_eq!(ex.col, usize::MAX - 1);
        assert_eq!(out, b"  ");
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
        let mut ex = Expand { stops: &s, col: 0 };
        assert!(ex.char(b"\t", Some(9), &mut Full).is_err());
        assert!(ex.char(b"a", Some(97), &mut Full).is_err());
    }
}
