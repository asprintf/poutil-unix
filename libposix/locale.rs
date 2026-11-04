/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * The locale: choosing it, and the few facts about it that the utilities ask
 * for. Character handling is in mb.
 *
 * Single thread only: setlocale changes the locale of the whole process,
 * and every locale-dependent call (mbrtowc, strcoll, localeconv, printf)
 * may run at the same time on another thread; localeconv and nl_langinfo
 * hand back storage the next call overwrites. The utilities have one
 * thread; unit tests that set a locale must hold a lock.
 */

use std::ffi::{CStr, CString};

/* Set the locale of all categories to the named one; false if there is no such locale. */
pub fn set(name: &str) -> bool {
    match CString::new(name) {
        Ok(c) => !unsafe { libc::setlocale(libc::LC_ALL, c.as_ptr()) }.is_null(),
        Err(_) => false,
    }
}

/* Back to the C locale. */
pub fn set_c() {
    unsafe { libc::setlocale(libc::LC_ALL, c"C".as_ptr()) };
}

/*
 * For a unit test that needs a locale the machine may not have: set it, or
 * say so on standard error and return false, and the test returns.
 */
pub fn try_set(name: &str) -> bool {
    if set(name) {
        true
    } else {
        eprintln!("skipped: no locale {name}");
        false
    }
}

/* The radix character and the thousands separator of LC_NUMERIC. */
pub fn numeric() -> (Vec<u8>, Vec<u8>) {
    unsafe {
        let lc = libc::localeconv();
        if lc.is_null() {
            return (Vec::new(), Vec::new());
        }
        let get = |p: *const libc::c_char| if p.is_null() { Vec::new() } else { CStr::from_ptr(p).to_bytes().to_vec() };
        (get((*lc).decimal_point), get((*lc).thousands_sep))
    }
}

/* Is the character encoding of the locale UTF-8? */
pub fn codeset_is_utf8() -> bool {
    unsafe {
        let cs = libc::nl_langinfo(libc::CODESET);
        !cs.is_null() && CStr::from_ptr(cs).to_bytes() == b"UTF-8"
    }
}
