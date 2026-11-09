/* SPDX-License-Identifier: GPL-2.0-only */
/* nl - line numbering filter. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::mb;
use posix::regex::Regex;
use std::io::{self, BufRead, BufReader, BufWriter, Write};

const SYNOPSIS: &str = "nl [-p] [-b type] [-d delim] [-f type] [-h type] [-i incr] [-l num]\n          [-n format] [-s sep] [-v startnum] [-w width] [file]";

/* Which lines of a section are numbered. */
enum Style {
    All,
    Text,
    Nothing,
    Matching(Regex),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Format {
    Left,
    Right,
    RightZeros,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Header,
    Body,
    Footer,
}

struct Opts {
    header: Style,
    body: Style,
    footer: Style,
    /* the two characters of a delimiter line, as bytes */
    delim: [Vec<u8>; 2],
    incr: i64,
    join: u64,
    format: Format,
    renumber: bool,
    sep: Vec<u8>,
    start: i64,
    width: usize,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            header: Style::Nothing,
            body: Style::Text,
            footer: Style::Nothing,
            delim: [b"\\".to_vec(), b":".to_vec()],
            incr: 1,
            join: 1,
            format: Format::Right,
            renumber: true,
            sep: b"\t".to_vec(),
            start: 1,
            width: 6,
        }
    }
}

/* The type argument of -b, -f and -h. */
fn parse_style(arg: &[u8]) -> Result<Style, String> {
    match arg {
        b"a" => Ok(Style::All),
        b"t" => Ok(Style::Text),
        b"n" => Ok(Style::Nothing),
        [b'p'] => Err("empty regular expression".to_string()),
        [b'p', re @ ..] => {
            let pat = posix::cstr(re);
            Regex::new(&pat, 0).map(Style::Matching).ok_or_else(|| "invalid regular expression".to_string())
        }
        _ => Err(format!("invalid line numbering type `{}'", String::from_utf8_lossy(arg))),
    }
}

/* The argument of -d: one or two characters; a lone one is followed by the default ':'. */
fn parse_delim(arg: &[u8]) -> Result<[Vec<u8>; 2], String> {
    let n1 = if arg.is_empty() { 0 } else { mb::char_at(arg).0 };
    let n2 = if n1 < arg.len() { mb::char_at(&arg[n1..]).0 } else { 0 };
    match (n1, n2) {
        (0, _) => Err("empty delimiter".to_string()),
        (_, 0) => Ok([arg.to_vec(), b":".to_vec()]),
        _ if n1 + n2 == arg.len() => Ok([arg[..n1].to_vec(), arg[n1..].to_vec()]),
        _ => Err("the delimiter is at most two characters".to_string()),
    }
}

/* A decimal number with an optional sign. */
fn parse_signed(arg: &[u8]) -> Option<i64> {
    let (neg, digits) = match arg.first() {
        Some(b'-') => (true, &arg[1..]),
        Some(b'+') => (false, &arg[1..]),
        _ => (false, arg),
    };
    let n = posix::parse_decimal(digits)?;
    if neg {
        0i64.checked_sub_unsigned(n)
    } else {
        i64::try_from(n).ok()
    }
}

/* n bytes of c, written in pieces so that a huge width costs time, not memory. */
fn fill<W: Write>(out: &mut W, c: u8, mut n: usize) -> io::Result<()> {
    let chunk = [c; 4096];
    while n > 0 {
        let k = n.min(chunk.len());
        out.write_all(&chunk[..k])?;
        n -= k;
    }
    Ok(())
}

/* The number, padded to the width as the format says. */
fn format_number<W: Write>(n: i64, f: Format, width: usize, out: &mut W) -> io::Result<()> {
    let digits = n.unsigned_abs().to_string();
    let len = digits.len() + usize::from(n < 0);
    let pad = width.saturating_sub(len);
    let sign: &[u8] = if n < 0 { b"-" } else { b"" };
    match f {
        Format::Right => {
            fill(out, b' ', pad)?;
            out.write_all(sign)?;
            out.write_all(digits.as_bytes())
        }
        Format::Left => {
            out.write_all(sign)?;
            out.write_all(digits.as_bytes())?;
            fill(out, b' ', pad)
        }
        Format::RightZeros => {
            out.write_all(sign)?;
            fill(out, b'0', pad)?;
            out.write_all(digits.as_bytes())
        }
    }
}

