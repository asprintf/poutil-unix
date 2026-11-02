/* SPDX-License-Identifier: GPL-2.0-only */
/* printf - write formatted output. */


use std::io::{self, Write};

const SYNOPSIS: &str = "printf format [argument...]";
const BLOCK: usize = 65536;

/* One conversion specification: %[flags][width][.precision]conversion. */
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Spec {
    minus: bool,
    plus: bool,
    space: bool,
    hash: bool,
    zero: bool,
    width: usize,
    prec: Option<usize>,
    conv: u8,
}

#[derive(Debug, PartialEq, Eq)]
enum Piece {
    /* bytes written as they are (escape sequences already turned into bytes) */
    Lit(Vec<u8>),
    Conv(Spec),
    /* a "%" that does not make a conversion specification; the message says why */
    Invalid(String),
}

/* The largest field width or precision: what a C int holds. */
const LIMIT: usize = i32::MAX as usize;

fn octal(c: u8) -> bool {
    (b'0'..=b'7').contains(&c)
}

/* The byte a backslash and one letter stand for (XBD File Format Notation), if it is one. */
fn simple_escape(c: u8) -> Option<u8> {
    Some(match c {
        b'\\' => b'\\',
        b'a' => 7,
        b'b' => 8,
        b'f' => 12,
        b'n' => b'\n',
        b'r' => b'\r',
        b't' => b'\t',
        b'v' => 11,
        _ => return None,
    })
}

/* Up to max octal digits from s, as a byte (the value wraps like a C char), and how many were used. */
fn octal_digits(s: &[u8], max: usize) -> (u8, usize) {
    let mut v: u32 = 0;
    let mut n = 0;
    while n < max && n < s.len() && octal(s[n]) {
        v = v * 8 + u32::from(s[n] - b'0');
        n += 1;
    }
    (v as u8, n)
}

/*
 * The format operand as pieces. A backslash sequence is one of the
 * standard's: \\ \a \b \f \n \r \t \v and \ddd of one to three octal digits.
 * What follows a backslash otherwise is unspecified: both are written. The
 * first "%" that is not a valid conversion specification ends the format
 * with an Invalid piece, since what it means is unspecified.
 */
fn parse_format(fmt: &[u8]) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut lit: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        if c == b'\\' {
            match fmt.get(i + 1) {
                Some(&d) if octal(d) => {
                    let (v, n) = octal_digits(&fmt[i + 1..], 3);
                    lit.push(v);
                    i += 1 + n;
                }
                Some(&d) => match simple_escape(d) {
                    Some(b) => {
                        lit.push(b);
                        i += 2;
                    }
                    None => {
                        lit.push(b'\\');
                        i += 1;
                    }
                },
                None => {
                    lit.push(b'\\');
                    i += 1;
                }
            }
        } else if c == b'%' {
            if !lit.is_empty() {
                pieces.push(Piece::Lit(std::mem::take(&mut lit)));
            }
            match parse_spec(&fmt[i..]) {
                Ok((None, n)) => {
                    lit.push(b'%');
                    i += n;
                }
                Ok((Some(spec), n)) => {
                    pieces.push(Piece::Conv(spec));
                    i += n;
                }
                Err(m) => {
                    pieces.push(Piece::Invalid(m));
                    return pieces;
                }
            }
        } else {
            lit.push(c);
            i += 1;
        }
    }
    if !lit.is_empty() {
        pieces.push(Piece::Lit(lit));
    }
    pieces
}

/* A decimal digit string at the start of s: its value (at most LIMIT + 1) and its length. */
fn digits(s: &[u8]) -> (usize, usize) {
    let mut v: usize = 0;
    let mut n = 0;
    while n < s.len() && s[n].is_ascii_digit() {
        v = (v * 10 + usize::from(s[n] - b'0')).min(LIMIT + 1);
        n += 1;
    }
    (v, n)
}

/*
 * The conversion specification at the start of s (which begins with "%"): the
 * specification, or None for "%%", and its length; or why it is not one.
 */
fn parse_spec(s: &[u8]) -> Result<(Option<Spec>, usize), String> {
    let mut sp = Spec::default();
    let mut i = 1;
    while let Some(&c) = s.get(i) {
        match c {
            b'-' => sp.minus = true,
            b'+' => sp.plus = true,
            b' ' => sp.space = true,
            b'#' => sp.hash = true,
            b'0' => sp.zero = true,
            _ => break,
        }
        i += 1;
    }
    let (w, n) = digits(&s[i..]);
    sp.width = w;
    i += n;
    let has_width = n > 0;
    let mut has_prec = false;
    if s.get(i) == Some(&b'.') {
        has_prec = true;
        let (p, n) = digits(&s[i + 1..]);
        sp.prec = Some(p);
        i += 1 + n;
    }
    let shown = || String::from_utf8_lossy(&s[..(i + 1).min(s.len())]).into_owned();
    if sp.width > LIMIT || sp.prec.is_some_and(|p| p > LIMIT) {
        return Err(format!("{}: field width or precision too large", shown()));
    }
    let Some(&conv) = s.get(i) else {
        return Err(format!("{}: missing conversion specifier", shown()));
    };
    match conv {
        b'%' if i == 1 && !has_width && !has_prec => Ok((None, 2)),
        b'd' | b'i' | b'o' | b'u' | b'x' | b'X' | b'c' | b's' | b'b' | b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => {
            sp.conv = conv;
            Ok((Some(sp), i + 1))
        }
        _ => Err(format!("{}: invalid conversion specification", shown())),
    }
}

/* An argument for a numeric conversion: the value as 64 bits and what is wrong with the argument, if anything. */
struct Num {
    bits: u64,
    problem: Option<&'static str>,
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 11 | 12 | b'\r')
}

/*
 * An argument as an unsuffixed C integer constant: decimal, octal with a
 * leading 0, hexadecimal with 0x; with an optional sign in front; or, after a
 * single or double quote, the value of the character that follows. The
 * rest is the strtoimax() and strtoumax() of signed and unsigned
 * conversions: a value too big is the biggest, and the argument is wrong; so
 * is one with something after the number, or with no number.
 */
