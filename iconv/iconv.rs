/* SPDX-License-Identifier: GPL-2.0-only */
/* iconv - codeset conversion. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::iconv::{self, Converter, Stop};
use std::io::{self, Read, Write};

const SYNOPSIS: &str = "iconv [-cs] -f frommap -t tomap [file...]\n       iconv -f fromcode [-cs] [-t tocode] [file...]\n       iconv -t tocode [-cs] [-f fromcode] [file...]\n       iconv -l";

/* Codesets -l lists, if the system has them. */
const NAMES: &[&str] = &[
    "UTF-8", "UTF-16", "UTF-16BE", "UTF-16LE", "UTF-32", "UTF-32BE", "UTF-32LE", "UCS-2", "UCS-2BE", "UCS-2LE", "UCS-4",
    "UCS-4BE", "UCS-4LE", "ASCII", "ISO8859-1", "ISO8859-2", "ISO8859-3", "ISO8859-4", "ISO8859-5", "ISO8859-6",
    "ISO8859-7", "ISO8859-8", "ISO8859-9", "ISO8859-10", "ISO8859-11", "ISO8859-13", "ISO8859-14", "ISO8859-15",
    "ISO8859-16", "CP437", "CP850", "CP852", "CP866", "CP1250", "CP1251", "CP1252", "CP1253", "CP1254", "CP1255",
    "CP1256", "CP1257", "CP1258", "KOI8-R", "KOI8-U", "MACINTOSH", "EUC-JP", "SHIFT_JIS", "ISO-2022-JP", "EUC-KR",
    "GB2312", "GBK", "GB18030", "BIG5", "EUC-TW",
];

/* Why one file stopped. */
#[derive(Debug)]
enum Fail {
    Read(io::Error),
    Write(io::Error),
    /* a byte offset in the file */
    Invalid(u64),
    Incomplete(u64),
}

struct Job {
    conv: Converter,
    from: Vec<u8>,
    to: Vec<u8>,
    omit: bool,
    /* under -c: the conversion now skips what it cannot convert */
    skipping: bool,
    /* something was left out under -c */
    omitted: bool,
}

impl Job {
    fn new(from: &[u8], to: &[u8], omit: bool) -> Result<Job, String> {
        let conv = Converter::open(to, from)
            .ok_or_else(|| format!("cannot convert from {} to {}", String::from_utf8_lossy(from), String::from_utf8_lossy(to)))?;
        Ok(Job { conv, from: from.to_vec(), to: to.to_vec(), omit, skipping: false, omitted: false })
    }

    /*
     * -c, at the first thing that cannot be converted: end the output as the
     * strict conversion would and go on with the system's own way of
     * dropping such input (the shift state of the input starts again).
     */
    fn start_skipping(&mut self, out: &mut Vec<u8>) {
        self.omitted = true;
        self.skipping = true;
        self.conv.finish(out);
        if let Some(c) = Converter::open(&[self.to.as_slice(), b"//IGNORE"].concat(), &self.from) {
            self.conv = c;
        }
    }

    /*
     * Convert data: the bytes used, and whether the end cuts a character.
     * Err is the offset of the first byte that cannot be converted (not -c).
     */
    fn chunk(&mut self, data: &[u8], out: &mut Vec<u8>) -> Result<(usize, bool), usize> {
        let mut pos = 0;
        loop {
            let (n, stop) = self.conv.convert(&data[pos..], out);
            pos += n;
            match stop {
                Stop::Done => return Ok((pos, false)),
                Stop::Incomplete => return Ok((pos, true)),
                Stop::Invalid if !self.omit => return Err(pos),
                Stop::Invalid if !self.skipping => self.start_skipping(out),
                /* the system dropped it and says so at the end of the input */
                Stop::Invalid if pos >= data.len() => return Ok((pos, false)),
                /* no progress: drop one byte */
                Stop::Invalid if n == 0 => pos += 1,
                Stop::Invalid => {}
            }
        }
    }

