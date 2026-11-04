/* SPDX-License-Identifier: GPL-2.0-only */
/* paste - merge corresponding or subsequent lines of files. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};

const SYNOPSIS: &str = "paste [-s] [-d list] file...";

/* An input with its end remembered: once at the end it stays there. */
struct Rd<R: Read> {
    r: BufReader<R>,
    eof: bool,
}

impl<R: Read> Rd<R> {
    fn new(r: R) -> Self {
        Rd { r: BufReader::with_capacity(65536, r), eof: false }
    }
}

/*
 * The next line without its newline into buf; false at the end. A last
 * line that has no newline is a line like any other. Output that is waiting
 * is flushed before a read that has to wait for more input, so that a
 * terminal or a slow pipe sees each finished row at once.
 */
fn read_line<R: Read, W: Write>(rd: &mut Rd<R>, buf: &mut Vec<u8>, out: &mut W) -> io::Result<bool> {
    if rd.eof {
        return Ok(false);
    }
    if rd.r.buffer().is_empty() {
        out.flush()?;
    }
    buf.clear();
    if rd.r.read_until(b'\n', buf)? == 0 {
        rd.eof = true;
        return Ok(false);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    Ok(true)
}

/*
 * The delimiter list: one element for each character, the elements
 * \n \t \\ and \0 (the empty string) for the escapes, and for any other
 * character after a backslash the character itself, as historical
 * implementations do. None for a list that ends in a lone backslash.
 */
fn parse_delims(list: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < list.len() {
        let (n, _) = posix::mb::char_at(&list[i..]);
        if list[i] == b'\\' && n == 1 {
            i += 1;
            if i >= list.len() {
                return None;
            }
            let (m, _) = posix::mb::char_at(&list[i..]);
            v.push(match list[i] {
                b'n' => b"\n".to_vec(),
                b't' => b"\t".to_vec(),
                b'0' => Vec::new(),
                _ => list[i..i + m].to_vec(),
            });
            i += m;
        } else {
            v.push(list[i..i + n].to_vec());
            i += n;
        }
    }
    Some(v)
}

/*
 * Without -s: row after row, one line from each input in turn (input i of
 * the operands is readers[map[i]], so that every "-" shares the standard
 * input), the delimiters between them used in turn, starting again in each
 * row. An input that has ended gives empty lines until all have.
 */
fn parallel<R: Read, W: Write>(
    readers: &mut [Rd<R>],
    map: &[usize],
    delims: &[Vec<u8>],
    out: &mut W,
) -> io::Result<()> {
    let mut row = Vec::new();
    let mut line = Vec::new();
    loop {
        row.clear();
        let mut any = false;
        for (i, &m) in map.iter().enumerate() {
            if i > 0 {
                row.extend_from_slice(&delims[(i - 1) % delims.len()]);
            }
            if read_line(&mut readers[m], &mut line, out)? {
                any = true;
                row.extend_from_slice(&line);
            }
        }
        if !any {
            return Ok(());
        }
        row.push(b'\n');
        out.write_all(&row)?;
    }
}

/*
 * With -s: all the lines of one input on one line, the delimiters used in
 * turn and started again for each input. An empty input is an empty line.
 * The error, if any, ends this input only.
 */
fn serial<R: Read, W: Write>(rd: &mut Rd<R>, delims: &[Vec<u8>], out: &mut W) -> io::Result<()> {
    let mut line = Vec::new();
    let mut count = 0usize;
    let mut result = Ok(());
    loop {
        match read_line(rd, &mut line, out) {
            Ok(true) => {
                if count > 0 {
                    out.write_all(&delims[(count - 1) % delims.len()])?;
                }
                out.write_all(&line)?;
                count += 1;
            }
            Ok(false) => break,
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    out.write_all(b"\n")?;
    result
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut serial_mode = false;
    let mut list: Vec<u8> = b"\t".to_vec();
    let mut g = Getopt::new(&args, b"sd:");
    for o in &mut g {
        match o {
            Opt::Flag(b's') => serial_mode = true,
            Opt::Arg(b'd', v) => list = v,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let files = g.operands();
    if files.is_empty() {
        posix::usage(SYNOPSIS);
    }
    let delims = match parse_delims(&list) {
        None => posix::fail(&prog, "delimiter list ends with an unescaped backslash"),
        Some(d) if d.is_empty() => posix::fail(&prog, "no delimiters specified"),
        Some(d) => d,
    };
    /* every "-" is the one standard input, read a line at a time in turn */
    let mut readers: Vec<Rd<posix::Input>> = Vec::new();
    let mut map: Vec<Option<usize>> = Vec::new();
    let mut stdin_at: Option<usize> = None;
    let mut status = 0;
    for f in files {
        if f == b"-" {
            if stdin_at.is_none() {
                match posix::open_input(f) {
                    Ok(h) => {
                        readers.push(Rd::new(h));
                        stdin_at = Some(readers.len() - 1);
                    }
                    Err(e) => {
                        posix::warn(&prog, &format!("standard input: {}", posix::strerror(&e)));
                        status = 1;
                    }
                }
            }
            map.push(stdin_at);
        } else {
            match posix::open_input(f) {
                Ok(h) => {
                    readers.push(Rd::new(h));
                    map.push(Some(readers.len() - 1));
                }
                Err(e) => {
                    posix::warn_path(&prog, f, &posix::strerror(&e));
                    status = 1;
                    map.push(None);
                }
            }
        }
    }
    /* without -s a file that cannot be opened means no output at all */
    if status != 0 && !serial_mode {
        std::process::exit(status);
    }
    let mut out = BufWriter::with_capacity(65536, posix::Stdout::new());
    if serial_mode {
        for (i, m) in map.iter().enumerate() {
            let Some(m) = m else { continue };
            match serial(&mut readers[*m], &delims, &mut out) {
                Ok(()) => {}
                Err(e) => {
                    let _ = out.flush();
                    posix::warn_path(&prog, &files[i], &posix::strerror(&e));
                    status = 1;
                }
            }
        }
    } else {
        let idx: Vec<usize> = map.iter().map(|m| m.unwrap_or(0)).collect();
        if let Err(e) = parallel(&mut readers, &idx, &delims, &mut out) {
            let _ = out.flush();
            posix::warn(&prog, &posix::strerror(&e).to_string());
            std::process::exit(1);
        }
    }
    if let Err(e) = out.flush() {
        posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Mutex;

    /* the locale is process-wide: tests that change it hold this */
    static LOCALE: Mutex<()> = Mutex::new(());

    fn d(list: &str) -> Vec<Vec<u8>> {
        parse_delims(list.as_bytes()).unwrap()
    }

    fn par(inputs: &[&str], list: &str) -> String {
        let mut rs: Vec<Rd<Cursor<Vec<u8>>>> = inputs.iter().map(|s| Rd::new(Cursor::new(s.as_bytes().to_vec()))).collect();
        let map: Vec<usize> = (0..rs.len()).collect();
        let mut out = Vec::new();
        parallel(&mut rs, &map, &d(list), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    /* every operand is the same standard input */
    fn par_stdin(operands: usize, input: &str, list: &str) -> String {
        let mut rs = vec![Rd::new(Cursor::new(input.as_bytes().to_vec()))];
        let map = vec![0; operands];
        let mut out = Vec::new();
        parallel(&mut rs, &map, &d(list), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn ser(inputs: &[&str], list: &str) -> String {
        let mut out = Vec::new();
        for s in inputs {
            let mut r = Rd::new(Cursor::new(s.as_bytes().to_vec()));
            serial(&mut r, &d(list), &mut out).unwrap();
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn delimiter_list() {
        assert_eq!(d("ab"), vec![b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(d("\\n\\t\\\\\\0"), vec![b"\n".to_vec(), b"\t".to_vec(), b"\\".to_vec(), b"".to_vec()]);
        /* any other character after a backslash stands for itself */
        assert_eq!(d("\\q"), vec![b"q".to_vec()]);
        assert_eq!(parse_delims(b"a\\"), None);
        assert_eq!(parse_delims(b""), Some(vec![]));
    }

    #[test]
    fn parallel_plain() {
        assert_eq!(par(&["a\nb\n", "1\n2\n"], "\t"), "a\t1\nb\t2\n");
        assert_eq!(par(&["a\nb\n", "1\n2\n", "x\ny\n"], ",:"), "a,1:x\nb,2:y\n");
        assert_eq!(par(&["a\nb\n", "1\n2\n", "x\ny\n"], ","), "a,1,x\nb,2,y\n");
    }

    #[test]
    fn parallel_short_inputs_give_empty_lines() {
        assert_eq!(par(&["a\nb\nc\n", "1\n"], "\t"), "a\t1\nb\t\nc\t\n");
        assert_eq!(par(&["a\n", "1\n2\n3\n"], "\t"), "a\t1\n\t2\n\t3\n");
        assert_eq!(par(&["", "1\n"], "-"), "-1\n");
        assert_eq!(par(&["", ""], "-"), "");
    }

    #[test]
    fn unterminated_last_line_is_a_line() {
        assert_eq!(par(&["a\nb", "1\n2"], "\t"), "a\t1\nb\t2\n");
        assert_eq!(ser(&["a\nb"], "\t"), "a\tb\n");
    }

    #[test]
    fn empty_delimiter_joins() {
        assert_eq!(par(&["a\n", "b\n", "c\n"], "\\0"), "abc\n");
        assert_eq!(par(&["a\n", "b\n", "c\n"], "\\0+"), "ab+c\n");
    }

    #[test]
    fn standard_input_in_turn() {
        assert_eq!(par_stdin(2, "1\n2\n3\n4\n", "\t"), "1\t2\n3\t4\n");
        assert_eq!(par_stdin(3, "1\n2\n3\n4\n", "\t"), "1\t2\t3\n4\t\t\n");
        assert_eq!(par_stdin(2, "1\n2\n3\n", ","), "1,2\n3,\n");
    }

    #[test]
    fn serial_plain() {
        assert_eq!(ser(&["a\nb\nc\n"], "\t"), "a\tb\tc\n");
        assert_eq!(ser(&["a\nb\nc\n", "1\n2\n"], ","), "a,b,c\n1,2\n");
    }

    #[test]
    fn serial_list_is_used_in_turn_and_restarts() {
        assert_eq!(ser(&["a\nb\nc\nd\n"], ",:"), "a,b:c,d\n");
        assert_eq!(ser(&["a\nb\nc\n", "1\n2\n3\n"], ",:"), "a,b:c\n1,2:3\n");
    }

    #[test]
    fn serial_empty_input_is_an_empty_line() {
        assert_eq!(ser(&["", "a\n", ""], "\t"), "\na\n\n");
        assert_eq!(ser(&["\n"], "\t"), "\n");
        assert_eq!(ser(&["\n\n"], ","), ",\n");
    }

    #[test]
    fn multibyte_delimiters() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        if !posix::locale::try_set("en_US.UTF-8") {
            return;
        }
        /* a character is one element, and \ before one takes the whole character */
        assert_eq!(d("\u{e9}\u{20ac}"), vec!["\u{e9}".as_bytes().to_vec(), "\u{20ac}".as_bytes().to_vec()]);
        assert_eq!(d("\\\u{20ac}"), vec!["\u{20ac}".as_bytes().to_vec()]);
        assert_eq!(par(&["a\n", "b\n", "c\n"], "\u{20ac}\u{e9}"), "a\u{20ac}b\u{e9}c\n");
        posix::locale::set_c();
    }
}
