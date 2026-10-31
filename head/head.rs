/* SPDX-License-Identifier: GPL-2.0-only */
/* head - copy the first part of files. */

use posix::getopt::{Getopt, Opt};
use std::io::{self, Read, Write};

const SYNOPSIS: &str = "head [-n number] [file...]";
const BLOCK: usize = 65536;

enum Fail {
    Read(io::Error),
    Write(io::Error),
}

/*
 * Copy the first `lines` lines of r to w and stop. Memory use does not
 * depend on the input: it is read block by block, and reading ends at the
 * block that holds the last wanted newline, so "head" of a huge file or of
 * an endless pipe is quick.
 *
 * Returns how many bytes were read beyond the cut. A caller whose input is
 * seekable moves the file offset back by that much, so the next process
 * sharing the open file continues right after the last line copied.
 */
fn head<R: Read, W: Write>(r: &mut R, w: &mut W, mut lines: u64) -> Result<u64, Fail> {
    if lines == 0 {
        return Ok(0);
    }
    let mut buf = vec![0u8; BLOCK];
    loop {
        let n = posix::read_block(r, &mut buf).map_err(Fail::Read)?;
        if n == 0 {
            return Ok(0);
        }
        let blk = &buf[..n];
        let mut cut = 0;
        while lines > 0 {
            match posix::find_byte(b'\n', &blk[cut..]) {
                Some(i) => {
                    cut += i + 1;
                    lines -= 1;
                }
                None => {
                    cut = n;
                    break;
                }
            }
        }
        w.write_all(&blk[..cut]).map_err(Fail::Write)?;
        if lines == 0 {
            return Ok((n - cut) as u64);
        }
    }
}

/* "\n==> %s <==\n", without the first newline for the first header. */
fn header(first: bool, name: &[u8]) -> Vec<u8> {
    let mut h = Vec::with_capacity(name.len() + 10);
    if !first {
        h.push(b'\n');
    }
    h.extend_from_slice(b"==> ");
    h.extend_from_slice(name);
    h.extend_from_slice(b" <==\n");
    h
}