/* Why a line could not be written. */
enum Fail {
    Overflow,
    Io(io::Error),
}

impl From<io::Error> for Fail {
    fn from(e: io::Error) -> Fail {
        Fail::Io(e)
    }
}

/* The state of the numbering. */
struct Nl {
    o: Opts,
    section: Section,
    /* None once an increment went past what a number can hold */
    number: Option<i64>,
    /* empty lines met in a row that -l counts */
    blank_run: u64,
    /* the delimiter lines: header, body, footer */
    marks: [Vec<u8>; 3],
}

impl Nl {
    fn new(o: Opts) -> Nl {
        let unit = [o.delim[0].clone(), o.delim[1].clone()].concat();
        let marks = [unit.repeat(3), unit.repeat(2), unit.clone()];
        let number = Some(o.start);
        Nl { o, section: Section::Body, number, blank_run: 0, marks }
    }

    /* Is the line to be numbered under the style of its section? */
    fn wanted(&mut self, line: &[u8]) -> bool {
        let style = match self.section {
            Section::Header => &self.o.header,
            Section::Body => &self.o.body,
            Section::Footer => &self.o.footer,
        };
        match style {
            Style::Nothing => false,
            Style::Text => !line.is_empty(),
            Style::Matching(re) => {
                /* a NUL ends the text the matcher sees */
                let end = line.iter().position(|&c| c == 0).unwrap_or(line.len());
                re.exec(&posix::cstr(&line[..end]), 1).is_some()
            }
            Style::All => {
                if !line.is_empty() {
                    self.blank_run = 0;
                    return true;
                }
                self.blank_run += 1;
                if self.blank_run >= self.o.join {
                    self.blank_run = 0;
                    true
                } else {
                    false
                }
            }
        }
    }

