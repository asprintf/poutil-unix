/* SPDX-License-Identifier: GPL-2.0-only */
/* pathchk - check pathnames. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

const SYNOPSIS: &str = "pathchk [-p] [-P] pathname...";

const POSIX_PATH_MAX: usize = 256;
const POSIX_NAME_MAX: usize = 14;

fn components(path: &[u8]) -> impl Iterator<Item = &[u8]> {
    path.split(|&c| c == b'/').filter(|c| !c.is_empty())
}

fn portable_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-')
}

/*
 * A name for a diagnostic: valid UTF-8 as it is, every other byte as \xNN,
 * so that nothing is silently replaced.
 */
fn show(b: &[u8]) -> String {
    let mut s = String::new();
    for chunk in b.utf8_chunks() {
        s.push_str(chunk.valid());
        for x in chunk.invalid() {
            s.push_str(&format!("\\x{:02x}", x));
        }
    }
    s
}

/*
 * The checks that look only at the name: -p (limits and the portable
 * filename character set) and -P (leading hyphen, empty). The limits
 * include the terminating NUL, hence ">=". An empty pathname is an error
 * only under -P.
 */
fn check_names(path: &[u8], p: bool, big_p: bool) -> Result<(), String> {
    if big_p && path.is_empty() {
        return Err("empty pathname".into());
    }
    if p {
        if path.len() >= POSIX_PATH_MAX {
            return Err(format!("pathname is {} bytes, limit is {}", path.len(), POSIX_PATH_MAX - 1));
        }
        for c in components(path) {
            if c.len() > POSIX_NAME_MAX {
                return Err(format!(
                    "component {} is {} bytes, limit is {}",
                    show(c),
                    c.len(),
                    POSIX_NAME_MAX
                ));
            }
            if let Some(&b) = c.iter().find(|&&b| !portable_char(b)) {
                return Err(format!(
                    "component {} has a character outside the portable set ({:#04x})",
                    show(c),
                    b
                ));
            }
        }
    }
    if big_p && components(path).any(|c| c[0] == b'-') {
        return Err("a component begins with a hyphen".into());
    }
    Ok(())
}

/*
 * The directory a walk of path starts in. A pathname that begins with exactly
 * two slashes is implementation-defined, so that root is kept as "//" and not
 * folded into "/"; three or more leading slashes are the plain root.
 */
fn start_dir(path: &[u8]) -> Vec<u8> {
    if path.starts_with(b"//") && path.get(2) != Some(&b'/') {
        b"//".to_vec()
    } else if path.first() == Some(&b'/') {
        b"/".to_vec()
    } else {
        b".".to_vec()
    }
}

/* A pathconf limit of a directory; None when there is none. */
fn limit(dir: &[u8], name: libc::c_int) -> Option<usize> {
    posix::clear_errno();
    let r = posix::sys::pathconf(&posix::cstr(dir), name);
    if r < 0 { None } else { Some(r as usize) }
}

/*
 * The checks against the file system: total length, and for each component
 * the NAME_MAX of its directory, whether that directory is searchable, and
 * whether the prefix really is a directory. Components that do not exist
 * are fine as long as they could be created.
 */
