/* SPDX-License-Identifier: GPL-2.0-only */
/* tail - copy the last part of a file. */

use posix::getopt::{Getopt, Opt};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileTypeExt;
use std::time::Duration;

const SYNOPSIS: &str = "tail [-f] [-c number|-n number] [file]";
const BLOCK: usize = 65536;
/* how long -f sleeps when the file has nothing new */
const POLL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Lines,
    Bytes,
}

/* Where copying starts. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    /* +N: at item N counted from the beginning, origin 1 */
    Begin(u64),
    /* N or -N: at the last N items */
    End(u64),
}

enum Fail {
    Read(io::Error),
    Write(io::Error),
}

/*
 * The option-argument of -c and -n: a decimal integer with an optional
 * sign. "+0" is not allowed by the spec; it is read as "+1", the whole file.
 */
fn parse_start(s: &[u8]) -> Option<Start> {
    match s.first() {
        Some(b'+') => posix::parse_decimal(&s[1..]).map(|n| Start::Begin(n.max(1))),
        Some(b'-') => posix::parse_decimal(&s[1..]).map(Start::End),
        _ => posix::parse_decimal(s).map(Start::End),
    }
}

/*
 * The index just after the k-th newline counted from the end of data, or 0
 * if there are fewer than k. With skip_final a newline that ends the data
 * is the terminator of the last line and is not counted.
 */
fn after_newline_from_end(data: &[u8], k: u64, skip_final: bool) -> usize {
    let mut end = data.len();
    if skip_final && end > 0 && data[end - 1] == b'\n' {
        end -= 1;
    }
    let mut seen = 0;
    while let Some(i) = data[..end].iter().rposition(|&b| b == b'\n') {
        seen += 1;
        if seen == k {
            return i + 1;
        }
        end = i;
    }
    0
}

/* Where the last n lines (n >= 1) of data, which is the whole rest of the input, begin. */
fn last_lines_start(data: &[u8], n: u64) -> usize {
    after_newline_from_end(data, n, true)
}

/*
 * Read all of r, keeping only what can still belong to the last n items
 * (n >= 1), and return exactly those. The memory needed is the size of the
 * tail, never of the input: whenever the buffer holds twice as much as the
 * tail needs, the front half is thrown away.
 */
fn last_of_stream<R: Read>(r: &mut R, unit: Unit, n: u64) -> io::Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut block = vec![0u8; BLOCK];
    let mut newlines: u64 = 0;
    loop {
        let got = posix::read_block(r, &mut block)?;
        if got == 0 {
            break;
        }
        buf.extend_from_slice(&block[..got]);
        match unit {
            Unit::Bytes => {
                if buf.len() as u64 >= n.saturating_mul(2) {
                    buf.drain(..buf.len() - n as usize);
                }
            }
            Unit::Lines => {
                newlines += posix::count_byte(b'\n', &block[..got]);
                if newlines >= n.saturating_add(1).saturating_mul(2) {
                    /* keep one newline more than needed: the end may or may not be one */
                    let start = after_newline_from_end(&buf, n.saturating_add(1), false);
                    buf.drain(..start);
                    newlines = posix::count_byte(b'\n', &buf);
                }
            }
        }
    }
    let start = match unit {
        Unit::Bytes => buf.len().saturating_sub(n.min(usize::MAX as u64) as usize),
        Unit::Lines => last_lines_start(&buf, n),
    };
    buf.drain(..start);
    Ok(buf)
}

/*
 * Position a seekable file at the start of its last n items (n >= 1)
 * without reading what comes before: bytes are a subtraction, lines are
 * found by reading backwards from the end block by block. Counting begins
 * at the current offset, so "(read x; tail) < file" works.
 */
