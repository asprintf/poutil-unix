/* SPDX-License-Identifier: GPL-2.0-only */
/* uudecode - decode a binary file. */

use posix::getopt::{Getopt, Opt};
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const SYNOPSIS: &str = "uudecode [-o outfile] [file]";

/* The first line of an encoded file, as far as decoding needs it. */
#[derive(Debug, PartialEq, Eq)]
struct Header {
    base64: bool,
    mode: u32,
    name: Vec<u8>,
}

/* Why decoding stopped before the end line. */
#[derive(Debug)]
enum Fail {
    Read(io::Error),
    Write(io::Error),
    /* the input ended before the line that ends the data */
    NoEnd,
    /* the Base64 data is wrong; the text says how */
    Bad(&'static str),
}

fn is_blank(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

/*
 * A line that begins an encoded file: "begin" (or "begin-base64"), blanks,
 * the mode (chmod octal or symbolic), blanks, the name, which runs to the
 * end of the line. The mode is read the way chmod reads it, with nothing to
 * start from and no umask, and only its permission bits are kept: a file
 * made from unknown data never gets a set-user-ID bit. A line that
 * only looks like a header (no mode, no name) is not one.
 */
fn parse_header(line: &[u8]) -> Option<Header> {
    let (base64, rest) = if let Some(r) = line.strip_prefix(b"begin-base64") {
        (true, r)
    } else {
        (false, line.strip_prefix(b"begin")?)
    };
    if !rest.first().is_some_and(|&c| is_blank(c)) {
        return None;
    }
    let rest = &rest[rest.iter().position(|&c| !is_blank(c))?..];
    let end = rest.iter().position(|&c| is_blank(c))?;
    let mode = posix::mode::parse(&rest[..end], 0, false, 0)? & 0o777;
    let name = &rest[end..];
    let name = &name[name.iter().position(|&c| !is_blank(c))?..];
    Some(Header { base64, mode, name: name.to_vec() })
}

/*
 * Read one line into buf without its newline, nor the carriage return that
 * a text file from elsewhere puts before it. False at the end of the input.
 */
fn read_line<R: BufRead>(r: &mut R, buf: &mut Vec<u8>) -> io::Result<bool> {
    buf.clear();
    if r.read_until(b'\n', buf)? == 0 {
        return Ok(false);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(true)
}

/* Skip to the first header; None if the input has none. */
fn find_header<R: BufRead>(r: &mut R) -> io::Result<Option<Header>> {
    let mut line = Vec::new();
    while read_line(r, &mut line)? {
        if let Some(h) = parse_header(&line) {
            return Ok(Some(h));
        }
    }
    Ok(None)
}

/* The value of a character of the historical encoding: 0x20 is 0, 0x60 too. */
fn uu_val(c: u8) -> u8 {
    c.wrapping_sub(b' ') & 0x3f
}

/* What a line of the historical encoding was. */
#[derive(Debug, PartialEq, Eq)]
enum UuLine {
    /* "end" */
    End,
    /* a length of zero: the data is over */
    Zero,
    /* an empty line, skipped; or octets, appended to the output */
    Data,
}

/*
 * One line between "begin" and "end": a length character, then the
 * characters for that many octets, four for every three. A line cut short
 * (a mailer trims trailing blanks, and a space is a zero) is filled up with
 * zeros; characters after the ones needed are ignored.
 */
fn uu_line(line: &[u8], out: &mut Vec<u8>) -> UuLine {
    if line == b"end" {
        return UuLine::End;
    }
    let Some(&first) = line.first() else {
        return UuLine::Data;
    };
    let n = uu_val(first) as usize;
    if n == 0 {
        return UuLine::Zero;
    }
    let body = &line[1..];
    let mut left = n;
    for g in 0..n.div_ceil(3) {
        let v = |i: usize| body.get(g * 4 + i).map_or(0, |&c| uu_val(c));
        let b = [(v(0) << 2) | (v(1) >> 4), (v(1) << 4) | (v(2) >> 2), (v(2) << 6) | v(3)];
        let k = left.min(3);
        out.extend_from_slice(&b[..k]);
        left -= k;
    }
    UuLine::Data
}

/* The decoder of the Base64 body, fed one line at a time. */
#[derive(Default)]
struct B64 {
    quad: [u8; 4],
    n: usize,
    /* one of the two "=" after two characters has been seen */
    half_pad: bool,
    /* the last quantum was padded: no more data may follow */
    padded: bool,
}

fn b64_val(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

impl B64 {
    /*
     * Characters that are not in the table are ignored (the text says line
     * breaks and anything else are); the padding must complete a quantum
     * and nothing may follow it.
     */
    fn feed(&mut self, line: &[u8], out: &mut Vec<u8>) -> Result<(), &'static str> {
        for &c in line {
            if c == b'=' {
                match (self.n, self.half_pad) {
                    (2, false) => self.half_pad = true,
                    (2, true) => {
                        out.push((self.quad[0] << 2) | (self.quad[1] >> 4));
                        self.finish_padded();
                    }
                    (3, false) => {
                        out.push((self.quad[0] << 2) | (self.quad[1] >> 4));
                        out.push((self.quad[1] << 4) | (self.quad[2] >> 2));
                        self.finish_padded();
                    }
                    _ => return Err("invalid padding in the Base64 data"),
                }
            } else if let Some(v) = b64_val(c) {
                if self.half_pad || self.padded {
                    return Err("data after the padding of the Base64 data");
                }
                self.quad[self.n] = v;
                self.n += 1;
                if self.n == 4 {
                    out.push((self.quad[0] << 2) | (self.quad[1] >> 4));
                    out.push((self.quad[1] << 4) | (self.quad[2] >> 2));
                    out.push((self.quad[2] << 6) | self.quad[3]);
                    self.n = 0;
                }
            }
        }
        Ok(())
    }

    fn finish_padded(&mut self) {
        self.n = 0;
        self.half_pad = false;
        self.padded = true;
    }

    /* At "====": a quantum must not be left unfinished. */
    fn end(&self) -> Result<(), &'static str> {
        if self.n != 0 || self.half_pad {
            Err("the Base64 data ends in the middle of a quantum")
        } else {
            Ok(())
        }
    }
}

/*
 * Decode the body that follows the header, up to its end line, and write
 * the octets to w. The data is written as it is decoded, so that what came
 * before an error is not lost. Anything after the end line is not read.
 */
fn decode_body<R: BufRead, W: Write>(r: &mut R, base64: bool, w: &mut W) -> Result<(), Fail> {
    let mut line = Vec::new();
    let mut out = Vec::new();
    let mut b64 = B64::default();
    let mut zero_seen = false;
    loop {
        if !read_line(r, &mut line).map_err(Fail::Read)? {
            return Err(Fail::NoEnd);
        }
        out.clear();
        if base64 {
            if line == b"====" {
                return b64.end().map_err(Fail::Bad);
            }
            /* what was decoded before a bad character is still written */
            if let Err(m) = b64.feed(&line, &mut out) {
                w.write_all(&out).map_err(Fail::Write)?;
                return Err(Fail::Bad(m));
            }
        } else if zero_seen {
            if line == b"end" {
                return Ok(());
            }
        } else {
            match uu_line(&line, &mut out) {
                UuLine::End => return Ok(()),
                UuLine::Zero => zero_seen = true,
                UuLine::Data => {}
            }
        }
        w.write_all(&out).map_err(Fail::Write)?;
    }
}

/* Names that stand for standard output (in the header; -o knows only the first). */
fn is_stdout_name(name: &[u8], from_header: bool) -> bool {
    name == b"/dev/stdout" || (from_header && name == b"-")
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut outfile: Option<Vec<u8>> = None;
    let mut g = Getopt::new(&args, b"o:");
    for o in g.by_ref() {
        match o {
            Opt::Arg(b'o', v) => outfile = Some(v),
            _ => posix::usage(SYNOPSIS),
        }
    }
    let ops = g.operands();
    if ops.len() > 1 {
        posix::usage(SYNOPSIS);
    }
    let in_name: &[u8] = ops.first().map_or(b"-", |v| v.as_slice());
    let input = match posix::open_input(in_name) {
        Ok(f) => f,
        Err(e) => {
            posix::warn_path(&prog, in_name, &posix::strerror(&e));
            std::process::exit(1);
        }
    };
    let mut input = BufReader::new(input);
    let header = match find_header(&mut input) {
        Ok(Some(h)) => h,
        Ok(None) => posix::fail(&prog, "no \"begin\" line in the input"),
        Err(e) => {
            posix::warn_path(&prog, in_name, &posix::strerror(&e));
            std::process::exit(1);
        }
    };
    /* the destination: -o replaces the name in the data */
    let (dest, from_header) = match &outfile {
        Some(o) => (o.as_slice(), false),
        None => (header.name.as_slice(), true),
    };
    let to_stdout = is_stdout_name(dest, from_header);
    let mut file = None;
    let mut out: Box<dyn Write> = if to_stdout {
        Box::new(BufWriter::with_capacity(65536, posix::Stdout::new()))
    } else {
        /*
         * Made with no access for anyone but us and opened up when the data
         * is in, so that the umask has no say in the final mode (and an
         * existing file keeps being written whatever its mode becomes).
         */
        match OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(std::ffi::OsStr::from_bytes(dest))
        {
            Ok(f) => {
                file = f.try_clone().ok();
                Box::new(BufWriter::with_capacity(65536, f))
            }
            Err(e) => {
                posix::warn_path(&prog, dest, &posix::strerror(&e));
                std::process::exit(1);
            }
        }
    };
    let mut status = 0;
    let res = decode_body(&mut input, header.base64, &mut out);
    let flushed = out.flush();
    match res {
        Ok(()) => {}
        Err(Fail::Read(e)) => {
            posix::warn_path(&prog, in_name, &posix::strerror(&e));
            status = 1;
        }
        Err(Fail::Write(e)) => {
            posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
        }
        Err(Fail::NoEnd) => {
            posix::warn(&prog, "unexpected end of the input: no end line");
            status = 1;
        }
        Err(Fail::Bad(m)) => {
            posix::warn(&prog, m);
            status = 1;
        }
    }
    if let Err(e) = flushed {
        posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
    }
    /* "if the mode bits cannot be set, uudecode shall not treat this as an error" */
    if let Some(f) = file {
        let _ = f.set_permissions(std::fs::Permissions::from_mode(header.mode));
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(line: &str) -> Option<Header> {
        parse_header(line.as_bytes())
    }

    fn hdr(base64: bool, mode: u32, name: &str) -> Option<Header> {
        Some(Header { base64, mode, name: name.as_bytes().to_vec() })
    }

    #[test]
    fn headers() {
        assert_eq!(h("begin 644 out"), hdr(false, 0o644, "out"));
        assert_eq!(h("begin-base64 600 out"), hdr(true, 0o600, "out"));
        assert_eq!(h("begin 7 x"), hdr(false, 7, "x"));
        assert_eq!(h("begin   644   out"), hdr(false, 0o644, "out"));
        assert_eq!(h("begin\t644\tout"), hdr(false, 0o644, "out"));
        assert_eq!(h("begin 644 my file"), hdr(false, 0o644, "my file"));
        assert_eq!(h("begin 644 /dev/stdout"), hdr(false, 0o644, "/dev/stdout"));
        assert_eq!(h("begin 644 -"), hdr(false, 0o644, "-"));
        assert_eq!(h("begin 644 ../x"), hdr(false, 0o644, "../x"));
    }

    #[test]
    fn modes_are_permission_bits_only() {
        assert_eq!(h("begin 4755 x"), hdr(false, 0o755, "x"));
        assert_eq!(h("begin 7777 x"), hdr(false, 0o777, "x"));
        assert_eq!(h("begin 1000 x"), hdr(false, 0, "x"));
        assert_eq!(h("begin 000 x"), hdr(false, 0, "x"));
        assert_eq!(h("begin 10000 x"), None);
        assert_eq!(h("begin 888 x"), None);
    }

    #[test]
    fn symbolic_modes() {
        assert_eq!(h("begin u=rw,go=r x"), hdr(false, 0o644, "x"));
        assert_eq!(h("begin a=rwx x"), hdr(false, 0o777, "x"));
        assert_eq!(h("begin u=rwx,g=rx,o= x"), hdr(false, 0o750, "x"));
        assert_eq!(h("begin a+x x"), hdr(false, 0o111, "x"));
        assert_eq!(h("begin a-w x"), hdr(false, 0, "x"));
        assert_eq!(h("begin u+s x"), hdr(false, 0, "x"));
        assert_eq!(h("begin u=rw,zz x"), None);
    }

    #[test]
    fn lines_that_are_not_headers() {
        assert_eq!(h(""), None);
        assert_eq!(h("begin"), None);
        assert_eq!(h("begin "), None);
        assert_eq!(h("begin 644"), None);
        assert_eq!(h("begin 644 "), None);
        assert_eq!(h("begin the meeting"), None);
        assert_eq!(h("beginning 644 x"), None);
        assert_eq!(h("begin-base64644 x"), None);
        assert_eq!(h(" begin 644 x"), None);
        assert_eq!(h("Begin 644 x"), None);
        assert_eq!(h("begin-base64"), None);
        assert_eq!(h("begin-base 644 x"), None);
    }

    #[test]
    fn the_first_header_is_found_after_other_lines() {
        let mut r = &b"hello\nbegin the meeting\nbegin 600 a\nbegin 644 b\n"[..];
        assert_eq!(find_header(&mut r).unwrap(), hdr(false, 0o600, "a"));
        assert_eq!(find_header(&mut r).unwrap(), hdr(false, 0o644, "b"));
        assert_eq!(find_header(&mut r).unwrap(), None);
        assert_eq!(find_header(&mut &b""[..]).unwrap(), None);
        /* a header with CR LF, and a last line without a newline */
        assert_eq!(find_header(&mut &b"begin 644 x\r\n"[..]).unwrap(), hdr(false, 0o644, "x"));
        assert_eq!(find_header(&mut &b"begin 644 x"[..]).unwrap(), hdr(false, 0o644, "x"));
    }

    fn dec(body: &str) -> (Vec<u8>, Result<(), Fail>) {
        let mut out = Vec::new();
        let r = decode_body(&mut body.as_bytes(), false, &mut out);
        (out, r)
    }

    fn dec64(body: &str) -> (Vec<u8>, Result<(), Fail>) {
        let mut out = Vec::new();
        let r = decode_body(&mut body.as_bytes(), true, &mut out);
        (out, r)
    }

    #[test]
    fn historical_bodies() {
        let (o, r) = dec("#0V%T\n \nend\n");
        assert!(r.is_ok());
        assert_eq!(o, b"Cat");
        /* the grave accent for zero, as most uuencoders write it */
        let (o, r) = dec("\"86(`\n`\nend\n");
        assert!(r.is_ok());
        assert_eq!(o, b"ab");
        let (o, r) = dec("!80``\n`\nend\n");
        assert!(r.is_ok());
        assert_eq!(o, b"a");
        let (o, r) = dec("`\nend\n");
        assert!(r.is_ok());
        assert_eq!(o, b"");
    }

    #[test]
    fn the_end_line_may_follow_the_data_directly() {
        let (o, r) = dec("#86)C\nend\n");
        assert!(r.is_ok());
        assert_eq!(o, b"abc");
    }

    #[test]
    fn a_missing_end_line_is_an_error_but_the_data_is_kept() {
        let (o, r) = dec("#86)C\n");
        assert!(matches!(r, Err(Fail::NoEnd)));
        assert_eq!(o, b"abc");
        let (o, r) = dec("#86)C\n \n");
        assert!(matches!(r, Err(Fail::NoEnd)));
        assert_eq!(o, b"abc");
        let (_, r) = dec("");
        assert!(matches!(r, Err(Fail::NoEnd)));
    }

    #[test]
    fn short_and_long_lines() {
        /* trailing characters trimmed: filled up with zeros */
        let (o, _) = dec("#86\nend\n");
        assert_eq!(o, b"a\x60\x00");
        let (o, _) = dec("#\nend\n");
        assert_eq!(o, b"\x00\x00\x00");
        /* more characters than the length asks for */
        let (o, _) = dec("!80``XXXX\nend\n");
        assert_eq!(o, b"a");
        /* an empty line is skipped */
        let (o, r) = dec("#86)C\n\n#86)C\n \nend\n");
        assert!(r.is_ok());
        assert_eq!(o, b"abcabc");
        /* CR LF line ends */
        let (o, r) = dec("#86)C\r\n \r\nend\r\n");
        assert!(r.is_ok());
        assert_eq!(o, b"abc");
    }

    #[test]
    fn nothing_is_decoded_after_the_zero_length_line() {
        let (o, r) = dec("#86)C\n \n#86)C\nend\n");
        assert!(r.is_ok());
        assert_eq!(o, b"abc");
    }

    #[test]
    fn a_full_line_of_45_octets() {
        let data: Vec<u8> = (0..45u8).collect();
        let mut enc = vec![b'M'];
        for g in data.chunks(3) {
            let (a, b, c) = (g[0], g[1], g[2]);
            for v in [a >> 2, (a << 4) | (b >> 4), (b << 2) | (c >> 6), c] {
                enc.push(b' ' + (v & 0x3f));
            }
        }
        enc.extend_from_slice(b"\n \nend\n");
        let (o, r) = dec(std::str::from_utf8(&enc).unwrap());
        assert!(r.is_ok());
        assert_eq!(o, data);
    }

    #[test]
    fn base64_bodies() {
        let (o, r) = dec64("YWI=\n====\n");
        assert!(r.is_ok());
        assert_eq!(o, b"ab");
        let (o, r) = dec64("YQ==\n====\n");
        assert!(r.is_ok());
        assert_eq!(o, b"a");
        let (o, r) = dec64("YWJj\nZGVm\n====\n");
        assert!(r.is_ok());
        assert_eq!(o, b"abcdef");
        let (o, r) = dec64("====\n");
        assert!(r.is_ok());
        assert_eq!(o, b"");
        let (o, r) = dec64("////\n++++\n====\n");
        assert!(r.is_ok());
        assert_eq!(o, b"\xff\xff\xff\xfb\xef\xbe");
        /* everything not in the table is ignored */
        let (o, r) = dec64("YW I=\n\n  \n====\n");
        assert!(r.is_ok());
        assert_eq!(o, b"ab");
        let (o, r) = dec64("Y-W_J*j\r\n====\r\n");
        assert!(r.is_ok());
        assert_eq!(o, b"abc");
        /* a quantum may be split across lines */
        let (o, r) = dec64("YW\nJj\nYQ=\n=\n====\n");
        assert!(r.is_ok());
        assert_eq!(o, b"abca");
    }

    #[test]
    fn base64_errors() {
        let (_, r) = dec64("YWI=\n");
        assert!(matches!(r, Err(Fail::NoEnd)));
        /* a quantum cut short, or a padding that is not complete */
        let (_, r) = dec64("YW\n====\n");
        assert!(matches!(r, Err(Fail::Bad(_))));
        let (_, r) = dec64("YQ=\n====\n");
        assert!(matches!(r, Err(Fail::Bad(_))));
        let (_, r) = dec64("Y\n====\n");
        assert!(matches!(r, Err(Fail::Bad(_))));
        /* "=" where data must be */
        let (_, r) = dec64("=\n====\n");
        assert!(matches!(r, Err(Fail::Bad(_))));
        let (_, r) = dec64("Y=\n====\n");
        assert!(matches!(r, Err(Fail::Bad(_))));
        let (_, r) = dec64("YWJj=\n====\n");
        assert!(matches!(r, Err(Fail::Bad(_))));
        let (_, r) = dec64("YQ===\n====\n");
        assert!(matches!(r, Err(Fail::Bad(_))));
        /* data after the padding */
        let (o, r) = dec64("YQ==YWJj\n====\n");
        assert!(matches!(r, Err(Fail::Bad(_))));
        assert_eq!(o, b"a");
        let (_, r) = dec64("YWI=\nYWJj\n====\n");
        assert!(matches!(r, Err(Fail::Bad(_))));
        /* "====" is the end only on a line of its own */
        let (_, r) = dec64("YWJj====\n");
        assert!(matches!(r, Err(Fail::Bad(_))));
    }

    #[test]
    fn what_comes_after_the_end_line_is_not_read() {
        let mut r = &b"#86)C\n \nend\nbegin 644 more\n"[..];
        let mut out = Vec::new();
        assert!(decode_body(&mut r, false, &mut out).is_ok());
        assert_eq!(find_header(&mut r).unwrap(), hdr(false, 0o644, "more"));
    }

    #[test]
    fn the_names_of_standard_output() {
        assert!(is_stdout_name(b"/dev/stdout", true));
        assert!(is_stdout_name(b"/dev/stdout", false));
        assert!(is_stdout_name(b"-", true));
        /* -o - is a file named "-" */
        assert!(!is_stdout_name(b"-", false));
        assert!(!is_stdout_name(b"/dev/stdout/", true));
        assert!(!is_stdout_name(b"stdout", true));
        assert!(!is_stdout_name(b"", true));
    }

    #[test]
    fn every_octet_value_survives_both_ways() {
        let data: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        /* encode by hand with the same rules the encoder uses */
        let mut enc = String::new();
        for chunk in data.chunks(45) {
            enc.push((b' ' + chunk.len() as u8) as char);
            for g in chunk.chunks(3) {
                let a = g[0];
                let b = *g.get(1).unwrap_or(&0);
                let c = *g.get(2).unwrap_or(&0);
                for v in [a >> 2, (a << 4) | (b >> 4), (b << 2) | (c >> 6), c] {
                    enc.push((b' ' + (v & 0x3f)) as char);
                }
            }
            enc.push('\n');
        }
        enc.push_str(" \nend\n");
        let (o, r) = dec(&enc);
        assert!(r.is_ok());
        assert_eq!(o, data);
        /* and the same with the grave accent for zero */
        let (o, r) = dec(&enc.replace(' ', "`"));
        assert!(r.is_ok());
        assert_eq!(o, data);
    }

    #[test]
    fn errors_are_told_apart() {
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from_raw_os_error(libc::ENOSPC))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let r = decode_body(&mut &b"#86)C\nend\n"[..], false, &mut Full);
        assert!(matches!(r, Err(Fail::Write(_))));
        let r = decode_body(&mut &b"YWJj\n====\n"[..], true, &mut Full);
        assert!(matches!(r, Err(Fail::Write(_))));
    }
}
