/* SPDX-License-Identifier: GPL-2.0-only */
/* dd - convert and copy a file. */

#![forbid(unsafe_code)]

use posix::mb;
use std::os::fd::RawFd;

const SYNOPSIS: &str = "dd [operand...]";

/* ASCII to EBCDIC, ASCII to IBM EBCDIC and EBCDIC to ASCII: the tables of the standard */
const A2E: [u8; 256] = [
    0x00, 0x01, 0x02, 0x03, 0x37, 0x2d, 0x2e, 0x2f, 0x16, 0x05, 0x25, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x3c, 0x3d, 0x32, 0x26, 0x18, 0x19, 0x3f, 0x27, 0x1c, 0x1d, 0x1e, 0x1f,
    0x40, 0x5a, 0x7f, 0x7b, 0x5b, 0x6c, 0x50, 0x7d, 0x4d, 0x5d, 0x5c, 0x4e, 0x6b, 0x60, 0x4b, 0x61,
    0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0x7a, 0x5e, 0x4c, 0x7e, 0x6e, 0x6f,
    0x7c, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xd1, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6,
    0xd7, 0xd8, 0xd9, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xad, 0xe0, 0xbd, 0x9a, 0x6d,
    0x79, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96,
    0x97, 0x98, 0x99, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xc0, 0x4f, 0xd0, 0x5f, 0x07,
    0x20, 0x21, 0x22, 0x23, 0x24, 0x15, 0x06, 0x17, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x09, 0x0a, 0x1b,
    0x30, 0x31, 0x1a, 0x33, 0x34, 0x35, 0x36, 0x08, 0x38, 0x39, 0x3a, 0x3b, 0x04, 0x14, 0x3e, 0xe1,
    0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57,
    0x58, 0x59, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x80, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0x6a, 0x9b, 0x9c, 0x9d, 0x9e,
    0x9f, 0xa0, 0xaa, 0xab, 0xac, 0x4a, 0xae, 0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7,
    0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xa1, 0xbe, 0xbf, 0xca, 0xcb, 0xcc, 0xcd, 0xce, 0xcf, 0xda, 0xdb,
    0xdc, 0xdd, 0xde, 0xdf, 0xea, 0xeb, 0xec, 0xed, 0xee, 0xef, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0xff,
];

const A2IBM: [u8; 256] = [
    0x00, 0x01, 0x02, 0x03, 0x37, 0x2d, 0x2e, 0x2f, 0x16, 0x05, 0x25, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x3c, 0x3d, 0x32, 0x26, 0x18, 0x19, 0x3f, 0x27, 0x1c, 0x1d, 0x1e, 0x1f,
    0x40, 0x5a, 0x7f, 0x7b, 0x5b, 0x6c, 0x50, 0x7d, 0x4d, 0x5d, 0x5c, 0x4e, 0x6b, 0x60, 0x4b, 0x61,
    0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0x7a, 0x5e, 0x4c, 0x7e, 0x6e, 0x6f,
    0x7c, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xd1, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6,
    0xd7, 0xd8, 0xd9, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xad, 0xe0, 0xbd, 0x5f, 0x6d,
    0x79, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96,
    0x97, 0x98, 0x99, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xc0, 0x4f, 0xd0, 0xa1, 0x07,
    0x20, 0x21, 0x22, 0x23, 0x24, 0x15, 0x06, 0x17, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x09, 0x0a, 0x1b,
    0x30, 0x31, 0x1a, 0x33, 0x34, 0x35, 0x36, 0x08, 0x38, 0x39, 0x3a, 0x3b, 0x04, 0x14, 0x3e, 0xe1,
    0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57,
    0x58, 0x59, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x80, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e,
    0x9f, 0xa0, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7,
    0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd, 0xbe, 0xbf, 0xca, 0xcb, 0xcc, 0xcd, 0xce, 0xcf, 0xda, 0xdb,
    0xdc, 0xdd, 0xde, 0xdf, 0xea, 0xeb, 0xec, 0xed, 0xee, 0xef, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0xff,
];

const E2A: [u8; 256] = [
    0x00, 0x01, 0x02, 0x03, 0x9c, 0x09, 0x86, 0x7f, 0x97, 0x8d, 0x8e, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x9d, 0x85, 0x08, 0x87, 0x18, 0x19, 0x92, 0x8f, 0x1c, 0x1d, 0x1e, 0x1f,
    0x80, 0x81, 0x82, 0x83, 0x84, 0x0a, 0x17, 0x1b, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x05, 0x06, 0x07,
    0x90, 0x91, 0x16, 0x93, 0x94, 0x95, 0x96, 0x04, 0x98, 0x99, 0x9a, 0x9b, 0x14, 0x15, 0x9e, 0x1a,
    0x20, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xd5, 0x2e, 0x3c, 0x28, 0x2b, 0x7c,
    0x26, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf, 0xb0, 0xb1, 0x21, 0x24, 0x2a, 0x29, 0x3b, 0x7e,
    0x2d, 0x2f, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xcb, 0x2c, 0x25, 0x5f, 0x3e, 0x3f,
    0xba, 0xbb, 0xbc, 0xbd, 0xbe, 0xbf, 0xc0, 0xc1, 0xc2, 0x60, 0x3a, 0x23, 0x40, 0x27, 0x3d, 0x22,
    0xc3, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9,
    0xca, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72, 0x5e, 0xcc, 0xcd, 0xce, 0xcf, 0xd0,
    0xd1, 0xe5, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0xd2, 0xd3, 0xd4, 0x5b, 0xd6, 0xd7,
    0xd8, 0xd9, 0xda, 0xdb, 0xdc, 0xdd, 0xde, 0xdf, 0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0x5d, 0xe6, 0xe7,
    0x7b, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0xe8, 0xe9, 0xea, 0xeb, 0xec, 0xed,
    0x7d, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x52, 0xee, 0xef, 0xf0, 0xf1, 0xf2, 0xf3,
    0x5c, 0x9f, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0xff,
];

