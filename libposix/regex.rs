/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * POSIX regular expressions (regcomp(3), regexec(3)) of the C library.
 */

use std::ffi::CStr;

/*
 * The compiled expression lives on the heap and never moves: POSIX does not
 * say that a regex_t may be copied to another address after regcomp.
 */
pub struct Regex(Box<libc::regex_t>);

impl Regex {
    /* Compile a pattern; flags as for regcomp (0 is a basic expression). None if it is invalid. */
    pub fn new(pattern: &CStr, flags: libc::c_int) -> Option<Regex> {
        let mut re: Box<libc::regex_t> = Box::new(crate::sys::zeroed());
        if unsafe { libc::regcomp(&mut *re, pattern.as_ptr(), flags) } != 0 {
            return None;
        }
        Some(Regex(re))
    }

    /*
     * Match against s. None if there is no match; otherwise the byte range of
     * the whole match and of the first nmatch - 1 subexpressions (None for one
     * that did not take part).
     */
    pub fn exec(&self, s: &CStr, nmatch: usize) -> Option<Vec<Option<(usize, usize)>>> {
        let mut m: Vec<libc::regmatch_t> = vec![unsafe { std::mem::zeroed() }; nmatch.max(1)];
        let r = unsafe { libc::regexec(&*self.0, s.as_ptr(), m.len(), m.as_mut_ptr(), 0) };
        if r != 0 {
            return None;
        }
        Some(
            m.iter()
                .map(|x| if x.rm_so < 0 { None } else { Some((x.rm_so as usize, x.rm_eo as usize)) })
                .collect(),
        )
    }
}

impl Regex {
    /*
     * Look for the first match in the text of s from byte offset start on (s must end in a NUL, which
     * ends the text). The range is counted from start; a match is not taken to be at the start of a line.
     */
    pub fn find_at(&self, s: &[u8], start: usize) -> Option<(usize, usize)> {
        if s.last() != Some(&0) || start >= s.len() {
            return None;
        }
        let mut m: [libc::regmatch_t; 1] = [unsafe { std::mem::zeroed() }];
        let flags = if start > 0 { libc::REG_NOTBOL } else { 0 };
        let text = unsafe { s.as_ptr().add(start) } as *const libc::c_char;
        if unsafe { libc::regexec(&*self.0, text, 1, m.as_mut_ptr(), flags) } != 0 || m[0].rm_so < 0 {
            return None;
        }
        Some((m[0].rm_so as usize, m[0].rm_eo as usize))
    }
}

impl Drop for Regex {
    fn drop(&mut self) {
        unsafe { libc::regfree(&mut *self.0) };
    }
}
