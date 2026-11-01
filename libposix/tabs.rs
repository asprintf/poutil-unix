/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * What expand and unexpand share: the -t tablist, the width of a character
 * in column positions, and a reader that hands out one character at a time.
 *
 * Columns are counted from 0 here: a tab stop N puts the next character at
 * offset N, which is the (N+1)th column position of XBD Column Position.
 */

use std::io::{self, Read};

/* Where the <tab> characters are set. */
#[derive(Debug, PartialEq, Eq)]
pub enum Stops {
    /* every n columns, without end (one number, or no -t at all) */
    Every(usize),
    /* these columns only: two or more numbers, strictly ascending */
    List(Vec<usize>),
}

/* Why a tablist was refused; the text is the diagnostic. */
#[derive(Debug, PartialEq, Eq)]
pub enum BadStops {
    Empty,
    NotANumber,
    Zero,
    TooLarge,
    NotAscending,
}

impl BadStops {
    pub fn message(&self) -> &'static str {
        match self {
            BadStops::Empty => "empty tab stop in the list",
            BadStops::NotANumber => "tab stops must be positive decimal integers",
            BadStops::Zero => "tab stops must be greater than zero",
            BadStops::TooLarge => "tab stop is too large",
            BadStops::NotAscending => "tab stops must be in strictly ascending order",
        }
    }
}

/* The default of expand and unexpand: a tab stop every 8 columns. */
pub const DEFAULT_STOPS: Stops = Stops::Every(8);

impl Stops {
    /*
     * The option-argument of -t: one positive decimal integer, or a list of
     * them in strictly ascending order, separated by <comma> or <blank>
     * characters (one separator between two numbers; an empty item is an
     * error). Only the space and tab characters count as <blank> here: the
     * argument is a list of digits, and any other character in it is an
     * error anyway.
     */
    pub fn parse(arg: &[u8]) -> Result<Stops, BadStops> {
        let mut list: Vec<usize> = Vec::new();
        for item in arg.split(|&c| c == b',' || c == b' ' || c == b'\t') {
            if item.is_empty() {
                return Err(BadStops::Empty);
            }
            let n = crate::parse_decimal(item).ok_or(BadStops::NotANumber)?;
            if n == 0 {
                return Err(BadStops::Zero);
            }
            /* half of the range: column arithmetic below cannot wrap */
            if n > (usize::MAX / 2) as u64 {
                return Err(BadStops::TooLarge);
            }
            let n = n as usize;
            if list.last().is_some_and(|&p| n <= p) {
                return Err(BadStops::NotAscending);
            }
            list.push(n);
        }
        if list.len() == 1 {
            Ok(Stops::Every(list[0]))
        } else {
            Ok(Stops::List(list))
        }
    }

    /*
     * The first tab stop after column col, or None if there is none (past
     * the last stop of a list). Columns saturate instead of wrapping.
     */
    pub fn next(&self, col: usize) -> Option<usize> {
        match self {
            Stops::Every(n) => Some(col.saturating_add(n - col % n)),
            Stops::List(l) => l.iter().copied().find(|&s| s > col),
        }
    }

    /* The last stop of a list: from there on nothing is converted. */
    pub fn last(&self) -> Option<usize> {
        match self {
            Stops::Every(_) => None,
            Stops::List(l) => l.last().copied(),
        }
    }
}

extern "C" {
    fn wcwidth(wc: libc::wchar_t) -> libc::c_int;
}

/*
 * The width in column positions of one character: wc is its wide value, or
 * None for a byte that is no character of this locale. The control
 * characters of ASCII (and DEL) take no column, as on a terminal; a byte
 * that is no character takes one, as a replacement symbol would; the rest
 * is what wcwidth(3) says, with 1 where it does not know the character (in
 * the C locale that is every byte above 0x7f, so UTF-8 text is counted a
 * column per byte, which is what the other implementations do).
 */
pub fn col_width(wc: Option<u32>) -> usize {
    match wc {
        None => 1,
        Some(c) if c < 0x20 || c == 0x7f => 0,
        Some(c) => {
            let w = unsafe { wcwidth(c as libc::wchar_t) };
            if w < 0 {
                1
            } else {
                w as usize
            }
        }
    }
}

/* Why copying a file stopped. */
#[derive(Debug)]
pub enum Fail {
    Read(io::Error),
    Write(io::Error),
}

/* Bytes kept between two reads: a character cut short by the end of a read. */
const BLOCK: usize = 65536;

/* What each_event reports. */
pub enum Ev<'a> {
    /* the next character: its bytes and its wide value (None: no character) */
    Char(&'a [u8], Option<u32>),
    /* everything read so far has been passed on; the next read may wait */
    Idle,
}

/*
 * Read r to its end and report every character in it, in order, to f, and
 * an Idle after each read, so that the caller can flush its output before
 * the next read blocks (a pipe or a terminal is served as the data arrives,
 * not at its end). A character cut in two by the end of a read waits for
 * the next one. A read error stops the copy; so does an error from f.
 */
