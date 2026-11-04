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
 * snprintf(3) of one value into buf: the length the full result has, or a
 * negative number. The format must hold one conversion that takes the type
 * of the value (an integer with the j length modifier, or a double).
 */
pub fn format_int(buf: &mut [u8], fmt: &CStr, v: i64) -> i32 {
    unsafe { libc::snprintf(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), v as libc::intmax_t) }
}

pub fn format_uint(buf: &mut [u8], fmt: &CStr, v: u64) -> i32 {
    unsafe { libc::snprintf(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), v as libc::uintmax_t) }
}

pub fn format_f64(buf: &mut [u8], fmt: &CStr, v: f64) -> i32 {
    unsafe { libc::snprintf(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), v) }
}

/* strtod(3): the value and how many bytes of s it used. errno is left as strtod set it. */
pub fn parse_f64(s: &CStr) -> (f64, usize) {
    let mut end: *mut libc::c_char = std::ptr::null_mut();
    let v = unsafe { strtod(s.as_ptr(), &mut end) };
    (v, end as usize - s.as_ptr() as usize)
}
