/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * test, [ - evaluate expression.
 *
 * One program for both names: when it is run as "[" the last argument must be
 * "]". The result is the exit status: 0 true, 1 false, 2 error.
 */

#![forbid(unsafe_code)]

use posix::num::{compare_ints, int};
use std::cmp::Ordering;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};

/* Parentheses may nest at least 32 deep; a deeper nesting costs stack. */
const NEST_MAX: usize = 200;

/* Why an expression could not be evaluated. */
#[derive(Debug, PartialEq, Eq)]
struct Error(String);

fn err(msg: &str) -> Error {
    Error(msg.to_string())
}

type R = Result<bool, Error>;

const UNARY: &[&[u8]] = &[
    b"-b", b"-c", b"-d", b"-e", b"-f", b"-g", b"-h", b"-L", b"-n", b"-p", b"-r", b"-S", b"-s", b"-t", b"-u", b"-w", b"-x", b"-z",
];

/* = != and the comparisons of integers; -a and -o too, which join expressions */
const BINARY: &[&[u8]] = &[b"=", b"!=", b"-eq", b"-ne", b"-gt", b"-ge", b"-lt", b"-le", b"-a", b"-o"];

fn is_unary(a: &[u8]) -> bool {
    UNARY.contains(&a)
}

fn is_binary(a: &[u8]) -> bool {
    BINARY.contains(&a)
}

/* The metadata of a pathname, following symbolic links or not. */
fn stat(path: &[u8], follow: bool) -> Option<std::fs::Metadata> {
    let p = std::ffi::OsStr::from_bytes(path);
    if follow { std::fs::metadata(p) } else { std::fs::symlink_metadata(p) }.ok()
}

/* Will the process get this access (the effective user and groups count)? */
fn access(path: &[u8], mode: libc::c_int) -> bool {
    if path.contains(&0) {
        return false;
    }
    let c = posix::cstr(path);
    posix::sys::faccessat(libc::AT_FDCWD, &c, mode, libc::AT_EACCESS) == 0
}

/* A unary primary applied to its operand. */
fn unary(op: &[u8], arg: &[u8]) -> R {
    let ft = |f: fn(&std::fs::FileType) -> bool| stat(arg, true).is_some_and(|m| f(&m.file_type()));
    Ok(match op {
        b"-n" => !arg.is_empty(),
        b"-z" => arg.is_empty(),
        b"-b" => ft(|t| t.is_block_device()),
        b"-c" => ft(|t| t.is_char_device()),
        b"-d" => ft(|t| t.is_dir()),
        b"-f" => ft(|t| t.is_file()),
        b"-p" => ft(|t| t.is_fifo()),
        b"-S" => ft(|t| t.is_socket()),
        b"-e" => stat(arg, true).is_some(),
        b"-h" | b"-L" => stat(arg, false).is_some_and(|m| m.file_type().is_symlink()),
        b"-g" => stat(arg, true).is_some_and(|m| m.mode() & libc::S_ISGID as u32 != 0),
        b"-u" => stat(arg, true).is_some_and(|m| m.mode() & libc::S_ISUID as u32 != 0),
        b"-s" => stat(arg, true).is_some_and(|m| m.len() > 0),
        b"-r" => access(arg, libc::R_OK),
        b"-w" => access(arg, libc::W_OK),
        b"-x" => access(arg, libc::X_OK),
        /* "False if file_descriptor is not a valid file descriptor number": no error, no message */
        b"-t" => match posix::parse_decimal(arg) {
            Some(fd) => fd <= libc::c_int::MAX as u64 && posix::sys::isatty(fd as libc::c_int),
            None => false,
        },
        _ => return Err(err("unknown operator")),
    })
}

/* An operand of -eq and friends: an integer, with or without a sign. */
fn integer(a: &[u8]) -> Result<posix::num::Int<'_>, Error> {
    let digits = a.strip_prefix(b"+").unwrap_or(a);
    int(digits).ok_or_else(|| err("integer expression expected"))
}

