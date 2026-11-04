/* SPDX-License-Identifier: GPL-2.0-only */
/* expr - evaluate arguments as an expression. */

#![forbid(unsafe_code)]

use posix::num::{compare_ints, int, value};
use std::cmp::Ordering;

const SYNOPSIS: &str = "expr operand...";

/*
 * Why an expression failed. The status is 2 for an invalid expression
 * (syntax, a non-integer operand, division by zero, a pattern the system
 * rejects) and 3 for an error while computing (an integer that does not fit).
 */
#[derive(Debug, PartialEq, Eq)]
struct Error {
    msg: &'static str,
    status: i32,
}

fn invalid(msg: &'static str) -> Error {
    Error { msg, status: 2 }
}

fn failed(msg: &'static str) -> Error {
    Error { msg, status: 3 }
}

/* Parentheses may nest at least {EXPR_NEST_MAX} (32) deep; more costs stack. */
const NEST_MAX: usize = 200;

/*
 * The words of an expression are the arguments, told apart by value only.
 * Every value is a string of bytes; whether it counts as an integer is
 * decided where it is used, as the spec says.
 */
type Val = Vec<u8>;

/* Null or zero: the values that count as false. */
fn is_false(v: &[u8]) -> bool {
    v.is_empty() || int(v).is_some_and(|i| i.mag.is_empty())
}

/* Order two strings by the collation sequence of the locale. */
fn collate(a: &[u8], b: &[u8]) -> Ordering {
    /* arguments never contain a NUL, and so nothing made from them does */
    posix::mb::strcoll(a, b)
}

/*
 * Does the basic regular expression have a \( \) subexpression? (regex_t
 * does not show its re_nsub through the libc crate.) The pattern has been
 * accepted by regcomp already, so only its shape matters: skip the second
 * byte of every backslash pair and whole bracket expressions, where \( is
 * an ordinary pair of characters.
 */
