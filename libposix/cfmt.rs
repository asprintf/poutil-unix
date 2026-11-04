/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * The C library's number formatting and parsing, for the places where the
 * locale or the exact printf rounding matters.
 */

use std::ffi::CStr;

extern "C" {
    /* the libc crate does not have it on every target */
    fn strtod(s: *const libc::c_char, end: *mut *mut libc::c_char) -> f64;
}

/*
 * Is fmt one conversion and nothing else: %, flags, a width, a precision, the
 * length modifier len and one of the conversion characters convs? snprintf
 * reads its arguments by what the format says, so anything else (a second
 * conversion, %s, %n, a * that takes an int) would read what was never
 * passed. These functions are safe because they refuse such a format.
 */
fn one_conversion(fmt: &[u8], len: &[u8], convs: &[u8]) -> bool {
    let Some(rest) = fmt.strip_prefix(b"%") else {
        return false;
    };
    let mut i = 0;
    while i < rest.len() && b"-+ #0".contains(&rest[i]) {
        i += 1;
    }
    while i < rest.len() && rest[i].is_ascii_digit() {
        i += 1;
    }
    if i < rest.len() && rest[i] == b'.' {
        i += 1;
        while i < rest.len() && rest[i].is_ascii_digit() {
            i += 1;
        }
    }
    let Some(tail) = rest[i..].strip_prefix(len) else {
        return false;
    };
    tail.len() == 1 && convs.contains(&tail[0])
}

const INT_CONVS: &[u8] = b"diouxX";
const REAL_CONVS: &[u8] = b"aAeEfFgG";

/*
 * snprintf(3) of one value into buf: the length the full result has, or a
 * negative number. The format is one conversion only: an integer one with
 * the j length modifier ("%-08.3jd"), or for a double one with none
 * ("%.2f"). Any other format writes nothing and returns -1.
 */
pub fn format_int(buf: &mut [u8], fmt: &CStr, v: i64) -> i32 {
    if !one_conversion(fmt.to_bytes(), b"j", INT_CONVS) {
        return -1;
    }
    unsafe { libc::snprintf(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), v as libc::intmax_t) }
}

pub fn format_uint(buf: &mut [u8], fmt: &CStr, v: u64) -> i32 {
    if !one_conversion(fmt.to_bytes(), b"j", INT_CONVS) {
        return -1;
    }
    unsafe { libc::snprintf(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), v as libc::uintmax_t) }
}

pub fn format_f64(buf: &mut [u8], fmt: &CStr, v: f64) -> i32 {
    if !one_conversion(fmt.to_bytes(), b"", REAL_CONVS) {
        return -1;
    }
    unsafe { libc::snprintf(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), v) }
}

/* strtod(3): the value and how many bytes of s it used. errno is left as strtod set it. */
pub fn parse_f64(s: &CStr) -> (f64, usize) {
    let mut end: *mut libc::c_char = std::ptr::null_mut();
    let v = unsafe { strtod(s.as_ptr(), &mut end) };
    (v, end as usize - s.as_ptr() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(fmt: &CStr, v: i64) -> Option<String> {
        let mut buf = [0u8; 64];
        let n = format_int(&mut buf, fmt, v);
        (n >= 0).then(|| String::from_utf8(buf[..n as usize].to_vec()).unwrap())
    }

    fn real(fmt: &CStr, v: f64) -> Option<String> {
        let mut buf = [0u8; 64];
        let n = format_f64(&mut buf, fmt, v);
        (n >= 0).then(|| String::from_utf8(buf[..n as usize].to_vec()).unwrap())
    }

    #[test]
    fn one_conversion_with_flags_width_and_precision_is_formatted() {
        assert_eq!(int(c"%jd", -42).as_deref(), Some("-42"));
        assert_eq!(int(c"%-+ #08.3jx", 255).as_deref(), Some("0x0ff   "));
        assert_eq!(int(c"%.jd", 0).as_deref(), Some(""));
        assert_eq!(real(c"%.2f", 1.005).as_deref(), Some("1.00"));
        assert_eq!(real(c"%#10.3e", 12.5).as_deref(), Some(" 1.250e+01"));
        let mut buf = [0u8; 64];
        assert_eq!(format_uint(&mut buf, c"%ju", u64::MAX), 20);
    }

    #[test]
    fn any_other_format_is_refused() {
        for f in [
            c"", c"%", c"%%", c"jd", c"x%jd", c"%jdx", c"%jd%jd", c"%d", c"%ld", c"%js", c"%jn", c"%*jd",
            c"%.*jd", c"%1$jd", c"%'jd", c"%jf",
        ] {
            assert_eq!(int(f, 1), None, "{f:?}");
        }
        for f in [c"%f%f", c"%s", c"%n", c"%jf", c"%lf", c"%Lf", c"%*f", c"%.2d", c"%.2f "] {
            assert_eq!(real(f, 1.0), None, "{f:?}");
        }
    }
}
