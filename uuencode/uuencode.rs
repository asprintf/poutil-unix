/* SPDX-License-Identifier: GPL-2.0-only */
/* uuencode - encode a binary file. */

use posix::getopt::{Getopt, Opt};
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;

const SYNOPSIS: &str = "uuencode [-m] [file] decode_pathname";

/* the Base64 alphabet of the uuencode Base64 Values table */
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/* octets per line: 45 make 60 characters, 57 make 76, the longest allowed */
const UU_LINE: usize = 45;
const B64_LINE: usize = 57;

enum Fail {
    Read(io::Error),
    Write(io::Error),
}

/*
 * Read into buf until it is full or the input ends; the number of bytes.
 * One read(2) can return less than a line, which must not end a line early.
 */
fn fill<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match posix::read_block(r, &mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/*
 * One 6-bit value as a character of the historical encoding: 0x20 plus the
 * value, as the text says (so a zero is a space, not the grave accent that
 * many implementations use to keep it from being lost at the end of a line).
 */
fn uu_char(v: u8) -> u8 {
    b' ' + (v & 0x3f)
}

/* Three octets (missing ones are zero) as four characters. */
fn uu_group(a: u8, b: u8, c: u8, out: &mut Vec<u8>) {
    out.push(uu_char(a >> 2));
    out.push(uu_char((a << 4) | (b >> 4)));
    out.push(uu_char((b << 2) | (c >> 6)));
    out.push(uu_char(c));
}

/* One line of the historical encoding for up to 45 octets, with its newline. */
fn uu_line(chunk: &[u8], out: &mut Vec<u8>) {
    out.push(uu_char(chunk.len() as u8));
    for g in chunk.chunks(3) {
        uu_group(g[0], *g.get(1).unwrap_or(&0), *g.get(2).unwrap_or(&0), out);
    }
    out.push(b'\n');
}

/* The whole body: the lines, the line of length zero, and "end". */
fn encode_uu<R: Read, W: Write>(r: &mut R, w: &mut W) -> Result<(), Fail> {
    let mut buf = [0u8; UU_LINE];
    let mut line = Vec::with_capacity(80);
    loop {
        let n = fill(r, &mut buf).map_err(Fail::Read)?;
        if n == 0 {
            break;
        }
        line.clear();
        uu_line(&buf[..n], &mut line);
        w.write_all(&line).map_err(Fail::Write)?;
        if n < UU_LINE {
            break;
        }
    }
    w.write_all(b" \nend\n").map_err(Fail::Write)
}

/* One line of Base64 for up to 57 octets, with its newline. */
fn b64_line(chunk: &[u8], out: &mut Vec<u8>) {
    for g in chunk.chunks(3) {
        let a = g[0];
        let b = g.get(1).copied();
        let c = g.get(2).copied();
        out.push(B64[(a >> 2) as usize]);
        out.push(B64[(((a << 4) & 0x30) | (b.unwrap_or(0) >> 4)) as usize]);
        match (b, c) {
            (None, _) => out.extend_from_slice(b"=="),
            (Some(b), None) => {
                out.push(B64[((b << 2) & 0x3c) as usize]);
                out.push(b'=');
            }
            (Some(b), Some(c)) => {
                out.push(B64[(((b << 2) & 0x3c) | (c >> 6)) as usize]);
                out.push(B64[(c & 0x3f) as usize]);
            }
        }
    }
    out.push(b'\n');
}

/* The whole body: the lines, then the terminating line. */
fn encode_b64<R: Read, W: Write>(r: &mut R, w: &mut W) -> Result<(), Fail> {
    let mut buf = [0u8; B64_LINE];
    let mut line = Vec::with_capacity(80);
    loop {
        let n = fill(r, &mut buf).map_err(Fail::Read)?;
        if n == 0 {
            break;
        }
        line.clear();
        b64_line(&buf[..n], &mut line);
        w.write_all(&line).map_err(Fail::Write)?;
        if n < B64_LINE {
            break;
        }
    }
    w.write_all(b"====\n").map_err(Fail::Write)
}

/* The first line: "begin" or "begin-base64", the mode in octal, the name. */
fn header(base64: bool, mode: u32, name: &[u8]) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(if base64 { b"begin-base64 " } else { b"begin " });
    h.extend_from_slice(format!("{:03o} ", mode & 0o777).as_bytes());
    h.extend_from_slice(name);
    h.push(b'\n');
    h
}

fn umask() -> u32 {
    unsafe {
        let m = libc::umask(0);
        libc::umask(m);
        m as u32
    }
}

