/* SPDX-License-Identifier: GPL-2.0-only */
/* tee - duplicate standard input. */

use posix::getopt::{Getopt, Opt};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::mem::ManuallyDrop;
use std::os::unix::ffi::OsStrExt;

const SYNOPSIS: &str = "tee [-ai] [file...]";

struct Sink {
    name: Vec<u8>,
    w: Box<dyn Write>,
    ok: bool,
}

/* Standard output: a descriptor that is neither closed nor buffered. */
struct Raw(ManuallyDrop<File>);

impl Write for Raw {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/*
 * Copy r to every sink, block by block and without buffering. A sink whose
 * write fails is reported through fail once and dropped; the others carry
 * on. Stops early when no sink is left. A read error is returned.
 */
fn tee<R: Read>(
    r: &mut R,
    sinks: &mut [Sink],
    fail: &mut dyn FnMut(&[u8], &io::Error),
) -> io::Result<()> {
    let mut buf = [0u8; 65536];
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let mut live = false;
        for s in sinks.iter_mut().filter(|s| s.ok) {
            match s.w.write_all(&buf[..n]) {
                Ok(()) => live = true,
                Err(e) => {
                    s.ok = false;
                    fail(&s.name, &e);
                }
            }
        }
        if !live {
            return Ok(());
        }
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut append = false;
    let mut g = Getopt::new(&args, b"ai");
    for o in g.by_ref() {
        match o {
            Opt::Flag(b'a') => append = true,
            Opt::Flag(b'i') => unsafe {
                libc::signal(libc::SIGINT, libc::SIG_IGN);
            },
            _ => posix::usage(SYNOPSIS),
        }
    }
    let mut status = 0;
    let mut sinks = vec![Sink {
        name: b"standard output".to_vec(),
        w: Box::new(Raw(posix::borrow_fd(1))),
        ok: true,
    }];
    /* "-" is a file named "-" here, not standard output */
    for f in g.operands() {
        let opened = OpenOptions::new()
            .write(true)
            .create(true)
            .append(append)
            .truncate(!append)
            .open(OsStr::from_bytes(f));
        match opened {
            Ok(h) => sinks.push(Sink { name: f.clone(), w: Box::new(h), ok: true }),
            Err(e) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                status = 1;
            }
        }
    }
    let mut input = posix::borrow_fd(0);
    let mut fail = |name: &[u8], e: &io::Error| {
        posix::warn_path(&prog, name, &format!("write error: {}", posix::strerror(e)));
    };
    if let Err(e) = tee(&mut *input, &mut sinks, &mut fail) {
        posix::warn(&prog, &format!("read error: {}", posix::strerror(&e)));
        status = 1;
    }
    if sinks.iter().any(|s| !s.ok) {
        status = 1;
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Shared(Rc<RefCell<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct Broken;

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from_raw_os_error(libc::ENOSPC))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn sink(name: &str, w: Box<dyn Write>) -> Sink {
        Sink { name: name.as_bytes().to_vec(), w, ok: true }
    }

    #[test]
    fn copies_to_every_sink() {
        let (a, b) = (Rc::new(RefCell::new(vec![])), Rc::new(RefCell::new(vec![])));
        let mut sinks = vec![
            sink("a", Box::new(Shared(a.clone()))),
            sink("b", Box::new(Shared(b.clone()))),
        ];
        let data: Vec<u8> = (0..=255u8).cycle().take(150_000).collect();
        tee(&mut &data[..], &mut sinks, &mut |_, _| panic!()).unwrap();
        assert_eq!(*a.borrow(), data);
        assert_eq!(*b.borrow(), data);
    }

    #[test]
    fn failing_sink_is_reported_once_and_others_continue() {
        let good = Rc::new(RefCell::new(vec![]));
        let mut sinks = vec![
            sink("bad", Box::new(Broken)),
            sink("good", Box::new(Shared(good.clone()))),
        ];
        let data = vec![7u8; 200_000];
        let mut reports = Vec::new();
        tee(&mut &data[..], &mut sinks, &mut |n, _| reports.push(n.to_vec())).unwrap();
        assert_eq!(reports, vec![b"bad".to_vec()]);
        assert!(!sinks[0].ok && sinks[1].ok);
        assert_eq!(good.borrow().len(), data.len());
    }

    #[test]
    fn stops_when_nothing_is_left() {
        let mut sinks = vec![sink("bad", Box::new(Broken))];
        let mut n = 0;
        tee(&mut &[1u8; 300_000][..], &mut sinks, &mut |_, _| n += 1).unwrap();
        assert_eq!(n, 1);
    }

    struct Flaky(u8);

    impl Read for Flaky {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0 += 1;
            match self.0 {
                1 => Err(io::Error::from(io::ErrorKind::Interrupted)),
                2 => {
                    b[..3].copy_from_slice(b"abc");
                    Ok(3)
                }
                _ => Err(io::Error::from_raw_os_error(libc::EIO)),
            }
        }
    }

    #[test]
    fn interrupted_read_is_retried_and_read_error_is_returned() {
        let out = Rc::new(RefCell::new(vec![]));
        let mut sinks = vec![sink("o", Box::new(Shared(out.clone())))];
        let r = tee(&mut Flaky(0), &mut sinks, &mut |_, _| panic!());
        assert_eq!(r.unwrap_err().raw_os_error(), Some(libc::EIO));
        assert_eq!(*out.borrow(), b"abc");
    }

    #[test]
    fn no_sinks_and_empty_input() {
        let mut none: Vec<Sink> = vec![];
        assert!(tee(&mut &b"data"[..], &mut none, &mut |_, _| panic!()).is_ok());
        let out = Rc::new(RefCell::new(vec![]));
        let mut sinks = vec![sink("o", Box::new(Shared(out.clone())))];
        assert!(tee(&mut &b""[..], &mut sinks, &mut |_, _| panic!()).is_ok());
        assert!(out.borrow().is_empty());
    }
}