fn has_group(pat: &[u8]) -> bool {
    let mut i = 0;
    while i < pat.len() {
        match pat[i] {
            b'\\' => {
                if pat.get(i + 1) == Some(&b'(') {
                    return true;
                }
                i += 2;
            }
            b'[' => {
                i += 1;
                if pat.get(i) == Some(&b'^') {
                    i += 1;
                }
                /* a ] first in the list is a member, not the end */
                if pat.get(i) == Some(&b']') {
                    i += 1;
                }
                while i < pat.len() && pat[i] != b']' {
                    if pat[i] == b'[' && matches!(pat.get(i + 1), Some(b':' | b'.' | b'=')) {
                        let kind = pat[i + 1];
                        i += 2;
                        while i + 1 < pat.len() && !(pat[i] == kind && pat[i + 1] == b']') {
                            i += 1;
                        }
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/*
 * The matching operator: the basic regular expression `pat`, anchored at the
 * start of `s`. The result is the number of characters matched, or, when the
 * pattern has a subexpression, what the first one matched (null if it did
 * not). No match is 0, or null with a subexpression.
 */
fn matches(s: &[u8], pat: &[u8]) -> Result<Val, Error> {
    /* regcomp leaves the empty expression undefined; it matches nothing, at length 0 */
    if pat.is_empty() {
        return Ok(b"0".to_vec());
    }
    let cs = posix::cstr(s);
    let cp = posix::cstr(pat);
    let Some(re) = posix::regex::Regex::new(&cp, 0) else {
        return Err(invalid("invalid regular expression"));
    };
    let groups = has_group(pat);
    let m = re.exec(&cs, 2);
    /* the leftmost match starts at 0 whenever any match does */
    let Some(whole) = m.as_ref().and_then(|m| m[0]).filter(|r| r.0 == 0) else {
        return Ok(if groups { Vec::new() } else { b"0".to_vec() });
    };
    if groups {
        return Ok(match m.as_ref().and_then(|m| m[1]) {
            Some((so, eo)) => s[so..eo].to_vec(),
            None => Vec::new(),
        });
    }
    Ok(posix::mb::count_chars(&s[..whole.1]).to_string().into_bytes())
}

fn arith(op: u8, a: &[u8], b: &[u8]) -> Result<Val, Error> {
    let (Some(x), Some(y)) = (int(a), int(b)) else {
        return Err(invalid("non-integer argument"));
    };
    let (Some(x), Some(y)) = (value(&x), value(&y)) else {
        return Err(failed("integer overflow"));
    };
    let r = match op {
        b'+' => x.checked_add(y),
        b'-' => x.checked_sub(y),
        b'*' => x.checked_mul(y),
        _ => {
            if y == 0 {
                return Err(invalid("division by zero"));
            }
            if op == b'/' {
                x.checked_div(y)
            } else {
                x.checked_rem(y)
            }
        }
    };
    match r {
        Some(n) => Ok(n.to_string().into_bytes()),
        None => Err(failed("integer overflow")),
    }
}

/* Integers compare as numbers when both operands are, anything else as strings. */
fn relation(op: &[u8], a: &[u8], b: &[u8]) -> Val {
    let o = match (int(a), int(b)) {
        (Some(x), Some(y)) => compare_ints(&x, &y),
        _ => collate(a, b),
    };
    let t = match op {
        b"=" => o == Ordering::Equal,
        b">" => o == Ordering::Greater,
        b">=" => o != Ordering::Less,
        b"<" => o == Ordering::Less,
        b"<=" => o != Ordering::Greater,
        _ => o != Ordering::Equal,
    };
    if t { b"1".to_vec() } else { b"0".to_vec() }
}

/*
 * Parser and evaluator in one: each level takes the arguments it needs and
 * returns the value of what it read. `live` is false for an operand of | or &
 * that the left side makes unnecessary: it is read for its syntax only, so
 * 1 | 1 / 0 is 1, and a long chain never needs more stack than its nesting.
 * Precedence, lowest first: |  &  comparisons  + -  * / %  :  and then the
 * operands and parentheses. All are left-associative.
 */
struct Parser<'a> {
    toks: &'a [Vec<u8>],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a [u8]> {
        self.toks.get(self.pos).map(|t| t.as_slice())
    }

    /* Take the next word if it is one of ops. */
    fn take(&mut self, ops: &[&[u8]]) -> Option<&'a [u8]> {
        let t = self.peek()?;
        if ops.contains(&t) {
            self.pos += 1;
            Some(t)
        } else {
            None
        }
    }

    fn or(&mut self, live: bool) -> Result<Val, Error> {
        let mut left = self.and(live)?;
        while self.take(&[b"|"]).is_some() {
            let need = live && is_false(&left);
            let right = self.and(need)?;
            if need {
                left = if right.is_empty() { b"0".to_vec() } else { right };
            }
        }
        Ok(left)
    }

    fn and(&mut self, live: bool) -> Result<Val, Error> {
        let mut left = self.relation(live)?;
        while self.take(&[b"&"]).is_some() {
            let need = live && !is_false(&left);
            let right = self.relation(need)?;
            if live && (!need || is_false(&right)) {
                left = b"0".to_vec();
            }
        }
        Ok(left)
    }

    fn relation(&mut self, live: bool) -> Result<Val, Error> {
        let mut left = self.sum(live)?;
        while let Some(op) = self.take(&[b"=", b">", b">=", b"<", b"<=", b"!="]) {
            let right = self.sum(live)?;
            if live {
                left = relation(op, &left, &right);
            }
        }
        Ok(left)
    }

    fn sum(&mut self, live: bool) -> Result<Val, Error> {
        let mut left = self.product(live)?;
        while let Some(op) = self.take(&[b"+", b"-"]) {
            let right = self.product(live)?;
            if live {
                left = arith(op[0], &left, &right)?;
            }
        }
        Ok(left)
    }

    fn product(&mut self, live: bool) -> Result<Val, Error> {
        let mut left = self.matching(live)?;
        while let Some(op) = self.take(&[b"*", b"/", b"%"]) {
            let right = self.matching(live)?;
            if live {
                left = arith(op[0], &left, &right)?;
            }
        }
        Ok(left)
    }

    fn matching(&mut self, live: bool) -> Result<Val, Error> {
        let mut left = self.primary(live)?;
        while self.take(&[b":"]).is_some() {
            let right = self.primary(live)?;
            if live {
                left = matches(&left, &right)?;
            }
        }
        Ok(left)
    }

    /* An operand, or an expression in parentheses. */
    fn primary(&mut self, live: bool) -> Result<Val, Error> {
        let Some(t) = self.peek() else {
            return Err(invalid("syntax error"));
        };
        self.pos += 1;
        if t != b"(" {
            /* any other word is an operand, whatever it looks like */
            return Ok(t.to_vec());
        }
        if self.depth >= NEST_MAX {
            return Err(invalid("expression nested too deeply"));
        }
        self.depth += 1;
        let v = self.or(live)?;
        self.depth -= 1;
        if self.take(&[b")"]).is_none() {
            return Err(invalid("syntax error: missing )"));
        }
        Ok(v)
    }
}

fn eval(toks: &[Vec<u8>]) -> Result<Val, Error> {
    let mut p = Parser { toks, pos: 0, depth: 0 };
    let v = p.or(true)?;
    if p.pos != toks.len() {
        return Err(invalid("syntax error"));
    }
    Ok(v)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    /*
     * Only a leading "--" is taken as the end of the options; anything else
     * (also "-5" or "-x") is an operand, as every implementation does.
     */
    let mut toks = &args[1.min(args.len())..];
    if toks.first().map(|t| t.as_slice()) == Some(b"--") {
        toks = &toks[1..];
    }
    if toks.is_empty() {
        posix::usage(SYNOPSIS);
    }
    match eval(toks) {
        Ok(mut v) => {
            let false_ = is_false(&v);
            v.push(b'\n');
            /* 1 and 2 are a null result and an invalid expression: a write error is 3 */
            posix::write_stdout_or(&prog, &v, 3);
            std::process::exit(if false_ { 1 } else { 0 });
        }
        Err(e) => {
            posix::warn(&prog, e.msg);
            std::process::exit(e.status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /* the locale is process-wide: tests that change it hold this */
    static LOCALE: Mutex<()> = Mutex::new(());

    fn run(a: &[&str]) -> Result<String, Error> {
        let toks: Vec<Vec<u8>> = a.iter().map(|s| s.as_bytes().to_vec()).collect();
        eval(&toks).map(|v| String::from_utf8_lossy(&v).into_owned())
    }

    fn ok(a: &[&str]) -> String {
        run(a).unwrap_or_else(|e| format!("error {}", e.msg))
    }

    fn status(a: &[&str]) -> i32 {
        run(a).err().map(|e| e.status).unwrap_or(0)
    }

    #[test]
    fn a_single_operand_is_its_own_value() {
        assert_eq!(ok(&["abc"]), "abc");
        assert_eq!(ok(&["-5"]), "-5");
        assert_eq!(ok(&[""]), "");
        assert_eq!(ok(&["007"]), "007");
    }

    #[test]
    fn arithmetic() {
        assert_eq!(ok(&["1", "+", "2"]), "3");
        assert_eq!(ok(&["5", "-", "7"]), "-2");
        assert_eq!(ok(&["6", "*", "7"]), "42");
        assert_eq!(ok(&["7", "/", "2"]), "3");
        assert_eq!(ok(&["-7", "/", "2"]), "-3");
        assert_eq!(ok(&["-7", "%", "2"]), "-1");
        assert_eq!(ok(&["7", "%", "-2"]), "1");
        assert_eq!(ok(&["007", "+", "-0"]), "7");
        assert_eq!(ok(&["-3", "*", "-3"]), "9");
    }

    #[test]
    fn precedence_and_left_associativity() {
        assert_eq!(ok(&["2", "+", "3", "*", "4"]), "14");
        assert_eq!(ok(&["(", "2", "+", "3", ")", "*", "4"]), "20");
        assert_eq!(ok(&["10", "-", "3", "-", "2"]), "5");
        assert_eq!(ok(&["100", "/", "10", "/", "5"]), "2");
        assert_eq!(ok(&["2", "*", "3", "%", "4"]), "2");
        assert_eq!(ok(&["1", "+", "2", "=", "3"]), "1");
        assert_eq!(ok(&["1", "=", "1", "=", "1"]), "1");
        assert_eq!(ok(&["2", "=", "2", "=", "2"]), "0");
        assert_eq!(ok(&["1", "|", "0", "&", "0"]), "1");
        assert_eq!(ok(&["(", "(", "(", "4", ")", ")", ")"]), "4");
    }

    #[test]
    fn colon_binds_tighter_than_arithmetic() {
        assert_eq!(ok(&["abc", ":", "a", "+", "1"]), "2");
        assert_eq!(ok(&["2", "*", "abc", ":", "ab"]), "4");
    }

    #[test]
    fn division_by_zero_and_non_integers() {
        assert_eq!(status(&["1", "/", "0"]), 2);
        assert_eq!(status(&["1", "%", "0"]), 2);
        assert_eq!(status(&["a", "+", "1"]), 2);
        assert_eq!(status(&["1", "+", "1.5"]), 2);
        assert_eq!(status(&["1", "+", "+1"]), 2);
        assert_eq!(status(&["", "+", "1"]), 2);
        assert_eq!(status(&["-", "+", "1"]), 2);
    }

    #[test]
    fn integers_that_do_not_fit() {
        assert_eq!(ok(&["9223372036854775807", "+", "0"]), "9223372036854775807");
        assert_eq!(ok(&["-9223372036854775808", "+", "0"]), "-9223372036854775808");
        assert_eq!(status(&["9223372036854775807", "+", "1"]), 3);
        assert_eq!(status(&["-9223372036854775808", "-", "1"]), 3);
        assert_eq!(status(&["-9223372036854775808", "/", "-1"]), 3);
        assert_eq!(status(&["99999999999999999999", "+", "1"]), 3);
        assert_eq!(status(&["4611686018427387904", "*", "2"]), 3);
    }

    #[test]
    fn comparisons_of_integers_of_any_size() {
        assert_eq!(ok(&["10", "<", "9"]), "0");
        assert_eq!(ok(&["10", ">", "9"]), "1");
        assert_eq!(ok(&["-10", "<", "-9"]), "1");
        assert_eq!(ok(&["007", "=", "7"]), "1");
        assert_eq!(ok(&["-0", "=", "0"]), "1");
        assert_eq!(ok(&["99999999999999999999999", ">", "99999999999999999999998"]), "1");
        assert_eq!(ok(&["-99999999999999999999999", "<", "5"]), "1");
        assert_eq!(ok(&["5", "!=", "5"]), "0");
        assert_eq!(ok(&["5", ">=", "5"]), "1");
        assert_eq!(ok(&["5", "<=", "4"]), "0");
    }

    #[test]
    fn strings_compare_by_collation_when_either_is_not_an_integer() {
        assert_eq!(ok(&["abc", "<", "abd"]), "1");
        assert_eq!(ok(&["abc", "=", "abc"]), "1");
        assert_eq!(ok(&["abc", "!=", "abd"]), "1");
        /* "10" and "9" as strings: 1 sorts before 9 */
        assert_eq!(ok(&["10", "<", "9a"]), "1");
        assert_eq!(ok(&["", "=", ""]), "1");
        assert_eq!(ok(&["a", ">", ""]), "1");
        assert_eq!(ok(&["+1", "=", "1"]), "0");
        assert_eq!(ok(&["1 ", "=", "1"]), "0");
    }

    #[test]
    fn or_and_and() {
        assert_eq!(ok(&["a", "|", "b"]), "a");
        assert_eq!(ok(&["", "|", "b"]), "b");
        assert_eq!(ok(&["0", "|", "b"]), "b");
        assert_eq!(ok(&["00", "|", "b"]), "b");
        assert_eq!(ok(&["0", "|", ""]), "0");
        assert_eq!(ok(&["", "|", ""]), "0");
        assert_eq!(ok(&["0", "|", "0"]), "0");
        assert_eq!(ok(&["a", "&", "b"]), "a");
        assert_eq!(ok(&["a", "&", "0"]), "0");
        assert_eq!(ok(&["a", "&", ""]), "0");
        assert_eq!(ok(&["0", "&", "b"]), "0");
        assert_eq!(ok(&["", "&", "b"]), "0");
        assert_eq!(ok(&["x", "|", "y", "|", "z"]), "x");
    }

    #[test]
    fn the_operand_that_is_not_needed_is_not_evaluated() {
        assert_eq!(ok(&["1", "|", "1", "/", "0"]), "1");
        assert_eq!(ok(&["0", "&", "1", "/", "0"]), "0");
        assert_eq!(ok(&["0", "|", "1", "/", "0"]), "error division by zero");
        assert_eq!(ok(&["1", "&", "1", "/", "0"]), "error division by zero");
        assert_eq!(ok(&["1", "|", "a", "+", "1"]), "1");
        /* ... but it must be well formed */
        assert_eq!(status(&["1", "|", "("]), 2);
        assert_eq!(status(&["1", "|", "2", "+"]), 2);
    }

    #[test]
    fn operators_are_told_from_operands_by_position() {
        assert_eq!(ok(&["=", "=", "="]), "1");
        assert_eq!(ok(&["+", "=", "+"]), "1");
        assert_eq!(ok(&["X=", "=", "X="]), "1");
        assert_eq!(ok(&["(", "=", ")"]), "=");
        assert_eq!(ok(&["|", "|", "x"]), "|");
        assert_eq!(ok(&["match"]), "match");
        assert_eq!(ok(&["length", "=", "length"]), "1");
    }

    #[test]
    fn syntax_errors() {
        assert_eq!(status(&["1", "+"]), 2);
        assert_eq!(status(&["1", "2"]), 2);
        assert_eq!(status(&["("]), 2);
        assert_eq!(status(&["(", "1"]), 2);
        assert_eq!(status(&["(", ")"]), 2);
        assert_eq!(status(&["1", ")"]), 2);
        assert_eq!(status(&["(", "1", ")", ")"]), 2);
        assert_eq!(status(&["1", "+", "+"]), 2);
        assert_eq!(status(&[]), 2);
        assert_eq!(status(&["1", ":"]), 2);
    }

    #[test]
    fn matching_counts_characters() {
        assert_eq!(ok(&["foo", ":", "fo*"]), "3");
        assert_eq!(ok(&["foo", ":", ".*"]), "3");
        assert_eq!(ok(&["foo", ":", "o"]), "0");
        assert_eq!(ok(&["", ":", ".*"]), "0");
        assert_eq!(ok(&["foo", ":", ""]), "0");
        assert_eq!(ok(&["abc", ":", "a\\{1\\}"]), "1");
    }

    #[test]
    fn matching_with_a_subexpression_returns_it() {
        assert_eq!(ok(&["/usr/abc/file", ":", ".*/\\(.*\\)"]), "file");
        assert_eq!(ok(&["file", ":", ".*/\\(.*\\)"]), "");
        assert_eq!(ok(&["abc", ":", "\\(b\\)"]), "");
        assert_eq!(ok(&["abc", ":", "\\(a\\)\\(b\\)"]), "a");
        assert_eq!(ok(&["abc", ":", "\\(x*\\)abc"]), "");
        assert_eq!(ok(&["abc", ":", "a\\(x\\)*bc"]), "");
        assert_eq!(ok(&["//file", ":", ".*/\\(.*\\)"]), "file");
        assert_eq!(ok(&["file", ":", ".*/\\(.*\\)", "|", "file"]), "file");
        /* the idiom for the length of a string */
        assert_eq!(ok(&["Xhello", ":", ".*", "-", "1"]), "5");
    }

    #[test]
    fn groups_are_found_by_shape() {
        assert!(has_group(b"\\(a\\)"));
        assert!(has_group(b"x*\\(.*\\)"));
        assert!(!has_group(b"abc"));
        assert!(!has_group(b"\\\\(a"));
        assert!(!has_group(b"[\\(]"));
        assert!(!has_group(b"[[:alpha:]\\(]"));
        assert!(!has_group(b"[]\\(]"));
        assert!(!has_group(b"[^]\\(]"));
        assert!(has_group(b"[]]\\(a\\)"));
        assert!(has_group(b"[[.].]]\\(a\\)"));
        assert!(!has_group(b""));
        assert!(!has_group(b"a\\"));
    }

    #[test]
    fn an_invalid_pattern_is_an_error() {
        assert_eq!(status(&["a", ":", "\\("]), 2);
        assert_eq!(status(&["a", ":", "a\\{"]), 2);
        assert_eq!(status(&["a", ":", "[a"]), 2);
        assert_eq!(status(&["a", ":", "[b-a]"]), 2);
        assert_eq!(status(&["ab", ":", "\\(a\\(b\\)\\)\\3"]), 2);
    }

    #[test]
    fn the_match_is_anchored_at_the_start() {
        assert_eq!(ok(&["abc", ":", "b"]), "0");
        assert_eq!(ok(&["abc", ":", "abc"]), "3");
        assert_eq!(ok(&["abc", ":", "ab"]), "2");
        assert_eq!(ok(&["abc", ":", "abcd"]), "0");
        assert_eq!(ok(&["aab", ":", "a*b"]), "3");
    }

    #[test]
    fn nesting_is_limited_but_long_chains_are_not() {
        let mut a: Vec<&str> = vec!["("; 100];
        a.push("1");
        a.extend(std::iter::repeat_n(")", 100));
        assert_eq!(ok(&a), "1");
        let mut a: Vec<&str> = vec!["("; 5000];
        a.push("1");
        a.extend(std::iter::repeat_n(")", 5000));
        assert_eq!(status(&a), 2);
        let mut a: Vec<&str> = vec!["0"];
        for _ in 0..200_000 {
            a.push("+");
            a.push("1");
        }
        assert_eq!(ok(&a), "200000");
        let mut a: Vec<&str> = vec!["1"];
        for _ in 0..200_000 {
            a.push("|");
            a.push("0");
        }
        assert_eq!(ok(&a), "1");
    }

    #[test]
    fn null_and_zero_are_false() {
        assert!(is_false(b""));
        assert!(is_false(b"0"));
        assert!(is_false(b"-0"));
        assert!(is_false(b"000"));
        assert!(!is_false(b"1"));
        assert!(!is_false(b"-1"));
        assert!(!is_false(b"a"));
        assert!(!is_false(b"0a"));
        assert!(!is_false(b"+0"));
        assert!(!is_false(b" 0"));
    }

    #[test]
    fn matching_counts_characters_not_bytes_in_utf8() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        if !posix::locale::try_set("en_US.UTF-8") {
            return;
        }
        assert_eq!(ok(&["\u{e9}\u{e9}x", ":", ".*"]), "3");
        assert_eq!(ok(&["\u{e9}\u{e9}x", ":", "..\\(.\\)"]), "x");
        posix::locale::set_c();
    }
}