/*
 * The permission bits to write for an input that is the file md describes: its
 * own if it is a file, the usual 0666 less the umask for a pipe or a terminal
 * (the text names only the mode of "the input file").
 */
fn mode_of(md: &std::fs::Metadata, named: bool, mask: u32) -> u32 {
    if named || md.is_file() {
        md.mode() & 0o777
    } else {
        0o666 & !mask
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut base64 = false;
    let mut g = Getopt::new(&args, b"m");
    while let Some(o) = g.next() {
        match o {
            Opt::Flag(b'm') => base64 = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let ops = g.operands();
    if ops.is_empty() || ops.len() > 2 {
        posix::usage(SYNOPSIS);
    }
    let name = &ops[ops.len() - 1];
    /* the input is opened before anything is written */
    let (mut input, mode) = if ops.len() == 2 {
        let path = &ops[0];
        match File::open(std::ffi::OsStr::from_bytes(path)).and_then(|f| f.metadata().map(|m| (f, m))) {
            Ok((f, md)) => {
                let mode = mode_of(&md, true, umask());
                (posix::Input::Named(f), mode)
            }
            Err(e) => posix::fail(&prog, &format!("{}: {}", String::from_utf8_lossy(path), posix::strerror(&e))),
        }
    } else {
        let stdin = posix::borrow_fd(0);
        let mode = match stdin.metadata() {
            Ok(md) => mode_of(&md, false, umask()),
            Err(_) => 0o666 & !umask(),
        };
        (posix::Input::Stdin(stdin), mode)
    };
    let mut out = BufWriter::with_capacity(65536, posix::Stdout::new());
    let res = out
        .write_all(&header(base64, mode, name))
        .map_err(Fail::Write)
        .and_then(|_| if base64 { encode_b64(&mut input, &mut out) } else { encode_uu(&mut input, &mut out) })
        .and_then(|_| out.flush().map_err(Fail::Write));
    match res {
        Ok(()) => {}
        Err(Fail::Read(e)) => {
            let _ = out.flush();
            let path = if ops.len() == 2 { ops[0].as_slice() } else { b"standard input" };
            posix::warn_path(&prog, path, &posix::strerror(&e));
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

    fn uu(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        assert!(encode_uu(&mut &data[..], &mut out).is_ok());
        out
    }

    fn b64(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        assert!(encode_b64(&mut &data[..], &mut out).is_ok());
        out
    }

    fn s(v: Vec<u8>) -> String {
        String::from_utf8(v).unwrap()
    }

    #[test]
    fn the_text_example_of_three_octets() {
        /* 0x20 + (A >> 2), ...: "Cat" is 0x43 0x61 0x74 */
        assert_eq!(s(uu(b"Cat")), "#0V%T\n \nend\n");
        /* the spec's formulas worked out by hand for A=0xff B=0x00 C=0x00 */
        assert_eq!(s(uu(b"\xff\x00\x00")), "#_P  \n \nend\n");
    }

    #[test]
    fn short_inputs_pad_with_zero_bits_and_a_length_character() {
        assert_eq!(s(uu(b"")), " \nend\n");
        assert_eq!(s(uu(b"a")), "!80  \n \nend\n");
        assert_eq!(s(uu(b"ab")), "\"86( \n \nend\n");
        assert_eq!(s(uu(b"abc")), "#86)C\n \nend\n");
    }

    #[test]
    fn a_line_holds_45_octets_and_a_full_line_is_not_followed_by_an_empty_one() {
        let d = vec![b'x'; 45];
        let o = s(uu(&d));
        assert_eq!(o.lines().count(), 3);
        assert!(o.starts_with('M'));
        assert_eq!(o.lines().next().unwrap().len(), 61);
        assert_eq!(o.lines().nth(1).unwrap(), " ");
        assert_eq!(o.lines().nth(2).unwrap(), "end");
        let d = vec![b'x'; 46];
        let o = s(uu(&d));
        assert_eq!(o.lines().count(), 4);
        assert_eq!(o.lines().nth(1).unwrap().as_bytes()[0], b'!');
        let d = vec![b'x'; 90];
        assert_eq!(s(uu(&d)).lines().count(), 4);
    }

    #[test]
    fn every_character_is_in_the_range_20_to_5f() {
        let d: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let o = s(uu(&d));
        let lines: Vec<&str> = o.lines().collect();
        assert_eq!(lines.last(), Some(&"end"));
        for l in &lines[..lines.len() - 1] {
            assert!(l.bytes().all(|c| (0x20..=0x5f).contains(&c)), "{l:?}");
        }
    }

    #[test]
    fn base64_follows_the_table_and_padding() {
        assert_eq!(s(b64(b"")), "====\n");
        assert_eq!(s(b64(b"a")), "YQ==\n====\n");
        assert_eq!(s(b64(b"ab")), "YWI=\n====\n");
        assert_eq!(s(b64(b"abc")), "YWJj\n====\n");
        assert_eq!(s(b64(b"abcd")), "YWJjZA==\n====\n");
        assert_eq!(s(b64(b"\xff\xff\xff")), "////\n====\n");
        assert_eq!(s(b64(b"\xfb\xef\xbe")), "++++\n====\n");
        assert_eq!(s(b64(b"\x00\x00\x00")), "AAAA\n====\n");
        assert_eq!(s(b64(b"Man")), "TWFu\n====\n");
        assert_eq!(s(b64(b"Ma")), "TWE=\n====\n");
        assert_eq!(s(b64(b"M")), "TQ==\n====\n");
    }

    #[test]
    fn base64_lines_are_at_most_76_characters() {
        let d = vec![0xa5u8; 1000];
        let o = s(b64(&d));
        for l in o.lines() {
            assert!(l.len() <= 76);
        }
        assert_eq!(o.lines().next().unwrap().len(), 76);
        assert!(o.ends_with("\n====\n"));
        /* 57 octets are exactly one full line, with no empty line after it */
        let o = s(b64(&vec![1u8; 57]));
        assert_eq!(o.lines().count(), 2);
        let o = s(b64(&vec![1u8; 58]));
        assert_eq!(o.lines().count(), 3);
    }

    #[test]
    fn the_header_has_the_mode_in_octal_and_the_name() {
        assert_eq!(header(false, 0o644, b"x"), b"begin 644 x\n");
        assert_eq!(header(true, 0o600, b"a b"), b"begin-base64 600 a b\n");
        assert_eq!(header(false, 0o7, b"x"), b"begin 007 x\n");
        assert_eq!(header(false, 0, b"x"), b"begin 000 x\n");
        /* only the permission bits */
        assert_eq!(header(false, 0o104755, b"x"), b"begin 755 x\n");
        assert_eq!(header(false, 0o644, b"/dev/stdout"), b"begin 644 /dev/stdout\n");
        assert_eq!(header(false, 0o644, b"\xff\n"), b"begin 644 \xff\n\n");
    }

    #[test]
    fn short_reads_do_not_end_a_line_early() {
        struct Dribble(Vec<u8>);
        impl Read for Dribble {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                let n = self.0.len().min(b.len()).min(5);
                b[..n].copy_from_slice(&self.0[..n]);
                self.0.drain(..n);
                Ok(n)
            }
        }
        let data: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        let mut a = Vec::new();
        assert!(encode_uu(&mut Dribble(data.clone()), &mut a).is_ok());
        assert_eq!(a, uu(&data));
        let mut a = Vec::new();
        assert!(encode_b64(&mut Dribble(data.clone()), &mut a).is_ok());
        assert_eq!(a, b64(&data));
    }

    #[test]
    fn the_modes_written() {
        let md = std::fs::metadata("/").unwrap();
        assert_eq!(mode_of(&md, true, 0o022), md.mode() & 0o777);
        /* a directory is not a file: stdin as a directory gets the default */
        assert_eq!(mode_of(&md, false, 0o022), 0o644);
        assert_eq!(mode_of(&md, false, 0o077), 0o600);
        assert_eq!(mode_of(&md, false, 0), 0o666);
        let path = format!("/tmp/poutil-uuencode-test-{}", std::process::id());
        std::fs::write(&path, b"x").unwrap();
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o640)).unwrap();
        let md = std::fs::metadata(&path).unwrap();
        assert_eq!(mode_of(&md, false, 0o022), 0o640);
        assert_eq!(mode_of(&md, true, 0o022), 0o640);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn errors_are_told_apart() {
        struct Broken;
        impl Read for Broken {
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
        assert!(matches!(encode_uu(&mut Broken, &mut Vec::new()), Err(Fail::Read(_))));
        assert!(matches!(encode_b64(&mut Broken, &mut Vec::new()), Err(Fail::Read(_))));
        assert!(matches!(encode_uu(&mut &b"x"[..], &mut Full), Err(Fail::Write(_))));
        assert!(matches!(encode_b64(&mut &b""[..], &mut Full), Err(Fail::Write(_))));
    }
}
