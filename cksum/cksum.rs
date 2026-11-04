/* SPDX-License-Identifier: GPL-2.0-only */
/* cksum - write file checksums and sizes. */

#![forbid(unsafe_code)]

use std::io::Read;

const SYNOPSIS: &str = "cksum [file...]";
const BLOCK: usize = 256 * 1024;

/*
 * The CRC of the standard: the polynomial 0x04c11db7, most significant bit
 * first, no reflection, starting from zero. TAB[0] is the table of the
 * RATIONALE (the effect of one octet); TAB[k] is the effect of an octet
 * followed by k octets of zero, so that eight octets can be folded in at
 * once ("slicing by eight").
 */
const fn make_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut j = 0;
        while j < 8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04c1_1db7 } else { c << 1 };
            j += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut k = 1;
    while k < 8 {
        let mut i = 0;
        while i < 256 {
            let p = t[k - 1][i];
            t[k][i] = (p << 8) ^ t[0][(p >> 24) as usize];
            i += 1;
        }
        k += 1;
    }
    t
}

static TAB: [[u32; 256]; 8] = make_tables();

/* A running checksum: feed it the octets in order, then finish. */
#[derive(Default)]
struct Crc {
    state: u32,
    len: u64,
}

impl Crc {
    fn update(&mut self, data: &[u8]) {
        self.len += data.len() as u64;
        let mut s = self.state;
        let mut chunks = data.chunks_exact(8);
        for c in &mut chunks {
            let w = s ^ u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
            s = TAB[7][(w >> 24) as usize]
                ^ TAB[6][((w >> 16) & 0xff) as usize]
                ^ TAB[5][((w >> 8) & 0xff) as usize]
                ^ TAB[4][(w & 0xff) as usize]
                ^ TAB[3][c[4] as usize]
                ^ TAB[2][c[5] as usize]
                ^ TAB[1][c[6] as usize]
                ^ TAB[0][c[7] as usize];
        }
        for &b in chunks.remainder() {
            s = (s << 8) ^ TAB[0][((s >> 24) as u8 ^ b) as usize];
        }
        self.state = s;
    }

    /*
     * Append the length, least significant octet first and as few octets
     * as will do (none for an empty file), and complement.
     */
    fn finish(self) -> (u32, u64) {
        let mut s = self.state;
        let mut n = self.len;
        while n != 0 {
            s = (s << 8) ^ TAB[0][((s >> 24) as u8 ^ (n & 0xff) as u8) as usize];
            n >>= 8;
        }
        (!s, self.len)
    }
}

/* "%u %d %s\n"; with no name (standard input) the name and its space are left out. */
fn format_line(crc: u32, len: u64, name: Option<&[u8]>) -> Vec<u8> {
    let mut out = format!("{} {}", crc, len).into_bytes();
    if let Some(n) = name {
        out.push(b' ');
        out.extend_from_slice(n);
    }
    out.push(b'\n');
    out
}

fn checksum<R: Read>(r: &mut R, buf: &mut [u8]) -> std::io::Result<(u32, u64)> {
    let mut crc = Crc::default();
    loop {
        let n = posix::read_block(r, buf)?;
        if n == 0 {
            return Ok(crc.finish());
        }
        crc.update(&buf[..n]);
    }
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let files = posix::no_options(&args, SYNOPSIS);
    let mut buf = vec![0u8; BLOCK];
    let mut status = 0;
    if files.is_empty() {
        let mut stdin = posix::borrow_fd(0);
        match checksum(&mut *stdin, &mut buf) {
            Ok((c, n)) => posix::write_stdout(&prog, &format_line(c, n, None)),
            Err(e) => {
                posix::warn(&prog, &format!("standard input: {}", posix::strerror(&e)));
                status = 1;
            }
        }
        std::process::exit(status);
    }
    for f in files {
        match posix::open_input(f).and_then(|mut h| checksum(&mut h, &mut buf)) {
            Ok((c, n)) => posix::write_stdout(&prog, &format_line(c, n, Some(f))),
            Err(e) => {
                posix::warn_path(&prog, f, &posix::strerror(&e));
                status = 1;
            }
        }
    }
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    /* the model program of the RATIONALE, octet by octet */
    fn model(b: &[u8]) -> u32 {
        let mut s: u32 = 0;
        for &c in b {
            s = (s << 8) ^ TAB[0][((s >> 24) ^ c as u32) as usize];
        }
        let mut n = b.len();
        while n != 0 {
            s = (s << 8) ^ TAB[0][((s >> 24) ^ (n & 0xff) as u32) as usize];
            n >>= 8;
        }
        !s
    }

    fn sum(b: &[u8]) -> (u32, u64) {
        let mut c = Crc::default();
        c.update(b);
        c.finish()
    }

    #[test]
    fn table_matches_the_standard() {
        /* some entries of the table printed in the RATIONALE */
        assert_eq!(TAB[0][0], 0x0000_0000);
        assert_eq!(TAB[0][1], 0x04c1_1db7);
        assert_eq!(TAB[0][2], 0x0982_3b6e);
        assert_eq!(TAB[0][32], 0x9823_b6e0);
        assert_eq!(TAB[0][255], 0xb1f7_40b4);
    }

    #[test]
    fn known_values() {
        assert_eq!(sum(b""), (4294967295, 0));
        assert_eq!(sum(b"a"), (1220704766, 1));
        assert_eq!(sum(b"123456789"), (930766865, 9));
    }

    #[test]
    fn slicing_agrees_with_the_model() {
        let mut data = Vec::new();
        let mut x: u32 = 12345;
        for _ in 0..1000 {
            x = x.wrapping_mul(1103515245).wrapping_add(12345);
            data.push((x >> 16) as u8);
        }
        for n in 0..data.len() {
            assert_eq!(sum(&data[..n]).0, model(&data[..n]), "length {}", n);
        }
        /* a long length needs more than one octet of length */
        let big = vec![0xa5u8; 70000];
        assert_eq!(sum(&big).0, model(&big));
    }

    #[test]
    fn pieces_do_not_matter() {
        let data: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let whole = sum(&data);
        for step in [1usize, 3, 7, 8, 9, 64, 1000] {
            let mut c = Crc::default();
            for p in data.chunks(step) {
                c.update(p);
            }
            assert_eq!(c.finish(), whole, "step {}", step);
        }
    }

    #[test]
    fn line_format() {
        assert_eq!(format_line(5, 6, None), b"5 6\n");
        assert_eq!(format_line(5, 6, Some(b"-")), b"5 6 -\n");
        assert_eq!(format_line(4294967295, 0, Some(b"a b")), b"4294967295 0 a b\n");
    }
}