    /* Convert one input to w. */
    fn file<R: Read, W: Write>(&mut self, r: &mut R, w: &mut W) -> Result<(), Fail> {
        let mut buf = vec![0u8; 65536];
        let mut out: Vec<u8> = Vec::new();
        let (mut have, mut off) = (0usize, 0u64);
        let result = loop {
            let eof = match posix::read_block(r, &mut buf[have..]) {
                Ok(0) => true,
                Ok(n) => {
                    have += n;
                    false
                }
                Err(e) => break Err(Fail::Read(e)),
            };
            let (used, cut) = match self.chunk(&buf[..have], &mut out) {
                Ok(x) => x,
                Err(at) => {
                    let _ = w.write_all(&out);
                    return Err(Fail::Invalid(off + at as u64));
                }
            };
            if let Err(e) = w.write_all(&out) {
                return Err(Fail::Write(e));
            }
            out.clear();
            off += used as u64;
            if eof {
                if cut && used < have {
                    break Err(Fail::Incomplete(off));
                }
                break Ok(());
            }
            buf.copy_within(used..have, 0);
            have -= used;
        };
        /* the sequence that ends the output, and a fresh start for the next file */
        let mut tail = Vec::new();
        self.conv.finish(&mut tail);
        self.conv.reset();
        if result.is_ok() {
            w.write_all(&tail).map_err(Fail::Write)?;
        }
        result
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let (mut omit, mut quiet, mut list) = (false, false, false);
    let (mut from, mut to): (Option<Vec<u8>>, Option<Vec<u8>>) = (None, None);
    let mut g = Getopt::new(&args, b"csf:t:l");
    for o in &mut g {
        match o {
            Opt::Flag(b'c') => omit = true,
            Opt::Flag(b's') => quiet = true,
            Opt::Flag(b'l') => list = true,
            Opt::Arg(b'f', v) => from = Some(v),
            Opt::Arg(b't', v) => to = Some(v),
            _ => posix::usage(SYNOPSIS),
        }
    }
    let files = g.operands();
    if list {
        if omit || quiet || from.is_some() || to.is_some() || !files.is_empty() {
            posix::usage(SYNOPSIS);
        }
        let mut text = Vec::new();
        for n in NAMES {
            if iconv::is_supported(&posix::cstr(n.as_bytes())) {
                text.extend_from_slice(n.as_bytes());
                text.push(b'\n');
            }
        }
        posix::write_stdout(&prog, &text);
        return;
    }
    for c in [&from, &to].into_iter().flatten() {
        if c.contains(&b'/') {
            posix::fail(&prog, "charmap files are not supported");
        }
    }
    let from = from.unwrap_or_else(posix::locale::codeset);
    let to = to.unwrap_or_else(posix::locale::codeset);
    let mut job = Job::new(&from, &to, omit).unwrap_or_else(|m| posix::fail(&prog, &m));
    let mut out = posix::Stdout::new();
    let stdin = [b"-".to_vec()];
    let names: &[Vec<u8>] = if files.is_empty() { &stdin } else { files };
    let mut status = 0;
    for name in names {
        let shown: &[u8] = if name == b"-" { b"standard input" } else { name };
        let mut input = match posix::open_input(name) {
            Ok(f) => f,
            Err(e) => {
                posix::warn_path(&prog, name, &posix::strerror(&e));
                status = 1;
                continue;
            }
        };
        match job.file(&mut input, &mut out) {
            Ok(()) => {}
            Err(Fail::Read(e)) => {
                posix::warn_path(&prog, shown, &posix::strerror(&e));
                status = 1;
            }
            Err(Fail::Write(e)) => posix::fail(&prog, &format!("write error: {}", posix::strerror(&e))),
            Err(f) => {
                if !quiet {
                    let (what, at) = match f {
                        Fail::Invalid(at) => ("invalid or unconvertible input", at),
                        Fail::Incomplete(at) => ("incomplete character at the end of the input", at),
                        _ => unreachable!(),
                    };
                    posix::warn_path(&prog, shown, &format!("{what} at byte {at}"));
                }
                std::process::exit(1);
            }
        }
    }
    if job.omitted {
        if !quiet {
            posix::warn(&prog, "characters that cannot be converted were omitted");
        }
        status = 1;
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    /* a //IGNORE conversion makes the others of the process skip too on macOS: one test at a time */
    static ONE: Mutex<()> = Mutex::new(());

    fn guard() -> MutexGuard<'static, ()> {
        ONE.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn run(from: &str, to: &str, omit: bool, input: &[u8]) -> (Vec<u8>, Result<(), Fail>, bool) {
        let mut job = Job::new(from.as_bytes(), to.as_bytes(), omit).unwrap();
        let mut out = Vec::new();
        let mut rd = input;
        let r = job.file(&mut rd, &mut out);
        (out, r, job.omitted)
    }

    /* a reader that gives one byte at a time */
    struct Slow<'a>(&'a [u8]);
    impl Read for Slow<'_> {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            b[0] = self.0[0];
            self.0 = &self.0[1..];
            Ok(1)
        }
    }

    #[test]
    fn plain_conversion() {
        let _g = guard();
        let (o, r, om) = run("ISO8859-1", "UTF-8", false, b"caf\xe9\n");
        assert_eq!((o, r.is_ok(), om), ("caf\u{e9}\n".as_bytes().to_vec(), true, false));
        assert_eq!(run("UTF-8", "UTF-8", false, b"").0, b"");
        assert_eq!(run("UTF-8", "UTF-16BE", false, b"hi").0, b"\0h\0i");
    }

    #[test]
    fn invalid_input_stops_with_the_offset() {
        let _g = guard();
        let (o, r, _) = run("UTF-8", "UTF-8", false, b"abc\xffdef");
        assert_eq!(o, b"abc");
        assert!(matches!(r, Err(Fail::Invalid(3))));
        /* nothing is converted past it, in a later block either */
        let big: Vec<u8> = b"x".repeat(70_000).into_iter().chain(*b"\xffy").collect();
        let (o, r, _) = run("UTF-8", "UTF-8", false, &big);
        assert_eq!(o.len(), 70_000);
        assert!(matches!(r, Err(Fail::Invalid(70_000))));
    }

    #[test]
    fn unmappable_characters_are_invalid_too() {
        let _g = guard();
        let (o, r, _) = run("UTF-8", "ASCII", false, "ab\u{e9}cd".as_bytes());
        assert_eq!(o, b"ab");
        assert!(matches!(r, Err(Fail::Invalid(2))));
    }

    #[test]
    fn incomplete_character_at_the_end() {
        let _g = guard();
        let (o, r, _) = run("UTF-8", "UTF-16BE", false, b"ab\xc3");
        assert_eq!(o, b"\0a\0b");
        assert!(matches!(r, Err(Fail::Incomplete(2))));
        let (_, r, _) = run("UTF-8", "UTF-16BE", true, b"ab\xc3");
        assert!(matches!(r, Err(Fail::Incomplete(2))));
    }

    #[test]
    fn omit_drops_what_cannot_be_converted_and_says_so() {
        let _g = guard();
        let (o, r, om) = run("UTF-8", "UTF-8", true, b"a\xffb\n");
        assert_eq!((o, r.is_ok(), om), (b"ab\n".to_vec(), true, true));
        let (o, r, om) = run("UTF-8", "ASCII", true, "a\u{e9}b\n".as_bytes());
        assert_eq!((o, r.is_ok(), om), (b"ab\n".to_vec(), true, true));
        /* nothing to drop: not reported */
        assert!(!run("UTF-8", "ASCII", true, b"abc").2);
    }

    #[test]
    fn omit_finds_an_error_in_a_later_block() {
        let _g = guard();
        let big: Vec<u8> = b"x".repeat(70_000).into_iter().chain(*b"\xffy").collect();
        let (o, r, om) = run("UTF-8", "UTF-8", true, &big);
        assert_eq!((o.len(), r.is_ok(), om), (70_001, true, true));
    }

    #[test]
    fn characters_cut_by_a_read() {
        let _g = guard();
        let mut job = Job::new(b"UTF-8", b"UTF-16BE", false).unwrap();
        let mut out = Vec::new();
        job.file(&mut Slow("a\u{20ac}\u{1f600}".as_bytes()), &mut out).unwrap();
        assert_eq!(out, b"\0a \xac\xd8\x3d\xde\x00");
    }

    #[test]
    fn state_starts_again_with_each_file() {
        let _g = guard();
        let mut job = Job::new(b"UTF-8", b"ISO-2022-JP", false).unwrap();
        let mut out = Vec::new();
        job.file(&mut "\u{3042}".as_bytes(), &mut out).unwrap();
        job.file(&mut "\u{3042}".as_bytes(), &mut out).unwrap();
        assert_eq!(out, b"\x1b$B$\"\x1b(B\x1b$B$\"\x1b(B");
    }

    #[test]
    fn unknown_codeset() {
        let _g = guard();
        assert!(Job::new(b"NO-SUCH", b"UTF-8", false).is_err());
        assert!(Job::new(b"UTF-8", b"NO-SUCH", true).is_err());
    }
}