/* A binary primary applied to its operands. */
fn binary(a: &[u8], op: &[u8], b: &[u8]) -> R {
    Ok(match op {
        b"=" => a == b,
        b"!=" => a != b,
        b"-a" => !a.is_empty() && !b.is_empty(),
        b"-o" => !a.is_empty() || !b.is_empty(),
        _ => {
            let o = compare_ints(&integer(a)?, &integer(b)?);
            match op {
                b"-eq" => o == Ordering::Equal,
                b"-ne" => o != Ordering::Equal,
                b"-gt" => o == Ordering::Greater,
                b"-ge" => o != Ordering::Less,
                b"-lt" => o == Ordering::Less,
                b"-le" => o != Ordering::Greater,
                _ => return Err(err("unknown operator")),
            }
        }
    })
}

/*
 * The result for up to four arguments, by the rules that count them. None
 * if no rule applies; the grammar below then has a go.
 */
fn by_count(a: &[&[u8]]) -> Option<R> {
    match a.len() {
        0 => Some(Ok(false)),
        1 => Some(Ok(!a[0].is_empty())),
        2 => {
            if a[0] == b"!" {
                Some(Ok(a[1].is_empty()))
            } else if is_unary(a[0]) {
                Some(unary(a[0], a[1]))
            } else {
                None
            }
        }
        3 => {
            if is_binary(a[1]) {
                Some(binary(a[0], a[1], a[2]))
            } else if a[0] == b"!" {
                by_count(&a[1..]).map(|r| r.map(|v| !v))
            } else if a[0] == b"(" && a[2] == b")" {
                Some(Ok(!a[1].is_empty()))
            } else {
                None
            }
        }
        4 => {
            if a[0] == b"!" {
                by_count(&a[1..]).map(|r| r.map(|v| !v))
            } else if a[0] == b"(" && a[3] == b")" {
                by_count(&a[1..3])
            } else {
                None
            }
        }
        _ => None,
    }
}

/*
 * The general case, with the XSI operators:
 *
 *   or      := and { -o and }
 *   and     := not { -a not }
 *   not     := ! not | primary
 *   primary := ( or ) | unary-op word | word binary-op word | word
 *
 * A word that is followed by = or != and one more word is compared, whatever
 * it looks like: the string comparisons bind tighter than the unary primaries.
 */
struct Parser<'a> {
    toks: &'a [&'a [u8]],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a [&'a [u8]] {
        &self.toks[self.pos..]
    }

    fn eat(&mut self, w: &[u8]) -> bool {
        if self.rest().first() == Some(&w) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> R {
        let mut v = self.and()?;
        while self.eat(b"-o") {
            let r = self.and()?;
            v = v || r;
        }
        Ok(v)
    }

    fn and(&mut self) -> R {
        let mut v = self.not()?;
        while self.eat(b"-a") {
            let r = self.not()?;
            v = v && r;
        }
        Ok(v)
    }

    fn not(&mut self) -> R {
        let mut neg = false;
        loop {
            let r = self.rest();
            let compare = r.len() >= 3 && (r[1] == b"=" || r[1] == b"!=");
            if r.len() >= 2 && r[0] == b"!" && !compare {
                neg = !neg;
                self.pos += 1;
            } else {
                break;
            }
        }
        Ok(self.primary()? != neg)
    }

    fn primary(&mut self) -> R {
        let r = self.rest();
        let Some(&t) = r.first() else {
            return Err(err("argument expected"));
        };
        if r.len() >= 3 && (r[1] == b"=" || r[1] == b"!=") {
            self.pos += 3;
            return binary(t, r[1], r[2]);
        }
        if t == b"(" {
            if self.depth >= NEST_MAX {
                return Err(err("expression nested too deeply"));
            }
            self.pos += 1;
            self.depth += 1;
            let v = self.or()?;
            self.depth -= 1;
            if !self.eat(b")") {
                return Err(err("closing paren expected"));
            }
            return Ok(v);
        }
        if is_unary(t) && r.len() >= 2 {
            self.pos += 2;
            return unary(t, r[1]);
        }
        if r.len() >= 3 && is_binary(r[1]) && r[1] != b"-a" && r[1] != b"-o" {
            self.pos += 3;
            return binary(t, r[1], r[2]);
        }
        self.pos += 1;
        Ok(!t.is_empty())
    }
}