fn check_fs(path: &[u8]) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }
    let mut dir = start_dir(path);
    if let Some(max) = limit(&dir, libc::_PC_PATH_MAX) {
        if path.len() >= max {
            return Err(format!("pathname is {} bytes, limit is {}", path.len(), max - 1));
        }
    }
    let mut exists = true;
    let mut comps = components(path).peekable();
    while let Some(c) = comps.next() {
        if exists && posix::sys::access(&posix::cstr(&dir), libc::X_OK) < 0 {
            let e = std::io::Error::last_os_error();
            return Err(format!(
                "{}: {}",
                show(&dir),
                posix::strerror(&e)
            ));
        }
        if let Some(max) = limit(&dir, libc::_PC_NAME_MAX) {
            if c.len() > max {
                return Err(format!(
                    "component {} is {} bytes, limit in {} is {}",
                    show(c),
                    c.len(),
                    show(&dir),
                    max
                ));
            }
        }
        if !exists {
            continue;
        }
        let mut next = dir.clone();
        if !next.ends_with(b"/") {
            next.push(b'/');
        }
        next.extend_from_slice(c);
        match std::fs::metadata(OsStr::from_bytes(&next)) {
            Ok(m) if m.is_dir() => dir = next,
            Ok(_) if comps.peek().is_some() => {
                return Err(format!("{}: Not a directory", show(&next)))
            }
            Ok(_) => {}
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => exists = false,
            Err(e) => {
                return Err(format!("{}: {}", show(&next), posix::strerror(&e)))
            }
        }
    }
    Ok(())
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let (mut p, mut big_p) = (false, false);
    let mut g = Getopt::new(&args, b"pP");
    for o in g.by_ref() {
        match o {
            Opt::Flag(b'p') => p = true,
            Opt::Flag(b'P') => big_p = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    if g.operands().is_empty() {
        posix::usage(SYNOPSIS);
    }
    let mut status = 0;
    for path in g.operands() {
        /* -p replaces the file system checks, -P adds to whatever is on */
        let r = check_names(path, p, big_p).and_then(|_| if p { Ok(()) } else { check_fs(path) });
        if let Err(msg) = r {
            posix::warn_path(&prog, path, &msg);
            status = 1;
        }
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str, p: bool, big_p: bool) -> bool {
        check_names(s.as_bytes(), p, big_p).is_ok()
    }

    #[test]
    fn show_keeps_bytes_visible() {
        assert_eq!(show(b"ab"), "ab");
        assert_eq!(show(b"a\xffb\xfe"), "a\\xffb\\xfe");
        assert_eq!(show("h\u{e9}".as_bytes()), "h\u{e9}");
    }

    #[test]
    fn portable_set() {
        assert!(n("a/b.c/D_e-f", true, false));
        assert!(!n("a/b,c", true, false));
        assert!(!n("a b", true, false));
        assert!(!n("a/\u{e9}", true, false));
        assert!(n("a b", false, false));
    }

    #[test]
    fn portable_limits() {
        assert!(n(&"a".repeat(14), true, false));
        assert!(!n(&"a".repeat(15), true, false));
        assert!(n(&format!("{}a", "a/".repeat(127)), true, false)); /* 255 bytes */
        assert!(!n(&"a/".repeat(128), true, false)); /* 256 bytes */
        assert!(n(&"a".repeat(15), false, false));
    }

    #[test]
    fn hyphen_and_empty() {
        assert!(!n("a/-b", false, true));
        assert!(n("a/b-", false, true));
        assert!(n("-a", false, false));
        assert!(n("", false, false));
        assert!(n("", true, false));
        assert!(!n("", false, true));
        assert!(!n("", true, true));
    }

    #[test]
    fn file_system_checks() {
        assert!(check_fs(b"/").is_ok());
        assert!(check_fs(b"/no/such/dir/file").is_ok());
        /* longer than PATH_MAX wherever that is (1024 on macOS, 4096 on Linux) */
        assert!(check_fs(&b"/a".repeat(libc::PATH_MAX as usize)).is_err());
        assert!(check_fs(format!("/{}", "a".repeat(300)).as_bytes()).is_err());
        assert!(check_fs(b"/etc/passwd/x").is_err());
    }

    #[test]
    fn empty_components_are_ignored() {
        assert_eq!(components(b"//a//b/").collect::<Vec<_>>(), [&b"a"[..], &b"b"[..]]);
        assert_eq!(components(b"///").count(), 0);
    }

    #[test]
    fn start_dir_keeps_exactly_two_slashes() {
        assert_eq!(start_dir(b"//"), b"//");
        assert_eq!(start_dir(b"//a/b"), b"//");
        assert_eq!(start_dir(b"///a"), b"/");
        assert_eq!(start_dir(b"/a"), b"/");
        assert_eq!(start_dir(b"a//b"), b".");
        assert_eq!(components(b"").count(), 0);
    }

    #[test]
    fn limits_apply_per_component_and_to_the_total() {
        assert!(n(&format!("{}/{}", "a".repeat(14), "b".repeat(14)), true, false));
        assert!(n(&"aaaaaaaaaaaaaa/".repeat(17), true, false)); /* 255 bytes */
        assert!(!n(&format!("{}a", "aaaaaaaaaaaaaa/".repeat(17)), true, false)); /* 256 */
        assert!(!n(&format!("a/{}/b", "c".repeat(15)), true, false));
    }

    #[test]
    fn hyphen_rules_for_big_p() {
        assert!(!n("a/-b", false, true));
        assert!(!n("a//-b", false, true));
        assert!(!n("-", false, true));
        assert!(!n("/-x", false, true));
        assert!(n("a-b", false, true));
        assert!(n("a/b-", false, true));
    }

    #[test]
    fn empty_pathname_rules() {
        assert!(n("", false, false));
        assert!(n("", true, false));
        assert!(!n("", false, true));
        assert!(!n("", true, true));
    }

    #[test]
    fn every_punctuation_mark_but_three_is_outside_the_portable_set() {
        for c in "~!@#$%^&*()+=[]{}|;:'\",<>?` \\\t".bytes() {
            assert!(!portable_char(c), "{c}");
        }
        for c in "._-0Zz".bytes() {
            assert!(portable_char(c), "{c}");
        }
        assert!(!portable_char(0));
        assert!(!portable_char(0xff));
    }

    #[test]
    fn messages_name_the_component_losslessly() {
        let e = check_names(b"a/x\xff", true, false).unwrap_err();
        assert!(e.contains("x\\xff") && e.contains("(0xff)"), "{e}");
        let e = check_names("a/bbbbbbbbbbbbbbb".as_bytes(), true, false).unwrap_err();
        assert!(e.contains("15 bytes, limit is 14"), "{e}");
        let e = check_names("a/-b".as_bytes(), false, true).unwrap_err();
        assert!(e.contains("hyphen"), "{e}");
    }

    #[test]
    fn file_system_checks_on_odd_paths() {
        assert!(check_fs(b"").is_ok());
        assert!(check_fs(b"/").is_ok());
        assert!(check_fs(b".").is_ok());
        assert!(check_fs(b"/nonexistent/a/b").is_ok());
        let e = check_fs(b"/etc/passwd/x").unwrap_err();
        assert!(e.contains("Not a directory"), "{e}");
    }
}
