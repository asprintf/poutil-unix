/* SPDX-License-Identifier: GPL-2.0-only */
/* cmp - compare two files. */

use posix::getopt::{Getopt, Opt};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;

const SYNOPSIS: &str = "cmp [-l|-s] file1 file2";
const BLOCK: usize = 1 << 18;

#[derive(Clone, Copy, Default)]
struct Opts {
    /* -l: every differing byte, byte number in decimal, bytes in octal */
    long: bool,
    /* -s: no output, only the exit status */
    silent: bool,
}

enum Fail {
    /* which input (1 or 2) could not be read */
    Read(u8, io::Error),
    Write(io::Error),
}

/* What the comparison found. */
struct Report {
    /* the files are not identical */
    differ: bool,
    /* one file ended while the other went on: the shorter one (1 or 2) */
    short: Option<u8>,
    /* how many bytes of each file were looked at: the offset to leave the inputs at */
    processed: u64,
}

/* Read until buf is full or the input ends; a short count means the end. */
fn fill<R: Read>(r: &mut R, buf: &mut [u8], which: u8) -> Result<usize, Fail> {
    let mut n = 0;
    while n < buf.len() {
        match posix::read_block(r, &mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) => return Err(Fail::Read(which, e)),
        }
    }
    Ok(n)
}

fn out<W: Write>(w: &mut W, parts: &[&[u8]]) -> Result<(), Fail> {
    for p in parts {
        w.write_all(p).map_err(Fail::Write)?;
    }
    Ok(())
}

/* Append v in decimal to line. */
fn decimal(line: &mut Vec<u8>, mut v: u64) {
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    line.extend_from_slice(&tmp[i..]);
}

/* Append a byte in octal to line (no leading zeros, "0" for zero). */
fn octal(line: &mut Vec<u8>, v: u8) {
    if v >= 0o100 {
        line.push(b'0' + (v >> 6));
    }
    if v >= 0o10 {
        line.push(b'0' + ((v >> 3) & 7));
    }
    line.push(b'0' + (v & 7));
}

/*
 * The -l lines for one block: "byte-number octal octal" for every position
 * where a and b differ; off is the number of bytes before the block. Equal
 * stretches are skipped 64 bytes at a time with memcmp, and the lines are
 * built by hand in one buffer: this is the hot loop of a file that differs
 * everywhere.
 */
fn list<W: Write>(w: &mut W, a: &[u8], b: &[u8], off: u64) -> Result<(), Fail> {
    const STEP: usize = 64;
    let mut line: Vec<u8> = Vec::with_capacity(48);
    let mut i = 0;
    while i < a.len() {
        let end = (i + STEP).min(a.len());
        if a[i..end] == b[i..end] {
            i = end;
            continue;
        }
        for j in i..end {
            if a[j] != b[j] {
                line.clear();
                decimal(&mut line, off + j as u64 + 1);
                line.push(b' ');
                octal(&mut line, a[j]);
                line.push(b' ');
                octal(&mut line, b[j]);
                line.push(b'\n');
                w.write_all(&line).map_err(Fail::Write)?;
            }
        }
        i = end;
    }
    Ok(())
}

/*
 * Compare a and b block by block. Without -l the first difference ends it,
 * and it is reported with the line number of file1 (1 + the newlines before
 * the byte); with -l every differing byte is listed; with -s nothing is
 * written. Where one file is a prefix of the other, the shorter one is
 * named in the report and main says so on standard error.
 */
fn cmp<A: Read, B: Read, W: Write>(a: &mut A, b: &mut B, w: &mut W, names: [&[u8]; 2], o: &Opts) -> Result<Report, Fail> {
    let mut ba = vec![0u8; BLOCK];
    let mut bb = vec![0u8; BLOCK];
    /* bytes in the blocks already compared, and the line number they lead to */
    let mut off: u64 = 0;
    let mut line: u64 = 1;
    let mut differ = false;
    loop {
        let na = fill(a, &mut ba, 1)?;
        let nb = fill(b, &mut bb, 2)?;
        let n = na.min(nb);
        if ba[..n] != bb[..n] {
            differ = true;
            if o.silent || !o.long {
                /* the first difference decides */
                let i = ba[..n].iter().zip(&bb[..n]).position(|(x, y)| x != y).unwrap_or(0);
                if !o.silent {
                    let at = off + i as u64 + 1;
                    let ln = line + posix::count_byte(b'\n', &ba[..i]);
                    out(w, &[names[0], b" ", names[1], format!(" differ: char {at}, line {ln}\n").as_bytes()])?;
                    w.flush().map_err(Fail::Write)?;
                }
                return Ok(Report { differ, short: None, processed: off + i as u64 + 1 });
            }
            list(w, &ba[..n], &bb[..n], off)?;
        }
        if !o.long && !o.silent {
            line += posix::count_byte(b'\n', &ba[..n]);
        }
        off += n as u64;
        if na != nb {
            w.flush().map_err(Fail::Write)?;
            return Ok(Report { differ: true, short: Some(if na < nb { 1 } else { 2 }), processed: off });
        }
        if n < BLOCK {
            w.flush().map_err(Fail::Write)?;
            return Ok(Report { differ, short: None, processed: off });
        }
    }
}

