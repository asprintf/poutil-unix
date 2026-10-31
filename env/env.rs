/* SPDX-License-Identifier: GPL-2.0-only */
/* env - set the environment for command invocation. */

use posix::getopt::{Getopt, Opt};
use std::os::unix::ffi::OsStringExt;

const SYNOPSIS: &str = "env [-i] [name=value]... [utility [argument...]]";

/* name=value with a non-empty name */
fn is_assignment(a: &[u8]) -> bool {
    matches!(a.iter().position(|&c| c == b'='), Some(i) if i > 0)
}

fn name_of(entry: &[u8]) -> &[u8] {
    match entry.iter().position(|&c| c == b'=') {
        Some(i) => &entry[..i],
        None => entry,
    }
}

/*
 * The environment after applying the assignments to base (or to nothing
 * with clear): an assignment replaces the variable where it is, a new one
 * goes to the end.
 */
fn build_env(base: Vec<Vec<u8>>, clear: bool, assigns: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut env = if clear { Vec::new() } else { base };
    for a in assigns {
        let name = name_of(a);
        match env.iter().position(|e| name_of(e) == name) {
            Some(i) => {
                env[i] = a.clone();
                let mut seen = false;
                env.retain(|e| {
                    let same = name_of(e) == name;
                    let keep = !same || !seen;
                    seen |= same;
                    keep
                });
            }
            None => env.push(a.clone()),
        }
    }
    env
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut clear = false;
    let mut g = Getopt::new(&args, b"i");
    while let Some(o) = g.next() {
        match o {
            Opt::Flag(b'i') => clear = true,
            _ => posix::usage(SYNOPSIS),
        }
    }
    let ops = g.operands();
    let n = ops.iter().take_while(|a| is_assignment(a)).count();
    let base = std::env::vars_os()
        .map(|(k, v)| {
            let mut e = k.into_vec();
            e.push(b'=');
            e.extend(v.into_vec());
            e
        })
        .collect();
    let env = build_env(base, clear, &ops[..n]);
    if n == ops.len() {
        let mut out = Vec::new();
        for e in &env {
            out.extend_from_slice(e);
            out.push(b'\n');
        }
        posix::write_stdout(&prog, &out);
        return;
    }
    posix::exec_utility(&prog, &ops[n..], Some(&env));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<Vec<u8>> {
        a.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    #[test]
    fn assignments() {
        assert!(is_assignment(b"a=b"));
        assert!(is_assignment(b"a="));
        assert!(!is_assignment(b"=b"));
        assert!(!is_assignment(b"ab"));
    }

    #[test]
    fn replace_append_clear() {
        let base = v(&["A=1", "B=2", "C=3"]);
        assert_eq!(build_env(base.clone(), false, &v(&["B=x", "D=4"])), v(&["A=1", "B=x", "C=3", "D=4"]));
        assert_eq!(build_env(base.clone(), true, &v(&["D=4"])), v(&["D=4"]));
        assert_eq!(build_env(base, true, &[]), Vec::<Vec<u8>>::new());
        assert_eq!(build_env(v(&["A=1", "B=2", "A=3"]), false, &v(&["A=z"])), v(&["A=z", "B=2"]));
        assert_eq!(build_env(v(&["A=1"]), false, &v(&["A=1", "A=2"])), v(&["A=2"]));
    }
}
