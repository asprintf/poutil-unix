/* SPDX-License-Identifier: GPL-2.0-only */
/* id - return user identity. */

#![forbid(unsafe_code)]

use posix::getopt::{Getopt, Opt};
use posix::user;

const SYNOPSIS: &str = "id [user]\n       id -G [-n] [user]\n       id -g [-nr] [user]\n       id -u [-nr] [user]";

/* Which single piece of information the options ask for. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /* no option: the whole line */
    Full,
    /* -G: every group ID */
    Groups,
    /* -g: the group ID */
    Group,
    /* -u: the user ID */
    User,
}

/* The identity to describe: the process's, or that of a named user. */
#[derive(Debug, Default)]
struct Ids {
    uid: u32,
    euid: u32,
    gid: u32,
    egid: u32,
    /* supplementary groups, in the order the system gave them */
    groups: Vec<u32>,
}

/* Name of a user or group ID, if the system can map it. */
type Lookup<'a> = &'a dyn Fn(u32) -> Option<Vec<u8>>;

/* "5(name)", or just "5" when the ID has no name. */
fn id_name(out: &mut Vec<u8>, id: u32, names: Lookup) {
    out.extend_from_slice(id.to_string().as_bytes());
    if let Some(n) = names(id) {
        out.push(b'(');
        out.extend_from_slice(&n);
        out.push(b')');
    }
}

/*
 * The line with no options. The effective IDs are added only where they
 * differ from the real ones, the groups only if there are any.
 */
fn full_line(ids: &Ids, user: Lookup, group: Lookup) -> Vec<u8> {
    let mut out = b"uid=".to_vec();
    id_name(&mut out, ids.uid, user);
    out.extend_from_slice(b" gid=");
    id_name(&mut out, ids.gid, group);
    if ids.euid != ids.uid {
        out.extend_from_slice(b" euid=");
        id_name(&mut out, ids.euid, user);
    }
    if ids.egid != ids.gid {
        out.extend_from_slice(b" egid=");
        id_name(&mut out, ids.egid, group);
    }
    for (i, &g) in ids.groups.iter().enumerate() {
        out.extend_from_slice(if i == 0 { b" groups=" } else { b"," });
        id_name(&mut out, g, group);
    }
    out.push(b'\n');
    out
}

/* -G: effective, real, then the supplementary groups, each ID once. */
fn all_groups(ids: &Ids) -> Vec<u32> {
    let mut v: Vec<u32> = Vec::new();
    for &g in [ids.egid, ids.gid].iter().chain(ids.groups.iter()) {
        if !v.contains(&g) {
            v.push(g);
        }
    }
    v
}

/*
 * One ID, or the IDs of -G, as a number or (with -n) a name. An ID that has
 * no name is reported in the second part of the result and left out.
 */
fn list(values: &[u32], names: Option<Lookup>) -> (Vec<u8>, Vec<u32>) {
    let mut out = Vec::new();
    let mut missing = Vec::new();
    for &v in values {
        let text = match names {
            None => v.to_string().into_bytes(),
            Some(f) => match f(v) {
                Some(n) => n,
                None => {
                    missing.push(v);
                    continue;
                }
            },
        };
        if !out.is_empty() {
            out.push(b' ');
        }
        out.extend_from_slice(&text);
    }
    out.push(b'\n');
    (out, missing)
}

/* The identity of this process. */
fn this_process() -> Ids {
    Ids {
        uid: user::uid(),
        euid: user::euid(),
        gid: user::gid(),
        egid: user::egid(),
        groups: user::process_groups(),
    }
}

/* The identity of a named user, real and effective IDs being the same; None if there is no such user. */
fn named_user(name: &[u8]) -> Option<Ids> {
    let u = user::user_by_name(name)?;
    Some(Ids { uid: u.uid, euid: u.uid, gid: u.gid, egid: u.gid, groups: user::user_groups(name, u.gid) })
}

/*
 * Which of the options were given. The synopsis allows one of -G -g -u,
 * -n with any of them, -r with -g and -u only.
 */