fn number(arg: &[u8], signed: bool) -> Num {
    if let Some(&q) = arg.first() {
        if q == b'\'' || q == b'"' {
            return match arg.get(1) {
                None => Num { bits: 0, problem: Some("expected numeric value") },
                Some(_) => {
                    let (_, wc) = posix::mb::char_at(&arg[1..]);
                    Num { bits: u64::from(wc.unwrap_or(u32::from(arg[1]))), problem: None }
                }
            };
        }
    }
    let mut i = 0;
    while i < arg.len() && is_space(arg[i]) {
        i += 1;
    }
    let mut neg = false;
    match arg.get(i) {
        Some(b'+') => i += 1,
        Some(b'-') => {
            neg = true;
            i += 1;
        }
        _ => {}
    }
    let (base, start) = match (arg.get(i), arg.get(i + 1), arg.get(i + 2)) {
        (Some(b'0'), Some(b'x' | b'X'), Some(h)) if h.is_ascii_hexdigit() => (16u32, i + 2),
        (Some(b'0'), _, _) => (8, i),
        _ => (10, i),
    };
    let mut j = start;
    let mut mag: u128 = 0;
    let mut over = false;
    while let Some(d) = arg.get(j).and_then(|&c| (c as char).to_digit(base)) {
        mag = mag * u128::from(base) + u128::from(d);
        if mag > u128::from(u64::MAX) * 2 {
            over = true;
            mag = u128::from(u64::MAX) * 2;
        }
        j += 1;
    }
    if j == start {
        return Num { bits: 0, problem: Some("expected numeric value") };
    }
    let bits: u64;
    if signed {
        let max = i64::MAX as u128;
        if !neg && mag > max {
            over = true;
            bits = i64::MAX as u64;
        } else if neg && mag > max + 1 {
            over = true;
            bits = i64::MIN as u64;
        } else if neg {
            bits = (mag as u64).wrapping_neg();
        } else {
            bits = mag as u64;
        }
    } else if mag > u128::from(u64::MAX) {
        over = true;
        bits = u64::MAX;
    } else if neg {
        bits = (mag as u64).wrapping_neg();
    } else {
        bits = mag as u64;
    }
    let problem = if over {
        Some("arithmetic overflow")
    } else if j < arg.len() {
        Some("not completely converted")
    } else {
        None
    };
    Num { bits, problem }
}

/* The C conversion specification for a Spec: flags that do not apply to the conversion are left out. */
fn c_format(sp: &Spec, length: &str) -> std::ffi::CString {
    let signed = matches!(sp.conv, b'd' | b'i') || is_real(sp.conv);
    let mut f = String::from("%");
    if sp.minus {
        f.push('-');
    }
    if signed && sp.plus {
        f.push('+');
    }
    if signed && sp.space && !sp.plus {
        f.push(' ');
    }
    if (matches!(sp.conv, b'o' | b'x' | b'X') || is_real(sp.conv)) && sp.hash {
        f.push('#');
    }
    if sp.zero {
        f.push('0');
    }
    if sp.width > 0 {
        f.push_str(&sp.width.to_string());
    }
    if let Some(p) = sp.prec {
        f.push('.');
        f.push_str(&p.to_string());
    }
    f.push_str(length);
    f.push(sp.conv as char);
    std::ffi::CString::new(f).unwrap_or_default()
}

/* Run snprintf with a buffer that grows until the result fits. */
fn snprintf_into(sp: &Spec, call: impl Fn(*mut libc::c_char, usize) -> libc::c_int) -> Vec<u8> {
    let mut buf: Vec<u8> = vec![0; 64 + sp.width.max(sp.prec.unwrap_or(0))];
    loop {
        let n = call(buf.as_mut_ptr() as *mut libc::c_char, buf.len());
        if n < 0 {
            return Vec::new();
        }
        if (n as usize) < buf.len() {
            buf.truncate(n as usize);
            return buf;
        }
        buf.resize(n as usize + 1, 0);
    }
}

/* An integer through the C library's printf, which does the flags, the width and the precision. */
fn format_int(sp: &Spec, bits: u64) -> Vec<u8> {
    let cf = c_format(sp, "j");
    let signed = sp.conv == b'd' || sp.conv == b'i';
    snprintf_into(sp, |p, n| unsafe {
        if signed {
            libc::snprintf(p, n, cf.as_ptr(), bits as i64 as libc::intmax_t)
        } else {
            libc::snprintf(p, n, cf.as_ptr(), bits as libc::uintmax_t)
        }
    })
}

