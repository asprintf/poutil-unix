/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * POSIX utility-syntax option parsing (XBD 12.2), operating on bytes.
 *
 * Parsing stops at the first operand or at a "--" terminator (which is
 * consumed). A lone "-" is an operand.
 */

#[derive(Debug, PartialEq, Eq)]
pub enum Opt {
    Flag(u8),
    Arg(u8, Vec<u8>),
    Unknown(u8),
    MissingArg(u8),
}

pub struct Getopt<'a> {
    args: &'a [Vec<u8>],
    spec: &'a [u8],
    /*
     * Index of the first unprocessed argument; the operands start here
     * once the iterator is exhausted.
     */
    pub optind: usize,
    pos: usize,
}

impl<'a> Getopt<'a> {
    /* spec is a getopt optstring, e.g. b"abf:". */
    pub fn new(args: &'a [Vec<u8>], spec: &'a [u8]) -> Self {
        Getopt { args, spec, optind: 1, pos: 0 }
    }

    pub fn operands(&self) -> &'a [Vec<u8>] {
        &self.args[self.optind.min(self.args.len())..]
    }
}

impl Iterator for Getopt<'_> {
    type Item = Opt;

    fn next(&mut self) -> Option<Opt> {
        if self.pos == 0 {
            let a = self.args.get(self.optind)?;
            if a == b"--" {
                self.optind += 1;
                return None;
            }
            if a.len() < 2 || a[0] != b'-' {
                return None;
            }
            self.pos = 1;
        }
        let a = &self.args[self.optind];
        let c = a[self.pos];
        self.pos += 1;
        let at_end = self.pos >= a.len();
        let rest = a[self.pos.min(a.len())..].to_vec();

        let found = if c == b':' { None } else { self.spec.iter().position(|&s| s == c) };
        let Some(i) = found else {
            if at_end {
                self.optind += 1;
                self.pos = 0;
            }
            return Some(Opt::Unknown(c));
        };
        if self.spec.get(i + 1) != Some(&b':') {
            if at_end {
                self.optind += 1;
                self.pos = 0;
            }
            return Some(Opt::Flag(c));
        }
        self.optind += 1;
        self.pos = 0;
        if !at_end {
            return Some(Opt::Arg(c, rest));
        }
        match self.args.get(self.optind) {
            Some(v) => {
                self.optind += 1;
                Some(Opt::Arg(c, v.clone()))
            }
            None => Some(Opt::MissingArg(c)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<Vec<u8>> {
        a.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    #[test]
    fn cluster_and_attached_arg() {
        let a = v(&["p", "-ab", "-fX", "-f", "Y", "file"]);
        let mut g = Getopt::new(&a, b"abf:");
        let got: Vec<_> = g.by_ref().collect();
        assert_eq!(
            got,
            vec![
                Opt::Flag(b'a'),
                Opt::Flag(b'b'),
                Opt::Arg(b'f', b"X".to_vec()),
                Opt::Arg(b'f', b"Y".to_vec()),
            ]
        );
        assert_eq!(g.operands(), &v(&["file"])[..]);
    }

    #[test]
    fn double_dash_and_lone_dash() {
        let a = v(&["p", "-a", "--", "-b"]);
        let mut g = Getopt::new(&a, b"ab");
        assert_eq!(g.by_ref().count(), 1);
        assert_eq!(g.operands(), &v(&["-b"])[..]);

        let a = v(&["p", "-", "-a"]);
        let mut g = Getopt::new(&a, b"a");
        assert_eq!(g.by_ref().count(), 0);
        assert_eq!(g.operands(), &v(&["-", "-a"])[..]);
    }

    #[test]
    fn unknown_and_missing() {
        let a = v(&["p", "-z", "-f"]);
        let got: Vec<_> = Getopt::new(&a, b"f:").collect();
        assert_eq!(got, vec![Opt::Unknown(b'z'), Opt::MissingArg(b'f')]);
    }

    #[test]
    fn stops_at_first_operand() {
        let a = v(&["p", "x", "-a"]);
        let mut g = Getopt::new(&a, b"a");
        assert_eq!(g.by_ref().count(), 0);
        assert_eq!(g.operands().len(), 2);
    }
}
