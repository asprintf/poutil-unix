/* SPDX-License-Identifier: GPL-2.0-only */
/* xargs - construct argument lists and invoke utility. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use std::io::{self, Read, Write};

const SYNOPSIS: &str = "xargs [-ptx] [-E eofstr] [[-I replstr]|[-L number]|[-n number]] [-s size] [utility [argument...]]";

/* default command line size, without -s */
const DEFAULT_SIZE: usize = 128 * 1024;
/* room left to the utility */
const RESERVE: usize = 2048;
/* cost of an argv pointer */
const PTR: usize = std::mem::size_of::<usize>();

/* <blank> */
fn is_blank(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

/* ---- the input ---- */

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    Arg(Vec<u8>),
    /* end of a logical line, for -L */
    Eol,
    Eof,
}

struct Lexer<R: Read> {
    r: R,
    buf: Vec<u8>,
    pos: usize,
    end: usize,
    eof: bool,
    /* -I: a line is an argument, blanks are part of it */
    insert: bool,
    /* longer than this can never be passed */
    max_arg: usize,
    /* state of the line being read */
    line_empty: bool,
    trailing_blank: bool,
    cont: bool,
}

impl<R: Read> Lexer<R> {
    fn new(r: R, insert: bool, max_arg: usize) -> Lexer<R> {
        Lexer { r, buf: vec![0; 64 * 1024], pos: 0, end: 0, eof: false, insert, max_arg, line_empty: true, trailing_blank: false, cont: false }
    }