/* Give back the bytes that were read but not used (seekable input only). */
fn rewind(fd: i32, unused: u64) {
    if unused > 0 {
        unsafe {
            libc::lseek(fd, -(unused.min(i64::MAX as u64) as i64) as libc::off_t, libc::SEEK_CUR);
        }
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut lines: u64 = 10;
    let mut g = Getopt::new(&args, b"n:");
    while let Some(o) = g.next() {
        match o {
            Opt::Arg(b'n', v) => match posix::parse_decimal(&v) {
                Some(n) => lines = n,
                None => {
                    posix::warn_path(&prog, &v, "invalid number of lines");
                    std::process::exit(2);
                }
            },
            _ => posix::usage(SYNOPSIS),
        }
    }
    let files = g.operands();
    let mut out = posix::borrow_fd(1);
    let mut status = 0;
    if files.is_empty() {
        let mut stdin = posix::borrow_fd(0);
        match head(&mut *stdin, &mut *out, lines) {
            Ok(unused) => rewind(0, unused),
            Err(Fail::Read(e)) => {
                posix::warn(&prog, &format!("standard input: {}", posix::strerror(&e)));
                status = 1;
            }
            Err(Fail::Write(e)) => {
                posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
            }
        }
        std::process::exit(status);
    }
    let mut first = true;
    for f in files {
        let mut h = match posix::open_input(f) {
            Ok(h) => h,
            Err(e) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                status = 1;
                continue;
            }
        };
        if files.len() > 1 {
            let name: &[u8] = if h.is_stdin() { b"standard input" } else { f };
            posix::write_stdout(&prog, &header(first, name));
            first = false;
        }
        match head(&mut *h, &mut *out, lines) {
            Ok(unused) => {
                if h.is_stdin() {
                    rewind(0, unused);
                }
            }
            Err(Fail::Read(e)) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                status = 1;
            }
            Err(Fail::Write(e)) => {
                posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
            }
        }
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    /* a reader that hands out at most k bytes per call */
    struct Chunked<'a>(&'a [u8], usize);

    impl Read for Chunked<'_> {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            let n = self.0.len().min(b.len()).min(self.1);
            b[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    fn first_lines(data: &[u8], n: u64) -> Vec<u8> {
        let mut out = Vec::new();
        assert!(head(&mut &data[..], &mut out, n).is_ok());
        out
    }

    #[test]
    fn copies_the_first_lines() {
        assert_eq!(first_lines(b"a\nb\nc\n", 2), b"a\nb\n");
        assert_eq!(first_lines(b"a\nb\nc\n", 1), b"a\n");
        assert_eq!(first_lines(b"a\nb\nc\n", 3), b"a\nb\nc\n");
    }

    #[test]
    fn a_short_file_is_copied_whole() {
        assert_eq!(first_lines(b"a\nb\n", 10), b"a\nb\n");
        assert_eq!(first_lines(b"", 10), b"");
        assert_eq!(first_lines(b"no newline", 10), b"no newline");
        assert_eq!(first_lines(b"a\nlast", 5), b"a\nlast");
    }

    #[test]
    fn the_last_line_may_lack_its_newline() {
        assert_eq!(first_lines(b"a\nb", 2), b"a\nb");
        assert_eq!(first_lines(b"a\nb", 1), b"a\n");
    }

    #[test]
    fn zero_lines_copies_nothing_and_reads_nothing() {
        struct Never;
        impl Read for Never {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("read");
            }
        }
        let mut out = Vec::new();
        assert!(matches!(head(&mut Never, &mut out, 0), Ok(0)));
        assert!(out.is_empty());
    }

    #[test]
    fn empty_lines_count() {
        assert_eq!(first_lines(b"\n\n\nx\n", 2), b"\n\n");
        assert_eq!(first_lines(b"\n", 5), b"\n");
    }

    #[test]
    fn binary_bytes_pass_through() {
        let d = b"a\0b\xff\n\x80\r\nz\n";
        assert_eq!(first_lines(d, 2), b"a\0b\xff\n\x80\r\n");
    }

    #[test]
    fn short_reads_give_the_same_result() {
        let data: Vec<u8> = (0..2000u32).flat_map(|i| format!("line {i}\n").into_bytes()).collect();
        for k in [1usize, 2, 3, 7, 4096, 70000] {
            for n in [0u64, 1, 2, 17, 1999, 2000, 2001] {
                let mut out = Vec::new();
                assert!(head(&mut Chunked(&data, k), &mut out, n).is_ok());
                assert_eq!(out, first_lines(&data, n), "k={k} n={n}");
            }
        }
    }

    #[test]
    fn stops_reading_at_the_cut() {
        /* an endless input: only the first block may be read */
        struct Endless(usize);
        impl Read for Endless {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                self.0 += 1;
                for x in b.iter_mut() {
                    *x = b'\n';
                }
                Ok(b.len())
            }
        }
        let mut src = Endless(0);
        let mut out = Vec::new();
        let unused = head(&mut src, &mut out, 5).ok().unwrap();
        assert_eq!(out, b"\n\n\n\n\n");
        assert_eq!(src.0, 1);
        assert_eq!(unused, (BLOCK - 5) as u64);
    }

    #[test]
    fn a_line_longer_than_a_block_streams() {
        let mut data = vec![b'x'; 5 * BLOCK + 3];
        data.push(b'\n');
        data.extend_from_slice(b"second\n");
        let out = first_lines(&data, 1);
        assert_eq!(out.len(), 5 * BLOCK + 4);
        assert!(out.ends_with(b"x\n"));
    }

    #[test]
    fn huge_counts_are_fine() {
        assert_eq!(first_lines(b"a\nb\n", u64::MAX), b"a\nb\n");
    }

    #[test]
    fn reports_the_unused_bytes() {
        let mut out = Vec::new();
        let unused = head(&mut &b"a\nb\nc\n"[..], &mut out, 1).ok().unwrap();
        assert_eq!((out.as_slice(), unused), (&b"a\n"[..], 4));
        let mut out = Vec::new();
        assert_eq!(head(&mut &b"a\nb\n"[..], &mut out, 2).ok().unwrap(), 0);
    }

    #[test]
    fn errors_are_told_apart() {
        struct Bad;
        impl Read for Bad {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from_raw_os_error(libc::EIO))
            }
        }
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from_raw_os_error(libc::ENOSPC))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(matches!(head(&mut Bad, &mut Vec::new(), 1), Err(Fail::Read(_))));
        assert!(matches!(head(&mut &b"x\n"[..], &mut Full, 1), Err(Fail::Write(_))));
    }

    #[test]
    fn headers() {
        assert_eq!(header(true, b"f"), b"==> f <==\n");
        assert_eq!(header(false, b"f g"), b"\n==> f g <==\n");
    }
}
