/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Characters of the current LC_CTYPE: decoding with mbrtowc(3) and the
 * <blank> class. The libc crate lacks these on macOS, so they are declared
 * here. mbstate_t is opaque to us and must start zeroed; 128 bytes with
 * 8-byte alignment cover macOS (128) and glibc (8).
 */

use std::sync::OnceLock;

#[repr(C, align(8))]
struct MbState([u8; 128]);

extern "C" {
    fn mbrtowc(
        pwc: *mut libc::wchar_t,
        s: *const libc::c_char,
        n: libc::size_t,
        ps: *mut MbState,
    ) -> libc::size_t;
    fn iswblank(wc: libc::c_uint) -> libc::c_int;
    fn iswalnum(wc: libc::c_uint) -> libc::c_int;
    fn iswprint(wc: libc::c_uint) -> libc::c_int;
    fn towupper(wc: libc::c_uint) -> libc::c_uint;
    fn wcrtomb(s: *mut libc::c_char, wc: libc::wchar_t, ps: *mut MbState) -> libc::size_t;
}

/*
 * Do the ASCII bytes stand for themselves in this locale? True for every
 * locale in use; checked once so that the common case can skip mbrtowc.
 */
fn ascii_is_identity() -> bool {
    static IDENT: OnceLock<bool> = OnceLock::new();
    *IDENT.get_or_init(|| {
        (1..0x80u8).all(|b| unsafe {
            let mut st = MbState([0; 128]);
            let mut wc: libc::wchar_t = 0;
            mbrtowc(&mut wc, &b as *const u8 as *const libc::c_char, 1, &mut st) == 1 && wc == b as libc::wchar_t
        })
    })
}

/*
 * The first character of s (not empty): its length in bytes and its wide
 * value. A byte that starts no valid character, or a sequence cut short by
 * the end of s, is a character of one byte with no value.
 */
pub fn char_at(s: &[u8]) -> (usize, Option<u32>) {
    if s[0] < 0x80 && ascii_is_identity() {
        return (1, Some(s[0] as u32));
    }
    let mut st = MbState([0; 128]);
    let mut wc: libc::wchar_t = 0;
    let r = unsafe { mbrtowc(&mut wc, s.as_ptr() as *const libc::c_char, s.len(), &mut st) };
    if r == usize::MAX || r == usize::MAX - 1 {
        (1, None)
    } else if r == 0 {
        (1, Some(0))
    } else {
        (r, Some(wc as u32))
    }
}

/*
 * Like char_at for data that goes on after s (a stream read in pieces):
 * None if s ends in the middle of a character, so that the caller reads
 * more before deciding. A byte that starts no valid character is still a
 * character of one byte with no value.
 */
pub fn char_at_stream(s: &[u8]) -> Option<(usize, Option<u32>)> {
    if s[0] < 0x80 && ascii_is_identity() {
        return Some((1, Some(s[0] as u32)));
    }
    let mut st = MbState([0; 128]);
    let mut wc: libc::wchar_t = 0;
    let r = unsafe { mbrtowc(&mut wc, s.as_ptr() as *const libc::c_char, s.len(), &mut st) };
    if r == usize::MAX - 1 {
        None
    } else if r == usize::MAX {
        Some((1, None))
    } else if r == 0 {
        Some((1, Some(0)))
    } else {
        Some((r, Some(wc as u32)))
    }
}

/* Is the wide character a <blank> (iswblank(3))? */
pub fn is_blank(wc: u32) -> bool {
    unsafe { iswblank(wc) != 0 }
}

/* Is the wide character alphanumeric (iswalnum(3))? */
pub fn is_alnum(wc: u32) -> bool {
    unsafe { iswalnum(wc) != 0 }
}

/* Is the wide character printable (iswprint(3))? */
pub fn is_print(wc: u32) -> bool {
    unsafe { iswprint(wc) != 0 }
}

