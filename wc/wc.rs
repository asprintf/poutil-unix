/* SPDX-License-Identifier: GPL-2.0-only */
/* wc - word, line, and byte or character count. */

use posix::getopt::{Getopt, Opt};
use std::fs::File;
use std::io::{self, Seek, SeekFrom};

const SYNOPSIS: &str = "wc [-c|-m] [-lw] [file...]";

/*
 * The libc crate has no multibyte conversion on every platform, so declare
 * what is needed. mbstate_t is opaque to us and must start zeroed; 128
 * bytes with 8-byte alignment cover macOS (128) and glibc (8).
 */
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct MbState([u8; 128]);

extern "C" {
    fn mbrtowc(
        pwc: *mut libc::wchar_t,
        s: *const libc::c_char,
        n: libc::size_t,
        ps: *mut MbState,
    ) -> libc::size_t;
    fn iswspace(wc: libc::c_uint) -> libc::c_int;
}

const FRESH: MbState = MbState([0; 128]);

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Counts {
    lines: u64,
    words: u64,
    bytes: u64,
    chars: u64,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        self.lines += o.lines;
        self.words += o.words;
        self.bytes += o.bytes;
        self.chars += o.chars;
    }
}

/* The fields to print, in the fixed order newlines, words, bytes or characters. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sel {
    lines: bool,
    words: bool,
    bytes: bool,
    chars: bool,
}

/*
 * Counts one input, block after block. Bytes and newlines need no decoding;
 * words and characters are decoded in the current LC_CTYPE with mbrtowc(3)
 * and iswspace(3), so a multibyte character that straddles two blocks is
 * still one character.
 *
 * A byte that is not part of a valid character counts as a character of its
 * own and as part of a word (it is not white space); the spec leaves this
 * undefined.
 */
struct Counter {
    decode: bool,
    /* every byte is a character of its own (C locale, ISO 8859-x, ...) */
    single: bool,
    /* bytes below 0x80 are characters of their own (UTF-8) */
    ascii: bool,
    /* iswspace() of the character each single byte is, as the slow path would */
    space: [bool; 256],
    state: MbState,
    /* bytes of a multibyte character not completed yet */
    pending: usize,
    in_word: bool,
    c: Counts,
}

/*
 * What every byte is on its own in the current locale: whether the locale is
 * single-byte, and which bytes are white space.
 *
 * A byte alone is a whole character (1), a NUL (0), invalid (-1) or the start
 * of a longer character (-2). A multibyte locale has some byte of the last
 * kind, since its first byte alone is incomplete; a locale with none is
 * single-byte, even when it refuses the bytes above 0x7f (US-ASCII), which
 * then count as characters of their own that are not white space, just as
 * the multibyte path counts an invalid byte.
 */
fn classify_bytes() -> (bool, [bool; 256]) {
    let mut single = true;
    let mut space = [false; 256];
    for b in 0..=255u8 {
        let mut st = FRESH;
        let mut wc: libc::wchar_t = 0;
        let r = unsafe { mbrtowc(&mut wc, &b as *const u8 as *const libc::c_char, 1, &mut st) };
        if r == 1 {
            space[b as usize] = unsafe { iswspace(wc as libc::c_uint) } != 0;
        } else if r != 0 && r != usize::MAX {
            single = false;
        }
    }
    (single, space)
}

fn utf8_locale() -> bool {
    unsafe {
        let cs = libc::nl_langinfo(libc::CODESET);
        !cs.is_null() && std::ffi::CStr::from_ptr(cs).to_bytes() == b"UTF-8"
    }
}

impl Counter {
    /* decode: are words or characters wanted at all? */
    fn new(decode: bool) -> Counter {
        let (single, space) = if decode { classify_bytes() } else { (false, [false; 256]) };
        Counter {
            decode,
            single,
            ascii: decode && utf8_locale(),
            space,
            state: FRESH,
            pending: 0,
            in_word: false,
            c: Counts::default(),
        }
    }

    fn step(&mut self, is_space: bool) {
        if is_space {
            self.in_word = false;
        } else if !self.in_word {
            self.in_word = true;
            self.c.words += 1;
        }
    }