fn evaluate(args: &[&[u8]]) -> R {
    if args.len() <= 4 {
        if let Some(r) = by_count(args) {
            return r;
        }
    }
    let mut p = Parser { toks: args, pos: 0, depth: 0 };
    let v = p.or()?;
    if p.pos != args.len() {
        return Err(err("too many arguments"));
    }
    Ok(v)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut ops: Vec<&[u8]> = args.iter().skip(1).map(|a| a.as_slice()).collect();
    if prog == b"[" {
        if ops.last() != Some(&b"]".as_slice()) {
            posix::warn(&prog, "missing ]");
            std::process::exit(2);
        }
        ops.pop();
    }
    match evaluate(&ops) {
        Ok(true) => std::process::exit(0),
        Ok(false) => std::process::exit(1),
        Err(e) => {
            posix::warn(&prog, &e.0);
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /* None for an error */
    fn t(a: &[&str]) -> Option<bool> {
        let v: Vec<&[u8]> = a.iter().map(|s| s.as_bytes()).collect();
        evaluate(&v).ok()
    }

    #[test]
    fn no_arguments_is_false_one_is_a_string() {
        assert_eq!(t(&[]), Some(false));
        assert_eq!(t(&["a"]), Some(true));
        assert_eq!(t(&[""]), Some(false));
        /* even words that are operators, as a single argument */
        for w in ["!", "(", ")", "-n", "-z", "-d", "=", "-a", "-o", "-t"] {
            assert_eq!(t(&[w]), Some(true), "{w}");
        }
    }

    #[test]
    fn two_arguments() {
        assert_eq!(t(&["!", ""]), Some(true));
        assert_eq!(t(&["!", "a"]), Some(false));
        assert_eq!(t(&["!", "!"]), Some(false));
        assert_eq!(t(&["-n", "a"]), Some(true));
        assert_eq!(t(&["-n", ""]), Some(false));
        assert_eq!(t(&["-z", ""]), Some(true));
        assert_eq!(t(&["-z", "a"]), Some(false));
        assert_eq!(t(&["-n", "-n"]), Some(true));
        assert_eq!(t(&["-z", "-z"]), Some(false));
        assert_eq!(t(&["-d", "/"]), Some(true));
        assert_eq!(t(&["-d", "/nonexistent-poutil"]), Some(false));
        assert_eq!(t(&["-q", "x"]), None);
        assert_eq!(t(&["a", "b"]), None);
    }

    #[test]
    fn three_arguments() {
        assert_eq!(t(&["a", "=", "a"]), Some(true));
        assert_eq!(t(&["a", "=", "b"]), Some(false));
        assert_eq!(t(&["a", "!=", "b"]), Some(true));
        assert_eq!(t(&["", "=", ""]), Some(true));
        /* the second argument decides: these are comparisons */
        assert_eq!(t(&["!", "=", "!"]), Some(true));
        assert_eq!(t(&["-n", "=", "-n"]), Some(true));
        assert_eq!(t(&["(", "=", ")"]), Some(false));
        assert_eq!(t(&["=", "=", "="]), Some(true));
        assert_eq!(t(&["-a", "=", "-a"]), Some(true));
        /* ! negates the two-argument test */
        assert_eq!(t(&["!", "-n", ""]), Some(true));
        assert_eq!(t(&["!", "-z", ""]), Some(false));
        assert_eq!(t(&["!", "-d", "/"]), Some(false));
        /* parentheses around one argument */
        assert_eq!(t(&["(", "a", ")"]), Some(true));
        assert_eq!(t(&["(", "", ")"]), Some(false));
        assert_eq!(t(&["(", "-n", ")"]), Some(true));
        /* -a and -o join two strings */
        assert_eq!(t(&["a", "-a", "b"]), Some(true));
        assert_eq!(t(&["a", "-a", ""]), Some(false));
        assert_eq!(t(&["", "-o", "b"]), Some(true));
        assert_eq!(t(&["", "-o", ""]), Some(false));
    }

    #[test]
    fn four_arguments() {
        assert_eq!(t(&["!", "a", "=", "a"]), Some(false));
        assert_eq!(t(&["!", "a", "=", "b"]), Some(true));
        assert_eq!(t(&["!", "!", "=", "!"]), Some(false));
        assert_eq!(t(&["(", "-n", "a", ")"]), Some(true));
        assert_eq!(t(&["(", "a", "=", ")"]), None);
        assert_eq!(t(&["(", "!", "a", ")"]), Some(false));
        assert_eq!(t(&["!", "(", "a", ")"]), Some(false));
        /* no rule: the grammar */
        assert_eq!(t(&["-n", "a", "-a", "b"]), Some(true));
        assert_eq!(t(&["a", "=", "a", "-a"]), None);
        assert_eq!(t(&["!", "!", "!", "a"]), Some(false));
    }

    #[test]
    fn integers() {
        assert_eq!(t(&["1", "-eq", "1"]), Some(true));
        assert_eq!(t(&["1", "-eq", "01"]), Some(true));
        assert_eq!(t(&["-1", "-lt", "0"]), Some(true));
        assert_eq!(t(&["+5", "-gt", "4"]), Some(true));
        assert_eq!(t(&["10", "-gt", "9"]), Some(true));
        assert_eq!(t(&["10", "-ge", "10"]), Some(true));
        assert_eq!(t(&["10", "-le", "9"]), Some(false));
        assert_eq!(t(&["10", "-ne", "9"]), Some(true));
        assert_eq!(t(&["99999999999999999999999", "-gt", "99999999999999999999998"]), Some(true));
        assert_eq!(t(&["-0", "-eq", "0"]), Some(true));
        assert_eq!(t(&["a", "-eq", "1"]), None);
        assert_eq!(t(&["1", "-eq", ""]), None);
        assert_eq!(t(&["1.5", "-eq", "1"]), None);
        assert_eq!(t(&[" 1", "-eq", "1"]), None);
        assert_eq!(t(&["--1", "-eq", "1"]), None);
    }

    #[test]
    fn and_binds_tighter_than_or() {
        assert_eq!(t(&["a", "-o", "b", "-a", ""]), Some(true));
        assert_eq!(t(&["", "-a", "b", "-o", "c"]), Some(true));
        assert_eq!(t(&["", "-a", "b", "-o", ""]), Some(false));
        assert_eq!(t(&["a", "=", "a", "-a", "b", "=", "b"]), Some(true));
        assert_eq!(t(&["a", "=", "b", "-o", "b", "=", "b"]), Some(true));
        assert_eq!(t(&["a", "=", "b", "-a", "b", "=", "b"]), Some(false));
        assert_eq!(t(&["1", "-lt", "2", "-a", "3", "-gt", "2"]), Some(true));
    }

    #[test]
    fn parentheses_and_negation_in_longer_expressions() {
        assert_eq!(t(&["(", "a", "=", "b", "-o", "c", "=", "c", ")", "-a", "d"]), Some(true));
        assert_eq!(t(&["!", "(", "a", "=", "b", ")", "-a", "x"]), Some(true));
        assert_eq!(t(&["!", "a", "=", "b", "-a", "x"]), Some(true));
        assert_eq!(t(&["(", "(", "(", "a", "=", "a", ")", ")", ")", "-a", "(", "b", ")"]), Some(true));
        assert_eq!(t(&["!", "!", "a", "=", "a", "-o", "z"]), Some(true));
        assert_eq!(t(&["a", "-o", "(", "b", "=", "c", ")", "-o", "d"]), Some(true));
    }

    #[test]
    fn a_comparison_binds_tighter_than_a_unary_primary() {
        /* "-n = x" is a comparison, so no unary -n with the operand "=" */
        assert_eq!(t(&["-n", "=", "x", "-o", "y"]), Some(true));
        assert_eq!(t(&["-d", "=", "-d", "-a", "x"]), Some(true));
        assert_eq!(t(&["-z", "!=", "-z", "-o", ""]), Some(false));
        assert_eq!(t(&["!", "=", "!", "-a", "x"]), Some(true));
    }

    #[test]
    fn syntax_errors() {
        assert_eq!(t(&["a", "=", "b", "c"]), None);
        assert_eq!(t(&["(", "a", "=", "b"]), None);
        assert_eq!(t(&["a", "=", "b", ")"]), None);
        assert_eq!(t(&["a", "-a"]), None);
        assert_eq!(t(&["-o", "a", "b", "c", "d"]), None);
        assert_eq!(t(&["a", "=", "b", "-a"]), None);
        assert_eq!(t(&["(", ")", "-a", "x", "y"]), None);
        assert_eq!(t(&["a", "b", "c", "d", "e"]), None);
        assert_eq!(t(&["-q", "a", "-a", "b"]), None);
    }

    #[test]
    fn extensions_are_refused() {
        assert_eq!(t(&["a", "==", "a"]), None);
        assert_eq!(t(&["a", "<", "b"]), None);
        assert_eq!(t(&["a", "-nt", "b"]), None);
        assert_eq!(t(&["-k", "/"]), None);
        assert_eq!(t(&["-O", "/"]), None);
        assert_eq!(t(&["-v", "x"]), None);
    }

    #[test]
    fn the_file_types() {
        assert_eq!(t(&["-e", "/"]), Some(true));
        assert_eq!(t(&["-e", ""]), Some(false));
        assert_eq!(t(&["-e", "/nonexistent-poutil"]), Some(false));
        assert_eq!(t(&["-f", "/"]), Some(false));
        assert_eq!(t(&["-f", "/etc/hosts"]), Some(true));
        assert_eq!(t(&["-d", "/"]), Some(true));
        assert_eq!(t(&["-c", "/dev/null"]), Some(true));
        assert_eq!(t(&["-c", "/"]), Some(false));
        assert_eq!(t(&["-b", "/dev/null"]), Some(false));
        assert_eq!(t(&["-p", "/dev/null"]), Some(false));
        assert_eq!(t(&["-S", "/dev/null"]), Some(false));
        assert_eq!(t(&["-s", "/etc/hosts"]), Some(true));
        assert_eq!(t(&["-s", "/dev/null"]), Some(false));
        assert_eq!(t(&["-s", "/nonexistent-poutil"]), Some(false));
        assert_eq!(t(&["-r", "/"]), Some(true));
        assert_eq!(t(&["-x", "/"]), Some(true));
        assert_eq!(t(&["-w", "/nonexistent-poutil"]), Some(false));
        assert_eq!(t(&["-u", "/"]), Some(false));
        assert_eq!(t(&["-g", "/"]), Some(false));
        assert_eq!(t(&["-h", "/"]), Some(false));
        assert_eq!(t(&["-L", "/nonexistent-poutil"]), Some(false));
    }

    #[test]
    fn file_descriptors() {
        assert_eq!(t(&["-t", "999"]), Some(false));
        assert_eq!(t(&["-t", "99999999999999999999"]), Some(false));
        /* not a valid file descriptor number: false, not an error */
        for bad in ["x", "-1", "+1", "-0", "", " 1", "1.5", "0x1", "1 ", "٣"] {
            assert_eq!(t(&["-t", bad]), Some(false), "{bad:?}");
        }
    }

    #[test]
    fn symbolic_links() {
        let dir = std::env::temp_dir().join(format!("poutil-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let (file, link, dangling) = (dir.join("f"), dir.join("l"), dir.join("d"));
        std::fs::write(&file, b"x").unwrap();
        std::os::unix::fs::symlink(&file, &link).unwrap();
        std::os::unix::fs::symlink(dir.join("nothing"), &dangling).unwrap();
        let (f, l, d) = (file.to_str().unwrap(), link.to_str().unwrap(), dangling.to_str().unwrap());
        /* everything but -h and -L follows the link */
        assert_eq!(t(&["-f", l]), Some(true));
        assert_eq!(t(&["-s", l]), Some(true));
        assert_eq!(t(&["-h", l]), Some(true));
        assert_eq!(t(&["-L", l]), Some(true));
        assert_eq!(t(&["-h", f]), Some(false));
        assert_eq!(t(&["-e", d]), Some(false));
        assert_eq!(t(&["-h", d]), Some(true));
        assert_eq!(t(&["-f", d]), Some(false));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_deep_nesting_is_refused_not_a_crash() {
        let mut a: Vec<&str> = vec!["("; 100];
        a.push("x");
        a.extend(std::iter::repeat_n(")", 100));
        assert_eq!(t(&a), Some(true));
        let mut a: Vec<&str> = vec!["("; 5000];
        a.push("x");
        a.extend(std::iter::repeat_n(")", 5000));
        assert_eq!(t(&a), None);
        let mut a: Vec<&str> = vec!["!"; 100_000];
        a.push("x");
        assert_eq!(t(&a), Some(true));
        let mut a: Vec<&str> = vec!["x"];
        for _ in 0..50_000 {
            a.push("-a");
            a.push("y");
        }
        assert_eq!(t(&a), Some(true));
    }

    #[test]
    fn operands_are_bytes() {
        let a: [&[u8]; 3] = [b"\xff", b"=", b"\xff"];
        assert_eq!(evaluate(&a), Ok(true));
        let a: [&[u8]; 3] = [b"\xff", b"=", b"\xfe"];
        assert_eq!(evaluate(&a), Ok(false));
    }
}
