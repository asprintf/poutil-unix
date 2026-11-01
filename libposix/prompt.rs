/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Asking the user a yes/no question on standard error and standard input,
 * as rm and cp do: the answer is affirmative if it matches the locale's
 * yesexpr (LC_MESSAGES).
 */

use std::ffi::CStr;
use std::io::Write;

/* Compiled form of the locale's yesexpr. */
pub struct Yes {
    re: libc::regex_t,
}

impl Yes {
    pub fn new() -> Yes {
        let fallback = c"^[yY]";
        let got = unsafe { libc::nl_langinfo(libc::YESEXPR) };
        let mut pats: Vec<&CStr> = Vec::new();
        if !got.is_null() {
            let s = unsafe { CStr::from_ptr(got) };
            if !s.to_bytes().is_empty() {
                pats.push(s);
            }
        }
        pats.push(fallback);
        for p in pats {
            let mut re: libc::regex_t = unsafe { std::mem::zeroed() };
            if unsafe { libc::regcomp(&mut re, p.as_ptr(), libc::REG_EXTENDED | libc::REG_NOSUB) } == 0 {
                return Yes { re };
            }
        }
        /* the fallback is a valid expression; this is not reached */
        std::process::exit(1)
    }

    pub fn matches(&self, line: &[u8]) -> bool {
        /* a C string stops at a NUL; the answer is what comes before it */
        let end = line.iter().position(|&c| c == 0).unwrap_or(line.len());
        let c = crate::cstr(&line[..end]);
        unsafe { libc::regexec(&self.re, c.as_ptr(), 0, std::ptr::null_mut(), 0) == 0 }
    }
}

impl Default for Yes {
    fn default() -> Self {
        Yes::new()
    }
}

/*
 * Read one line from standard input, a byte at a time so that nothing past the
 * newline is consumed (the descriptor may be shared with other processes).
 * End of file or an error ends the line; the newline is not returned.
 */
pub fn read_line() -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let mut b = [0u8; 1];
        let n = unsafe { libc::read(0, b.as_mut_ptr().cast(), 1) };
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if n <= 0 || b[0] == b'\n' {
            return line;
        }
        line.push(b[0]);
    }
}

/* Write the prompt to standard error, read the answer from standard input. */
pub fn ask(yes: &Yes, prompt: &[u8]) -> bool {
    {
        let mut e = std::io::stderr().lock();
        let _ = e.write_all(prompt);
        let _ = e.flush();
    }
    yes.matches(&read_line())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yes_default_pattern() {
        let y = Yes::new();
        assert!(y.matches(b"y"));
        assert!(y.matches(b"Y"));
        assert!(y.matches(b"yes"));
        assert!(!y.matches(b"n"));
        assert!(!y.matches(b""));
        assert!(!y.matches(b" y"));
        assert!(y.matches(b"y\0n"));
        assert!(!y.matches(b"\0y"));
    }
}