pub fn each_event<R, F>(r: &mut R, mut f: F) -> Result<(), Fail>
where
    R: Read,
    F: FnMut(Ev) -> io::Result<()>,
{
    let mut buf = vec![0u8; BLOCK];
    let mut have = 0;
    loop {
        let eof = match crate::read_block(r, &mut buf[have..]) {
            Ok(0) => true,
            Ok(n) => {
                have += n;
                false
            }
            Err(e) => return Err(Fail::Read(e)),
        };
        let mut i = 0;
        while i < have {
            let step = if eof {
                Some(crate::mb::char_at(&buf[i..have]))
            } else {
                crate::mb::char_at_stream(&buf[i..have])
            };
            let Some((n, wc)) = step else { break };
            f(Ev::Char(&buf[i..i + n], wc)).map_err(Fail::Write)?;
            i += n;
        }
        if eof {
            return Ok(());
        }
        f(Ev::Idle).map_err(Fail::Write)?;
        /* the undecoded rest, a few bytes at most, goes to the front */
        buf.copy_within(i..have, 0);
        have -= i;
    }
}

/* each_event for a caller that wants the characters only. */
pub fn each_char<R, F>(r: &mut R, mut f: F) -> Result<(), Fail>
where
    R: Read,
    F: FnMut(&[u8], Option<u32>) -> io::Result<()>,
{
    each_event(r, |ev| match ev {
        Ev::Char(c, wc) => f(c, wc),
        Ev::Idle => Ok(()),
    })
}