fn seek_to_tail<F: Read + Seek>(f: &mut F, unit: Unit, n: u64) -> io::Result<()> {
    let pos = f.stream_position()?;
    let end = f.seek(SeekFrom::End(0))?;
    if end <= pos {
        f.seek(SeekFrom::Start(pos))?;
        return Ok(());
    }
    if unit == Unit::Bytes {
        f.seek(SeekFrom::Start(end.saturating_sub(n).max(pos)))?;
        return Ok(());
    }
    let mut buf = vec![0u8; BLOCK];
    let mut cur = end;
    let mut seen = 0;
    let mut last_block = true;
    while cur > pos {
        let lo = cur.saturating_sub(BLOCK as u64).max(pos);
        let len = (cur - lo) as usize;
        f.seek(SeekFrom::Start(lo))?;
        f.read_exact(&mut buf[..len])?;
        let mut hay = &buf[..len];
        /* the newline that ends the file ends the last line; it starts nothing */
        if last_block && hay[len - 1] == b'\n' {
            hay = &hay[..len - 1];
        }
        last_block = false;
        while let Some(i) = hay.iter().rposition(|&b| b == b'\n') {
            seen += 1;
            if seen == n {
                f.seek(SeekFrom::Start(lo + i as u64 + 1))?;
                return Ok(());
            }
            hay = &hay[..i];
        }
        cur = lo;
    }
    f.seek(SeekFrom::Start(pos))?;
    Ok(())
}

/*
 * Copy r to w without its first `skip` items: the "+N" forms (skip = N-1)
 * and, with skip 0, the plain copy that follows a seek. Constant memory.
 */
fn copy_after<R: Read, W: Write>(r: &mut R, w: &mut W, unit: Unit, mut skip: u64) -> Result<(), Fail> {
    let mut block = vec![0u8; BLOCK];
    loop {
        let n = posix::read_block(r, &mut block).map_err(Fail::Read)?;
        if n == 0 {
            return Ok(());
        }
        let mut from = 0;
        while skip > 0 && from < n {
            match unit {
                Unit::Bytes => {
                    let take = skip.min((n - from) as u64);
                    from += take as usize;
                    skip -= take;
                }
                Unit::Lines => match posix::find_byte(b'\n', &block[from..n]) {
                    Some(i) => {
                        from += i + 1;
                        skip -= 1;
                    }
                    None => from = n,
                },
            }
        }
        if from < n {
            w.write_all(&block[from..n]).map_err(Fail::Write)?;
        }
    }
}

/* -f: copy what is appended, forever; the user ends it with a signal. */
fn follow<R: Read, W: Write>(r: &mut R, w: &mut W) -> Result<(), Fail> {
    let mut block = vec![0u8; BLOCK];
    loop {
        let n = posix::read_block(r, &mut block).map_err(Fail::Read)?;
        if n == 0 {
            std::thread::sleep(POLL);
        } else {
            w.write_all(&block[..n]).map_err(Fail::Write)?;
        }
    }
}

