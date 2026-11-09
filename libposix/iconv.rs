/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Codeset conversion with iconv(3). The libc crate only declares it on
 * macOS and calls it deprecated, so it is declared here; on macOS it lives
 * in libiconv.
 */

use std::ffi::{c_char, c_int, c_void, CStr};

#[cfg_attr(target_vendor = "apple", link(name = "iconv"))]
extern "C" {
    fn iconv_open(tocode: *const c_char, fromcode: *const c_char) -> *mut c_void;
    fn iconv(
        cd: *mut c_void,
        inbuf: *mut *mut c_char,
        inleft: *mut libc::size_t,
        outbuf: *mut *mut c_char,
        outleft: *mut libc::size_t,
    ) -> libc::size_t;
    fn iconv_close(cd: *mut c_void) -> c_int;
}

/* Why Converter::convert stopped. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /* all of the input was converted */
    Done,
    /* the next bytes are no character of the input codeset, or have none in the output codeset */
    Invalid,
    /* the input ends in the middle of a character */
    Incomplete,
}

/* A conversion from one codeset to another, with its shift state. */
pub struct Converter {
    cd: *mut c_void,
}

impl Converter {
    /*
     * None if the system knows no such conversion. A name ending in //IGNORE
     * drops what cannot be converted; on macOS that holds for every other
     * conversion of the process from then on, so do not keep both kinds.
     */
    pub fn open(to: &[u8], from: &[u8]) -> Option<Converter> {
        let (t, f) = (crate::cstr(to), crate::cstr(from));
        let cd = unsafe { iconv_open(t.as_ptr(), f.as_ptr()) };
        if cd as usize == usize::MAX {
            None
        } else {
            Some(Converter { cd })
        }
    }

    /*
     * Convert input and append the result to out. Returns how many bytes of
     * the input were used and why it stopped there.
     */
    pub fn convert(&mut self, input: &[u8], out: &mut Vec<u8>) -> (usize, Stop) {
        let mut inptr = input.as_ptr() as *mut c_char;
        let mut inleft: libc::size_t = input.len();
        let mut buf = [0u8; 16384];
        loop {
            let mut outptr = buf.as_mut_ptr() as *mut c_char;
            let mut outleft: libc::size_t = buf.len();
            let r = unsafe { iconv(self.cd, &mut inptr, &mut inleft, &mut outptr, &mut outleft) };
            let err = if r == usize::MAX { std::io::Error::last_os_error().raw_os_error() } else { None };
            out.extend_from_slice(&buf[..buf.len() - outleft]);
            let used = input.len() - inleft;
            match err {
                None => return (used, Stop::Done),
                Some(libc::E2BIG) => continue,
                Some(libc::EINVAL) => return (used, Stop::Incomplete),
                Some(_) => return (used, Stop::Invalid),
            }
        }
    }

    /* Append what returns the output to its initial shift state (nothing for most codesets). */
    pub fn finish(&mut self, out: &mut Vec<u8>) -> bool {
        let mut buf = [0u8; 64];
        let mut outptr = buf.as_mut_ptr() as *mut c_char;
        let mut outleft: libc::size_t = buf.len();
        let r = unsafe { iconv(self.cd, std::ptr::null_mut(), std::ptr::null_mut(), &mut outptr, &mut outleft) };
        out.extend_from_slice(&buf[..buf.len() - outleft]);
        r != usize::MAX
    }

    /* Forget the shift state of both sides. */
    pub fn reset(&mut self) {
        unsafe {
            iconv(self.cd, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
        };
    }
}

impl Drop for Converter {
    fn drop(&mut self) {
        unsafe { iconv_close(self.cd) };
    }
}

/* Is there a conversion between the codeset and UTF-8 both ways? */
pub fn is_supported(name: &CStr) -> bool {
    Converter::open(name.to_bytes(), b"UTF-8").is_some() && Converter::open(b"UTF-8", name.to_bytes()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(to: &str, from: &str, input: &[u8]) -> (Vec<u8>, usize, Stop) {
        let mut c = Converter::open(to.as_bytes(), from.as_bytes()).unwrap();
        let mut out = Vec::new();
        let (n, s) = c.convert(input, &mut out);
        (out, n, s)
    }

    #[test]
    fn latin1_and_utf8() {
        assert_eq!(conv("UTF-8", "ISO8859-1", b"caf\xe9"), ("caf\u{e9}".as_bytes().to_vec(), 4, Stop::Done));
        assert_eq!(conv("ISO8859-1", "UTF-8", "caf\u{e9}".as_bytes()), (b"caf\xe9".to_vec(), 5, Stop::Done));
    }

    #[test]
    fn invalid_incomplete_and_unmappable() {
        assert_eq!(conv("UTF-16BE", "UTF-8", b"a\xffb"), (b"\0a".to_vec(), 1, Stop::Invalid));
        assert_eq!(conv("UTF-16BE", "UTF-8", b"ab\xc3"), (b"\0a\0b".to_vec(), 2, Stop::Incomplete));
        /* an e-acute has no ASCII form */
        assert_eq!(conv("ASCII", "UTF-8", "a\u{e9}b".as_bytes()), (b"a".to_vec(), 1, Stop::Invalid));
    }

    #[test]
    fn output_larger_than_the_buffer() {
        let input = vec![b'x'; 100_000];
        let (out, n, s) = conv("UTF-16LE", "UTF-8", &input);
        assert_eq!((out.len(), n, s), (200_000, 100_000, Stop::Done));
    }

    #[test]
    fn unknown_codesets() {
        assert!(Converter::open(b"UTF-8", b"NO-SUCH-CODESET").is_none());
        assert!(Converter::open(b"NO-SUCH-CODESET", b"UTF-8").is_none());
    }

    #[test]
    fn shift_state_is_reset_by_finish() {
        let mut c = Converter::open(b"ISO-2022-JP", b"UTF-8").unwrap();
        let mut out = Vec::new();
        assert_eq!(c.convert("\u{3042}".as_bytes(), &mut out), (3, Stop::Done));
        assert!(c.finish(&mut out));
        /* ESC $ B, the character, ESC ( B */
        assert_eq!(out, b"\x1b$B$\"\x1b(B");
    }

    #[test]
    fn is_supported_names() {
        assert!(is_supported(c"UTF-8"));
        assert!(is_supported(c"ISO8859-1"));
        assert!(!is_supported(c"NO-SUCH-CODESET"));
    }
}