    /* Write one input line (without its newline). */
    fn line<W: Write>(&mut self, line: &[u8], out: &mut W) -> Result<(), Fail> {
        let section = [Section::Header, Section::Body, Section::Footer];
        if let Some(i) = self.marks.iter().position(|m| m == line) {
            self.section = section[i];
            if i == 0 && self.o.renumber {
                self.number = Some(self.o.start);
                self.blank_run = 0;
            }
            out.write_all(b"\n")?;
            return Ok(());
        }
        if self.wanted(line) {
            let n = self.number.ok_or(Fail::Overflow)?;
            format_number(n, self.o.format, self.o.width, out)?;
            out.write_all(&self.o.sep)?;
            self.number = n.checked_add(self.o.incr);
        }
        out.write_all(line)?;
        out.write_all(b"\n")?;
        Ok(())
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut o = Opts::default();
    let die = |m: &str| -> ! { posix::fail(&prog, m) };
    let mut g = Getopt::new(&args, b"pb:d:f:h:i:l:n:s:v:w:");
    for opt in &mut g {
        match opt {
            Opt::Flag(b'p') => o.renumber = false,
            Opt::Arg(c @ (b'b' | b'f' | b'h'), v) => {
                let s = parse_style(&v).unwrap_or_else(|m| die(&m));
                match c {
                    b'b' => o.body = s,
                    b'f' => o.footer = s,
                    _ => o.header = s,
                }
            }
            Opt::Arg(b'd', v) => o.delim = parse_delim(&v).unwrap_or_else(|m| die(&m)),
            Opt::Arg(b'i', v) => o.incr = parse_signed(&v).unwrap_or_else(|| die("invalid increment")),
            Opt::Arg(b'l', v) => o.join = posix::parse_decimal(&v).unwrap_or_else(|| die("invalid number of blank lines")),
            Opt::Arg(b'n', v) => {
                o.format = match v.as_slice() {
                    b"ln" => Format::Left,
                    b"rn" => Format::Right,
                    b"rz" => Format::RightZeros,
                    _ => die("invalid line numbering format"),
                }
            }
            Opt::Arg(b's', v) => o.sep = v,
            Opt::Arg(b'v', v) => o.start = parse_signed(&v).unwrap_or_else(|| die("invalid starting number")),
            Opt::Arg(b'w', v) => {
                o.width = match posix::parse_decimal(&v) {
                    Some(n) if n > 0 => usize::try_from(n).unwrap_or(usize::MAX),
                    _ => die("invalid line number width"),
                }
            }
            _ => posix::usage(SYNOPSIS),
        }
    }
    let ops = g.operands();
    if ops.len() > 1 {
        posix::usage(SYNOPSIS);
    }
    let name: &[u8] = ops.first().map_or(b"-", |v| v.as_slice());
    let input = match posix::open_input(name) {
        Ok(f) => f,
        Err(e) => {
            if name == b"-" {
                posix::fail(&prog, &format!("standard input: {}", posix::strerror(&e)));
            }
            posix::warn_path(&prog, name, &posix::strerror(&e));
            std::process::exit(1);
        }
    };
    let mut rd = BufReader::with_capacity(65536, input);
    let mut nl = Nl::new(o);
    let mut out = BufWriter::with_capacity(65536, posix::Stdout::new());
    let mut line = Vec::new();
    loop {
        line.clear();
        match rd.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let _ = out.flush();
                if name == b"-" {
                    posix::fail(&prog, &format!("standard input: {}", posix::strerror(&e)));
                }
                posix::warn_path(&prog, name, &posix::strerror(&e));
                std::process::exit(1);
            }
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        match nl.line(&line, &mut out) {
            Ok(()) => {}
            Err(Fail::Overflow) => {
                let _ = out.flush();
                posix::fail(&prog, "line number overflow");
            }
            Err(Fail::Io(e)) => posix::fail(&prog, &format!("write error: {}", posix::strerror(&e))),
        }
    }
    if let Err(e) = out.flush() {
        posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /* the locale is process-wide: tests that read it hold this */
    static LOCALE: Mutex<()> = Mutex::new(());

    /* nl with options given as the command line would, on the input */
    fn nl(args: &[&str], input: &str) -> String {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        posix::locale::set_c();
        run(args, input)
    }

    fn run(args: &[&str], input: &str) -> String {
        let mut v: Vec<Vec<u8>> = vec![b"nl".to_vec()];
        v.extend(args.iter().map(|a| a.as_bytes().to_vec()));
        let mut o = Opts::default();
        let mut g = Getopt::new(&v, b"pb:d:f:h:i:l:n:s:v:w:");
        for opt in &mut g {
            match opt {
                Opt::Flag(b'p') => o.renumber = false,
                Opt::Arg(b'b', a) => o.body = parse_style(&a).unwrap(),
                Opt::Arg(b'f', a) => o.footer = parse_style(&a).unwrap(),
                Opt::Arg(b'h', a) => o.header = parse_style(&a).unwrap(),
                Opt::Arg(b'd', a) => o.delim = parse_delim(&a).unwrap(),
                Opt::Arg(b'i', a) => o.incr = parse_signed(&a).unwrap(),
                Opt::Arg(b'l', a) => o.join = posix::parse_decimal(&a).unwrap(),
                Opt::Arg(b'n', a) => {
                    o.format = match a.as_slice() {
                        b"ln" => Format::Left,
                        b"rn" => Format::Right,
                        _ => Format::RightZeros,
                    }
                }
                Opt::Arg(b's', a) => o.sep = a,
                Opt::Arg(b'v', a) => o.start = parse_signed(&a).unwrap(),
                Opt::Arg(b'w', a) => o.width = posix::parse_decimal(&a).unwrap() as usize,
                _ => panic!("option"),
            }
        }
        let mut n = Nl::new(o);
        let mut out = Vec::new();
        for l in input.split_inclusive('\n') {
            assert!(n.line(l.trim_end_matches('\n').as_bytes(), &mut out).is_ok());
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn example_from_the_spec() {
        /* nl -v 10 -i 10 -d \!+ file1 */
        assert_eq!(nl(&["-v", "10", "-i", "10", "-d", "!+"], "a\nb\nc\n"), "    10\ta\n    20\tb\n    30\tc\n");
        assert_eq!(nl(&["-v", "10", "-i", "10", "-d", "!+", "-h", "a"], "a\n!+!+!+\nb\n"), "    10\ta\n\n    10\tb\n");
    }

    #[test]
    fn default_numbers_the_non_empty_lines_of_the_body() {
        assert_eq!(nl(&[], "a\n\nb\n"), "     1\ta\n\n     2\tb\n");
        /* a line of blanks is not empty */
        assert_eq!(nl(&[], " \nb\n"), "     1\t \n     2\tb\n");
    }

    #[test]
    fn types() {
        assert_eq!(nl(&["-b", "a"], "a\n\nb\n"), "     1\ta\n     2\t\n     3\tb\n");
        assert_eq!(nl(&["-b", "n"], "a\nb\n"), "a\nb\n");
        assert_eq!(nl(&["-b", "t"], "a\n\n"), "     1\ta\n\n");
        assert_eq!(nl(&["-b", "pb"], "abc\nxyz\nab\n"), "     1\tabc\nxyz\n     2\tab\n");
        assert_eq!(nl(&["-b", "p^x.z$"], "abc\nxyz\n"), "abc\n     1\txyz\n");
    }

    #[test]
    fn header_and_footer_default_to_no_numbers() {
        let input = "\\:\\:\\:\nh\n\\:\\:\nb\n\\:\nf\n";
        assert_eq!(nl(&[], input), "\nh\n\n     1\tb\n\nf\n");
        assert_eq!(nl(&["-h", "a", "-f", "a"], input), "\n     1\th\n\n     2\tb\n\n     3\tf\n");
    }

    #[test]
    fn numbering_restarts_at_each_header_only() {
        let input = "a\n\\:\\:\\:\nb\n\\:\\:\nc\n\\:\nd\n\\:\\:\\:\ne\n";
        let a = "-h a -f a -b a".split(' ').collect::<Vec<_>>();
        assert_eq!(
            nl(&a, input),
            "     1\ta\n\n     1\tb\n\n     2\tc\n\n     3\td\n\n     1\te\n"
        );
        /* -p keeps counting */
        let mut p = vec!["-p"];
        p.extend(&a);
        assert_eq!(
            nl(&p, input),
            "     1\ta\n\n     2\tb\n\n     3\tc\n\n     4\td\n\n     5\te\n"
        );
    }

    #[test]
    fn delimiter_lines_are_exact() {
        /* nothing but the delimiter characters, whole */
        let t = |l: &str| nl(&["-h", "a"], &format!("\\:\\:\\:\nx\n{l}\ny\n"));
        assert_eq!(t("\\:\\:\\: "), "\n     1\tx\n     2\t\\:\\:\\: \n     3\ty\n");
        assert_eq!(t("\\:\\:\\:\\:"), "\n     1\tx\n     2\t\\:\\:\\:\\:\n     3\ty\n");
        assert_eq!(t("\\: \\:"), "\n     1\tx\n     2\t\\: \\:\n     3\ty\n");
    }

    #[test]
    fn delimiter_option() {
        /* two characters */
        assert_eq!(nl(&["-d", "xy", "-h", "a"], "a\nxyxyxy\nb\nxyxy\nc\n"), "     1\ta\n\n     1\tb\n\n     2\tc\n");
        /* one character: the second stays ':' */
        assert_eq!(nl(&["-d", "x", "-h", "a"], "x:x:x:\na\n\\:\\:\\:\n"), "\n     1\ta\n     2\t\\:\\:\\:\n");
    }

    #[test]
    fn delimiter_of_multibyte_characters() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        if !posix::locale::try_set("en_US.UTF-8") {
            return;
        }
        let r = run(&["-d", "\u{e9}\u{20ac}", "-h", "a"], "\u{e9}\u{20ac}\u{e9}\u{20ac}\u{e9}\u{20ac}\nx\n\u{e9}:\n");
        posix::locale::set_c();
        assert_eq!(r, "\n     1\tx\n     2\t\u{e9}:\n");
        assert!(parse_delim("\u{e9}\u{20ac}x".as_bytes()).is_err());
        assert!(parse_delim(b"").is_err());
        assert!(parse_delim(b"abc").is_err());
    }

    #[test]
    fn blank_lines_counted_as_one() {
        assert_eq!(nl(&["-b", "a", "-l", "2"], "a\n\n\n\n\nb\n"), "     1\ta\n\n     2\t\n\n     3\t\n     4\tb\n");
        /* a line with text starts the count again */
        assert_eq!(nl(&["-b", "a", "-l", "2"], "\na\n\n"), "\n     1\ta\n\n");
        assert_eq!(nl(&["-b", "a", "-l", "3"], "\n\n\n\n"), "\n\n     1\t\n\n");
        /* 0 and 1 number every one */
        assert_eq!(nl(&["-b", "a", "-l", "0"], "\n\n"), "     1\t\n     2\t\n");
    }

    #[test]
    fn formats_widths_and_separators() {
        assert_eq!(nl(&["-n", "ln"], "a\n"), "1     \ta\n");
        assert_eq!(nl(&["-n", "rn"], "a\n"), "     1\ta\n");
        assert_eq!(nl(&["-n", "rz"], "a\n"), "000001\ta\n");
        assert_eq!(nl(&["-n", "rz", "-w", "3", "-v", "-5", "-i", "3"], "a\nb\nc\n"), "-05\ta\n-02\tb\n001\tc\n");
        assert_eq!(nl(&["-n", "ln", "-s", ":", "-w", "4"], "a\nb\n"), "1   :a\n2   :b\n");
        assert_eq!(nl(&["-w", "1", "-v", "9"], "a\nb\n"), "9\ta\n10\tb\n");
        assert_eq!(nl(&["-s", ""], "a\n"), "     1a\n");
        assert_eq!(nl(&["-s", "ab"], "a\n"), "     1aba\n");
        assert_eq!(nl(&["-n", "ln", "-v", "-3", "-w", "4"], "a\n"), "-3  \ta\n");
    }

    #[test]
    fn numbers_without_a_number_have_no_separator() {
        assert_eq!(nl(&["-s", "XX"], "a\n\nb\n"), "     1XXa\n\n     2XXb\n");
    }

    #[test]
    fn increments() {
        assert_eq!(nl(&["-i", "0"], "a\nb\n"), "     1\ta\n     1\tb\n");
        assert_eq!(nl(&["-i", "-2", "-v", "5"], "a\nb\nc\n"), "     5\ta\n     3\tb\n     1\tc\n");
        assert_eq!(parse_signed(b"+7"), Some(7));
        assert_eq!(parse_signed(b"-7"), Some(-7));
        assert_eq!(parse_signed(b"9223372036854775808"), None);
        assert_eq!(parse_signed(b"-9223372036854775808"), Some(i64::MIN));
        assert_eq!(parse_signed(b"x"), None);
        assert_eq!(parse_signed(b""), None);
    }

    #[test]
    fn a_number_past_the_limit_is_an_error_before_the_line() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let mut n = Nl::new(Opts { start: i64::MAX - 1, ..Opts::default() });
        let mut out = Vec::new();
        assert!(n.line(b"a", &mut out).is_ok());
        assert!(n.line(b"b", &mut out).is_ok());
        assert!(matches!(n.line(b"c", &mut out), Err(Fail::Overflow)));
        assert!(out.ends_with(b"9223372036854775807\tb\n"));
        /* a header starts again */
        assert!(n.line(b"\\:\\:\\:", &mut out).is_ok());
        assert!(n.line(b"d", &mut out).is_ok());
    }

    #[test]
    fn huge_width_is_written_in_pieces() {
        let mut out = Vec::new();
        format_number(5, Format::Right, 10_000, &mut out).unwrap();
        assert_eq!(out.len(), 10_000);
        assert!(out.ends_with(b" 5"));
    }

    #[test]
    fn style_arguments() {
        assert!(parse_style(b"x").is_err());
        assert!(parse_style(b"").is_err());
        assert!(parse_style(b"at").is_err());
        assert!(parse_style(b"p[").is_err());
        assert!(parse_style(b"p").is_err());
        assert!(parse_style(b"pa").is_ok());
    }
}