fn is_real(conv: u8) -> bool {
    matches!(conv, b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G')
}

/* A floating-point number through the C library's printf (LC_NUMERIC gives the radix character). */
fn format_real(sp: &Spec, v: f64) -> Vec<u8> {
    if v.is_nan() {
        /*
         * The standard has "[-]nan" for a NaN. Some C libraries (macOS) leave
         * the sign out, so it is written here: with the sign flags, in
         * capitals for A E F G, blanks to the field width (the 0 flag does not
         * apply to a NaN).
         */
        let mut body = Vec::new();
        if v.is_sign_negative() {
            body.push(b'-');
        } else if sp.plus {
            body.push(b'+');
        } else if sp.space {
            body.push(b' ');
        }
        body.extend_from_slice(if sp.conv.is_ascii_uppercase() { b"NAN" } else { b"nan" });
        let mut out = Vec::new();
        pad(&mut out, &body, &Spec { minus: sp.minus, width: sp.width, ..Spec::default() });
        return out;
    }
    let cf = c_format(sp, "");
    snprintf_into(sp, |p, n| unsafe { libc::snprintf(p, n, cf.as_ptr(), v) })
}

/* An argument for a floating-point conversion: the value and what is wrong with the argument. */
struct Real {
    value: f64,
    problem: Option<&'static str>,
}

extern "C" {
    /* the libc crate does not have it on every target */
    fn strtod(s: *const libc::c_char, end: *mut *mut libc::c_char) -> f64;
}

/*
 * An argument as strtod() reads it (the standard: "evaluated as if by the
 * strtod() function"), in the radix character of LC_NUMERIC. A single or
 * double quote is no special case here, only for the integer conversions.
 * No number at all, something after the number, and a value too big for a
 * double are the arguments that are wrong; the value is what strtod gives.
 */
fn real(arg: &[u8]) -> Real {
    let c = std::ffi::CString::new(arg).unwrap_or_default();
    posix::clear_errno();
    let mut end: *mut libc::c_char = std::ptr::null_mut();
    let mut value = unsafe { strtod(c.as_ptr(), &mut end) };
    if value.is_nan() {
        /* the sign of a NaN is in the text, whatever the C library's strtod does with it */
        let text = arg.iter().position(|&b| !is_space(b)).map_or(&arg[..0], |i| &arg[i..]);
        let neg = u64::from(text.first() == Some(&b'-'));
        value = f64::from_bits((value.to_bits() & !(1 << 63)) | (neg << 63));
    }
    let used = end as usize - c.as_ptr() as usize;
    let range = std::io::Error::last_os_error().raw_os_error() == Some(libc::ERANGE);
    let problem = if used == 0 {
        Some("expected numeric value")
    } else if range && value.is_infinite() {
        Some("arithmetic overflow")
    } else if used < arg.len() {
        Some("not completely converted")
    } else {
        None
    };
    Real { value, problem }
}

/* The bytes of a string conversion padded with blanks to the field width. */
fn pad(out: &mut Vec<u8>, s: &[u8], sp: &Spec) {
    let fill = sp.width.saturating_sub(s.len());
    if sp.minus {
        out.extend_from_slice(s);
        out.resize(out.len() + fill, b' ');
    } else {
        out.resize(out.len() + fill, b' ');
        out.extend_from_slice(s);
    }
}

/*
 * The argument of %b with its backslash sequences turned into the bytes
 * they stand for: \\ \a \b \f \n \r \t \v, \0ddd (zero to three octal
 * digits after the 0) and, as the shells have it, \ddd; \c ends it and
 * says that everything else is to be ignored (the return value). What follows
 * a backslash otherwise is unspecified: both are kept.
 */
fn expand_b(arg: &[u8], out: &mut Vec<u8>) -> bool {
    let mut i = 0;
    while i < arg.len() {
        if arg[i] != b'\\' {
            out.push(arg[i]);
            i += 1;
            continue;
        }
        match arg.get(i + 1) {
            Some(b'c') => return true,
            Some(b'0') => {
                let (v, n) = octal_digits(&arg[i + 2..], 3);
                out.push(v);
                i += 2 + n;
            }
            Some(&d) if octal(d) => {
                let (v, n) = octal_digits(&arg[i + 1..], 3);
                out.push(v);
                i += 1 + n;
            }
            Some(&d) => match simple_escape(d) {
                Some(b) => {
                    out.push(b);
                    i += 2;
                }
                None => {
                    out.push(b'\\');
                    i += 1;
                }
            },
            None => {
                out.push(b'\\');
                i += 1;
            }
        }
    }
    false
}

/*
 * Write the format with the arguments. The format is used again as long as
 * arguments are left; a conversion that finds none is given a null string
 * (b, c, s) or a zero. A conversion error is a diagnostic and a non-zero
 * status, and the output goes on with the value found so far. Returns the
 * exit status.
 */
fn printf<W: Write>(fmt: &[u8], args: &[Vec<u8>], w: &mut W, warn: &mut dyn FnMut(&str)) -> io::Result<i32> {
    let pieces = parse_format(fmt);
    let uses = pieces.iter().filter(|p| matches!(p, Piece::Conv(_))).count();
    let mut status = 0;
    let mut next = 0;
    let mut out: Vec<u8> = Vec::new();
    loop {
        for p in &pieces {
            match p {
                Piece::Lit(b) => out.extend_from_slice(b),
                Piece::Invalid(m) => {
                    w.write_all(&out)?;
                    warn(m);
                    return Ok(1);
                }
                Piece::Conv(sp) => {
                    let given: Option<&Vec<u8>> = args.get(next);
                    next += 1;
                    let arg: &[u8] = given.map_or(&[][..], |a| a.as_slice());
                    match sp.conv {
                        b'd' | b'i' | b'o' | b'u' | b'x' | b'X' => {
                            /* no argument is a zero; an empty one is not a number */
                            let n = match given {
                                Some(_) => number(arg, sp.conv == b'd' || sp.conv == b'i'),
                                None => Num { bits: 0, problem: None },
                            };
                            if let Some(m) = n.problem {
                                warn(&format!("{}: {}", String::from_utf8_lossy(arg), m));
                                status = 1;
                            }
                            out.extend_from_slice(&format_int(sp, n.bits));
                        }
                        conv if is_real(conv) => {
                            /* no argument is a zero; an empty one is not a number */
                            let r = match given {
                                Some(_) => real(arg),
                                None => Real { value: 0.0, problem: None },
                            };
                            if let Some(m) = r.problem {
                                warn(&format!("{}: {}", String::from_utf8_lossy(arg), m));
                                status = 1;
                            }
                            out.extend_from_slice(&format_real(sp, r.value));
                        }
                        b'c' => pad(&mut out, &arg[..arg.len().min(1)], sp),
                        b's' => {
                            let cut = sp.prec.map_or(arg.len(), |p| p.min(arg.len()));
                            pad(&mut out, &arg[..cut], sp);
                        }
                        _ => {
                            let mut conv = Vec::new();
                            let stop = expand_b(arg, &mut conv);
                            let cut = sp.prec.map_or(conv.len(), |p| p.min(conv.len()));
                            pad(&mut out, &conv[..cut], sp);
                            if stop {
                                w.write_all(&out)?;
                                return Ok(status);
                            }
                        }
                    }
                }
            }
            if out.len() >= BLOCK {
                w.write_all(&out)?;
                out.clear();
            }
        }
        if uses == 0 || next >= args.len() {
            break;
        }
    }
    w.write_all(&out)?;
    Ok(status)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let operands = posix::no_options(&args, SYNOPSIS);
    let Some((fmt, rest)) = operands.split_first() else {
        posix::usage(SYNOPSIS);
    };
    let mut w = io::BufWriter::with_capacity(BLOCK, posix::Stdout::new());
    let mut warn = |m: &str| posix::warn(&prog, m);
    let status = match printf(fmt, rest, &mut w, &mut warn).and_then(|s| w.flush().map(|_| s)) {
        Ok(s) => s,
        Err(e) => posix::fail(&prog, &format!("write error: {}", posix::strerror(&e))),
    };
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /* the locale is process-wide: tests that change it hold this */
    static LOCALE: Mutex<()> = Mutex::new(());

    struct R {
        out: Vec<u8>,
        diags: Vec<String>,
        status: i32,
    }

    impl R {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.out).into_owned()
        }
    }

    fn run_b(fmt: &[u8], args: &[&[u8]]) -> R {
        let args: Vec<Vec<u8>> = args.iter().map(|a| a.to_vec()).collect();
        let mut out = Vec::new();
        let mut diags = Vec::new();
        let status = printf(fmt, &args, &mut out, &mut |m: &str| diags.push(m.to_string())).unwrap();
        R { out, diags, status }
    }

    fn run(fmt: &str, args: &[&str]) -> R {
        let a: Vec<&[u8]> = args.iter().map(|s| s.as_bytes()).collect();
        run_b(fmt.as_bytes(), &a)
    }

    /* EXAMPLES: the alert and prompt */
    #[test]
    fn example_alert_and_prompt() {
        let r = run("\\aPlease fill in the following: \\nName: ", &[]);
        assert_eq!(r.out, b"\x07Please fill in the following: \nName: ");
        assert_eq!((r.status, r.diags.len()), (0, 0));
        assert_eq!(run("Phone number: ", &[]).text(), "Phone number: ");
    }

    /* EXAMPLES: right-justified numbers separated by a tab, a percentage as a string */
    #[test]
    fn example_right_and_wrong() {
        let r = run("%2d right\\t%2d wrong\\t(%s%%)\\n", &["3", "1", "75.0"]);
        assert_eq!(r.text(), " 3 right\t 1 wrong\t(75.0%)\n");
        assert_eq!(r.status, 0);
    }

    /* EXAMPLES: the format is used three times and a 0 is supplied for the last %4d */
    #[test]
    fn example_format_reused_and_a_zero_supplied() {
        let r = run("%5d%4d\\n", &["1", "21", "321", "4321", "54321"]);
        assert_eq!(r.text(), "    1  21\n  3214321\n54321   0\n");
        assert_eq!((r.status, r.diags.len()), (0, 0));
    }

    /* EXAMPLES: the conversion errors, for 64-bit integers; the value written is what strtoimax gives */
    #[test]
    fn example_conversion_errors() {
        let r = run("%d\\n", &["5a"]);
        assert_eq!((r.text(), r.status), ("5\n".to_string(), 1));
        assert_eq!(r.diags, ["5a: not completely converted"]);
        let r = run("%d\\n", &["9999999999999999999999"]);
        assert_eq!((r.text(), r.status), ("9223372036854775807\n".to_string(), 1));
        assert_eq!(r.diags, ["9999999999999999999999: arithmetic overflow"]);
        let r = run("%d\\n", &["-9999999999999999999999"]);
        assert_eq!((r.text(), r.status), ("-9223372036854775808\n".to_string(), 1));
        assert_eq!(r.diags, ["-9999999999999999999999: arithmetic overflow"]);
        let r = run("%d\\n", &["ABC"]);
        assert_eq!((r.text(), r.status), ("0\n".to_string(), 1));
        assert_eq!(r.diags, ["ABC: expected numeric value"]);
    }

    /* EXAMPLES: in a locale with ISO/IEC 646 as the codeset */
    #[test]
    fn example_numbers_and_quoted_characters() {
        let r = run("%d\\n", &["3", "+3", "-3", "'3", "\"+3", "'-3"]);
        assert_eq!(r.text(), "3\n3\n-3\n51\n43\n45\n");
        assert_eq!((r.status, r.diags.len()), (0, 0));
    }

    #[test]
    fn a_space_in_the_format_is_an_ordinary_character() {
        assert_eq!(run("a b  c %s d\\n", &["x"]).text(), "a b  c x d\n");
        assert_eq!(run(" %d %d ", &["1", "2"]).text(), " 1 2 ");
    }

    #[test]
    fn square_brackets_are_square_brackets_not_a_space() {
        assert_eq!(run("a[]b", &[]).text(), "a[]b");
        assert_eq!(run("%s[]%s", &["x", "y"]).text(), "x[]y");
    }

    #[test]
    fn escape_sequences_of_the_format() {
        assert_eq!(run("\\\\|\\a|\\b|\\f|\\n|\\r|\\t|\\v", &[]).out, b"\\|\x07|\x08|\x0c|\n|\r|\t|\x0b");
    }

    #[test]
    fn octal_escapes_of_one_to_three_digits() {
        /* \1 \12 \101 are one, two and three digits; \1012 is \101 and a 2; \0101 is \010 and a 1 */
        assert_eq!(run("\\1|\\12|\\101|\\1012|\\0|\\00|\\000|\\0101", &[]).out, b"\x01|\n|A|A2|\0|\0|\0|\x081");
    }

    #[test]
    fn octal_escapes_give_any_byte() {
        assert_eq!(run("\\377\\200\\0", &[]).out, [0xff, 0x80, 0]);
        assert_eq!(run("\\1234", &[]).out, b"S4");
        /* a value over 255 wraps like a C char */
        assert_eq!(run("\\777", &[]).out, [0xff]);
        assert_eq!(run("\\18", &[]).out, b"\x018");
    }

    #[test]
    fn what_follows_a_backslash_otherwise_is_written_with_it() {
        assert_eq!(run("a\\qb\\%c", &[]).out, b"a\\qb\\");
        assert_eq!(run("x\\", &[]).out, b"x\\");
        assert_eq!(run("a\\cb", &[]).out, b"a\\cb");
        assert_eq!(run("\\x41", &[]).out, b"\\x41");
    }

    #[test]
    fn percent_percent_is_a_percent_and_takes_no_argument() {
        assert_eq!(run("100%%", &[]).text(), "100%");
        assert_eq!(run("%%%s%%", &["x"]).text(), "%x%");
        assert_eq!(run("%d%%\\n", &["5", "6"]).text(), "5%\n6%\n");
    }

    #[test]
    fn d_and_u_are_not_padded_with_blanks_that_the_format_does_not_have() {
        for (f, a, want) in [("%d", "5", "5"), ("%d", "-5", "-5"), ("%i", "42", "42"), ("%u", "7", "7"), ("%d", "0", "0")] {
            assert_eq!(run(f, &[a]).text(), want, "{f}");
        }
    }

    #[test]
    fn o_has_no_zeros_that_the_format_does_not_have() {
        assert_eq!(run("%o", &["8"]).text(), "10");
        assert_eq!(run("%o", &["0"]).text(), "0");
        assert_eq!(run("%o", &["64"]).text(), "100");
        assert_eq!(run("%#o", &["8"]).text(), "010");
        assert_eq!(run("%5o|%-5o|%05o", &["8", "8", "8"]).text(), "   10|10   |00010");
    }

    #[test]
    fn integer_conversions() {
        assert_eq!(run("%x %X %#x %#X", &["255", "255", "255", "255"]).text(), "ff FF 0xff 0XFF");
        assert_eq!(run("%#x|%#o", &["0", "0"]).text(), "0|0");
        assert_eq!(run("%u", &["-1"]).text(), "18446744073709551615");
        assert_eq!(run("%x", &["-1"]).text(), "ffffffffffffffff");
        assert_eq!(run("%d", &["9223372036854775807"]).text(), "9223372036854775807");
        assert_eq!(run("%d", &["-9223372036854775808"]).text(), "-9223372036854775808");
        assert_eq!(run("%u", &["18446744073709551615"]).text(), "18446744073709551615");
        assert_eq!(run("%i", &["7"]).text(), "7");
    }

    #[test]
    fn flags_width_and_precision_of_integers() {
        assert_eq!(run("%+d|% d|%+d|% d", &["5", "5", "-5", "-5"]).text(), "+5| 5|-5|-5");
        assert_eq!(run("%+ d|% +d", &["5", "5"]).text(), "+5|+5");
        assert_eq!(run("%5d|%-5d|%05d|%-05d", &["42", "42", "42", "42"]).text(), "   42|42   |00042|42   ");
        assert_eq!(run("%.3d|%.0d|%.0d|%5.3d|%05.3d", &["5", "0", "7", "5", "5"]).text(), "005||7|  005|  005");
        assert_eq!(run("%+05d|%+.3d|%08.3d", &["5", "5", "-5"]).text(), "+0005|+005|    -005");
        assert_eq!(run("%.d", &["0"]).text(), "");
        assert_eq!(run("%5.d|%.x|%#.o", &["0", "0", "0"]).text(), "     ||0");
        assert_eq!(run("%3d", &["12345"]).text(), "12345");
        assert_eq!(run("%0*d", &[]).status, 1);
    }

    #[test]
    fn unsigned_conversions_ignore_plus_and_space() {
        assert_eq!(run("%+u|% u|%+x|% o", &["5", "5", "255", "8"]).text(), "5|5|ff|10");
    }

    #[test]
    fn numeric_arguments_are_c_integer_constants() {
        for (a, want) in [("10", "10"), ("010", "8"), ("0x1f", "31"), ("0X1F", "31"), ("+7", "7"), ("-7", "-7"), ("-010", "-8"), ("-0x10", "-16"), ("0", "0"), ("00", "0"), (" 5", "5"), ("\t\n 12", "12")] {
            let r = run("%d", &[a]);
            assert_eq!(r.text(), want, "{a:?}");
            assert_eq!((r.status, r.diags.len()), (0, 0), "{a:?}");
        }
    }

    #[test]
    fn numeric_arguments_that_are_not_wholly_numbers_are_errors_with_the_value_so_far() {
        for (a, want, msg) in [
            ("5a", "5", "not completely converted"),
            ("08", "0", "not completely converted"),
            ("0x", "0", "not completely converted"),
            ("0xg", "0", "not completely converted"),
            ("1.5", "1", "not completely converted"),
            ("5 ", "5", "not completely converted"),
            ("12abc", "12", "not completely converted"),
            ("--5", "0", "expected numeric value"),
            ("+", "0", "expected numeric value"),
            ("-", "0", "expected numeric value"),
            ("", "0", "expected numeric value"),
            ("abc", "0", "expected numeric value"),
            ("   ", "0", "expected numeric value"),
        ] {
            let r = run("%d", &[a]);
            assert_eq!(r.text(), want, "{a:?}");
            assert_eq!(r.status, 1, "{a:?}");
            assert_eq!(r.diags, [format!("{a}: {msg}")], "{a:?}");
        }
    }

    #[test]
    fn a_conversion_error_goes_on_with_the_other_arguments_and_the_rest_of_the_format() {
        let r = run("[%d][%d][%d]\\n", &["1", "x", "3"]);
        assert_eq!(r.text(), "[1][0][3]\n");
        assert_eq!((r.status, r.diags.len()), (1, 1));
        let r = run("%d %d\\n", &["1a", "2b", "3c"]);
        assert_eq!(r.text(), "1 2\n3 0\n");
        assert_eq!((r.status, r.diags.len()), (1, 3));
    }

    #[test]
    fn an_unsigned_value_that_does_not_fit_is_the_biggest() {
        let r = run("%u", &["18446744073709551616"]);
        assert_eq!(r.text(), "18446744073709551615");
        assert_eq!(r.diags, ["18446744073709551616: arithmetic overflow"]);
        let r = run("%x", &["99999999999999999999999999999999999999999999"]);
        assert_eq!(r.text(), "ffffffffffffffff");
        assert_eq!(r.status, 1);
    }

    #[test]
    fn a_character_constant_is_the_value_of_the_character_after_the_quote() {
        for (a, want) in [("'A", "65"), ("\"A", "65"), ("' ", "32"), ("'''", "39"), ("'\\", "92"), ("'ab", "97"), ("\"abc", "97"), ("'0", "48"), ("'-", "45")] {
            let r = run("%d", &[a]);
            assert_eq!(r.text(), want, "{a:?}");
            assert_eq!((r.status, r.diags.len()), (0, 0), "{a:?}");
        }
        assert_eq!(run("%x|%o|%u", &["'A", "'A", "'A"]).text(), "41|101|65");
        /* a quote and nothing after it */
        let r = run("%d", &["'"]);
        assert_eq!((r.text(), r.status), ("0".to_string(), 1));
        /* a quote that is not the first character is no quote */
        assert_eq!(run_b(b"%d", &[b"5'"]).status, 1);
    }

    #[test]
    fn a_byte_that_is_no_character_is_its_own_value() {
        let r = run_b(b"%d", &[b"'\xff"]);
        assert_eq!(r.text(), "255");
        assert_eq!(r.status, 0);
    }

    #[test]
    fn a_multibyte_character_is_its_wide_character_value() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let loc = std::ffi::CString::new("en_US.UTF-8").unwrap();
        if unsafe { libc::setlocale(libc::LC_ALL, loc.as_ptr()) }.is_null() {
            eprintln!("skipped: no locale en_US.UTF-8");
            return;
        }
        let r = run("%d|%d|%d", &["'\u{e9}", "\"\u{20ac}", "'\u{1f600}x"]);
        assert_eq!(r.text(), "233|8364|128512");
        assert_eq!(r.status, 0);
        unsafe { libc::setlocale(libc::LC_ALL, c"C".as_ptr()) };
    }

    #[test]
    fn s_writes_the_string_with_width_and_precision_in_bytes() {
        assert_eq!(run("%s|%5s|%-5s|%.2s|%5.2s|%-5.2s|", &["abc", "abc", "abc", "abc", "abc", "abc"]).text(), "abc|  abc|abc  |ab|   ab|ab   |");
        assert_eq!(run("%.0s|%.s|%2s", &["abc", "abc", "abcdef"]).text(), "||abcdef");
        assert_eq!(run("%s", &[""]).text(), "");
        /* bytes, not characters */
        assert_eq!(run_b(b"%.1s|%3s|", &["\u{e9}".as_bytes(), "\u{e9}".as_bytes()]).out, b"\xc3|\x20\xc3\xa9|");
    }

    #[test]
    fn the_zero_flag_does_not_change_strings() {
        assert_eq!(run("%05s|%05c", &["ab", "x"]).text(), "   ab|    x");
    }

    #[test]
    fn c_writes_the_first_byte_and_ignores_the_rest() {
        assert_eq!(run("%c|%c|%c", &["abc", "x", "123"]).text(), "a|x|1");
        let r = run("%c", &["abc"]);
        assert_eq!((r.status, r.diags.len()), (0, 0));
        /* the first byte, not the first character */
        assert_eq!(run_b(b"%c", &["\u{e9}".as_bytes()]).out, b"\xc3");
        assert_eq!(run("%3c|%-3c|", &["a", "b"]).text(), "  a|b  |");
    }

    #[test]
    fn c_of_an_empty_string_writes_nothing() {
        assert_eq!(run("[%c]", &[""]).out, b"[]");
        assert_eq!(run("[%c]", &[]).out, b"[]");
    }

    #[test]
    fn b_turns_backslash_sequences_into_bytes() {
        assert_eq!(run("%b", &["\\\\|\\a|\\b|\\f|\\n|\\r|\\t|\\v"]).out, b"\\|\x07|\x08|\x0c|\n|\r|\t|\x0b");
        assert_eq!(run("%b", &["a\\tb"]).text(), "a\tb");
    }

    #[test]
    fn b_octal_is_0_and_up_to_three_digits() {
        assert_eq!(run("%b", &["\\0|\\01|\\012|\\0101|\\01012"]).out, b"\0|\x01|\n|A|A2");
        assert_eq!(run("%b", &["\\033[0m"]).out, b"\x1b[0m");
        /* \0 and then up to three digits: 003 is 3, and the last 3 is a character */
        assert_eq!(run("%b", &["\\00033"]).out, b"\x033");
        assert_eq!(run("%b", &["\\0378"]).out, b"\x1f8");
        assert_eq!(run("%b", &["\\0777"]).out, [0xff]);
    }

    #[test]
    fn b_also_takes_a_three_digit_octal_as_the_shells_do() {
        assert_eq!(run("%b", &["\\101\\1012"]).out, b"AA2");
    }

    #[test]
    fn b_c_ends_everything() {
        /* the rest of the string, the other operands and the rest of the format */
        let r = run("%b|%s|\\n", &["ab\\ccd", "next"]);
        assert_eq!(r.text(), "ab");
        assert_eq!(r.status, 0);
        let r = run("[%b][%b]", &["x\\cy", "z"]);
        assert_eq!(r.text(), "[x");
        assert_eq!(run("%b", &["\\cab"]).text(), "");
        assert_eq!(run("a%bb", &["\\c"]).text(), "a");
        /* what came before \\c in the same conversion is written, padded */
        assert_eq!(run("%5b|", &["ab\\c"]).text(), "   ab");
        assert_eq!(run("%d %b %d", &["1", "x\\c", "2"]).text(), "1 x");
        /* the format goes on in the next pass if there is no \\c */
        assert_eq!(run("%b\\n", &["a", "b\\c", "c"]).text(), "a\nb");
    }

    #[test]
    fn b_c_with_an_error_before_it_keeps_the_status() {
        let r = run("%d%b%d", &["1x", "y\\c", "2"]);
        assert_eq!(r.text(), "1y");
        assert_eq!(r.status, 1);
    }

    #[test]
    fn b_precision_cuts_the_converted_bytes() {
        assert_eq!(run("%.2b|", &["a\\tb"]).text(), "a\t|");
        assert_eq!(run("%.3b|", &["a\\nb\\nc"]).text(), "a\nb|");
        assert_eq!(run("%.0b|%.b|", &["abc", "abc"]).text(), "||");
        assert_eq!(run("%6.4b|%-6.4b|", &["ab\\tcd", "ab\\tcd"]).text(), "  ab\tc|ab\tc  |");
    }

    #[test]
    fn b_unspecified_sequences_are_kept_as_they_are() {
        assert_eq!(run("%b", &["a\\qb"]).text(), "a\\qb");
        assert_eq!(run("%b", &["x\\"]).text(), "x\\");
        assert_eq!(run("%b", &["\\x41"]).text(), "\\x41");
    }

    #[test]
    fn b_and_s_differ_only_in_the_escapes() {
        assert_eq!(run("%s|%b", &["a\\nb", "a\\nb"]).text(), "a\\nb|a\nb");
    }

    #[test]
    fn the_format_is_used_again_as_long_as_there_are_arguments() {
        assert_eq!(run("%s\\n", &["a", "b", "c"]).text(), "a\nb\nc\n");
        assert_eq!(run("<%s %s>", &["a", "b", "c", "d"]).text(), "<a b><c d>");
        assert_eq!(run("%d,", &["1", "2", "3"]).text(), "1,2,3,");
    }

    #[test]
    fn a_format_used_up_exactly_is_not_used_again() {
        assert_eq!(run("%s %s\\n", &["a", "b"]).text(), "a b\n");
        assert_eq!(run("%d", &["1"]).text(), "1");
    }

    #[test]
    fn extra_conversions_get_a_null_string_or_a_zero() {
        assert_eq!(run("[%s][%b][%c][%d][%i][%u][%o][%x][%X]", &[]).text(), "[][][][0][0][0][0][0][0]");
        assert_eq!(run("%s-%s-%s|", &["a"]).text(), "a--|");
        assert_eq!(run("%s %d|", &["a", "1", "b"]).text(), "a 1|b 0|");
        assert_eq!(run("%5s|%-5s|%5d|%05d|", &[]).text(), "     |     |    0|00000|");
        assert_eq!(run("%d %s\\n", &[]).text(), "0 \n");
        let r = run("%s%d", &["x"]);
        assert_eq!((r.text(), r.status, r.diags.len()), ("x0".to_string(), 0, 0));
    }

    #[test]
    fn a_format_with_no_conversion_is_written_once_whatever_the_arguments() {
        /* unspecified; the arguments are left alone */
        assert_eq!(run("hello\\n", &["a", "b"]).text(), "hello\n");
        let r = run("hello", &["a"]);
        assert_eq!((r.status, r.diags.len()), (0, 0));
        assert_eq!(run("100%%", &["a", "b"]).text(), "100%");
    }

    #[test]
    fn an_empty_format_writes_nothing() {
        assert_eq!(run("", &["a", "b"]).text(), "");
        assert_eq!(run("", &[]).status, 0);
    }

    #[test]
    fn an_argument_not_wholly_used_by_b_c_or_s_is_no_error() {
        for (f, a) in [("%c", "abc"), ("%.1s", "abc"), ("%.1b", "abc"), ("%b", "abc\\cdef")] {
            let r = run(f, &[a]);
            assert_eq!((r.status, r.diags.len()), (0, 0), "{f}");
        }
    }

    #[test]
    fn a_percent_that_is_no_conversion_specification_ends_the_format() {
        for bad in ["%z", "%", "%5", "%.", "%-", "%ld", "%lld", "%hd", "%5%", "%-%", "%*d", "%.*s", "%1$d", "%n", "%p", "%C", "%S", "%B", "%j", "%q", "% ", "%+", "%#", "%0", "%5.5"] {
            let r = run(&format!("before{bad}!after"), &["1", "2"]);
            assert_eq!(r.status, 1, "{bad}");
            assert_eq!(r.diags.len(), 1, "{bad}");
            assert_eq!(r.text(), "before", "{bad}");
        }
    }

    #[test]
    fn an_invalid_specification_is_found_only_when_the_output_gets_there() {
        let r = run("%d %z %d", &["1", "2", "3"]);
        assert_eq!(r.text(), "1 ");
        assert_eq!(r.status, 1);
        assert!(r.diags[0].contains("%z"), "{:?}", r.diags);
    }

    #[test]
    fn a_huge_width_or_precision_is_an_error_not_a_crash() {
        let r = run("%99999999999d", &["1"]);
        assert_eq!(r.status, 1);
        let r = run("%.99999999999s", &["1"]);
        assert_eq!(r.status, 1);
        let r = run("%99999999999999999999999d", &["1"]);
        assert_eq!(r.status, 1);
    }

    #[test]
    fn a_wide_field_is_made() {
        let r = run("%100000d|%-100000s|", &["7", "x"]);
        assert_eq!(r.out.len(), 100_000 + 1 + 100_000 + 1);
        assert_eq!(r.out[99_999], b'7');
    }

    #[test]
    fn precision_and_width_digits_are_decimal_and_the_precision_may_be_empty() {
        assert_eq!(run("%08d", &["5"]).text(), "00000005");
        assert_eq!(run("%0008d", &["5"]).text(), "00000005");
        assert_eq!(run("%10.4d", &["5"]).text(), "      0005");
        assert_eq!(run("%.s|", &["abc"]).text(), "|");
        assert_eq!(run("%-+5d|", &["3"]).text(), "+3   |");
    }

    #[test]
    fn flags_may_come_in_any_order_and_repeat() {
        assert_eq!(run("%-0+5d|%+-5d|%0-5d|%--5d|", &["3", "3", "3", "3"]).text(), "+3   |+3   |3    |3    |");
        assert_eq!(run("%++d|%  d|", &["3", "3"]).text(), "+3| 3|");
    }

    #[test]
    fn nul_bytes_come_out() {
        assert_eq!(run("a\\0b%b%c", &["\\0", ""]).out, b"a\0b\0");
        assert_eq!(run("%b|", &["x\\0y"]).out, b"x\0y|");
    }

    #[test]
    fn a_long_output_is_written_in_pieces() {
        let args: Vec<String> = (0..20_000).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let r = run("%s\\n", &refs);
        assert_eq!(r.text().lines().count(), 20_000);
        assert_eq!(r.text().lines().last(), Some("19999"));
    }

    #[test]
    fn real_conversions_default_to_six_digits() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        for (f, a, want) in [
            ("%f", "1.5", "1.500000"),
            ("%F", "1.5", "1.500000"),
            ("%e", "1234", "1.234000e+03"),
            ("%E", "1234", "1.234000E+03"),
            ("%g", "1.5", "1.5"),
            ("%g", "100000", "100000"),
            ("%g", "1000000", "1e+06"),
            ("%g", "0.0001", "0.0001"),
            ("%g", "0.00001", "1e-05"),
            ("%G", "0.00001", "1E-05"),
            ("%G", "1234567", "1.23457E+06"),
            ("%f", "0", "0.000000"),
            ("%e", "0", "0.000000e+00"),
            ("%f", "-0.5", "-0.500000"),
        ] {
            let r = run(f, &[a]);
            assert_eq!(r.text(), want, "{f} {a}");
            assert_eq!((r.status, r.diags.len()), (0, 0), "{f} {a}");
        }
    }

    #[test]
    fn real_precision_is_the_digits_after_the_radix_character() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(run("%.5e|%.2E|%.3f|%.1f", &["1234", "1234", "1234", "2.25"]).text(), "1.23400e+03|1.23E+03|1234.000|2.2");
        assert_eq!(run("%.10f|%.0f|%.1f", &["0.5", "3.7", "0.05"]).text(), "0.5000000000|4|0.1");
        assert_eq!(run("%.3g|%.10g|%.1g", &["3.14159", "3.14159", "15"]).text(), "3.14|3.14159|2e+01");
    }

    #[test]
    fn a_zero_precision_leaves_out_the_radix_character() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(run("%.0f|%.0e|%.0E|%.0F", &["3", "3", "3", "3"]).text(), "3|3e+00|3E+00|3");
        assert_eq!(run("%.f|%.e", &["3", "3"]).text(), "3|3e+00");
    }

    #[test]
    fn the_hash_flag_keeps_the_radix_character() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(run("%#.0f|%#.0e|%#.0E|%#.0F", &["3", "3", "3", "3"]).text(), "3.|3.e+00|3.E+00|3.");
        /* and does not take trailing zeros off a %g */
        assert_eq!(run("%#g|%g|%#.3g", &["1", "1", "1"]).text(), "1.00000|1|1.00");
    }

    #[test]
    fn real_width_pads_with_blanks_or_zeros() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(run("%10f|%-10.2f|%10.1e|%10g", &["1.5", "1.5", "1.5", "1.5"]).text(), "  1.500000|1.50      |   1.5e+00|       1.5");
        assert_eq!(run("%010.2f|%010.2f|%08.2f", &["1.5", "-1.5", "-1.5"]).text(), "0000001.50|-000001.50|-0001.50");
        assert_eq!(run("%-010.2f|%0-10.2f|", &["1.5", "1.5"]).text(), "1.50      |1.50      |");
        assert_eq!(run("%3.1f", &["12345.67"]).text(), "12345.7");
    }

    #[test]
    fn real_sign_flags() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(run("%+f|% f|%+ f|% +f|%+.1f|% .1f", &["1.5", "1.5", "1.5", "1.5", "-1.5", "-1.5"]).text(), "+1.500000| 1.500000|+1.500000|+1.500000|-1.5|-1.5");
        assert_eq!(run("%+e|% g|%+g", &["1", "1", "0"]).text(), "+1.000000e+00| 1|+0");
    }

    #[test]
    fn real_arguments_are_read_as_strtod_reads_them() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        for (a, want) in [
            ("1.5", "1.500000"),
            ("-1.5", "-1.500000"),
            ("+2.5", "2.500000"),
            ("1e3", "1000.000000"),
            ("1E3", "1000.000000"),
            ("2.5e-1", "0.250000"),
            (".5", "0.500000"),
            ("5.", "5.000000"),
            ("0x10", "16.000000"),
            ("0x1p4", "16.000000"),
            (" 2.5", "2.500000"),
            ("007", "7.000000"),
            ("inf", "inf"),
            ("-inf", "-inf"),
            ("INFINITY", "inf"),
        ] {
            let r = run("%f", &[a]);
            assert_eq!(r.text(), want, "{a:?}");
            assert_eq!((r.status, r.diags.len()), (0, 0), "{a:?}");
        }
        assert_eq!(run("%F|%f", &["inf", "-inf"]).text(), "INF|-inf");
        assert_eq!(run("%010f|%-6f|", &["inf", "nan"]).text(), "       inf|nan   |");
        assert_eq!(run("%f", &["nan"]).text(), "nan");
    }

    #[test]
    fn a_nan_keeps_its_sign_and_is_capitals_for_a_e_f_g_upper_case() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        for (f, a, want) in [
            ("%f", "nan", "nan"),
            ("%f", "-nan", "-nan"),
            ("%f", "-nAn", "-nan"),
            ("%f", "+nan", "nan"),
            ("%F", "-nan", "-NAN"),
            ("%e", "-nan", "-nan"),
            ("%E", "-nan", "-NAN"),
            ("%g", "-nan", "-nan"),
            ("%G", "nan", "NAN"),
            ("%a", "-nan", "-nan"),
            ("%A", "-nan", "-NAN"),
            ("%6f", "-NAN", "  -nan"),
            ("%-6f", "NAN", "nan   "),
            ("%-06f", "-NAN", "-nan  "),
            ("%06f", "-NAN", "  -nan"),
            ("%06f", "NAN", "   nan"),
            ("%+f", "nan", "+nan"),
            ("% f", "nan", " nan"),
            ("%+f", "-nan", "-nan"),
            ("%10.3g", "-nAn", "      -nan"),
            ("%.0f", "nan", "nan"),
            ("%3f", "nan", "nan"),
            ("%2f", "nan", "nan"),
        ] {
            let r = run(f, &[a]);
            assert_eq!(r.text(), want, "{f} {a}");
            assert_eq!((r.status, r.diags.len()), (0, 0), "{f} {a}");
        }
    }

    #[test]
    fn real_arguments_that_are_wrong_give_the_value_so_far_and_a_diagnostic() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        for (a, want, msg) in [
            ("abc", "0.000000", "expected numeric value"),
            ("", "0.000000", "expected numeric value"),
            ("  ", "0.000000", "expected numeric value"),
            ("'A", "0.000000", "expected numeric value"),
            ("\"1", "0.000000", "expected numeric value"),
            ("1.5x", "1.500000", "not completely converted"),
            ("1.5 ", "1.500000", "not completely converted"),
            ("1,5", "1.000000", "not completely converted"),
            ("1e", "1.000000", "not completely converted"),
            ("--1", "0.000000", "expected numeric value"),
            ("1e999", "inf", "arithmetic overflow"),
            ("-1e999", "-inf", "arithmetic overflow"),
        ] {
            let r = run("%f", &[a]);
            assert_eq!(r.text(), want, "{a:?}");
            assert_eq!(r.status, 1, "{a:?}");
            assert_eq!(r.diags, [format!("{a}: {msg}")], "{a:?}");
        }
    }

    #[test]
    fn a_number_too_small_to_hold_is_not_an_error() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let r = run("%f|%e", &["1e-999", "1e-999"]);
        assert_eq!(r.text(), "0.000000|0.000000e+00");
        assert_eq!((r.status, r.diags.len()), (0, 0));
    }

    #[test]
    fn a_missing_argument_is_zero_for_the_real_conversions() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let r = run("%f|%e|%g|%E|%G|%F|%5.1f|", &[]);
        assert_eq!(r.text(), "0.000000|0.000000e+00|0|0.000000E+00|0|0.000000|  0.0|");
        assert_eq!((r.status, r.diags.len()), (0, 0));
        assert_eq!(run("%f %f\n", &["1", "2", "3"]).text(), "1.000000 2.000000\n3.000000 0.000000\n");
    }

    #[test]
    fn hexadecimal_floating_point() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(run("%a", &["1"]).text().starts_with("0x1p"));
        assert!(run("%A", &["1"]).text().starts_with("0X1P"));
        assert!(run("%a", &["0.5"]).text().contains('p'));
    }

    #[test]
    fn a_long_real_number_is_written_whole() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let r = run("%f", &["1e300"]);
        assert_eq!(r.out.len(), 301 + 7);
        /* Rust writes the exact decimal expansion of a double, as the C library does */
        assert_eq!(r.text(), format!("{:.6}", 1e300_f64));
        assert_eq!(run("%400.2f", &["1"]).out.len(), 400);
        assert_eq!(run("%.300f", &["1"]).out.len(), 302);
    }

    #[test]
    fn integers_and_reals_and_strings_in_one_format() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(run("%s=%d (%.1f)\n", &["a", "1", "1.25", "b", "2", "2.5"]).text(), "a=1 (1.2)\nb=2 (2.5)\n");
    }

    #[test]
    fn the_radix_character_follows_lc_numeric() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let loc = std::ffi::CString::new("de_DE.UTF-8").unwrap();
        if unsafe { libc::setlocale(libc::LC_ALL, loc.as_ptr()) }.is_null() {
            eprintln!("skipped: no locale de_DE.UTF-8");
            return;
        }
        let r = run("%.1f|%e", &["1,5", "2,5"]);
        assert_eq!(r.text(), "1,5|2,500000e+00");
        assert_eq!((r.status, r.diags.len()), (0, 0));
        /* the integer conversions do not use the radix character */
        assert_eq!(run("%d", &["12"]).text(), "12");
        unsafe { libc::setlocale(libc::LC_ALL, c"C".as_ptr()) };
    }

    #[test]
    fn a_write_error_is_reported() {
        struct Bad;
        impl Write for Bad {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from_raw_os_error(libc::ENOSPC))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(printf(b"hello\n", &[], &mut Bad, &mut |_: &str| {}).is_err());
        let big: Vec<Vec<u8>> = (0..100_000).map(|_| b"xxxxxxxxxx".to_vec()).collect();
        assert!(printf(b"%s", &big, &mut Bad, &mut |_: &str| {}).is_err());
    }
}