fn bad_number(prog: &[u8], v: &[u8], what: &str) -> ! {
    posix::warn_path(prog, v, &format!("invalid number of {what}"));
    std::process::exit(2)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut unit = Unit::Lines;
    let mut start = Start::End(10);
    let mut want_follow = false;
    let mut g = Getopt::new(&args, b"fc:n:");
    for o in g.by_ref() {
        match o {
            Opt::Flag(b'f') => want_follow = true,
            /* the last of -c and -n wins */
            Opt::Arg(b'c', v) => match parse_start(&v) {
                Some(s) => {
                    unit = Unit::Bytes;
                    start = s;
                }
                None => bad_number(&prog, &v, "bytes"),
            },
            Opt::Arg(b'n', v) => match parse_start(&v) {
                Some(s) => {
                    unit = Unit::Lines;
                    start = s;
                }
                None => bad_number(&prog, &v, "lines"),
            },
            _ => posix::usage(SYNOPSIS),
        }
    }
    let files = g.operands();
    if files.len() > 1 {
        posix::usage(SYNOPSIS);
    }
    let name: &[u8] = files.first().map(|v| v.as_slice()).unwrap_or(b"-");
    let mut h = match posix::open_input(name) {
        Ok(h) => h,
        Err(e) => {
            posix::warn_path(&prog, name, &posix::strerror(&e));
            std::process::exit(1);
        }
    };
    let (regular, fifo) = match h.metadata() {
        Ok(m) => (m.is_file(), m.file_type().is_fifo()),
        Err(_) => (false, false),
    };
    /*
     * -f applies to a regular file and to a FIFO named as an operand; on a
     * pipe or FIFO that is standard input it is ignored.
     */
    let following = want_follow && (regular || (fifo && !h.is_stdin()));
    let mut out = posix::borrow_fd(1);

    let res = (|| -> Result<(), Fail> {
        match start {
            Start::End(0) => {
                /* nothing to copy, but -f goes on from the end */
                if regular {
                    h.seek(SeekFrom::End(0)).map_err(Fail::Read)?;
                }
            }
            Start::End(n) if regular => {
                seek_to_tail(&mut *h, unit, n).map_err(Fail::Read)?;
                copy_after(&mut *h, &mut *out, unit, 0)?;
            }
            Start::End(n) => {
                let tail = last_of_stream(&mut *h, unit, n).map_err(Fail::Read)?;
                out.write_all(&tail).map_err(Fail::Write)?;
            }
            Start::Begin(n) => {
                let mut skip = n - 1;
                /* skipping bytes of a regular file is a seek, not a read */
                if unit == Unit::Bytes && regular {
                    let by = skip.min(i64::MAX as u64) as i64;
                    if h.seek(SeekFrom::Current(by)).is_ok() {
                        skip = 0;
                    }
                }
                copy_after(&mut *h, &mut *out, unit, skip)?;
            }
        }
        if following {
            follow(&mut *h, &mut *out)?;
        }
        Ok(())
    })();
    match res {
        Ok(()) => {}
        Err(Fail::Read(e)) => {
            posix::warn_path(&prog, name, &posix::strerror(&e));
            std::process::exit(1);
        }
        Err(Fail::Write(e)) => {
            posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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

    /* the slow, obviously right answer */
    fn expect(data: &[u8], unit: Unit, n: u64) -> Vec<u8> {
        match unit {
            Unit::Bytes => data[data.len().saturating_sub(n.min(usize::MAX as u64) as usize)..].to_vec(),
            Unit::Lines => {
                let pieces: Vec<&[u8]> = data.split_inclusive(|&b| b == b'\n').collect();
                let from = pieces.len().saturating_sub(n.min(usize::MAX as u64) as usize);
                pieces[from..].concat()
            }
        }
    }

    fn by_stream(data: &[u8], unit: Unit, n: u64, k: usize) -> Vec<u8> {
        last_of_stream(&mut Chunked(data, k), unit, n).unwrap()
    }

    fn by_seek(data: &[u8], unit: Unit, n: u64) -> Vec<u8> {
        let mut c = Cursor::new(data.to_vec());
        seek_to_tail(&mut c, unit, n).unwrap();
        let mut out = Vec::new();
        c.read_to_end(&mut out).unwrap();
        out
    }

    fn small_corpus() -> Vec<Vec<u8>> {
        let mut v: Vec<Vec<u8>> = [
            &b""[..],
            b"\n",
            b"\n\n",
            b"a",
            b"a\n",
            b"a\nb",
            b"a\nb\n",
            b"a\n\nb\n\n",
            b"\nx",
            b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\ntwelve\n",
            b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\ntwelve",
            b"bin\0\xff\n\x80\r\nlast",
        ]
        .iter()
        .map(|s| s.to_vec())
        .collect();
        /* lines and runs that cross the block size in every way */
        for len in [BLOCK - 1, BLOCK, BLOCK + 1, 2 * BLOCK, 2 * BLOCK + 1] {
            let mut d = vec![b'x'; len];
            v.push(d.clone());
            d.push(b'\n');
            v.push(d.clone());
            d.extend_from_slice(b"tail\nend");
            v.push(d);
        }
        let mut many = Vec::new();
        for i in 0..30000u32 {
            many.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        v.push(many);
        v
    }

    const COUNTS: [u64; 14] = [1, 2, 3, 4, 5, 9, 10, 11, 12, 13, 100, 4095, 70000, u64::MAX];

    #[test]
    fn stream_and_seek_agree_with_the_reference() {
        for data in small_corpus() {
            for unit in [Unit::Lines, Unit::Bytes] {
                for n in COUNTS {
                    let want = expect(&data, unit, n);
                    assert_eq!(by_seek(&data, unit, n), want, "seek {unit:?} {n} len {}", data.len());
                    for k in [1usize, 7, 4096, 1 << 20] {
                        if data.len() > 5000 && k < 4096 {
                            continue;
                        }
                        assert_eq!(by_stream(&data, unit, n, k), want, "stream {unit:?} {n} k={k} len {}", data.len());
                    }
                }
            }
        }
    }

    #[test]
    fn the_spec_origin_is_one() {
        assert_eq!(by_seek(b"a\nb\nc\n", Unit::Lines, 1), b"c\n");
        assert_eq!(by_seek(b"abc", Unit::Bytes, 1), b"c");
        assert_eq!(by_stream(b"a\nb\nc", Unit::Lines, 1, 1 << 20), b"c");
    }

    #[test]
    fn a_seekable_file_is_read_from_its_current_offset() {
        let data = b"skip\na\nb\nc\n";
        let mut c = Cursor::new(data.to_vec());
        c.seek(SeekFrom::Start(5)).unwrap();
        seek_to_tail(&mut c, Unit::Lines, 10).unwrap();
        let mut out = Vec::new();
        c.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"a\nb\nc\n");
        let mut c = Cursor::new(data.to_vec());
        c.seek(SeekFrom::Start(9)).unwrap();
        seek_to_tail(&mut c, Unit::Bytes, 100).unwrap();
        let mut out = Vec::new();
        c.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"c\n");
    }

    #[test]
    fn seeking_reads_only_the_tail_of_a_big_file() {
        /* counts the bytes actually read */
        struct Spy(Cursor<Vec<u8>>, usize);
        impl Read for Spy {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                let n = self.0.read(b)?;
                self.1 += n;
                Ok(n)
            }
        }
        impl Seek for Spy {
            fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
                self.0.seek(p)
            }
        }
        let mut data = Vec::new();
        for i in 0..200000u32 {
            data.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        assert!(data.len() > 1_000_000);
        let mut s = Spy(Cursor::new(data), 0);
        seek_to_tail(&mut s, Unit::Lines, 10).unwrap();
        assert!(s.1 <= BLOCK, "read {} bytes to find 10 lines", s.1);
        seek_to_tail(&mut s, Unit::Bytes, 10).unwrap();
        assert!(s.1 <= BLOCK);
    }

    fn after(data: &[u8], unit: Unit, skip: u64, k: usize) -> Vec<u8> {
        let mut out = Vec::new();
        assert!(copy_after(&mut Chunked(data, k), &mut out, unit, skip).is_ok());
        out
    }

    #[test]
    fn plus_forms_skip_from_the_beginning() {
        let d = b"a\nb\nc\nd";
        for k in [1usize, 2, 3, 100] {
            assert_eq!(after(d, Unit::Lines, 0, k), d);
            assert_eq!(after(d, Unit::Lines, 1, k), b"b\nc\nd");
            assert_eq!(after(d, Unit::Lines, 3, k), b"d");
            assert_eq!(after(d, Unit::Lines, 4, k), b"");
            assert_eq!(after(d, Unit::Lines, 99, k), b"");
            assert_eq!(after(d, Unit::Bytes, 0, k), d);
            assert_eq!(after(d, Unit::Bytes, 3, k), b"\nc\nd");
            assert_eq!(after(d, Unit::Bytes, 7, k), b"");
            assert_eq!(after(d, Unit::Bytes, u64::MAX, k), b"");
        }
        assert_eq!(after(b"", Unit::Lines, 3, 5), b"");
    }

    #[test]
    fn plus_forms_across_blocks() {
        let mut d = vec![b'x'; 3 * BLOCK];
        d.extend_from_slice(b"\nrest\n");
        assert_eq!(after(&d, Unit::Lines, 1, 1 << 20), b"rest\n");
        assert_eq!(after(&d, Unit::Bytes, (3 * BLOCK + 1) as u64, 1 << 20), b"rest\n");
        assert_eq!(after(&d, Unit::Bytes, BLOCK as u64, 1 << 20).len(), 2 * BLOCK + 6);
    }

    #[test]
    fn numbers() {
        assert_eq!(parse_start(b"5"), Some(Start::End(5)));
        assert_eq!(parse_start(b"-5"), Some(Start::End(5)));
        assert_eq!(parse_start(b"+5"), Some(Start::Begin(5)));
        assert_eq!(parse_start(b"0"), Some(Start::End(0)));
        assert_eq!(parse_start(b"-0"), Some(Start::End(0)));
        assert_eq!(parse_start(b"+0"), Some(Start::Begin(1)));
        assert_eq!(parse_start(b"+1"), Some(Start::Begin(1)));
        assert_eq!(parse_start(b""), None);
        assert_eq!(parse_start(b"+"), None);
        assert_eq!(parse_start(b"-"), None);
        assert_eq!(parse_start(b"5c"), None);
        assert_eq!(parse_start(b"+-5"), None);
        assert_eq!(parse_start(b"--5"), None);
        assert_eq!(parse_start(b"1.5"), None);
        assert_eq!(parse_start(b"99999999999999999999999"), Some(Start::End(u64::MAX)));
    }

    #[test]
    fn newline_search_from_the_end() {
        assert_eq!(after_newline_from_end(b"a\nb\nc\n", 1, false), 6);
        assert_eq!(after_newline_from_end(b"a\nb\nc\n", 1, true), 4);
        assert_eq!(after_newline_from_end(b"a\nb\nc", 1, true), 4);
        assert_eq!(after_newline_from_end(b"a\nb\nc\n", 9, true), 0);
        assert_eq!(after_newline_from_end(b"", 1, true), 0);
        assert_eq!(after_newline_from_end(b"\n", 1, true), 0);
    }

    #[test]
    fn a_failing_read_is_reported() {
        struct Bad;
        impl Read for Bad {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from_raw_os_error(libc::EIO))
            }
        }
        assert!(last_of_stream(&mut Bad, Unit::Lines, 3).is_err());
        assert!(matches!(copy_after(&mut Bad, &mut Vec::new(), Unit::Lines, 0), Err(Fail::Read(_))));
    }

    #[test]
    fn a_failing_write_is_reported() {
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from_raw_os_error(libc::ENOSPC))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(matches!(copy_after(&mut &b"x\n"[..], &mut Full, Unit::Lines, 0), Err(Fail::Write(_))));
    }

    #[test]
    fn a_pipe_is_not_buffered_whole() {
        /* 30 MB through a 3-line tail: the answer is right, and the buffer cannot have kept it all */
        struct Gen(u64);
        impl Read for Gen {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                if self.0 >= 30_000_000 {
                    return Ok(0);
                }
                let line = b"0123456789abcdef\n";
                let mut n = 0;
                while n + line.len() <= b.len() {
                    b[n..n + line.len()].copy_from_slice(line);
                    n += line.len();
                }
                self.0 += n as u64;
                Ok(n)
            }
        }
        let out = last_of_stream(&mut Gen(0), Unit::Lines, 3).unwrap();
        assert_eq!(out, b"0123456789abcdef\n".repeat(3));
        let out = last_of_stream(&mut Gen(0), Unit::Bytes, 5).unwrap();
        assert_eq!(out, b"cdef\n");
    }
}
