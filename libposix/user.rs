/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Process identity and the user and group database, as safe functions: the
 * get*id calls, getpwuid, getpwnam, getgrgid, getgroups and getgrouplist.
 * Names are byte strings, never UTF-8.
 *
 * Single thread only: getpwuid, getpwnam and getgrgid return storage that
 * the next lookup, on any thread, may overwrite. The names are copied out
 * at once, which is enough with one thread.
 */

use std::ffi::{CStr, CString};

pub fn uid() -> u32 {
    unsafe { libc::getuid() }
}

pub fn euid() -> u32 {
    unsafe { libc::geteuid() }
}

pub fn gid() -> u32 {
    unsafe { libc::getgid() }
}

pub fn egid() -> u32 {
    unsafe { libc::getegid() }
}

/* A user of the database: the user ID and the primary group ID. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
}

/* A copy of a C string of the system's, None for a null pointer. */
fn copy_cstr(p: *const libc::c_char) -> Option<Vec<u8>> {
    if p.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
    }
}

/* Name of a user ID, if the database has one. */
pub fn user_name(uid: u32) -> Option<Vec<u8>> {
    let p = unsafe { libc::getpwuid(uid) };
    if p.is_null() {
        None
    } else {
        copy_cstr(unsafe { (*p).pw_name })
    }
}

/* Name of a group ID, if the database has one. */
pub fn group_name(gid: u32) -> Option<Vec<u8>> {
    let p = unsafe { libc::getgrgid(gid) };
    if p.is_null() {
        None
    } else {
        copy_cstr(unsafe { (*p).gr_name })
    }
}

/* The user of that name; None if there is none (or the name holds a NUL). */
pub fn user_by_name(name: &[u8]) -> Option<User> {
    let c = CString::new(name).ok()?;
    let p = unsafe { libc::getpwnam(c.as_ptr()) };
    if p.is_null() {
        return None;
    }
    let (uid, gid) = unsafe { ((*p).pw_uid as u32, (*p).pw_gid as u32) };
    Some(User { uid, gid })
}

/* The group ID of the group of that name; None if there is none (or the name holds a NUL). */
pub fn group_by_name(name: &[u8]) -> Option<u32> {
    let c = CString::new(name).ok()?;
    let p = unsafe { libc::getgrnam(c.as_ptr()) };
    if p.is_null() {
        return None;
    }
    Some(unsafe { (*p).gr_gid } as u32)
}

/* The supplementary groups of this process, in the order the system gave them. */
pub fn process_groups() -> Vec<u32> {
    loop {
        let n = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
        if n <= 0 {
            return Vec::new();
        }
        let mut v = vec![0 as libc::gid_t; n as usize];
        let m = unsafe { libc::getgroups(n, v.as_mut_ptr()) };
        if m >= 0 {
            v.truncate(m as usize);
            return v.into_iter().map(|g| g as u32).collect();
        }
        /* EINVAL: the list grew between the two calls, so ask again */
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINVAL) {
            return Vec::new();
        }
    }
}

/*
 * The groups the database lists for a user, the primary group gid too. If the
 * list would not fit in a million entries, only gid is returned.
 */
pub fn user_groups(name: &[u8], gid: u32) -> Vec<u32> {
    #[cfg(target_vendor = "apple")]
    type Gid = libc::c_int;
    #[cfg(not(target_vendor = "apple"))]
    type Gid = libc::gid_t;
    let Ok(c) = CString::new(name) else {
        return vec![gid];
    };
    let mut size: usize = 64;
    loop {
        let mut v: Vec<Gid> = vec![0; size];
        let mut n = size as libc::c_int;
        let r = unsafe { libc::getgrouplist(c.as_ptr(), gid as Gid, v.as_mut_ptr(), &mut n) };
        if r >= 0 {
            v.truncate(n.max(0) as usize);
            return v.into_iter().map(|g| g as u32).collect();
        }
        if size >= 1 << 20 {
            return vec![gid];
        }
        size = size.max(n.max(0) as usize).saturating_mul(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_user_zero() {
        assert_eq!(user_by_name(b"root").map(|u| u.uid), Some(0));
        assert_eq!(user_name(0).as_deref(), Some(&b"root"[..]));
    }

    /*
     * An ID with no entry in the database. Fixed values are not portable: on
     * macOS -2 is the user and the group nobody and -1 the group nogroup,
     * and directory services hand out IDs almost anywhere.
     */
    fn unused(lookup: fn(u32) -> Option<Vec<u8>>) -> u32 {
        (1..=1000u32).map(|k| u32::MAX - 1000 - k).find(|&id| lookup(id).is_none()).expect("no unused ID in the range")
    }

    #[test]
    fn unknown_names_are_none() {
        assert_eq!(user_by_name(b"no-such-user-poutil"), None);
        assert_eq!(user_by_name(b"a\0b"), None);
        /* found with the function under test; root_is_user_zero checks that a known ID is found */
        let (u, g) = (unused(user_name), unused(group_name));
        assert_eq!(user_name(u), None);
        assert_eq!(group_name(g), None);
    }

    #[test]
    fn groups_by_name() {
        assert_eq!(group_by_name(b"no-such-group-poutil"), None);
        assert_eq!(group_by_name(b"a\0b"), None);
        assert_eq!(group_by_name(b""), None);
        /* the group of this process has a name on a normal system; whatever it is must map back */
        if let Some(n) = group_name(egid()) {
            let back = group_by_name(&n);
            assert!(back.is_some());
        }
    }

    #[test]
    fn user_groups_has_the_primary_group() {
        let root = user_by_name(b"root").unwrap();
        assert!(user_groups(b"root", root.gid).contains(&root.gid));
        assert_eq!(user_groups(b"a\0b", 7), vec![7]);
    }

    #[test]
    fn effective_ids_are_the_real_ones_in_a_plain_process() {
        assert_eq!(uid(), euid());
        assert_eq!(gid(), egid());
    }

    #[test]
    fn process_groups_does_not_repeat_forever() {
        /* the list may be empty; the point is that the call returns */
        let _ = process_groups();
    }
}