/* The uppercase equivalent of a wide character (towupper(3)). */
pub fn to_upper(wc: u32) -> u32 {
    unsafe { towupper(wc) }
}

/* Append the bytes of the wide character to out; false if it has none here. */
pub fn encode(wc: u32, out: &mut Vec<u8>) -> bool {
    let mut st = MbState([0; 128]);
    /* MB_LEN_MAX is 16 on macOS and 16 on glibc */
    let mut buf = [0 as libc::c_char; 32];
    let n = unsafe { wcrtomb(buf.as_mut_ptr(), wc as libc::wchar_t, &mut st) };
    if n == usize::MAX {
        return false;
    }
    out.extend(buf[..n].iter().map(|&c| c as u8));
    true
}

/* Does LC_COLLATE ask for plain byte order (the C and POSIX locales)? */
pub fn collation_is_c() -> bool {
    unsafe {
        let p = libc::setlocale(libc::LC_COLLATE, std::ptr::null());
        if p.is_null() {
            return true;
        }
        let name = std::ffi::CStr::from_ptr(p).to_bytes();
        name == b"C" || name == b"POSIX"
    }
}

/*
 * Order two strings by the collating sequence of the current locale. With
 * c_locale (see collation_is_c) that is a byte comparison. Otherwise strcoll
 * does it, on copies in the scratch vectors: strcoll wants a NUL at the end,
 * and a NUL inside a string ends its comparison, so the strings are taken
 * piece by piece between NULs.
 */
pub fn collate(a: &[u8], b: &[u8], c_locale: bool, sa: &mut Vec<u8>, sb: &mut Vec<u8>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if c_locale {
        return a.cmp(b);
    }
    let mut a = a.split(|&c| c == 0);
    let mut b = b.split(|&c| c == 0);
    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) => {
                sa.clear();
                sa.extend_from_slice(x);
                sa.push(0);
                sb.clear();
                sb.extend_from_slice(y);
                sb.push(0);
                let r = unsafe { libc::strcoll(sa.as_ptr() as *const libc::c_char, sb.as_ptr() as *const libc::c_char) };
                if r != 0 {
                    return r.cmp(&0);
                }
            }
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (None, None) => return Ordering::Equal,
        }
    }
}

/* The first character of s (not empty): its length and whether it is a <blank>. */
pub fn blank_at(s: &[u8]) -> (usize, bool) {
    let (n, wc) = char_at(s);
    (n, wc.is_some_and(is_blank))
}

/* Offset in s after skipping up to n characters. */
pub fn skip_chars(s: &[u8], n: usize) -> usize {
    let mut i = 0;
    let mut k = 0;
    while k < n && i < s.len() {
        i += char_at(&s[i..]).0;
        k += 1;
    }
    i
}

/* The number of characters in s. */
pub fn count_chars(s: &[u8]) -> usize {
    let mut i = 0;
    let mut n = 0;
    while i < s.len() {
        i += char_at(&s[i..]).0;
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_invalid_bytes_in_the_c_locale() {
        assert_eq!(char_at(b"a"), (1, Some(97)));
        assert_eq!(char_at(b"\0x"), (1, Some(0)));
        assert!(blank_at(b" ").1 && blank_at(b"\t").1);
        assert!(!blank_at(b"\n").1 && !blank_at(b"a").1);
    }

    #[test]
    fn count_chars_in_the_c_locale_is_the_byte_count() {
        assert_eq!(count_chars(b""), 0);
        assert_eq!(count_chars(b"abc"), 3);
        assert_eq!(count_chars(b"\xff\xfe"), 2);
    }

    #[test]
    fn skip_counts_characters() {
        assert_eq!(skip_chars(b"abc", 0), 0);
        assert_eq!(skip_chars(b"abc", 2), 2);
        assert_eq!(skip_chars(b"abc", 9), 3);
        assert_eq!(skip_chars(b"", 3), 0);
    }
}