/* The conversions asked for with conv=. */
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Conv {
    ascii: bool,
    ebcdic: bool,
    ibm: bool,
    block: bool,
    unblock: bool,
    lcase: bool,
    ucase: bool,
    swab: bool,
    noerror: bool,
    notrunc: bool,
    sync: bool,
}

impl Conv {
    /* conversions other than sync, noerror and notrunc */
    fn changes_data(&self) -> bool {
        self.ascii || self.ebcdic || self.ibm || self.block || self.unblock || self.lcase || self.ucase || self.swab
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Opts {
    input: Option<Vec<u8>>,
    output: Option<Vec<u8>>,
    ibs: usize,
    obs: usize,
    /* bs= was given: both sizes, and input blocks are not aggregated */
    bs: bool,
    cbs: usize,
    skip: u64,
    seek: u64,
    count: Option<u64>,
    conv: Conv,
}

/*
 * A size: positive decimal numbers, each with k (1024) or b (512) after it if
 * wanted, joined by x. Zero is no size, except that cbs may be zero.
 */
fn parse_size(s: &[u8]) -> Option<u64> {
    let mut product: u64 = 1;
    for part in s.split(|&c| c == b'x') {
        let (digits, mult) = match part.last() {
            Some(b'k') => (&part[..part.len() - 1], 1024u64),
            Some(b'b') => (&part[..part.len() - 1], 512),
            _ => (part, 1),
        };
        if digits.is_empty() || !digits.iter().all(|c| c.is_ascii_digit()) {
            return None;
        }
        /* parse_decimal saturates; a number that does not fit is an error here */
        let n = digits.iter().try_fold(0u64, |n, &c| n.checked_mul(10)?.checked_add((c - b'0') as u64))?.checked_mul(mult)?;
        product = product.checked_mul(n)?;
    }
    Some(product)
}

fn parse_conv(list: &[u8], c: &mut Conv) -> Result<(), String> {
    for v in list.split(|&b| b == b',') {
        match v {
            b"ascii" => c.ascii = true,
            b"ebcdic" => c.ebcdic = true,
            b"ibm" => c.ibm = true,
            b"block" => c.block = true,
            b"unblock" => c.unblock = true,
            b"lcase" => c.lcase = true,
            b"ucase" => c.ucase = true,
            b"swab" => c.swab = true,
            b"noerror" => c.noerror = true,
            b"notrunc" => c.notrunc = true,
            b"sync" => c.sync = true,
            _ => return Err(format!("unknown conversion '{}'", String::from_utf8_lossy(v))),
        }
    }
    if u8::from(c.ascii) + u8::from(c.ebcdic) + u8::from(c.ibm) > 1 {
        return Err("ascii, ebcdic and ibm exclude each other".into());
    }
    if c.block && c.unblock {
        return Err("block and unblock exclude each other".into());
    }
    if c.lcase && c.ucase {
        return Err("lcase and ucase exclude each other".into());
    }
    Ok(())
}

/* The most a block (ibs, obs, bs or cbs) may be: dd holds a block, and a record of cbs, in memory. */
const MAX_BLOCK: u64 = 1 << 30;

fn parse_operands(ops: &[Vec<u8>]) -> Result<Opts, String> {
    let mut o = Opts { input: None, output: None, ibs: 512, obs: 512, bs: false, cbs: 0, skip: 0, seek: 0, count: None, conv: Conv::default() };
    let mut bs = None;
    let size = |name: &str, v: &[u8], zero: bool| -> Result<usize, String> {
        match parse_size(v) {
            Some(0) if !zero => Err(format!("{name}: a size is more than zero")),
            Some(n) if n > MAX_BLOCK => Err(format!("{name}: a size over {MAX_BLOCK} bytes is not supported")),
            Some(n) => usize::try_from(n).map_err(|_| format!("{name}: too big")),
            None => Err(format!("{name}: invalid number '{}'", String::from_utf8_lossy(v))),
        }
    };
    let num = |name: &str, v: &[u8]| -> Result<u64, String> { parse_size(v).ok_or_else(|| format!("{name}: invalid number '{}'", String::from_utf8_lossy(v))) };
    for op in ops {
        let Some(eq) = op.iter().position(|&c| c == b'=') else {
            return Err(format!("unknown operand '{}'", String::from_utf8_lossy(op)));
        };
        let (name, v) = (&op[..eq], &op[eq + 1..]);
        match name {
            b"if" => o.input = Some(v.to_vec()),
            b"of" => o.output = Some(v.to_vec()),
            b"ibs" => o.ibs = size("ibs", v, false)?,
            b"obs" => o.obs = size("obs", v, false)?,
            b"bs" => bs = Some(size("bs", v, false)?),
            b"cbs" => o.cbs = size("cbs", v, true)?,
            b"skip" => o.skip = num("skip", v)?,
            b"seek" => o.seek = num("seek", v)?,
            b"count" => o.count = Some(num("count", v)?),
            b"conv" => parse_conv(v, &mut o.conv)?,
            _ => return Err(format!("unknown operand '{}'", String::from_utf8_lossy(name))),
        }
    }
    if let Some(b) = bs {
        o.ibs = b;
        o.obs = b;
        o.bs = true;
    }
    if (o.conv.block || o.conv.unblock) && o.cbs == 0 {
        return Err("block and unblock need cbs".into());
    }
    Ok(o)
}

/* The conversions of the data, in the order of the standard, over blocks that follow each other. */
struct Convert {
    conv: Conv,
    cbs: usize,
    /* block: the line so far, cut at cbs; unblock: the record so far */
    rec: Vec<u8>,
    /* block: this line is longer than cbs */
    over: bool,
    /* the start of a character that the end of a block cut */
    carry: Vec<u8>,
    truncated: u64,
}

impl Convert {
    fn new(conv: Conv, cbs: usize) -> Convert {
        /* with cbs, ascii is read as unblock and ebcdic and ibm as block */
        let mut conv = conv;
        if cbs > 0 && !conv.block && !conv.unblock {
            conv.unblock = conv.ascii;
            conv.block = conv.ebcdic || conv.ibm;
        }
        Convert { conv, cbs, rec: Vec::new(), over: false, carry: Vec::new(), truncated: 0 }
    }

    /* What the data of one input block (or, last, the end of the input) becomes. */
    fn feed(&mut self, data: &[u8], last: bool) -> Vec<u8> {
        let mut d = data.to_vec();
        if self.conv.swab {
            for p in d.chunks_exact_mut(2) {
                p.swap(0, 1);
            }
        }
        if self.conv.ascii {
            d.iter_mut().for_each(|b| *b = E2A[*b as usize]);
        }
        if self.conv.block {
            d = self.block(&d, last);
        } else if self.conv.unblock {
            d = self.unblock(&d, last);
        }
        if self.conv.lcase || self.conv.ucase {
            d = self.case(&d, last);
        }
        if self.conv.ebcdic {
            d.iter_mut().for_each(|b| *b = A2E[*b as usize]);
        } else if self.conv.ibm {
            d.iter_mut().for_each(|b| *b = A2IBM[*b as usize]);
        }
        d
    }

    /* Lines to records of cbs bytes: no newline, blanks to fill, cut if too long. */
    fn block(&mut self, d: &[u8], last: bool) -> Vec<u8> {
        let mut out = Vec::new();
        for &b in d {
            if b == b'\n' {
                self.end_line(&mut out);
            } else if self.rec.len() < self.cbs {
                self.rec.push(b);
            } else {
                self.over = true;
            }
        }
        if last && (!self.rec.is_empty() || self.over) {
            self.end_line(&mut out);
        }
        out
    }

    fn end_line(&mut self, out: &mut Vec<u8>) {
        if self.over {
            self.truncated += 1;
        }
        out.extend_from_slice(&self.rec);
        out.resize(out.len() + (self.cbs - self.rec.len()), b' ');
        self.rec.clear();
        self.over = false;
    }

    /* Records of cbs bytes to lines: the trailing blanks go, a newline comes. */
    fn unblock(&mut self, d: &[u8], last: bool) -> Vec<u8> {
        let mut out = Vec::new();
        for &b in d {
            self.rec.push(b);
            if self.rec.len() == self.cbs {
                self.end_record(&mut out);
            }
        }
        if last && !self.rec.is_empty() {
            self.end_record(&mut out);
        }
        out
    }

    fn end_record(&mut self, out: &mut Vec<u8>) {
        while self.rec.last() == Some(&b' ') {
            self.rec.pop();
        }
        out.extend_from_slice(&self.rec);
        out.push(b'\n');
        self.rec.clear();
    }

    /* lcase and ucase by the characters of the locale; what has no mapping stays. */
    fn case(&mut self, d: &[u8], last: bool) -> Vec<u8> {
        let mut buf = std::mem::take(&mut self.carry);
        buf.extend_from_slice(d);
        let mut out = Vec::with_capacity(buf.len());
        let mut i = 0;
        while i < buf.len() {
            match mb::char_at_stream(&buf[i..]) {
                None if !last => {
                    self.carry = buf[i..].to_vec();
                    break;
                }
                None => {
                    out.extend_from_slice(&buf[i..]);
                    break;
                }
                Some((len, Some(wc))) => {
                    let m = if self.conv.lcase { mb::to_lower(wc) } else { mb::to_upper(wc) };
                    if m == wc || !mb::encode(m, &mut out) {
                        out.extend_from_slice(&buf[i..i + len]);
                    }
                    i += len;
                }
                Some((len, None)) => {
                    out.extend_from_slice(&buf[i..i + len]);
                    i += len;
                }
            }
        }
        out
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Stats {
    in_full: u64,
    in_part: u64,
    out_full: u64,
    out_part: u64,
    truncated: u64,
}

impl Stats {
    fn text(&self) -> String {
        let mut s = format!("{}+{} records in\n{}+{} records out\n", self.in_full, self.in_part, self.out_full, self.out_part);
        if self.truncated > 0 {
            s.push_str(&format!("{} truncated {}\n", self.truncated, if self.truncated == 1 { "record" } else { "records" }));
        }
        s
    }
}

fn last_err() -> std::io::Error {
    std::io::Error::last_os_error()
}

/* The copy: its descriptors, counts and what has to be written. */
struct Dd {
    prog: Vec<u8>,
    o: Opts,
    stats: Stats,
    status: i32,
    out: Vec<u8>,
    /* where the output is, for the end of a seek without notrunc */
    out_pos: u64,
    /* the unit tests collect messages here */
    log: Option<Vec<String>>,
}

impl Dd {
    fn say(&mut self, msg: String) {
        match self.log.as_mut() {
            Some(l) => l.push(msg),
            None => posix::warn(&self.prog, &msg),
        }
    }

    fn report_stats(&mut self) {
        let t = self.stats.text();
        match self.log.as_mut() {
            Some(l) => l.push(t),
            None => {
                use std::io::Write;
                let _ = std::io::stderr().lock().write_all(t.as_bytes());
            }
        }
    }

    fn input_name(&self) -> Vec<u8> {
        self.o.input.clone().unwrap_or_else(|| b"standard input".to_vec())
    }

    fn output_name(&self) -> Vec<u8> {
        self.o.output.clone().unwrap_or_else(|| b"standard output".to_vec())
    }

    /* All of data to the output; false if that failed (said). */
    fn write_all(&mut self, ofd: RawFd, mut data: &[u8]) -> bool {
        while !data.is_empty() {
            let n = posix::sys::write(ofd, data);
            if n < 0 {
                let e = last_err();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                let name = self.output_name();
                self.say(format!("writing {}: {}", String::from_utf8_lossy(&name), posix::strerror(&e)));
                return false;
            }
            if n == 0 {
                let name = self.output_name();
                self.say(format!("writing {}: nothing written", String::from_utf8_lossy(&name)));
                return false;
            }
            data = &data[n as usize..];
        }
        true
    }

    /* One block of the output, counted. */
    fn put_block(&mut self, ofd: RawFd, block: &[u8], full: bool) -> bool {
        if !self.write_all(ofd, block) {
            return false;
        }
        self.out_pos += block.len() as u64;
        if full {
            self.stats.out_full += 1;
        } else {
            self.stats.out_part += 1;
        }
        true
    }

    /* The blocks of obs bytes that are complete in self.out go; false if a write failed. */
    fn drain(&mut self, ofd: RawFd) -> bool {
        let obs = self.o.obs;
        let mut done = 0;
        while self.out.len() - done >= obs {
            let block = self.out[done..done + obs].to_vec();
            if !self.put_block(ofd, &block, true) {
                return false;
            }
            done += obs;
        }
        self.out.drain(..done);
        true
    }

    /* What is left at the end, as a block that is as short as it is. */
    fn finish(&mut self, ofd: RawFd) -> bool {
        if self.out.is_empty() {
            return true;
        }
        let block = std::mem::take(&mut self.out);
        self.put_block(ofd, &block, false)
    }

    /* The whole copy on open descriptors. */
    fn run(&mut self, ifd: RawFd, ofd: RawFd) -> i32 {
        let o = &self.o;
        let (ibs, obs, conv, cbs) = (o.ibs, o.obs, o.conv, o.cbs);
        let direct = o.bs && !conv.changes_data();
        let pad = if conv.block || conv.unblock || (cbs > 0 && (conv.ascii || conv.ebcdic || conv.ibm)) { b' ' } else { 0 };
        let mut cv = Convert::new(conv, cbs);
        let mut buf = vec![0u8; ibs];
        let mut blocks = 0u64;
        let seekable_in = posix::sys::lseek(ifd, 0, libc::SEEK_CUR) >= 0;
        let mut failed = false;
        /* read errors in a row: with noerror a file that always fails would never end */
        let mut errors = 0u32;
        while self.o.count.is_none_or(|c| blocks < c) {
            if posix::sys::sigint_seen() {
                self.report_stats();
                posix::sys::die_by_sigint();
            }
            let before = if seekable_in { posix::sys::lseek(ifd, 0, libc::SEEK_CUR) } else { -1 };
            let n = posix::sys::read(ifd, &mut buf);
            let mut got;
            if n < 0 {
                let e = last_err();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                let name = self.input_name();
                self.say(format!("reading {}: {}", String::from_utf8_lossy(&name), posix::strerror(&e)));
                self.status = 1;
                errors += 1;
                if !conv.noerror || errors >= 100 {
                    failed = true;
                    break;
                }
                self.report_stats();
                /* a failed read of a file leaves the offset where it was: go past the block */
                if before >= 0 {
                    posix::sys::lseek(ifd, before + ibs as i64, libc::SEEK_SET);
                }
                blocks += 1;
                if !conv.sync {
                    continue;
                }
                got = vec![pad; ibs];
            } else if n == 0 {
                break;
            } else {
                errors = 0;
                blocks += 1;
                let n = n as usize;
                if n == ibs {
                    self.stats.in_full += 1;
                } else {
                    self.stats.in_part += 1;
                }
                got = buf[..n].to_vec();
                if conv.sync && n < ibs {
                    got.resize(ibs, pad);
                }
            }
            if direct {
                let full = got.len() == obs;
                if !self.put_block(ofd, &got, full) {
                    failed = true;
                    break;
                }
            } else {
                let d = cv.feed(&got, false);
                self.out.extend_from_slice(&d);
                if !self.drain(ofd) {
                    failed = true;
                    break;
                }
            }
        }
        if !direct {
            let d = cv.feed(&[], true);
            self.out.extend_from_slice(&d);
            if !failed && !self.drain(ofd) {
                failed = true;
            }
            /* after an error the block that is half full is written too */
            if !self.finish(ofd) {
                failed = true;
            }
        }
        self.stats.truncated = cv.truncated;
        if failed {
            self.status = 1;
        }
        self.status
    }
}

fn open_input(prog: &[u8], o: &Opts) -> Result<RawFd, ()> {
    match &o.input {
        None => Ok(0),
        Some(name) => {
            let fd = posix::sys::open(&posix::cstr(name), libc::O_RDONLY | libc::O_NOCTTY, 0);
            if fd < 0 {
                posix::warn_path(prog, name, &posix::strerror(&last_err()));
                return Err(());
            }
            Ok(fd)
        }
    }
}

fn open_output(prog: &[u8], o: &Opts) -> Result<RawFd, ()> {
    match &o.output {
        None => Ok(1),
        Some(name) => {
            /* truncated at the start unless a seek has to keep what it passes, or notrunc */
            let mut flags = libc::O_WRONLY | libc::O_CREAT | libc::O_NOCTTY;
            if o.seek == 0 && !o.conv.notrunc {
                flags |= libc::O_TRUNC;
            }
            let fd = posix::sys::open(&posix::cstr(name), flags, 0o666);
            if fd < 0 {
                posix::warn_path(prog, name, &posix::strerror(&last_err()));
                return Err(());
            }
            Ok(fd)
        }
    }
}

/* Pass over skip input blocks: a seek on a regular file, else read and drop the bytes. */
fn skip_input(prog: &[u8], o: &Opts, ifd: RawFd) -> Result<(), ()> {
    let bytes = o.skip.checked_mul(o.ibs as u64).filter(|&b| b <= i64::MAX as u64);
    let Some(bytes) = bytes else {
        posix::warn(prog, "skip: too big");
        return Err(());
    };
    if bytes == 0 {
        return Ok(());
    }
    let regular = posix::sys::fstat(ifd).is_ok_and(|s| s.st_mode & libc::S_IFMT == libc::S_IFREG);
    if regular && posix::sys::lseek(ifd, bytes as i64, libc::SEEK_CUR) >= 0 {
        return Ok(());
    }
    let mut left = bytes;
    let mut buf = vec![0u8; o.ibs.min(1 << 20)];
    while left > 0 {
        let want = buf.len().min(left.min(usize::MAX as u64) as usize);
        let n = posix::sys::read(ifd, &mut buf[..want]);
        if n < 0 {
            let e = last_err();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            posix::warn(prog, &format!("skipping: {}", posix::strerror(&e)));
            return Err(());
        }
        if n == 0 {
            break;
        }
        left -= n as u64;
    }
    Ok(())
}

/* Go seek output blocks into the output: a seek, or null bytes on what cannot seek. Returns the offset. */
fn seek_output(prog: &[u8], o: &Opts, ofd: RawFd) -> Result<u64, ()> {
    let bytes = o.seek.checked_mul(o.obs as u64).filter(|&b| b <= i64::MAX as u64);
    let Some(bytes) = bytes else {
        posix::warn(prog, "seek: too big");
        return Err(());
    };
    if bytes == 0 {
        return Ok(0);
    }
    if posix::sys::lseek(ofd, bytes as i64, libc::SEEK_SET) >= 0 {
        return Ok(bytes);
    }
    let nulls = vec![0u8; o.obs.min(1 << 20)];
    let mut left = bytes;
    while left > 0 {
        let n = nulls.len().min(left.min(usize::MAX as u64) as usize);
        let w = posix::sys::write(ofd, &nulls[..n]);
        if w < 0 {
            let e = last_err();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            posix::warn(prog, &format!("seeking: {}", posix::strerror(&e)));
            return Err(());
        }
        left -= w as u64;
    }
    Ok(bytes)
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let operands = posix::no_options(&args, SYNOPSIS);
    let o = match parse_operands(operands) {
        Ok(o) => o,
        Err(m) => posix::fail(&prog, &m),
    };
    posix::sys::catch_sigint();
    let Ok(ifd) = open_input(&prog, &o) else { std::process::exit(1) };
    let Ok(ofd) = open_output(&prog, &o) else { std::process::exit(1) };
    if skip_input(&prog, &o, ifd).is_err() {
        std::process::exit(1);
    }
    let Ok(start) = seek_output(&prog, &o, ofd) else { std::process::exit(1) };
    let (seek, notrunc, to_file) = (o.seek, o.conv.notrunc, o.output.is_some());
    let mut dd = Dd { prog, o, stats: Stats::default(), status: 0, out: Vec::new(), out_pos: start, log: None };
    let status = dd.run(ifd, ofd);
    /* a seek keeps the blocks it passes and nothing after the copy */
    if to_file && seek > 0 && !notrunc && posix::sys::fstat(ofd).is_ok_and(|s| s.st_mode & libc::S_IFMT == libc::S_IFREG) {
        let end = dd.out_pos;
        if posix::sys::ftruncate(ofd, end as i64) < 0 {
            let e = last_err();
            let name = dd.output_name();
            posix::warn_path(&dd.prog, &name, &posix::strerror(&e));
            dd.status = 1;
        }
    }
    dd.report_stats();
    if posix::sys::close(ofd) < 0 && to_file {
        let e = last_err();
        let name = dd.output_name();
        posix::warn_path(&dd.prog, &name, &posix::strerror(&e));
        dd.status = 1;
    }
    std::process::exit(if status != 0 { status } else { dd.status });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("dd-test-{}-{}", std::process::id(), n));
            std::fs::create_dir(&p).unwrap();
            Tmp(p)
        }
        fn p(&self, s: &str) -> String {
            self.0.join(s).to_str().unwrap().to_string()
        }
        fn write(&self, s: &str, c: &[u8]) {
            std::fs::write(self.0.join(s), c).unwrap();
        }
        fn read(&self, s: &str) -> Vec<u8> {
            std::fs::read(self.0.join(s)).unwrap()
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ops(v: &[&str]) -> Vec<Vec<u8>> {
        v.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    fn opts(v: &[&str]) -> Opts {
        parse_operands(&ops(v)).unwrap()
    }

    /* dd on files: the output, the status and the report */
    fn dd(t: &Tmp, v: &[&str], input: &[u8]) -> (Vec<u8>, i32, Vec<String>) {
        t.write("in", input);
        dd_from(t, &t.p("in"), v)
    }

    /* the same, the input being the file at path */
    fn dd_from(t: &Tmp, path: &str, v: &[&str]) -> (Vec<u8>, i32, Vec<String>) {
        let mut args = vec![format!("if={path}"), format!("of={}", t.p("out"))];
        args.extend(v.iter().map(|s| s.to_string()));
        let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let o = opts(&a);
        let ifd = open_input(b"dd", &o).unwrap();
        let ofd = open_output(b"dd", &o).unwrap();
        skip_input(b"dd", &o, ifd).unwrap();
        let start = seek_output(b"dd", &o, ofd).unwrap();
        let (seek, notrunc) = (o.seek, o.conv.notrunc);
        let mut d = Dd { prog: b"dd".to_vec(), o, stats: Stats::default(), status: 0, out: Vec::new(), out_pos: start, log: Some(Vec::new()) };
        let st = d.run(ifd, ofd);
        if seek > 0 && !notrunc {
            posix::sys::ftruncate(ofd, d.out_pos as i64);
        }
        posix::sys::close(ifd);
        posix::sys::close(ofd);
        let mut log = d.log.take().unwrap();
        log.push(d.stats.text());
        (t.read("out"), st, log)
    }

    #[test]
    fn a_block_is_at_most_a_gibibyte() {
        for ok in ["bs=1073741824", "ibs=1073741824", "obs=1k", "cbs=1073741824", "cbs=0"] {
            assert!(parse_operands(&ops(&[ok])).is_ok(), "{ok}");
        }
        for bad in ["bs=1073741825", "ibs=2097152x1024", "obs=4000000000000", "cbs=1073741825", "bs=1024x1024x1024x1024"] {
            let e = parse_operands(&ops(&[bad])).unwrap_err();
            assert!(e.contains("not supported"), "{bad}: {e}");
        }
    }

    /* a directory cannot be read: every read of it is an error */
    #[test]
    fn a_read_error_ends_the_copy() {
        let t = Tmp::new();
        let (out, st, log) = dd_from(&t, &t.p(""), &["ibs=10"]);
        assert_eq!((out.len(), st), (0, 1));
        assert_eq!(log.iter().filter(|l| l.starts_with("reading ")).count(), 1);
        assert_eq!(log.last().unwrap(), "0+0 records in\n0+0 records out\n");
    }

    #[test]
    fn noerror_goes_on_and_gives_up_after_100_in_a_row() {
        let t = Tmp::new();
        let (out, st, log) = dd_from(&t, &t.p(""), &["ibs=10", "conv=noerror"]);
        assert_eq!((out.len(), st), (0, 1));
        assert_eq!(log.iter().filter(|l| l.starts_with("reading ")).count(), 100);
        /* with sync each failed block is a block of nulls; the 100th ends it */
        let (out, st, _) = dd_from(&t, &t.p(""), &["ibs=10", "conv=noerror,sync"]);
        assert_eq!((out.len(), st), (990, 1));
        assert!(out.iter().all(|&b| b == 0));
        /* count counts the failed blocks too */
        let (out, st, log) = dd_from(&t, &t.p(""), &["ibs=10", "conv=noerror,sync", "count=3"]);
        assert_eq!((out.len(), st), (30, 1));
        assert_eq!(log.iter().filter(|l| l.starts_with("reading ")).count(), 3);
        /* blanks, not nulls, with block: the two blocks are one line, cut to the record */
        let (out, _, _) = dd_from(&t, &t.p(""), &["ibs=10", "cbs=10", "conv=noerror,sync,block", "count=2"]);
        assert_eq!(out, vec![b' '; 10]);
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size(b"512"), Some(512));
        assert_eq!(parse_size(b"1k"), Some(1024));
        assert_eq!(parse_size(b"2b"), Some(1024));
        assert_eq!(parse_size(b"2x3"), Some(6));
        assert_eq!(parse_size(b"2kx3bx4"), Some(2048 * 1536 * 4));
        assert_eq!(parse_size(b"0"), Some(0));
        for bad in ["", "k", "x", "1x", "x1", "1xx2", "-1", "+1", "1.5", "1m", "1 ", "99999999999999999999", "9999999999999999999x9999999999999999999"] {
            assert_eq!(parse_size(bad.as_bytes()), None, "{bad:?}");
        }
    }

    #[test]
    fn operands_and_their_rules() {
        let o = opts(&["if=a", "of=b", "ibs=10", "obs=20", "cbs=5", "skip=2", "seek=3", "count=4", "conv=sync,noerror", "conv=swab"]);
        assert_eq!((o.input.as_deref(), o.output.as_deref(), o.ibs, o.obs, o.cbs, o.skip, o.seek, o.count), (Some(&b"a"[..]), Some(&b"b"[..]), 10, 20, 5, 2, 3, Some(4)));
        assert!(o.conv.sync && o.conv.noerror && o.conv.swab && !o.bs);
        /* bs wins over ibs and obs wherever it stands */
        for v in [["bs=7", "ibs=3", "obs=4"], ["ibs=3", "obs=4", "bs=7"]] {
            let o = opts(&v);
            assert_eq!((o.ibs, o.obs, o.bs), (7, 7, true));
        }
        assert_eq!(opts(&[]).ibs, 512);
        assert_eq!(opts(&[]).obs, 512);
        for bad in [&["x"][..], &["foo=1"], &["ibs=0"], &["bs=0"], &["ibs=x"], &["skip=-1"], &["count="], &["conv=nope"], &["conv=ascii,ebcdic"], &["conv=ascii,ibm"], &["conv=ebcdic,ibm"], &["conv=block,unblock"], &["conv=lcase,ucase"], &["conv=block"], &["conv=unblock", "cbs=0"], &["conv=ascii", "conv=ebcdic"], &["conv="], &["=1"]] {
            assert!(parse_operands(&ops(bad)).is_err(), "{bad:?}");
        }
        assert!(parse_operands(&ops(&["conv=block", "cbs=4"])).is_ok());
        assert!(parse_operands(&ops(&["cbs=0"])).is_ok());
    }

    #[test]
    fn tables_are_those_of_the_standard() {
        /* ASCII 0012 (LF) is 0045 in EBCDIC */
        assert_eq!(A2E[0o12], 0o45);
        assert_eq!(A2E[b'0' as usize], 0xf0);
        assert_eq!(A2E[b'|' as usize], 0x4f);
        assert_eq!(A2E[b'[' as usize], 0xad);
        assert_eq!(A2E[b']' as usize], 0xbd);
        assert_eq!(A2IBM[b'[' as usize], 0xad);
        /* the standard table is one to one, and its inverse is the other */
        let mut seen = [false; 256];
        for b in A2E {
            seen[b as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
        assert!((0..256).all(|x| E2A[A2E[x] as usize] as usize == x));
        /* the tables differ in five entries; EBCDIC 0137 (not) stands for ASCII ^, 0241 for ~ in the IBM one */
        let diff: Vec<usize> = (0..256).filter(|&x| A2E[x] != A2IBM[x]).collect();
        assert_eq!(diff.len(), 5);
        assert_eq!(A2IBM[b'^' as usize], 0o137);
        assert_eq!(A2IBM[b'~' as usize], 0o241);
        /* and the IBM one is not one to one: [ and ] appear twice */
        assert_eq!(A2IBM.iter().filter(|&&b| b == 0xad).count(), 2);
        assert_eq!(A2IBM.iter().filter(|&&b| b == 0xbd).count(), 2);
    }

    #[test]
    fn plain_copy_with_the_counts_of_blocks() {
        let t = Tmp::new();
        let data: Vec<u8> = (0..1300u32).map(|i| (i % 251) as u8).collect();
        let (out, st, log) = dd(&t, &[], &data);
        assert_eq!((out, st), (data.clone(), 0));
        /* 1300 = 2 whole blocks of 512 and one of 276 */
        assert_eq!(log.last().unwrap(), "2+1 records in\n2+1 records out\n");
        let (out, _, log) = dd(&t, &["ibs=100", "obs=300"], &data);
        assert_eq!(out, data);
        assert_eq!(log.last().unwrap(), "13+0 records in\n4+1 records out\n");
    }

    #[test]
    fn bs_keeps_the_blocks_as_they_come_and_obs_collects_them() {
        let t = Tmp::new();
        let data = vec![7u8; 25];
        /* bs=10: 10 10 5, three output blocks, one partial */
        let (out, _, log) = dd(&t, &["bs=10"], &data);
        assert_eq!((out.len(), log.last().unwrap().as_str()), (25, "2+1 records in\n2+1 records out\n"));
        /* ibs=10 obs=20: collected into 20 and 5 */
        let (_, _, log) = dd(&t, &["ibs=10", "obs=20"], &data);
        assert_eq!(log.last().unwrap(), "2+1 records in\n1+1 records out\n");
    }

    #[test]
    fn count_skip_and_seek() {
        let t = Tmp::new();
        let data: Vec<u8> = (0..100u8).collect();
        let (out, _, log) = dd(&t, &["bs=10", "count=3"], &data);
        assert_eq!((out, log.last().unwrap().as_str()), (data[..30].to_vec(), "3+0 records in\n3+0 records out\n"));
        let (out, _, _) = dd(&t, &["bs=10", "skip=7"], &data);
        assert_eq!(out, data[70..].to_vec());
        let (out, _, _) = dd(&t, &["ibs=10", "skip=2", "count=1"], &data);
        assert_eq!(out, data[20..30].to_vec());
        /* skip past the end: nothing */
        let (out, _, log) = dd(&t, &["bs=10", "skip=50"], &data);
        assert_eq!((out.len(), log.last().unwrap().as_str()), (0, "0+0 records in\n0+0 records out\n"));
        /* seek: the blocks passed are kept, the rest of the file goes */
        t.write("out", &[9u8; 200]);
        let (out, _, _) = dd(&t, &["bs=10", "seek=2", "count=1", "conv=notrunc"], &data);
        assert_eq!(out.len(), 200);
        assert_eq!(&out[..20], &[9u8; 20]);
        assert_eq!(&out[20..30], &data[..10]);
        assert_eq!(&out[30..], &[9u8; 170][..]);
    }

    #[test]
    fn seek_without_notrunc_cuts_the_file_after_the_copy() {
        let t = Tmp::new();
        let data: Vec<u8> = (0..30u8).collect();
        t.write("out", &[9u8; 200]);
        /* the output is opened by dd(), which truncates when seek is 0 only */
        t.write("in", &data);
        let o = opts(&[&format!("if={}", t.p("in")), &format!("of={}", t.p("out")), "bs=10", "seek=2"]);
        let ifd = open_input(b"dd", &o).unwrap();
        let ofd = open_output(b"dd", &o).unwrap();
        let start = seek_output(b"dd", &o, ofd).unwrap();
        let mut d = Dd { prog: b"dd".to_vec(), o, stats: Stats::default(), status: 0, out: Vec::new(), out_pos: start, log: Some(Vec::new()) };
        d.run(ifd, ofd);
        assert_eq!(d.out_pos, 50);
        posix::sys::ftruncate(ofd, d.out_pos as i64);
        posix::sys::close(ofd);
        let out = t.read("out");
        assert_eq!((out.len(), &out[..20], &out[20..]), (50, &[9u8; 20][..], &data[..]));
        /* an empty input with a seek past the end gives a file of that size */
        let t2 = Tmp::new();
        let (out, _, _) = dd(&t2, &["bs=10", "seek=3"], b"");
        assert_eq!(out, vec![0u8; 30]);
    }

    #[test]
    fn sync_pads_each_input_block() {
        let t = Tmp::new();
        let (out, _, log) = dd(&t, &["ibs=4", "conv=sync"], b"abcdef");
        /* the last output block is not padded: it is as short as the data */
        assert_eq!(out, b"abcdef\0\0");
        assert_eq!(log.last().unwrap(), "1+1 records in\n0+1 records out\n");
        let (out, _, _) = dd(&t, &["bs=4", "conv=sync"], b"abcdef");
        assert_eq!(out, b"abcdef\0\0");
        /* with block the pad is blanks */
        let (out, _, _) = dd(&t, &["ibs=4", "cbs=3", "conv=sync,block"], b"ab\n");
        assert_eq!(out, b"ab    ");
    }

    #[test]
    fn swab_ucase_lcase() {
        let t = Tmp::new();
        assert_eq!(dd(&t, &["conv=swab"], b"abcdef").0, b"badcfe");
        /* an odd byte at the end of a block is not swapped */
        assert_eq!(dd(&t, &["conv=swab"], b"abcde").0, b"badce");
        /* blocks are swapped each on its own */
        assert_eq!(dd(&t, &["ibs=3", "conv=swab"], b"abcdef").0, b"bacedf");
        assert_eq!(dd(&t, &["conv=ucase"], b"Hello, World 123 \xff").0, b"HELLO, WORLD 123 \xff");
        assert_eq!(dd(&t, &["conv=lcase"], b"Hello, World 123").0, b"hello, world 123");
        assert_eq!(dd(&t, &["conv=lcase,swab"], b"ABCD").0, b"badc");
    }

    #[test]
    fn block_and_unblock() {
        let t = Tmp::new();
        /* lines to records of 5: blanks to fill, cut if long, with a count */
        let (out, _, log) = dd(&t, &["cbs=5", "conv=block"], b"ab\nabcdefgh\nxyz12\n\nlast");
        assert_eq!(out, b"ab   abcdexyz12     last ");
        assert!(log.last().unwrap().ends_with("1 truncated record\n"), "{log:?}");
        let (_, _, log) = dd(&t, &["cbs=3", "conv=block"], b"abcd\nefgh\nij\n");
        assert!(log.last().unwrap().ends_with("2 truncated records\n"), "{log:?}");
        /* a line may go on over blocks of the input */
        let (out, _, _) = dd(&t, &["ibs=2", "cbs=6", "conv=block"], b"abcd\nef\n");
        assert_eq!(out, b"abcd  ef    ");
        /* records of 4 to lines, the trailing blanks go, the last record may be short */
        let (out, _, _) = dd(&t, &["cbs=4", "conv=unblock"], b"ab  c   dddde ");
        assert_eq!(out, b"ab\nc\ndddd\ne\n");
        let (out, _, _) = dd(&t, &["ibs=3", "cbs=4", "conv=unblock"], b"ab  cdef");
        assert_eq!(out, b"ab\ncdef\n");
    }

    #[test]
    fn ascii_and_ebcdic() {
        let t = Tmp::new();
        /* what the test suite has: digits, |, newline */
        let ebc = b"\xf0\xf1\xf2\xf3\xf4\xf5\xf6\xf7\xf8\xf9\x4f\x25";
        assert_eq!(dd(&t, &["conv=ascii"], ebc).0, b"0123456789|\n");
        assert_eq!(dd(&t, &["conv=ebcdic"], b"0123456789|\n").0, ebc);
        assert_eq!(dd(&t, &["conv=ibm"], b"0123456789[]\n").0, b"\xf0\xf1\xf2\xf3\xf4\xf5\xf6\xf7\xf8\xf9\xad\xbd\x25");
        /* with cbs: ascii reads records (blanks are 0x40 before and 0x20 after), ebcdic writes them */
        let (out, _, _) = dd(&t, &["cbs=4", "conv=ebcdic"], b"ab\ncd\n");
        assert_eq!(out, b"\x81\x82\x40\x40\x83\x84\x40\x40");
        let (out, _, _) = dd(&t, &["cbs=4", "conv=ascii"], b"\x81\x82\x40\x40\x83\x84\x40\x40");
        assert_eq!(out, b"ab\ncd\n");
        let (out, _, _) = dd(&t, &["cbs=4", "conv=ascii,ucase"], b"\x81\x82\x40\x40");
        assert_eq!(out, b"AB\n");
        let (out, _, _) = dd(&t, &["cbs=3", "conv=ebcdic,ucase"], b"ab\n");
        assert_eq!(out, b"\xc1\xc2\x40");
    }

    #[test]
    fn a_missing_input_is_an_error() {
        let t = Tmp::new();
        let o = opts(&[&format!("if={}", t.p("nosuch"))]);
        assert!(open_input(b"dd", &o).is_err());
    }
}