    fn feed(&mut self, buf: &[u8]) {
        self.c.bytes += buf.len() as u64;
        self.c.lines += posix::count_byte(b'\n', buf);
        if !self.decode {
            return;
        }
        if self.single {
            self.c.chars += buf.len() as u64;
            for &b in buf {
                self.step(self.space[b as usize]);
            }
            return;
        }
        let mut i = 0;
        while i < buf.len() {
            let b = buf[i];
            if self.ascii && self.pending == 0 && b < 0x80 {
                self.c.chars += 1;
                self.step(self.space[b as usize]);
                i += 1;
                continue;
            }
            let mut wc: libc::wchar_t = 0;
            let r = unsafe {
                mbrtowc(
                    &mut wc,
                    buf[i..].as_ptr() as *const libc::c_char,
                    buf.len() - i,
                    &mut self.state,
                )
            };
            if r == usize::MAX {
                /*
                 * Invalid. The bytes of the unfinished character are
                 * characters of their own; start over at this byte, which
                 * is then the first of a new one.
                 */
                self.state = FRESH;
                if self.pending > 0 {
                    self.c.chars += self.pending as u64;
                    self.pending = 0;
                    self.step(false);
                } else {
                    self.c.chars += 1;
                    self.step(false);
                    i += 1;
                }
            } else if r == usize::MAX - 1 {
                /* incomplete: the rest of this block belongs to it */
                self.pending += buf.len() - i;
                i = buf.len();
            } else if r == 0 {
                /* a NUL character */
                self.pending = 0;
                self.c.chars += 1;
                self.step(false);
                i += 1;
            } else {
                self.pending = 0;
                self.c.chars += 1;
                self.step(unsafe { iswspace(wc as libc::c_uint) } != 0);
                i += r;
            }
        }
    }

    fn finish(mut self) -> Counts {
        if self.pending > 0 {
            /* the input ended inside a character */
            self.c.chars += self.pending as u64;
            self.step(false);
        }
        self.c
    }
}

/* "%d %d %d %s\n" with only the selected fields; name is None for standard input. */
fn format_line(sel: Sel, c: &Counts, name: Option<&[u8]>) -> Vec<u8> {
    let mut f: Vec<u64> = Vec::new();
    if sel.lines {
        f.push(c.lines);
    }
    if sel.words {
        f.push(c.words);
    }
    if sel.bytes {
        f.push(c.bytes);
    }
    if sel.chars {
        f.push(c.chars);
    }
    let strs: Vec<String> = f.iter().map(|n| n.to_string()).collect();
    let mut out = strs.join(" ").into_bytes();
    if let Some(n) = name {
        out.push(b' ');
        out.extend_from_slice(n);
    }
    out.push(b'\n');
    out
}

fn count<R: io::Read>(r: &mut R, decode: bool) -> io::Result<Counts> {
    let mut ctr = Counter::new(decode);
    let mut buf = vec![0u8; 65536];
    loop {
        let n = posix::read_block(r, &mut buf)?;
        if n == 0 {
            return Ok(ctr.finish());
        }
        ctr.feed(&buf[..n]);
    }
}

/*
 * The bytes from the current offset to the end of a regular file, found
 * without reading it, and the offset moved to the end as reading would. None
 * if h is not a regular file (a pipe, a terminal, ...).
 */
fn remaining_bytes(h: &mut File) -> Option<u64> {
    if !h.metadata().ok()?.is_file() {
        return None;
    }
    let pos = h.stream_position().ok()?;
    let end = h.seek(SeekFrom::End(0)).ok()?;
    if end >= pos {
        return Some(end - pos);
    }
    h.seek(SeekFrom::Start(pos)).ok()?;
    None
}