    fn peek(&mut self) -> io::Result<Option<u8>> {
        if self.pos == self.end {
            if self.eof {
                return Ok(None);
            }
            loop {
                match self.r.read(&mut self.buf) {
                    Ok(0) => {
                        self.eof = true;
                        return Ok(None);
                    }
                    Ok(n) => {
                        self.pos = 0;
                        self.end = n;
                        break;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
        }
        let c = self.buf[self.pos];
        if c == 0 {
            return Err(io::Error::other("input contains a NUL character"));
        }
        Ok(Some(c))
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    /* Text up to the closing quote; a newline or end of input before it is an error. */
    fn quoted(&mut self, q: u8, out: &mut Vec<u8>) -> io::Result<()> {
        loop {
            match self.peek()? {
                None | Some(b'\n') => {
                    return Err(io::Error::other(if q == b'"' { "unmatched double quote" } else { "unmatched single quote" }));
                }
                Some(c) => {
                    self.bump();
                    if c == q {
                        return Ok(());
                    }
                    out.push(c);
                    self.too_long(out)?;
                }
            }
        }
    }

    fn too_long(&self, out: &[u8]) -> io::Result<()> {
        if out.len() >= self.max_arg {
            return Err(io::Error::other("argument line too long"));
        }
        Ok(())
    }

    /* Does this newline end a logical line? */
    fn newline(&mut self) -> bool {
        let was_empty = self.line_empty;
        self.line_empty = true;
        if was_empty && self.cont {
            return false;
        }
        self.cont = self.trailing_blank;
        self.trailing_blank = false;
        !self.cont
    }

    fn next(&mut self) -> io::Result<Tok> {
        if self.insert {
            return self.next_line();
        }
        loop {
            let Some(c) = self.peek()? else { return Ok(Tok::Eof) };
            if is_blank(c) {
                self.bump();
                self.line_empty = false;
                self.trailing_blank = true;
            } else if c == b'\n' {
                self.bump();
                if self.newline() {
                    return Ok(Tok::Eol);
                }
            } else {
                return self.token();
            }
        }
    }

    /* One argument, quotes and backslashes taken out. */
    fn token(&mut self) -> io::Result<Tok> {
        let mut out = Vec::new();
        self.line_empty = false;
        self.trailing_blank = false;
        while let Some(c) = self.peek()? {
            if is_blank(c) || c == b'\n' {
                break;
            }
            self.bump();
            match c {
                b'"' | b'\'' => self.quoted(c, &mut out)?,
                /* a backslash at the end of the input is dropped */
                b'\\' => {
                    if let Some(n) = self.peek()? {
                        self.bump();
                        out.push(n);
                    }
                }
                _ => out.push(c),
            }
            self.too_long(&out)?;
        }
        Ok(Tok::Arg(out))
    }

    /* -I: a line is one argument. */
    fn next_line(&mut self) -> io::Result<Tok> {
        loop {
            let mut out = Vec::new();
            let mut any = false;
            while let Some(c) = self.peek()? {
                if !is_blank(c) {
                    break;
                }
                self.bump();
            }
            while let Some(c) = self.peek()? {
                self.bump();
                if c == b'\n' {
                    break;
                }
                any = true;
                match c {
                    b'"' | b'\'' => self.quoted(c, &mut out)?,
                    b'\\' => {
                        if let Some(n) = self.peek()? {
                            self.bump();
                            out.push(n);
                        }
                    }
                    _ => out.push(c),
                }
                self.too_long(&out)?;
            }
            if any {
                return Ok(Tok::Arg(out));
            }
            if self.peek()?.is_none() {
                return Ok(Tok::Eof);
            }
        }
    }
}

/* ---- the command lines ---- */

#[derive(Clone, PartialEq, Eq, Debug)]
enum Mode {
    /* as many as fit */
    Default,
    /* at most n arguments */
    N(usize),
    /* n logical lines */
    L(usize),
    /* one line, put in place of the string */
    I(Vec<u8>),
}

struct Limits {
    /* -s */
    size: Option<usize>,
    /* what {ARG_MAX} leaves */
    room: usize,
    /* -n without -s: {LINE_MAX} cuts the arguments short */
    line_max: Option<usize>,
    /* without -n and -s: "at least {LINE_MAX}", whatever {LINE_MAX} is here */
    default_size: usize,
}

impl Limits {
    /* total: bytes with their nulls; count: strings */
    fn fits(&self, total: usize, count: usize) -> bool {
        let user = match self.size {
            Some(s) => total < s,
            None => total <= self.default_size,
        };
        user && total.saturating_add(PTR.saturating_mul(count + 1)) <= self.room
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Done,
    /* exited non-zero: go on, end with an error */
    Failed,
    /* end now with this status */
    Stop(i32),
}

struct Core<'a> {
    mode: Mode,
    eof: Option<Vec<u8>>,
    /* utility and arguments */
    fixed: Vec<Vec<u8>>,
    lim: Limits,
    /* -x with -n or -L; always with -I */
    exit_if_too_big: bool,
    exec: &'a mut dyn FnMut(&[Vec<u8>]) -> Outcome,
    complain: &'a mut dyn FnMut(&str),
    failed: bool,
}

fn size_of_all(v: &[Vec<u8>]) -> usize {
    v.iter().map(|a| a.len() + 1).sum()
}

/* s with each what replaced by with; None if the result passes max bytes */
fn replace(s: &[u8], what: &[u8], with: &[u8], max: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with(what) {
            out.extend_from_slice(with);
            i += what.len();
        } else {
            out.push(s[i]);
            i += 1;
        }
        if out.len() > max {
            return None;
        }
    }
    Some(out)
}

impl Core<'_> {
    fn stop(&mut self, msg: &str, status: i32) -> i32 {
        (self.complain)(msg);
        status
    }

    fn run(&mut self, argv: &[Vec<u8>]) -> Option<i32> {
        match (self.exec)(argv) {
            Outcome::Done => None,
            Outcome::Failed => {
                self.failed = true;
                None
            }
            Outcome::Stop(s) => Some(s),
        }
    }

    /* Returns the exit status. */
    fn process<R: Read>(&mut self, lx: &mut Lexer<R>) -> i32 {
        let end = |c: &Core| if c.failed { 123 } else { 0 };
        let fixed_size = size_of_all(&self.fixed);
        if !matches!(self.mode, Mode::I(_)) && !self.lim.fits(fixed_size, self.fixed.len()) {
            return self.stop("argument list too long", 1);
        }
        /* -x: "terminate if a constructed command line will not fit in the implied size"; the utility and its own arguments are part of every line */
        if self.exit_if_too_big && self.lim.line_max.is_some_and(|l| fixed_size > l) {
            return self.stop("argument list too long", 1);
        }
        let mut batch: Vec<Vec<u8>> = Vec::new();
        let mut size = fixed_size;
        let (mut lines, mut line_args) = (0usize, 0usize);
        let mut ran = false;
        let mut seen_any = false;
        loop {
            let tok = match lx.next() {
                Ok(t) => t,
                Err(e) => return self.stop(&posix::strerror(&e), 1),
            };
            let a = match tok {
                Tok::Eof => break,
                Tok::Eol => {
                    if line_args > 0 {
                        line_args = 0;
                        lines += 1;
                        if matches!(self.mode, Mode::L(n) if lines == n) {
                            ran = true;
                            if let Some(s) = self.run(&[self.fixed.as_slice(), batch.as_slice()].concat()) {
                                return s;
                            }
                            batch.clear();
                            size = fixed_size;
                            lines = 0;
                        }
                    }
                    continue;
                }
                Tok::Arg(a) => a,
            };
            if self.eof.as_deref() == Some(a.as_slice()) {
                break;
            }
            seen_any = true;
            if let Mode::I(repl) = &self.mode {
                /* the utility name is not an argument */
                let mut argv = vec![self.fixed[0].clone()];
                for f in &self.fixed[1..] {
                    match replace(f, repl, &a, self.lim.room) {
                        Some(r) => argv.push(r),
                        None => return self.stop("argument line too long", 1),
                    }
                }
                if !self.lim.fits(size_of_all(&argv), argv.len()) {
                    return self.stop("argument line too long", 1);
                }
                ran = true;
                if let Some(s) = self.run(&argv) {
                    return s;
                }
                continue;
            }
            let cost = a.len() + 1;
            let over = !self.lim.fits(size + cost, self.fixed.len() + batch.len() + 1)
                || self.lim.line_max.is_some_and(|l| (!batch.is_empty() || self.exit_if_too_big) && size + cost > l);
            if over {
                if batch.is_empty() {
                    return self.stop("argument too long", 1);
                }
                if self.exit_if_too_big {
                    return self.stop("argument line too long", 1);
                }
                ran = true;
                if let Some(s) = self.run(&[self.fixed.as_slice(), batch.as_slice()].concat()) {
                    return s;
                }
                batch.clear();
                size = fixed_size;
                lines = 0;
                if !self.lim.fits(size + cost, self.fixed.len() + 1) {
                    return self.stop("argument too long", 1);
                }
            }
            batch.push(a);
            size += cost;
            line_args += 1;
            if matches!(self.mode, Mode::N(n) if batch.len() == n) {
                ran = true;
                if let Some(s) = self.run(&[self.fixed.as_slice(), batch.as_slice()].concat()) {
                    return s;
                }
                batch.clear();
                size = fixed_size;
                line_args = 0;
            }
        }
        if !batch.is_empty() {
            if let Some(s) = self.run(&[self.fixed.as_slice(), batch.as_slice()].concat()) {
                return s;
            }
        } else if !ran && !seen_any && !matches!(self.mode, Mode::I(_)) {
            let fixed = self.fixed.clone();
            if let Some(s) = self.run(&fixed) {
                return s;
            }
        }
        end(self)
    }
}

/* ---- the real thing ---- */

/*
 * An argument as -t shows it: "the utility and its constructed argument list ... as it will be invoked".
 * Plain when nothing in it needs quoting, otherwise written so that a shell reads it back as the same
 * argument and the arguments stay apart ('a b' is one, a b is two): single quotes (a quote as '\''),
 * double quotes when only quotes and blanks are in it, and $'..' for what cannot be seen (control bytes,
 * bytes that are no printable character of the locale). The same text as GNU xargs 4.9.
 */
fn trace_quote(arg: &[u8]) -> Vec<u8> {
    if arg.is_empty() {
        return b"''".to_vec();
    }
    /* (bytes, printable) for each character */
    let mut items: Vec<(&[u8], bool)> = Vec::new();
    let mut i = 0;
    while i < arg.len() {
        let b = arg[i];
        let (n, print) = if b < 0x80 {
            (1, (0x20..0x7f).contains(&b))
        } else {
            match posix::mb::char_at(&arg[i..]) {
                (n, Some(wc)) => (n, posix::mb::is_print(wc)),
                (n, None) => (n, false),
            }
        };
        items.push((&arg[i..i + n], print));
        i += n;
    }
    let special = |c: u8| b" !\"$&()*;<=>?[^`|\\'".contains(&c);
    let lead = matches!(arg[0], b'#' | b'~');
    let brace = arg == b"{" || arg == b"}";
    let all_print = items.iter().all(|it| it.1);
    let other = lead || brace || arg.iter().any(|&c| c != b'\'' && c != b' ' && special(c));
    if all_print && !lead && !brace && !arg.iter().any(|&c| special(c)) {
        return arg.to_vec();
    }
    if all_print && !other && arg.contains(&b'\'') {
        /* only quotes and blanks: double quotes hold them all */
        let mut o = vec![b'"'];
        o.extend_from_slice(arg);
        o.push(b'"');
        return o;
    }
    let mut o = vec![b'\''];
    let mut esc = false;
    for (bytes, print) in items {
        if print {
            if bytes == b"'" {
                o.extend_from_slice(b"'\\''");
                esc = false;
            } else {
                if esc {
                    o.extend_from_slice(b"''");
                    esc = false;
                }
                o.extend_from_slice(bytes);
            }
        } else {
            if !esc {
                o.extend_from_slice(b"'$'");
                esc = true;
            }
            for &c in bytes {
                match c {
                    7 => o.extend_from_slice(b"\\a"),
                    8 => o.extend_from_slice(b"\\b"),
                    9 => o.extend_from_slice(b"\\t"),
                    10 => o.extend_from_slice(b"\\n"),
                    11 => o.extend_from_slice(b"\\v"),
                    12 => o.extend_from_slice(b"\\f"),
                    13 => o.extend_from_slice(b"\\r"),
                    _ => o.extend_from_slice(format!("\\{:03o}", c).as_bytes()),
                }
            }
        }
    }
    o.push(b'\'');
    o
}

struct Real {
    prog: Vec<u8>,
    trace: bool,
    /* for -p */
    tty: Option<std::fs::File>,
    yes: Option<posix::prompt::Yes>,
}

impl Real {
    fn exec(&mut self, argv: &[Vec<u8>]) -> Outcome {
        if self.trace {
            let mut line = argv.iter().map(|a| trace_quote(a)).collect::<Vec<_>>().join(&b' ');
            if self.tty.is_none() {
                line.push(b'\n');
            }
            let mut e = io::stderr().lock();
            let _ = e.write_all(&line);
            let _ = e.flush();
        }
        if let (Some(tty), Some(yes)) = (&self.tty, &self.yes) {
            use std::os::fd::AsRawFd;
            if !posix::prompt::ask_fd(yes, tty.as_raw_fd(), b"?...") {
                return Outcome::Done;
            }
        }
        match posix::sys::run_command(argv, true) {
            Err(e) => {
                posix::warn_path(&self.prog, &argv[0], &posix::strerror(&e));
                let missing = matches!(e.raw_os_error(), Some(libc::ENOENT) | Some(libc::ENOTDIR));
                Outcome::Stop(if missing { 127 } else { 126 })
            }
            Ok(posix::sys::Wait::Exited(0)) => Outcome::Done,
            Ok(posix::sys::Wait::Exited(255)) => {
                posix::warn(&self.prog, &format!("{}: exited with status 255; aborting", String::from_utf8_lossy(&argv[0])));
                Outcome::Stop(124)
            }
            Ok(posix::sys::Wait::Exited(_)) => Outcome::Failed,
            Ok(posix::sys::Wait::Signaled(sig)) => {
                posix::warn(&self.prog, &format!("{}: terminated by signal {}", String::from_utf8_lossy(&argv[0]), sig));
                Outcome::Stop(125)
            }
        }
    }
}

/* A positive decimal integer option-argument. */
fn number(prog: &[u8], what: &str, v: &[u8]) -> usize {
    match posix::parse_decimal(v) {
        Some(n) if n > 0 => usize::try_from(n).unwrap_or(usize::MAX),
        _ => posix::fail(prog, &format!("{what}: invalid number '{}'", String::from_utf8_lossy(v))),
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let (mut prompt, mut trace, mut x) = (false, false, false);
    let mut eof: Option<Vec<u8>> = None;
    let mut mode = Mode::Default;
    let mut size: Option<usize> = None;
    let mut g = Getopt::new(&args, b"ptxE:I:L:n:s:");
    for o in g.by_ref() {
        match o {
            Opt::Flag(b'p') => prompt = true,
            Opt::Flag(b't') => trace = true,
            Opt::Flag(b'x') => x = true,
            Opt::Arg(b'E', v) => eof = if v.is_empty() { None } else { Some(v) },
            Opt::Arg(b'I', v) => {
                if v.is_empty() {
                    posix::usage(SYNOPSIS);
                }
                mode = Mode::I(v);
            }
            /* the last of -L and -n wins; -I stays */
            Opt::Arg(b'L', v) => {
                let n = number(&prog, "-L", &v);
                if !matches!(mode, Mode::I(_)) {
                    mode = Mode::L(n);
                }
            }
            Opt::Arg(b'n', v) => {
                let n = number(&prog, "-n", &v);
                if !matches!(mode, Mode::I(_)) {
                    mode = Mode::N(n);
                }
            }
            Opt::Arg(b's', v) => size = Some(number(&prog, "-s", &v)),
            _ => posix::usage(SYNOPSIS),
        }
    }
    let ops = g.operands();
    let fixed: Vec<Vec<u8>> = if ops.is_empty() { vec![b"echo".to_vec()] } else { ops.to_vec() };

    let tty = if prompt {
        match std::fs::OpenOptions::new().read(true).open("/dev/tty") {
            Ok(f) => Some(f),
            Err(e) => posix::fail(&prog, &format!("/dev/tty: {}", posix::strerror(&e))),
        }
    } else {
        None
    };
    let yes = prompt.then(posix::prompt::Yes::new);
    let mut real = Real { prog: prog.clone(), trace: trace || prompt, tty, yes };

    let (mut env_bytes, mut envc) = (0usize, 0usize);
    for (k, v) in std::env::vars_os() {
        env_bytes += k.len() + v.len() + 2;
        envc += 1;
    }
    let arg_max = posix::sys::arg_max().unwrap_or(DEFAULT_SIZE);
    let room = arg_max.saturating_sub(RESERVE + env_bytes + PTR * (envc + 1));
    let line_max = (size.is_none() && matches!(mode, Mode::N(_))).then(posix::sys::line_max);
    let lim = Limits { size, room, line_max, default_size: DEFAULT_SIZE.max(posix::sys::line_max()) };

    let insert = matches!(mode, Mode::I(_));
    let exit_if_too_big = insert || (x && !matches!(mode, Mode::Default));
    /* not std's stdin: it takes a closed descriptor for an empty input */
    let input = posix::borrow_fd(0);
    let mut lx = Lexer::new(&*input, insert, room.max(1));
    let prog2 = prog.clone();
    let mut complain = move |m: &str| posix::warn(&prog2, m);
    let mut exec = |argv: &[Vec<u8>]| real.exec(argv);
    let mut core = Core { mode, eof, fixed, lim, exit_if_too_big, exec: &mut exec, complain: &mut complain, failed: false };
    let status = core.process(&mut lx);
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex_with(input: &[u8], insert: bool) -> Result<Vec<Tok>, String> {
        let mut lx = Lexer::new(input, insert, 1 << 20);
        let mut v = Vec::new();
        loop {
            match lx.next() {
                Ok(Tok::Eof) => return Ok(v),
                Ok(t) => v.push(t),
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    /* a reader that gives at most n bytes a call and fails with Interrupted before every other one */
    struct Dribble<'a> {
        data: &'a [u8],
        n: usize,
        flip: bool,
    }

    impl Read for Dribble<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.flip = !self.flip;
            if self.flip {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            let k = self.n.min(buf.len()).min(self.data.len());
            buf[..k].copy_from_slice(&self.data[..k]);
            self.data = &self.data[k..];
            Ok(k)
        }
    }

    #[test]
    fn the_tokens_do_not_depend_on_how_the_input_arrives() {
        /* quotes, escapes, blanks, newlines, continuation lines and multibyte characters cut at every place */
        let inputs: [&[u8]; 16] = [
            b"a b\tc\n d  e \n\n f\n",
            b"\"a b\" 'c d' \"e'f\" 'g\"h'\n",
            b"a\"b c\"d'e f'g\n",
            b"a\\ b c\\\"d \\'e \\\\f \\x\n",
            b"a\\\nb\n",
            b"a \nb\n  \nc d \n\ne\n",
            b"a\\ \nb\n",
            b"\"\" '' x\n",
            b"\"a\nb\"\n",
            b"'a\nb'\n",
            b"\xc3\xa9 \xf0\x9f\x98\x80 \xe4\xbd\xa0\xe5\xa5\xbd\n\xf0\x9f\x91\xa8\xe2\x80\x8d\xf0\x9f\x91\xa9 x\n",
            b"\xff \xc3 \xe4\xbd x\n",
            b"a\\",
            b"\"a",
            b"'a b",
            b"x",
        ];
        for input in inputs {
            for insert in [false, true] {
                let whole = lex_with(input, insert);
                for n in [1, 2, 3, 5, 7] {
                    let mut lx = Lexer::new(Dribble { data: input, n, flip: false }, insert, 1 << 20);
                    let mut v = Vec::new();
                    let got = loop {
                        match lx.next() {
                            Ok(Tok::Eof) => break Ok(v),
                            Ok(t) => v.push(t),
                            Err(e) => break Err(e.to_string()),
                        }
                    };
                    assert_eq!(got, whole, "input {:?}, insert {insert}, {n} bytes a read", String::from_utf8_lossy(input));
                }
            }
        }
    }

    fn args_of(input: &str) -> Vec<String> {
        lex_with(input.as_bytes(), false)
            .unwrap()
            .into_iter()
            .filter_map(|t| if let Tok::Arg(a) = t { Some(String::from_utf8(a).unwrap()) } else { None })
            .collect()
    }

    fn lines_of(input: &str) -> Vec<String> {
        lex_with(input.as_bytes(), true)
            .unwrap()
            .into_iter()
            .filter_map(|t| if let Tok::Arg(a) = t { Some(String::from_utf8(a).unwrap()) } else { None })
            .collect()
    }

    #[test]
    fn blanks_and_newlines_separate() {
        assert_eq!(args_of("a b\tc\nd  e \n\n f"), ["a", "b", "c", "d", "e", "f"]);
        assert_eq!(args_of(""), Vec::<String>::new());
        assert_eq!(args_of("   \n \t\n"), Vec::<String>::new());
        /* other white space is not a separator */
        assert_eq!(args_of("a\rb\x0bc\x0cd"), ["a\rb\x0bc\x0cd"]);
    }

    #[test]
    fn quotes_hold_blanks_and_the_other_quote() {
        assert_eq!(args_of("\"a b\" 'c d' \"e'f\" 'g\"h'"), ["a b", "c d", "e'f", "g\"h"]);
        /* quoted text joins what is next to it */
        assert_eq!(args_of("a\"b c\"d'e f'g"), ["ab cde fg"]);
        /* an empty string is an argument */
        assert_eq!(args_of("\"\" '' x"), ["", "", "x"]);
        /* backslash is no escape inside quotes */
        assert_eq!(args_of("\"a\\b\" 'c\\d'"), ["a\\b", "c\\d"]);
    }

    #[test]
    fn unmatched_quotes_are_errors() {
        assert_eq!(lex_with(b"a \"b c", false).unwrap_err(), "unmatched double quote");
        assert_eq!(lex_with(b"a 'b c", false).unwrap_err(), "unmatched single quote");
        /* a newline cannot be quoted */
        assert_eq!(lex_with(b"\"a\nb\"", false).unwrap_err(), "unmatched double quote");
        assert_eq!(lex_with(b"'a\nb'", true).unwrap_err(), "unmatched single quote");
    }

    #[test]
    fn backslash_escapes_any_unquoted_character() {
        assert_eq!(args_of("a\\ b c\\\"d \\'e \\\\f \\x"), ["a b", "c\"d", "'e", "\\f", "x"]);
        assert_eq!(args_of("a\\\nb"), ["a\nb"]);
        assert_eq!(args_of("a\\"), ["a"]);
        assert_eq!(lines_of("a\\"), ["a"]);
        assert_eq!(args_of("\\ "), [" "]);
    }

    #[test]
    fn nul_in_the_input_is_an_error() {
        assert_eq!(lex_with(b"a\0b", false).unwrap_err(), "input contains a NUL character");
        assert_eq!(lex_with(b"a\0b", true).unwrap_err(), "input contains a NUL character");
    }

    #[test]
    fn insert_mode_lines_are_arguments() {
        assert_eq!(lines_of("a b c\n  d  e \n\n\t\nf"), ["a b c", "d  e ", "f"]);
        assert_eq!(lines_of("\"a b\"  'c' \\ d\n"), ["a b  c  d"]);
        /* a line of an empty string is an argument; a line of blanks is not */
        assert_eq!(lines_of("\"\"\n   \n"), [""]);
        /* leading blanks that are escaped or quoted stay */
        assert_eq!(lines_of("\\  x\n\" y\"\n"), ["  x", " y"]);
        assert_eq!(lines_of("a\\\nb\n"), ["a\nb"]);
    }

    /* the Eol tokens of -L: where a logical line ends */
    fn shape(input: &str) -> String {
        lex_with(input.as_bytes(), false)
            .unwrap()
            .into_iter()
            .map(|t| match t {
                Tok::Arg(a) => String::from_utf8(a).unwrap(),
                Tok::Eol => "|".to_string(),
                Tok::Eof => unreachable!(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_trailing_blank_continues_the_line_to_the_next_non_empty_one() {
        assert_eq!(shape("a b\nc\n"), "a b | c |");
        assert_eq!(shape("a b \nc\n"), "a b c |");
        assert_eq!(shape("a\t\nc\n"), "a c |");
        /* empty lines in between are passed over */
        assert_eq!(shape("a \n\n\nc\nd\n"), "a c | d |");
        /* an escaped blank is no continuation, a quoted one neither */
        assert_eq!(shape("a\\ \nc\n"), "a  | c |");
        assert_eq!(shape("\"a \"\nc\n"), "a  | c |");
        /* a line of blanks continues too, to the next one */
        assert_eq!(shape("a\n \nb\n"), "a | b |");
        assert_eq!(shape("a \n \nb\n"), "a b |");
        /* the end of the input ends everything */
        assert_eq!(shape("a b \n"), "a b");
    }

    #[test]
    fn replace_all_occurrences() {
        let r = |s: &[u8], w: &[u8], x: &[u8]| replace(s, w, x, 1 << 20).unwrap();
        assert_eq!(r(b"a{}b{}c", b"{}", b"X"), b"aXbXc");
        assert_eq!(r(b"{}{}", b"{}", b"-"), b"--");
        assert_eq!(r(b"abc", b"{}", b"X"), b"abc");
        assert_eq!(r(b"{", b"{}", b"X"), b"{");
        assert_eq!(r(b"{}", b"{}", b""), b"");
        assert_eq!(r(b"{}", b"{}", b"{}{}"), b"{}{}");
        /* the result is cut off as soon as it is too big */
        assert_eq!(replace(b"{}{}{}", b"{}", b"abcd", 8), None);
        assert_eq!(replace(b"{}{}", b"{}", b"abcd", 8).unwrap().len(), 8);
    }

    struct Run {
        batches: Vec<Vec<String>>,
        msgs: Vec<String>,
        status: i32,
    }

    /* Run the core over input; outcomes lists what each invocation does (default: succeeds). */
    fn go(input: &str, mode: Mode, eof: Option<&str>, fixed: &[&str], size: Option<usize>, x: bool, outcomes: &[Outcome]) -> Run {
        let insert = matches!(mode, Mode::I(_));
        let exit_if_too_big = insert || (x && !matches!(mode, Mode::Default));
        let line_max = (size.is_none() && matches!(mode, Mode::N(_))).then_some(2048);
        let mut batches = Vec::new();
        let mut msgs = Vec::new();
        let mut it = outcomes.iter().copied();
        let mut exec = |argv: &[Vec<u8>]| {
            batches.push(argv.iter().map(|a| String::from_utf8_lossy(a).into_owned()).collect::<Vec<_>>());
            it.next().unwrap_or(Outcome::Done)
        };
        let mut complain = |m: &str| msgs.push(m.to_string());
        let mut lx = Lexer::new(input.as_bytes(), insert, 1 << 20);
        let status = {
            let mut core = Core {
                mode,
                eof: eof.map(|e| e.as_bytes().to_vec()),
                fixed: fixed.iter().map(|s| s.as_bytes().to_vec()).collect(),
                lim: Limits { size, room: 1 << 20, line_max, default_size: DEFAULT_SIZE },
                exit_if_too_big,
                exec: &mut exec,
                complain: &mut complain,
                failed: false,
            };
            core.process(&mut lx)
        };
        Run { batches, msgs, status }
    }

    fn b(v: &[&[&str]]) -> Vec<Vec<String>> {
        v.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect()
    }

    #[test]
    fn default_runs_once_with_everything() {
        let r = go("a b\nc\n", Mode::Default, None, &["echo"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["echo", "a", "b", "c"]]));
        assert_eq!(r.status, 0);
    }

    #[test]
    fn empty_input_runs_the_utility_once_without_arguments() {
        let r = go("", Mode::Default, None, &["echo", "abc"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["echo", "abc"]]));
        let r = go("  \n\n", Mode::N(2), None, &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u"]]));
        let r = go("", Mode::L(1), None, &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u"]]));
        /* a logical end of file first is the same as no input */
        let r = go("_\na\n", Mode::Default, Some("_"), &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u"]]));
        /* -I has no line, so nothing to do */
        let r = go("", Mode::I(b"{}".to_vec()), None, &["u", "{}"], None, false, &[]);
        assert!(r.batches.is_empty());
        assert_eq!(r.status, 0);
    }

    #[test]
    fn n_limits_the_arguments_and_the_last_run_has_the_rest() {
        let r = go("a b c d e\n", Mode::N(2), None, &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a", "b"], &["u", "c", "d"], &["u", "e"]]));
        let r = go("a b c d\n", Mode::N(2), None, &["u"], None, false, &[]);
        assert_eq!(r.batches.len(), 2);
        let r = go("a b\n", Mode::N(1), None, &["u", "-x"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "-x", "a"], &["u", "-x", "b"]]));
    }

    #[test]
    fn l_counts_non_empty_logical_lines() {
        let r = go("a b\n\nc\nd e f\n", Mode::L(2), None, &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a", "b", "c"], &["u", "d", "e", "f"]]));
        let r = go("a\nb\nc\n", Mode::L(2), None, &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a", "b"], &["u", "c"]]));
        /* continued lines count as one */
        let r = go("a \nb\nc\nd\n", Mode::L(2), None, &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a", "b", "c"], &["u", "d"]]));
        /* a last line without a newline is a line */
        let r = go("a\nb", Mode::L(1), None, &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a"], &["u", "b"]]));
    }

    #[test]
    fn insert_runs_once_per_line_and_replaces_everywhere() {
        let r = go("one two\n\n three \n", Mode::I(b"{}".to_vec()), None, &["cp", "{}", "{}.bak", "x{}y"], None, false, &[]);
        assert_eq!(
            r.batches,
            b(&[&["cp", "one two", "one two.bak", "xone twoy"], &["cp", "three ", "three .bak", "xthree y"]])
        );
        /* no replacement string means nothing is added */
        let r = go("a\nb\n", Mode::I(b"%".to_vec()), None, &["echo", "hi"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["echo", "hi"], &["echo", "hi"]]));
        /* the utility name is not an argument: it stays */
        let r = go("ls\n", Mode::I(b"@".to_vec()), None, &["@", "-l", "@"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["@", "-l", "ls"]]));
    }

    #[test]
    fn logical_end_of_file_after_quote_processing() {
        let r = go("a b _ c\n", Mode::Default, Some("_"), &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a", "b"]]));
        /* quoted or escaped, it still is the string */
        let r = go("a \"_\" c\n", Mode::Default, Some("_"), &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a"]]));
        let r = go("a \\_ c\n", Mode::Default, Some("_"), &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a"]]));
        /* only a whole argument is it */
        let r = go("a _b b_ c\n", Mode::Default, Some("_"), &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a", "_b", "b_", "c"]]));
        /* with -n the arguments before it are still used */
        let r = go("a b c _ d\n", Mode::N(2), Some("_"), &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a", "b"], &["u", "c"]]));
        /* in -I mode a whole line is compared */
        let r = go("x\nEOF\ny\n", Mode::I(b"{}".to_vec()), Some("EOF"), &["u", "{}"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "x"]]));
        /* no string: underscore is an argument */
        let r = go("a _ c\n", Mode::Default, None, &["u"], None, false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a", "_", "c"]]));
    }

    #[test]
    fn n_without_s_is_cut_short_by_line_max() {
        /* 60 arguments of 49 bytes: 3 fit in 2048 with "u" */
        let input = format!("{}\n", vec!["a".repeat(48); 60].join(" "));
        let r = go(&input, Mode::N(100), None, &["u"], None, false, &[]);
        assert_eq!(r.batches.iter().map(|b| b.len() - 1).collect::<Vec<_>>(), vec![41, 19]);
        assert_eq!(r.status, 0);
        /* with -x that is an error */
        let r = go(&input, Mode::N(100), None, &["u"], None, true, &[]);
        assert!(r.batches.is_empty());
        assert_eq!(r.status, 1);
        /* -s lifts it, and the first argument is never refused for it */
        let r = go(&input, Mode::N(100), None, &["u"], Some(100_000), false, &[]);
        assert_eq!(r.batches.len(), 1);
        /* without -x an argument that is over {LINE_MAX} alone has nothing to be cut: it runs */
        let big = format!("{}\n", "b".repeat(3000));
        let r = go(&big, Mode::N(2), None, &["u"], None, false, &[]);
        assert_eq!(r.batches.len(), 1);
        /* with -x it does not fit the implied size: terminate, as for a line of several arguments */
        let r = go(&big, Mode::N(1), None, &["u"], None, true, &[]);
        assert!(r.batches.is_empty());
        assert_eq!((r.status, r.msgs), (1, vec!["argument too long".to_string()]));
        /* the utility and its own arguments over {LINE_MAX} under -x -n: nothing is run */
        let long = "c".repeat(3000);
        let r = go("a\n", Mode::N(1), None, &["u", &long], None, true, &[]);
        assert!(r.batches.is_empty());
        assert_eq!(r.status, 1);
        /* -x without -n has no implied size: lines are cut as usual */
        let r = go(&big, Mode::Default, None, &["u"], None, true, &[]);
        assert_eq!(r.batches.len(), 1);
    }

    #[test]
    fn trace_shows_arguments_so_that_a_shell_reads_them_back() {
        let q = |s: &str| String::from_utf8(trace_quote(s.as_bytes())).unwrap();
        /* nothing to quote */
        for s in ["a", "-n", "--", "{}", "a,b", "a:b", "a/b", "a.b", "a+b", "%", "@", "]", "_", "a#b", "a~b", "a{b", "a}b"] {
            assert_eq!(q(s), s);
        }
        /* quoted: the empty string, blanks, characters the shell reads */
        assert_eq!(q(""), "''");
        assert_eq!(q("a b"), "'a b'");
        assert_eq!(q(" "), "' '");
        for s in ["$HOME", "*", "?", "[x]", "a=b", "^", "!", "a\\b", "a\"b", "a`b", "a;b", "a&b", "a|b", "(a)", "<a>", "#x", "~", "{", "}"] {
            assert_eq!(q(s), format!("'{s}'"), "{s}");
        }
        /* a quote: double quotes if only quotes and blanks need it, else '\'' */
        assert_eq!(q("it's"), "\"it's\"");
        assert_eq!(q("it's x"), "\"it's x\"");
        assert_eq!(q("'"), "\"'\"");
        assert_eq!(q("it's *"), "'it'\\''s *'");
        assert_eq!(q("it's $x"), "'it'\\''s $x'");
        assert_eq!(q("a'b\"c"), "'a'\\''b\"c'");
        /* what cannot be seen */
        assert_eq!(q("a\nb"), "'a'$'\\n''b'");
        assert_eq!(q("a\tb"), "'a'$'\\t''b'");
        assert_eq!(q("x\x01y"), "'x'$'\\001''y'");
        assert_eq!(q("\x7f"), "''$'\\177'");
        assert_eq!(q("a\x1bb"), "'a'$'\\033''b'");
        assert_eq!(q("a\x07b\x08\x0c\x0b\r"), "'a'$'\\a''b'$'\\b\\f\\v\\r'");
        assert_eq!(q("\x01'"), "''$'\\001'\\'''");
        assert_eq!(q("it's\nx"), "'it'\\''s'$'\\n''x'");
        /* bytes that are no character: the C locale, as the tests run, has no printable byte above 0x7f */
        assert_eq!(trace_quote(b"\xff"), b"''$'\\377'");
        assert_eq!(trace_quote(b"a b\xff"), b"'a b'$'\\377'");
        assert_eq!(trace_quote(b"\xc3\xa9 x"), b"''$'\\303\\251'' x'");
    }

    #[test]
    fn size_splits_the_command_line_and_counts_the_null_bytes() {
        /* "u" is 2 bytes with its null; each 4 byte argument takes 5 */
        let r = go("aaaa bbbb cccc dddd\n", Mode::Default, None, &["u"], Some(13), false, &[]);
        assert_eq!(r.batches, b(&[&["u", "aaaa", "bbbb"], &["u", "cccc", "dddd"]]));
        /* "less than size": 12 is 2 + 5 + 5 */
        let r = go("aaaa bbbb cccc dddd\n", Mode::Default, None, &["u"], Some(12), false, &[]);
        assert_eq!(r.batches.len(), 4);
        let r = go("aaaa bbbb cccc dddd\n", Mode::Default, None, &["u"], Some(12), false, &[]);
        assert_eq!(r.batches[0], ["u", "aaaa"]);
        /* with -n as well: whichever comes first */
        let r = go("a b c d e f\n", Mode::N(3), None, &["u"], Some(8), false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a", "b"], &["u", "c", "d"], &["u", "e", "f"]]));
    }

    #[test]
    fn an_argument_that_cannot_fit_alone_is_an_error_before_anything_after_it_runs() {
        let r = go("a bbbbbbbbbbbbbbbbbbbbbbbbbbb c\n", Mode::Default, None, &["u"], Some(10), false, &[]);
        assert_eq!(r.batches, b(&[&["u", "a"]]));
        assert_eq!(r.status, 1);
        assert_eq!(r.msgs, ["argument too long"]);
        /* the utility and its arguments alone are too much */
        let r = go("a\n", Mode::Default, None, &["u", "long-argument"], Some(5), false, &[]);
        assert!(r.batches.is_empty());
        assert_eq!(r.status, 1);
    }

    #[test]
    fn x_ends_xargs_when_n_or_l_or_i_cannot_be_met() {
        /* -L: the lines of the group */
        let r = go("aaaa\nbbbb\ncccc\n", Mode::L(3), None, &["u"], Some(13), true, &[]);
        assert!(r.batches.is_empty());
        assert_eq!(r.status, 1);
        /* -n 2 wants two arguments of 5 bytes and 2 for the utility: 12 does not fit in 11 */
        let r = go("aaaa bbbb cccc\n", Mode::N(2), None, &["u"], Some(11), true, &[]);
        assert!(r.batches.is_empty());
        assert_eq!(r.status, 1);
        assert_eq!(r.msgs, ["argument line too long"]);
        /* it fits */
        let r = go("aaaa bbbb\n", Mode::N(2), None, &["u"], Some(13), true, &[]);
        assert_eq!(r.batches, b(&[&["u", "aaaa", "bbbb"]]));
        assert_eq!(r.status, 0);
        /* earlier command lines were run */
        let r = go("a b ccccccccc\n", Mode::N(2), None, &["u"], Some(9), true, &[]);
        assert_eq!(r.batches, b(&[&["u", "a", "b"]]));
        assert_eq!(r.status, 1);
        /* without -x the line is cut short, as -s says; -x alone (no -n, -L) cuts lines as usual */
        let r = go("aaaa bbbb cccc\n", Mode::N(3), None, &["u"], Some(13), false, &[]);
        assert_eq!(r.batches.len(), 2);
        assert_eq!(r.status, 0);
        let r = go("aaaa bbbb cccc\n", Mode::Default, None, &["u"], Some(13), true, &[]);
        assert_eq!(r.batches.len(), 2);
        assert_eq!(r.status, 0);
        /* -I: the line made does not fit */
        let r = go("0123456789\n", Mode::I(b"{}".to_vec()), None, &["u", "{}"], Some(10), false, &[]);
        assert!(r.batches.is_empty());
        assert_eq!(r.status, 1);
    }

    #[test]
    fn exit_statuses() {
        let r = go("a b c\n", Mode::N(1), None, &["u"], None, false, &[Outcome::Failed, Outcome::Done, Outcome::Failed]);
        assert_eq!(r.batches.len(), 3);
        assert_eq!(r.status, 123);
        let r = go("a b c\n", Mode::N(1), None, &["u"], None, false, &[Outcome::Done, Outcome::Stop(124)]);
        assert_eq!(r.batches.len(), 2);
        assert_eq!(r.status, 124);
        let r = go("a b c\n", Mode::N(1), None, &["u"], None, false, &[Outcome::Stop(127)]);
        assert_eq!(r.batches.len(), 1);
        assert_eq!(r.status, 127);
    }

    #[test]
    fn limits() {
        let l = Limits { size: Some(10), room: 1000, line_max: None, default_size: DEFAULT_SIZE };
        assert!(l.fits(9, 1));
        assert!(!l.fits(10, 1));
        let l = Limits { size: None, room: 1000, line_max: None, default_size: DEFAULT_SIZE };
        assert!(l.fits(900, 1));
        assert!(!l.fits(985, 2), "pointers count against the room");
        let l = Limits { size: None, room: usize::MAX, line_max: None, default_size: DEFAULT_SIZE };
        assert!(l.fits(DEFAULT_SIZE, 1));
        assert!(!l.fits(DEFAULT_SIZE + 1, 1));
        /* "at least {LINE_MAX}": a larger {LINE_MAX} raises the default */
        let l = Limits { size: None, room: usize::MAX, line_max: None, default_size: 1 << 20 };
        assert!(l.fits(1 << 20, 1));
    }

    #[test]
    fn spec_example_2_and_5_input_forms() {
        /* example 2: every non-alphanumeric character escaped, then pairs */
        assert_eq!(args_of("a\\.b c\\-d\ne\\ f g\n"), ["a.b", "c-d", "e f", "g"]);
        /* example 5: quoted file names with a double quote inside, one per line */
        assert_eq!(lines_of("\"a b\"\n\"c\"\\\"\"d\"\n"), ["a b", "c\"d"]);
        /* example 1: apostrophes around a word are removed */
        assert_eq!(args_of("'logname' 'date' 'x y'\n"), ["logname", "date", "x y"]);
    }
}
