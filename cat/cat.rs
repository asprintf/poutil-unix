/* SPDX-License-Identifier: GPL-2.0-only */
/* cat - concatenate and print files. */

use posix::getopt::{Getopt, Opt};
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;

const SYNOPSIS: &str = "cat [-u] [file...]";

enum Fail {
    Read(io::Error),
    Write(io::Error),
}

/*
 * Copy r to w in blocks. Every block is written as soon as it is read, so
 * the output is unbuffered whether or not -u was given.
 */
fn copy<R: Read, W: Write>(r: &mut R, w: &mut W) -> Result<(), Fail> {
    let mut buf = [0u8; 65536];
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Fail::Read(e)),
        };
        w.write_all(&buf[..n]).map_err(Fail::Write)?;
    }
}

/*
 * The spec lets cat treat "output is a regular file that is also an input" as
 * an error. Without that, "cat f >> f" never ends. Refuse only when unread
 * bytes remain, so "cat f > f" (already truncated) stays a harmless no-op.
 * out is (device, inode) of a regular-file standard output, if it is one.
 */
fn is_output(h: &mut File, out: Option<(u64, u64)>) -> bool {
    let (dev, ino) = match out {
        Some(v) => v,
        None => return false,
    };
    match h.metadata() {
        Ok(m) if m.is_file() && m.dev() == dev && m.ino() == ino => {
            matches!(h.stream_position(), Ok(pos) if pos < m.len())
        }
        _ => false,
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    /* -u is accepted; output is always unbuffered here */
    let mut g = Getopt::new(&args, b"u");
    while let Some(o) = g.next() {
        match o {
            Opt::Flag(b'u') => {}
            _ => posix::usage(SYNOPSIS),
        }
    }
    let mut files: Vec<&[u8]> = g.operands().iter().map(|v| v.as_slice()).collect();
    if files.is_empty() {
        files.push(b"-");
    }
    let mut out = posix::borrow_fd(1);
    let out_id = match out.metadata() {
        Ok(m) if m.is_file() => Some((m.dev(), m.ino())),
        _ => None,
    };
    let mut status = 0;
    for f in files {
        let res = if f == b"-" {
            let mut stdin = posix::borrow_fd(0);
            if is_output(&mut stdin, out_id) {
                posix::warn_path(&prog, f, "input file is output file");
                status = 1;
                continue;
            }
            copy(&mut *stdin, &mut *out)
        } else {
            match File::open(std::ffi::OsStr::from_bytes(f)) {
                Ok(mut h) => {
                    if is_output(&mut h, out_id) {
                        posix::warn_path(&prog, f, "input file is output file");
                        status = 1;
                        continue;
                    }
                    copy(&mut h, &mut *out)
                }
                Err(e) => {
                    posix::warn_path(&prog, f, &posix::strerror(&e));
                    status = 1;
                    continue;
                }
            }
        };
        match res {
            Ok(()) => {}
            Err(Fail::Read(e)) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                status = 1;
            }
            Err(Fail::Write(e)) => {
                posix::fail(&prog, &format!("write error: {}", posix::strerror(&e)));
            }
        }
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Short(Vec<u8>);

    impl Read for Short {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            let n = self.0.len().min(b.len()).min(3);
            b[..n].copy_from_slice(&self.0[..n]);
            self.0.drain(..n);
            Ok(n)
        }
    }

    struct Full;

    impl Write for Full {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from_raw_os_error(libc::ENOSPC))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn copies_bytes_unchanged() {
        let data: Vec<u8> = (0..=255u8).cycle().take(200_000).collect();
        let mut out = Vec::new();
        assert!(copy(&mut &data[..], &mut out).is_ok());
        assert_eq!(out, data);
        let mut out = Vec::new();
        assert!(copy(&mut Short(b"a\0b\xffc\n".to_vec()), &mut out).is_ok());
        assert_eq!(out, b"a\0b\xffc\n");
    }

    #[test]
    fn refuses_unread_input_that_is_the_output() {
        let path = format!("/tmp/poutil-cat-test-{}", std::process::id());
        std::fs::write(&path, b"data").unwrap();
        let mut h = File::open(&path).unwrap();
        let m = h.metadata().unwrap();
        let id = Some((m.dev(), m.ino()));
        assert!(is_output(&mut h, id));
        assert!(!is_output(&mut h, None));
        assert!(!is_output(&mut h, Some((m.dev(), m.ino() + 1))));
        std::fs::write(&path, b"").unwrap();
        let mut h = File::open(&path).unwrap();
        assert!(!is_output(&mut h, id));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn empty_input() {
        let mut out = Vec::new();
        assert!(copy(&mut &b""[..], &mut out).is_ok());
        assert!(out.is_empty());
    }

    #[test]
    fn write_failure_is_reported_as_write() {
        assert!(matches!(copy(&mut &b"x"[..], &mut Full), Err(Fail::Write(_))));
    }

    struct ReadFail;

    impl Read for ReadFail {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from_raw_os_error(libc::EIO))
        }
    }

    struct Interrupted(bool);

    impl Read for Interrupted {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            if self.0 {
                return Ok(0);
            }
            self.0 = true;
            Err(io::Error::from(io::ErrorKind::Interrupted))
        }
    }

    #[test]
    fn read_failure_is_reported_as_read() {
        assert!(matches!(copy(&mut ReadFail, &mut Vec::new()), Err(Fail::Read(_))));
    }

    #[test]
    fn interrupted_read_is_retried() {
        let mut out = Vec::new();
        assert!(copy(&mut Interrupted(false), &mut out).is_ok());
    }

    #[test]
    fn exact_block_boundaries() {
        for len in [65535usize, 65536, 65537, 131072] {
            let data = vec![0xa5u8; len];
            let mut out = Vec::new();
            assert!(copy(&mut &data[..], &mut out).is_ok());
            assert_eq!(out, data, "{len}");
        }
    }

    #[test]
    fn a_different_file_is_not_the_output() {
        let path = format!("/tmp/poutil-cat-test2-{}", std::process::id());
        std::fs::write(&path, b"data").unwrap();
        let mut h = File::open(&path).unwrap();
        let other = std::fs::metadata("/").unwrap();
        assert!(!is_output(&mut h, Some((other.dev(), other.ino()))));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn partly_read_input_is_still_refused_and_fully_read_is_not() {
        use std::io::SeekFrom;
        let path = format!("/tmp/poutil-cat-test3-{}", std::process::id());
        std::fs::write(&path, b"abcdef").unwrap();
        let mut h = File::open(&path).unwrap();
        let m = h.metadata().unwrap();
        let id = Some((m.dev(), m.ino()));
        h.seek(SeekFrom::Start(3)).unwrap();
        assert!(is_output(&mut h, id));
        h.seek(SeekFrom::End(0)).unwrap();
        assert!(!is_output(&mut h, id));
        std::fs::remove_file(&path).unwrap();
    }
}