/* Count an input file; "wc -c" alone on a regular file does not read it. */
fn count_file(h: &mut File, sel: Sel) -> io::Result<Counts> {
    if sel.bytes && !sel.lines && !sel.words && !sel.chars {
        if let Some(n) = remaining_bytes(h) {
            return Ok(Counts { bytes: n, ..Counts::default() });
        }
    }
    count(h, sel.words || sel.chars)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut sel = Sel { lines: false, words: false, bytes: false, chars: false };
    let mut g = Getopt::new(&args, b"cmlw");
    let mut any = false;
    for o in g.by_ref() {
        any = true;
        match o {
            /* -c and -m exclude each other; the last one wins */
            Opt::Flag(b'c') => {
                sel.bytes = true;
                sel.chars = false;
            }
            Opt::Flag(b'm') => {
                sel.chars = true;
                sel.bytes = false;
            }
            Opt::Flag(b'l') => sel.lines = true,
            Opt::Flag(b'w') => sel.words = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    if !any {
        sel = Sel { lines: true, words: true, bytes: true, chars: false };
    }
    let files = g.operands();
    let mut total = Counts::default();
    let mut status = 0;
    if files.is_empty() {
        let mut stdin = posix::borrow_fd(0);
        match count_file(&mut stdin, sel) {
            Ok(c) => posix::write_stdout(&prog, &format_line(sel, &c, None)),
            Err(e) => {
                posix::warn(&prog, &format!("standard input: {}", posix::strerror(&e)));
                status = 1;
            }
        }
        std::process::exit(status);
    }
    for f in files {
        let res = posix::open_input(f).and_then(|mut h| count_file(&mut h, sel));
        match res {
            Ok(c) => {
                posix::write_stdout(&prog, &format_line(sel, &c, Some(f)));
                total.add(&c);
            }
            Err(e) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                status = 1;
            }
        }
    }
    if files.len() > 1 {
        posix::write_stdout(&prog, &format_line(sel, &total, Some(b"total")));
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /* the locale is process-wide: every test that counts holds this */
    static LOCALE: Mutex<()> = Mutex::new(());

    /*
     * Count the chunks in locale loc; None if this machine lacks the locale
     * (the test then proves nothing and says so).
     */
    fn run(loc: &str, chunks: &[&[u8]]) -> Option<Counts> {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let l = std::ffi::CString::new(loc).unwrap();
        if unsafe { libc::setlocale(libc::LC_ALL, l.as_ptr()) }.is_null() {
            eprintln!("skipped: no locale {loc}");
            return None;
        }
        let mut c = Counter::new(true);
        for ch in chunks {
            c.feed(ch);
        }
        Some(c.finish())
    }

    fn c(loc: &str, data: &[u8]) -> Counts {
        run(loc, &[data]).unwrap_or_default()
    }

    const U: &str = "en_US.UTF-8";

    #[test]
    fn counts_lines_words_bytes() {
        let k = c("C", b"hello world\nfoo\n");
        assert_eq!((k.lines, k.words, k.bytes, k.chars), (2, 3, 16, 16));
        let k = c("C", b"");
        assert_eq!(k, Counts::default());
    }

    #[test]
    fn last_line_without_newline_has_words_but_no_line() {
        let k = c("C", b"a b");
        assert_eq!((k.lines, k.words, k.bytes), (0, 2, 3));
    }

    /* classify_bytes() in locale loc; None if this machine lacks it */
    fn classify_in(loc: &str) -> Option<(bool, [bool; 256])> {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let l = std::ffi::CString::new(loc).unwrap();
        if unsafe { libc::setlocale(libc::LC_ALL, l.as_ptr()) }.is_null() {
            eprintln!("skipped: no locale {loc}");
            return None;
        }
        Some(classify_bytes())
    }

    #[test]
    fn single_byte_locales_are_told_from_multibyte_ones() {
        /* US-ASCII refuses the bytes above 0x7f but is still one byte per character */
        for loc in ["C", "POSIX", "en_US.ISO8859-1", "ru_RU.KOI8-R", "en_US.US-ASCII"] {
            if let Some((single, _)) = classify_in(loc) {
                assert!(single, "{loc}");
            }
        }
        for loc in [U, "ja_JP.eucJP", "zh_TW.Big5", "ja_JP.SJIS", "ko_KR.CP949"] {
            if let Some((single, _)) = classify_in(loc) {
                assert!(!single, "{loc}");
            }
        }
    }

    #[test]
    fn white_space_bytes_in_the_c_locale() {
        let (_, space) = classify_in("C").unwrap();
        for (b, &is_space) in space.iter().enumerate() {
            assert_eq!(is_space, b == 0x20 || (0x09..=0x0d).contains(&b), "{b:#x}");
        }
    }

    #[test]
    fn every_isspace_byte_separates_words() {
        let k = c("C", b"a b\tc\nd\x0be\x0cf\rg");
        assert_eq!(k.words, 7);
        let k = c("C", b" \t\n\x0b\x0c\r");
        assert_eq!((k.words, k.lines), (0, 1));
        let k = c("C", b"\n\n\n");
        assert_eq!((k.words, k.lines), (0, 3));
    }

    #[test]
    fn nul_and_high_bytes_are_word_characters() {
        let k = c("C", b"a\0b \xff\x80 \0");
        assert_eq!((k.words, k.bytes, k.chars), (3, 8, 8));
    }

    #[test]
    fn a_word_continues_across_blocks() {
        for loc in ["C", U] {
            let k = run(loc, &[b"ab", b"cd ef", b"", b"gh\n"]).unwrap_or_default();
            assert_eq!((k.words, k.lines, k.bytes), (2, 1, 10), "{loc}");
            let k = run(loc, &[b"a ", b" b"]).unwrap_or_default();
            assert_eq!(k.words, 2, "{loc}");
        }
    }

    #[test]
    fn utf8_characters_differ_from_bytes() {
        /* two characters (é, €) in 5 bytes, one word */
        let k = c(U, "é€".as_bytes());
        if k != Counts::default() {
            assert_eq!((k.bytes, k.chars, k.words), (5, 2, 1));
        }
        /* the same bytes in the C locale: five characters */
        let k = c("C", "é€".as_bytes());
        assert_eq!((k.bytes, k.chars), (5, 5));
    }

    #[test]
    fn a_character_split_between_blocks_is_one_character() {
        let s = "a€b".as_bytes();
        for cut1 in 0..=s.len() {
            for cut2 in cut1..=s.len() {
                if let Some(k) = run(U, &[&s[..cut1], &s[cut1..cut2], &s[cut2..]]) {
                    assert_eq!((k.bytes, k.chars, k.words), (5, 3, 1), "{cut1} {cut2}");
                }
            }
        }
    }

    #[test]
    fn unicode_white_space_separates_words() {
        /* U+2003 EM SPACE is iswspace() in a UTF-8 locale */
        if let Some(k) = run(U, &["a\u{2003}b".as_bytes()]) {
            assert_eq!((k.words, k.chars, k.bytes), (2, 3, 5));
        }
        /* and is just three bytes of one word in the C locale */
        let k = c("C", "a\u{2003}b".as_bytes());
        assert_eq!(k.words, 1);
    }

    #[test]
    fn invalid_bytes_are_characters_of_their_own() {
        if let Some(k) = run(U, &[b"a\xffb \xc3 \xe2\x82"]) {
            /* a, 0xff, b | 0xc3 | 0xe2 0x82 cut short at the end */
            assert_eq!((k.words, k.chars, k.bytes), (3, 8, 8));
        }
        /* a lead byte followed by a plain letter: both stay separate */
        if let Some(k) = run(U, &[b"\xc3", b"x"]) {
            assert_eq!((k.chars, k.words), (2, 1));
        }
    }

    #[test]
    fn lines_and_bytes_need_no_locale() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let mut k = Counter::new(false);
        k.feed(b"a b\nc\n");
        let k = k.finish();
        assert_eq!((k.lines, k.bytes, k.words, k.chars), (2, 6, 0, 0));
    }

    #[test]
    fn output_format_is_single_spaced() {
        let s = Sel { lines: true, words: true, bytes: true, chars: false };
        let k = Counts { lines: 1, words: 2, bytes: 3, chars: 9 };
        assert_eq!(format_line(s, &k, None), b"1 2 3\n");
        assert_eq!(format_line(s, &k, Some(b"f")), b"1 2 3 f\n");
        let s = Sel { lines: false, words: false, bytes: false, chars: true };
        assert_eq!(format_line(s, &k, Some(b"x y")), b"9 x y\n");
        let s = Sel { lines: true, words: false, bytes: true, chars: false };
        assert_eq!(format_line(s, &k, None), b"1 3\n");
    }

    #[test]
    fn totals_add_up() {
        let mut t = Counts::default();
        t.add(&Counts { lines: 1, words: 2, bytes: 3, chars: 4 });
        t.add(&Counts { lines: 10, words: 20, bytes: 30, chars: 40 });
        assert_eq!(t, Counts { lines: 11, words: 22, bytes: 33, chars: 44 });
    }

    #[test]
    fn bytes_alone_come_from_the_file_size() {
        let path = format!("/tmp/poutil-wc-test-{}", std::process::id());
        std::fs::write(&path, b"abcdef\n").unwrap();
        let mut h = File::open(&path).unwrap();
        assert_eq!(remaining_bytes(&mut h), Some(7));
        /* the offset is at the end now, as after reading to the end */
        assert_eq!(remaining_bytes(&mut h), Some(0));
        let mut h = File::open(&path).unwrap();
        h.seek(SeekFrom::Start(3)).unwrap();
        assert_eq!(remaining_bytes(&mut h), Some(4));
        let only_bytes = Sel { lines: false, words: false, bytes: true, chars: false };
        let mut h = File::open(&path).unwrap();
        let k = count_file(&mut h, only_bytes).unwrap();
        assert_eq!((k.bytes, k.lines, k.words), (7, 0, 0));
        /* any other selection reads the file */
        let both = Sel { lines: true, ..only_bytes };
        let mut h = File::open(&path).unwrap();
        let k = count_file(&mut h, both).unwrap();
        assert_eq!((k.bytes, k.lines), (7, 1));
        /* not a regular file: no shortcut */
        let mut dev = File::open("/dev/null").unwrap();
        assert_eq!(remaining_bytes(&mut dev), None);
        let k = count_file(&mut dev, only_bytes).unwrap();
        assert_eq!(k.bytes, 0);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn counts_a_big_input_in_blocks() {
        let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
        let data: Vec<u8> = b"word another\n".iter().cycle().take(13 * 100_000).copied().collect();
        let k = count(&mut &data[..], true).unwrap();
        assert_eq!((k.lines, k.words, k.bytes), (100_000, 200_000, 1_300_000));
    }
}
