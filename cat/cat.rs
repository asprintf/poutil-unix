/* SPDX-License-Identifier: GPL-2.0-only */
/* cat - concatenate and print files. */

use posix::getopt::{Getopt, Opt};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;

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
    let mut status = 0;
    for f in files {
        let res = if f == b"-" {
            copy(&mut *posix::borrow_fd(0), &mut *out)
        } else {
            match File::open(std::ffi::OsStr::from_bytes(f)) {
                Ok(mut h) => copy(&mut h, &mut *out),
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
    fn empty_input() {
        let mut out = Vec::new();
        assert!(copy(&mut &b""[..], &mut out).is_ok());
        assert!(out.is_empty());
    }

    #[test]
    fn write_failure_is_reported_as_write() {
        assert!(matches!(copy(&mut &b"x"[..], &mut Full), Err(Fail::Write(_))));
    }
}