/* Append n space characters to w without building them all in memory. */
pub fn write_spaces<W: io::Write>(w: &mut W, mut n: usize) -> io::Result<()> {
    const SPACES: [u8; 64] = [b' '; 64];
    while n > 0 {
        let k = n.min(SPACES.len());
        w.write_all(&SPACES[..k])?;
        n -= k;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> Stops {
        Stops::parse(s.as_bytes()).unwrap()
    }

    fn bad(s: &str) -> BadStops {
        Stops::parse(s.as_bytes()).unwrap_err()
    }

    #[test]
    fn one_number_is_a_spacing() {
        assert_eq!(ok("8"), Stops::Every(8));
        assert_eq!(ok("1"), Stops::Every(1));
        assert_eq!(ok("08"), Stops::Every(8));
        assert_eq!(ok("2046"), Stops::Every(2046));
    }

    #[test]
    fn several_numbers_are_a_list() {
        assert_eq!(ok("3,10"), Stops::List(vec![3, 10]));
        assert_eq!(ok("3 10"), Stops::List(vec![3, 10]));
        assert_eq!(ok("3\t10"), Stops::List(vec![3, 10]));
        assert_eq!(ok("1,2,3"), Stops::List(vec![1, 2, 3]));
        assert_eq!(ok("9 12,21"), Stops::List(vec![9, 12, 21]));
        assert_eq!(ok("1,2046"), Stops::List(vec![1, 2046]));
    }

    #[test]
    fn refused_lists() {
        assert_eq!(bad(""), BadStops::Empty);
        assert_eq!(bad(","), BadStops::Empty);
        assert_eq!(bad("3,"), BadStops::Empty);
        assert_eq!(bad(",3"), BadStops::Empty);
        assert_eq!(bad("3,,6"), BadStops::Empty);
        assert_eq!(bad("3, 6"), BadStops::Empty);
        assert_eq!(bad("3 "), BadStops::Empty);
        assert_eq!(bad("0"), BadStops::Zero);
        assert_eq!(bad("00"), BadStops::Zero);
        assert_eq!(bad("3,0"), BadStops::Zero);
        assert_eq!(bad("-1"), BadStops::NotANumber);
        assert_eq!(bad("+3"), BadStops::NotANumber);
        assert_eq!(bad("3x"), BadStops::NotANumber);
        assert_eq!(bad("0x8"), BadStops::NotANumber);
        assert_eq!(bad("1.5"), BadStops::NotANumber);
        assert_eq!(bad("\u{663}"), BadStops::NotANumber);
        assert_eq!(bad("3,3"), BadStops::NotAscending);
        assert_eq!(bad("5,2"), BadStops::NotAscending);
        assert_eq!(bad("2,5,5"), BadStops::NotAscending);
        assert_eq!(bad("99999999999999999999999"), BadStops::TooLarge);
        assert_eq!(bad("18446744073709551615"), BadStops::TooLarge);
    }

    #[test]
    fn every_n_columns() {
        let s = Stops::Every(8);
        assert_eq!(s.next(0), Some(8));
        assert_eq!(s.next(7), Some(8));
        assert_eq!(s.next(8), Some(16));
        assert_eq!(s.next(9), Some(16));
        let s = Stops::Every(1);
        assert_eq!(s.next(0), Some(1));
        assert_eq!(s.next(41), Some(42));
        assert_eq!(s.last(), None);
    }

    #[test]
    fn a_list_ends() {
        let s = Stops::List(vec![3, 10]);
        assert_eq!(s.next(0), Some(3));
        assert_eq!(s.next(2), Some(3));
        assert_eq!(s.next(3), Some(10));
        assert_eq!(s.next(9), Some(10));
        assert_eq!(s.next(10), None);
        assert_eq!(s.next(500), None);
        assert_eq!(s.last(), Some(10));
    }

    #[test]
    fn huge_columns_saturate() {
        let s = Stops::Every(usize::MAX / 2);
        assert_eq!(s.next(usize::MAX - 1), Some(usize::MAX));
        assert_eq!(Stops::Every(8).next(usize::MAX), Some(usize::MAX));
    }

    #[test]
    fn widths_in_the_c_locale() {
        assert_eq!(col_width(Some(b'a' as u32)), 1);
        assert_eq!(col_width(Some(b' ' as u32)), 1);
        assert_eq!(col_width(Some(0)), 0);
        assert_eq!(col_width(Some(1)), 0);
        assert_eq!(col_width(Some(0x0d)), 0);
        assert_eq!(col_width(Some(0x1b)), 0);
        assert_eq!(col_width(Some(0x7f)), 0);
        /* above ASCII it is wcwidth's business; an unknown one takes a column */
        assert_eq!(col_width(Some(0x85)), 1);
        assert_eq!(col_width(Some(0xe9)), 1);
        assert_eq!(col_width(None), 1);
    }

    /* a reader that gives a few bytes per call */
    struct Slow<'a>(&'a [u8], usize);

    impl Read for Slow<'_> {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            let n = self.0.len().min(b.len()).min(self.1);
            b[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    fn chars<R: Read>(mut r: R) -> Vec<(Vec<u8>, Option<u32>)> {
        let mut v = Vec::new();
        each_char(&mut r, |c, w| {
            v.push((c.to_vec(), w));
            Ok(())
        })
        .unwrap();
        v
    }

    #[test]
    fn each_char_sees_every_byte_once_in_order() {
        let data: Vec<u8> = (0..=255u8).cycle().take(200_000).collect();
        for step in [1usize, 7, 4096, 70_000] {
            let got: Vec<u8> = chars(Slow(&data, step)).into_iter().flat_map(|(b, _)| b).collect();
            assert_eq!(got, data, "step {step}");
        }
        assert!(chars(Slow(b"", 1)).is_empty());
    }

    #[test]
    fn each_char_values_in_the_c_locale() {
        let got = chars(Slow(b"a\tb\n", 2));
        let vals: Vec<Option<u32>> = got.into_iter().map(|(_, w)| w).collect();
        assert_eq!(vals, vec![Some(97), Some(9), Some(98), Some(10)]);
    }

    #[test]
    fn exact_block_boundaries() {
        for len in [BLOCK - 1, BLOCK, BLOCK + 1, 2 * BLOCK, 2 * BLOCK + 17] {
            let data = vec![b'x'; len];
            assert_eq!(chars(Slow(&data, 1 << 20)).len(), len, "{len}");
        }
    }

    #[test]
    fn idle_comes_after_each_read_but_not_at_the_end() {
        let mut log = Vec::new();
        each_event(&mut Slow(b"abcde", 2), |ev| {
            match ev {
                Ev::Char(c, _) => log.push(c[0]),
                Ev::Idle => log.push(b'|'),
            }
            Ok(())
        })
        .ok()
        .unwrap();
        assert_eq!(log, b"ab|cd|e|");
        let mut n = 0;
        each_event(&mut Slow(b"", 2), |ev| {
            if matches!(ev, Ev::Idle) {
                n += 1;
            }
            Ok(())
        })
        .ok()
        .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn an_error_from_idle_is_a_write_error() {
        let r = each_event(&mut Slow(b"a", 1), |ev| match ev {
            Ev::Idle => Err(io::Error::from_raw_os_error(libc::EPIPE)),
            _ => Ok(()),
        });
        assert!(matches!(r, Err(Fail::Write(_))));
    }

    #[test]
    fn a_write_error_stops_the_copy_and_is_a_write_error() {
        let r = each_char(&mut &b"abc"[..], |_, _| Err(io::Error::from_raw_os_error(libc::ENOSPC)));
        assert!(matches!(r, Err(Fail::Write(_))));
    }

    struct Broken;

    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from_raw_os_error(libc::EIO))
        }
    }

    #[test]
    fn a_read_error_is_a_read_error() {
        assert!(matches!(each_char(&mut Broken, |_, _| Ok(())), Err(Fail::Read(_))));
    }

    #[test]
    fn spaces_are_written_in_pieces() {
        for n in [0usize, 1, 63, 64, 65, 1000] {
            let mut v = Vec::new();
            write_spaces(&mut v, n).unwrap();
            assert_eq!(v, vec![b' '; n]);
        }
    }
}