/* Where a descriptor stands, if it can seek (a pipe or terminal cannot). */
fn tell(f: &impl AsRawFd) -> Option<i64> {
    let p = unsafe { libc::lseek(f.as_raw_fd(), 0, libc::SEEK_CUR) };
    if p < 0 { None } else { Some(p as i64) }
}

/* Leave a seekable input just past the last byte looked at, as the standard asks. */
fn seek_to(f: &impl AsRawFd, start: Option<i64>, processed: u64) {
    if let Some(s) = start {
        if let Ok(n) = i64::try_from(processed) {
            if let Some(p) = s.checked_add(n) {
                unsafe { libc::lseek(f.as_raw_fd(), p, libc::SEEK_SET) };
            }
        }
    }
}

/* An error is exit status 2: 1 means "different". */
fn die(prog: &[u8], msg: &str) -> ! {
    posix::warn(prog, msg);
    std::process::exit(2)
}

fn open(prog: &[u8], name: &[u8]) -> posix::Input {
    match posix::open_input(name) {
        Ok(h) => h,
        Err(e) => {
            posix::warn_path(prog, name, &posix::strerror(&e));
            std::process::exit(2);
        }
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut o = Opts::default();
    let mut g = Getopt::new(&args, b"ls");
    while let Some(opt) = g.next() {
        match opt {
            Opt::Flag(b'l') => o.long = true,
            Opt::Flag(b's') => o.silent = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    /* the synopsis makes -l and -s exclusive */
    if o.long && o.silent {
        posix::usage(SYNOPSIS);
    }
    let files = g.operands();
    if files.len() != 2 {
        posix::usage(SYNOPSIS);
    }
    let mut a = open(&prog, &files[0]);
    let mut b = open(&prog, &files[1]);
    let (sa, sb) = (tell(&*a), tell(&*b));
    let mut w = io::BufWriter::with_capacity(1 << 16, posix::Stdout::new());
    let names = [files[0].as_slice(), files[1].as_slice()];
    let r = match cmp(&mut a, &mut b, &mut w, names, &o) {
        Ok(r) => r,
        Err(Fail::Read(n, e)) => {
            let name = String::from_utf8_lossy(names[usize::from(n) - 1]).into_owned();
            die(&prog, &format!("{}: {}", name, posix::strerror(&e)))
        }
        Err(Fail::Write(e)) => die(&prog, &format!("write error: {}", posix::strerror(&e))),
    };
    seek_to(&*a, sa, r.processed);
    seek_to(&*b, sb, r.processed);
    if let (Some(n), false) = (r.short, o.silent) {
        let mut e = io::stderr().lock();
        let _ = e.write_all(b"cmp: EOF on ");
        let _ = e.write_all(names[usize::from(n) - 1]);
        let _ = e.write_all(b"\n");
    }
    std::process::exit(if r.differ { 1 } else { 0 });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with(a: &[u8], b: &[u8], o: Opts) -> (String, Report) {
        let mut out = Vec::new();
        let mut ra = io::Cursor::new(a);
        let mut rb = io::Cursor::new(b);
        let r = match cmp(&mut ra, &mut rb, &mut out, [b"f1", b"f2"], &o) {
            Ok(r) => r,
            Err(_) => panic!("cmp failed"),
        };
        (String::from_utf8_lossy(&out).into_owned(), r)
    }

    fn run(flags: &str, a: &[u8], b: &[u8]) -> (String, Report) {
        run_with(a, b, Opts { long: flags.contains('l'), silent: flags.contains('s') })
    }

    #[test]
    fn identical_files_say_nothing() {
        for flags in ["", "l", "s"] {
            let (o, r) = run(flags, b"abc\ndef\n", b"abc\ndef\n");
            assert_eq!(o, "");
            assert!(!r.differ && r.short.is_none());
            assert_eq!(r.processed, 8);
        }
        assert!(!run("", b"", b"").1.differ);
    }

    #[test]
    fn the_first_difference_has_a_byte_and_a_line_number() {
        let (o, r) = run("", b"abcd\nefgh\n", b"abcd\nefXh\n");
        assert_eq!(o, "f1 f2 differ: char 8, line 2\n");
        assert!(r.differ);
        /* the offset is just past the differing byte */
        assert_eq!(r.processed, 8);
        assert_eq!(run("", b"abcd", b"Xbcd").0, "f1 f2 differ: char 1, line 1\n");
        /* the newline itself differs: it is still on the line it ends */
        assert_eq!(run("", b"a\nb", b"a b").0, "f1 f2 differ: char 2, line 1\n");
    }

    #[test]
    fn long_lists_every_difference_in_octal() {
        let (o, r) = run("l", b"abcabc", b"aXcabY");
        assert_eq!(o, "2 142 130\n6 143 131\n");
        assert!(r.differ);
        assert_eq!(run("l", b"\x00\xff", b"\x01\x80").0, "1 0 1\n2 377 200\n");
    }

    #[test]
    fn silent_writes_nothing_but_still_differs() {
        let (o, r) = run("s", b"abc", b"abd");
        assert_eq!(o, "");
        assert!(r.differ);
        assert!(run("s", b"abc", b"ab").1.differ);
    }

    #[test]
    fn a_prefix_is_a_difference_and_names_the_shorter_file() {
        let (o, r) = run("", b"abc", b"abcdef");
        assert_eq!(o, "");
        assert!(r.differ);
        assert_eq!(r.short, Some(1));
        assert_eq!(r.processed, 3);
        assert_eq!(run("", b"abcdef", b"abc").1.short, Some(2));
        assert_eq!(run("", b"", b"x").1.short, Some(1));
        assert_eq!(run("s", b"abc", b"abcdef").1.short, Some(1));
    }

    #[test]
    fn long_reports_the_differences_and_then_the_end() {
        let (o, r) = run("l", b"aXc", b"abcdef");
        assert_eq!(o, "2 130 142\n");
        assert_eq!(r.short, Some(1));
        assert!(r.differ);
        let (o, r) = run("l", b"abc", b"abcdef");
        assert_eq!(o, "");
        assert_eq!(r.short, Some(1));
    }

    #[test]
    fn difference_before_the_end_wins_over_a_prefix() {
        let (o, r) = run("", b"abX", b"abcdef");
        assert_eq!(o, "f1 f2 differ: char 3, line 1\n");
        assert_eq!(r.short, None);
    }

    #[test]
    fn lines_count_the_newlines_of_file1() {
        /* file2 has no newlines at all: the line is file1's */
        assert_eq!(run("", b"\n\n\nabc", b"\n\n\nabX").0, "f1 f2 differ: char 6, line 4\n");
    }

    #[test]
    fn binary_data_and_nul_bytes() {
        assert_eq!(run("", b"a\0b", b"a\0c").0, "f1 f2 differ: char 3, line 1\n");
        assert_eq!(run("l", b"\0\0", b"\0\x01").0, "2 0 1\n");
    }

    #[test]
    fn differences_across_block_boundaries() {
        let n = BLOCK * 2 + 100;
        let mut a: Vec<u8> = (0..n).map(|i| if i % 100 == 99 { b'\n' } else { b'x' }).collect();
        let mut b = a.clone();
        b[BLOCK + 5] = b'y';
        let (o, r) = run("", &a, &b);
        assert_eq!(o, format!("f1 f2 differ: char {}, line {}\n", BLOCK + 6, (BLOCK + 5) / 100 + 1));
        assert_eq!(r.processed, (BLOCK + 6) as u64);
        /* the last byte of the first block and the first of the second */
        b[BLOCK + 5] = b'x';
        b[BLOCK - 1] = b'y';
        b[BLOCK] = b'y';
        assert_eq!(run("l", &a, &b).0, format!("{} 170 171\n{} 170 171\n", BLOCK, BLOCK + 1));
        /* an exact multiple of the block size, equal */
        a.truncate(BLOCK * 2);
        let (o, r) = run("", &a, &a);
        assert_eq!(o, "");
        assert!(!r.differ);
        assert_eq!(r.processed, (BLOCK * 2) as u64);
        /* an exact multiple of the block size, one file longer by one byte */
        let mut c = a.clone();
        c.push(b'z');
        assert_eq!(run("", &a, &c).1.short, Some(1));
    }

    #[test]
    fn a_read_error_names_the_input() {
        struct Bad;
        impl Read for Bad {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from_raw_os_error(libc::EIO))
            }
        }
        let mut out = Vec::new();
        let mut ra = io::Cursor::new(&b"a\n"[..]);
        match cmp(&mut ra, &mut Bad, &mut out, [b"f1", b"f2"], &Opts::default()) {
            Err(Fail::Read(2, _)) => {}
            _ => panic!("expected a read error on input 2"),
        }
    }
    #[test]
    fn the_chunked_list_agrees_with_a_byte_loop() {
        /* every length around the 64-byte step, every position of one difference, and a few with several */
        for len in (0..70usize).chain([127, 128, 129, 200]) {
            let a: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            let mut cases: Vec<Vec<u8>> = vec![a.clone()];
            for pos in 0..len {
                let mut b = a.clone();
                b[pos] ^= 0x81;
                cases.push(b);
            }
            if len > 3 {
                let mut b = a.clone();
                b[0] ^= 1;
                b[len / 2] ^= 2;
                b[len - 1] ^= 0xff;
                cases.push(b);
            }
            for b in cases {
                for off in [0u64, 262_144, 10_000_000_000, u64::MAX / 4] {
                    let mut got = Vec::new();
                    assert!(list(&mut got, &a, &b, off).is_ok());
                    let mut want = String::new();
                    for i in 0..len {
                        if a[i] != b[i] {
                            want.push_str(&format!("{} {:o} {:o}\n", off + i as u64 + 1, a[i], b[i]));
                        }
                    }
                    assert_eq!(String::from_utf8_lossy(&got), want, "len {len} off {off}");
                }
            }
        }
    }

}