fn parse_options(flags: &[u8]) -> Option<(Mode, bool, bool)> {
    let mut mode = Mode::Full;
    let (mut name, mut real) = (false, false);
    for &f in flags {
        let m = match f {
            b'G' => Mode::Groups,
            b'g' => Mode::Group,
            b'u' => Mode::User,
            b'n' => {
                name = true;
                continue;
            }
            b'r' => {
                real = true;
                continue;
            }
            _ => return None,
        };
        if mode != Mode::Full && mode != m {
            return None;
        }
        mode = m;
    }
    if (name && mode == Mode::Full) || (real && (mode == Mode::Full || mode == Mode::Groups)) {
        return None;
    }
    Some((mode, name, real))
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let mut flags = Vec::new();
    let mut g = Getopt::new(&args, b"Ggunr");
    for o in &mut g {
        match o {
            Opt::Flag(c) => flags.push(c),
            _ => posix::usage(SYNOPSIS),
        }
    }
    let Some((mode, name, real)) = parse_options(&flags) else {
        posix::usage(SYNOPSIS);
    };
    let operands = g.operands();
    if operands.len() > 1 {
        posix::usage(SYNOPSIS);
    }
    let ids = match operands.first() {
        None => this_process(),
        Some(u) => match named_user(u) {
            Some(i) => i,
            None => {
                posix::warn_path(&prog, u, "no such user");
                std::process::exit(1);
            }
        },
    };
    let mut status = 0;
    let out = match mode {
        Mode::Full => full_line(&ids, &user::user_name, &user::group_name),
        Mode::Groups => {
            let (o, missing) = list(&all_groups(&ids), name.then_some(&user::group_name as Lookup));
            for m in missing {
                posix::warn(&prog, &format!("cannot find name for group ID {}", m));
                status = 1;
            }
            o
        }
        Mode::Group | Mode::User => {
            let (id, names): (u32, Lookup) = match (mode, real) {
                (Mode::Group, false) => (ids.egid, &user::group_name),
                (Mode::Group, true) => (ids.gid, &user::group_name),
                (_, false) => (ids.euid, &user::user_name),
                (_, true) => (ids.uid, &user::user_name),
            };
            let (o, missing) = list(&[id], name.then_some(names));
            for m in missing {
                let what = if mode == Mode::Group { "group" } else { "user" };
                posix::warn(&prog, &format!("cannot find name for {} ID {}", what, m));
                status = 1;
            }
            o
        }
    };
    /* nothing but a newline is left when the only name was missing */
    if status != 0 && out == b"\n" {
        std::process::exit(status);
    }
    posix::write_stdout(&prog, &out);
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn users(id: u32) -> Option<Vec<u8>> {
        match id {
            0 => Some(b"root".to_vec()),
            501 => Some(b"mario".to_vec()),
            _ => None,
        }
    }

    fn groups(id: u32) -> Option<Vec<u8>> {
        match id {
            0 => Some(b"wheel".to_vec()),
            20 => Some(b"staff".to_vec()),
            12 => Some(b"everyone".to_vec()),
            _ => None,
        }
    }

    fn ids(uid: u32, euid: u32, gid: u32, egid: u32, g: &[u32]) -> Ids {
        Ids { uid, euid, gid, egid, groups: g.to_vec() }
    }

    fn line(i: &Ids) -> String {
        String::from_utf8(full_line(i, &users, &groups)).unwrap()
    }

    #[test]
    fn matching_ids() {
        assert_eq!(line(&ids(501, 501, 20, 20, &[])), "uid=501(mario) gid=20(staff)\n");
    }

    #[test]
    fn effective_user_differs() {
        assert_eq!(line(&ids(501, 0, 20, 20, &[])), "uid=501(mario) gid=20(staff) euid=0(root)\n");
    }

    #[test]
    fn effective_group_differs() {
        assert_eq!(line(&ids(501, 501, 20, 0, &[])), "uid=501(mario) gid=20(staff) egid=0(wheel)\n");
    }

    #[test]
    fn both_differ_user_comes_first() {
        assert_eq!(
            line(&ids(501, 0, 20, 0, &[])),
            "uid=501(mario) gid=20(staff) euid=0(root) egid=0(wheel)\n"
        );
    }

    #[test]
    fn groups_follow() {
        assert_eq!(line(&ids(501, 501, 20, 20, &[20])), "uid=501(mario) gid=20(staff) groups=20(staff)\n");
        assert_eq!(
            line(&ids(501, 501, 20, 0, &[20, 12])),
            "uid=501(mario) gid=20(staff) egid=0(wheel) groups=20(staff),12(everyone)\n"
        );
    }

    #[test]
    fn unmappable_names_are_left_out() {
        assert_eq!(
            line(&ids(1234, 1234, 4321, 4321, &[4321, 12, 777])),
            "uid=1234 gid=4321 groups=4321,12(everyone),777\n"
        );
    }

    #[test]
    fn all_groups_once_effective_first() {
        assert_eq!(all_groups(&ids(1, 1, 20, 0, &[20, 12, 0, 12])), vec![0, 20, 12]);
        assert_eq!(all_groups(&ids(1, 1, 5, 5, &[])), vec![5]);
    }

    #[test]
    fn list_numbers_and_names() {
        assert_eq!(list(&[20, 12], None), (b"20 12\n".to_vec(), vec![]));
        assert_eq!(list(&[20, 12], Some(&groups)), (b"staff everyone\n".to_vec(), vec![]));
        assert_eq!(list(&[20, 99, 12], Some(&groups)), (b"staff everyone\n".to_vec(), vec![99]));
        assert_eq!(list(&[99], Some(&groups)), (b"\n".to_vec(), vec![99]));
    }

    #[test]
    fn option_combinations() {
        assert_eq!(parse_options(b""), Some((Mode::Full, false, false)));
        assert_eq!(parse_options(b"G"), Some((Mode::Groups, false, false)));
        assert_eq!(parse_options(b"Gn"), Some((Mode::Groups, true, false)));
        assert_eq!(parse_options(b"gnr"), Some((Mode::Group, true, true)));
        assert_eq!(parse_options(b"ru"), Some((Mode::User, false, true)));
        assert_eq!(parse_options(b"uu"), Some((Mode::User, false, false)));
        /* more than one of -G -g -u; -n alone; -r alone or with -G */
        assert_eq!(parse_options(b"gu"), None);
        assert_eq!(parse_options(b"Gg"), None);
        assert_eq!(parse_options(b"n"), None);
        assert_eq!(parse_options(b"r"), None);
        assert_eq!(parse_options(b"Gr"), None);
    }
}
