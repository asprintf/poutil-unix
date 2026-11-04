/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * The mounted file systems and their space, for df.
 *
 * The space of one file system comes from statvfs, which POSIX has, so
 * "df file" works wherever it exists. POSIX has no way to list the mounts
 * or to name the file system and mount point of a file, so that part is
 * system specific: macOS and FreeBSD ask the kernel with getfsstat and statfs
 * (also the right way there: APFS volumes share one device), Linux reads the
 * mount table with getmntent. A system with no such backend still reports
 * "df file" (the file system named "-") and says "unsupported" for plain "df".
 */

use std::ffi::CStr;
use std::io;

/* What the system says about one file system, in its own units. */
#[derive(Clone, Debug)]
pub struct Fs {
    /* the file system name and the directory it is mounted on */
    pub name: Vec<u8>,
    pub root: Vec<u8>,
    /* the size of a block, and the counts of blocks: all, free, free for an unprivileged user */
    pub bsize: u64,
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    /*
     * the bytes the files of this file system take, when the system says: on
     * APFS the blocks and the free blocks are those of the whole container,
     * which holds other volumes too
     */
    pub spaceused: Option<u64>,
}

pub use imp::{fs_of, mounts};

#[cfg(any(target_vendor = "apple", target_os = "freebsd"))]
mod imp {
    use super::*;
    use crate::sys::zeroed;

    /* The bytes of a NUL-terminated name in a fixed array. */
    fn name_bytes(a: &[libc::c_char]) -> Vec<u8> {
        a.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect()
    }

    fn from_statfs(s: &libc::statfs) -> Fs {
        Fs {
            name: name_bytes(&s.f_mntfromname),
            root: name_bytes(&s.f_mntonname),
            /* the widths and signs of these differ between the systems */
            bsize: s.f_bsize as u64,
            blocks: s.f_blocks as u64,
            bfree: s.f_bfree as u64,
            bavail: s.f_bavail as u64,
            spaceused: None,
        }
    }

    /* What getattrlist gives for ATTR_VOL_SPACEUSED: a length, then the bytes. */
    #[cfg(target_vendor = "apple")]
    #[repr(C, packed(4))]
    struct SpaceUsed {
        len: u32,
        bytes: i64,
    }

    /* Only APFS says it; elsewhere the blocks and free blocks are the file system's own. */
    #[cfg(not(target_vendor = "apple"))]
    fn space_used(_root: &[u8]) -> Option<u64> {
        None
    }

    /* The bytes used by the files of the file system mounted on root, if it can say. */
    #[cfg(target_vendor = "apple")]
    fn space_used(root: &[u8]) -> Option<u64> {
        let p = crate::cstr(root);
        let mut al: libc::attrlist = zeroed();
        al.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
        al.volattr = libc::ATTR_VOL_INFO | libc::ATTR_VOL_SPACEUSED;
        let mut b = SpaceUsed { len: 0, bytes: -1 };
        let r = unsafe {
            libc::getattrlist(p.as_ptr(), &mut al as *mut libc::attrlist as *mut libc::c_void, &mut b as *mut SpaceUsed as *mut libc::c_void, std::mem::size_of::<SpaceUsed>(), 0)
        };
        let (len, bytes) = (b.len, b.bytes);
        if r != 0 || (len as usize) < std::mem::size_of::<SpaceUsed>() || bytes < 0 {
            return None;
        }
        Some(bytes as u64)
    }

    /* A file system from statfs: with the figure per volume where the system has one. */
    fn with_used(s: &libc::statfs) -> Fs {
        let mut fs = from_statfs(s);
        fs.spaceused = space_used(&fs.root);
        fs
    }

    /* The file system a file is on. */
    pub fn fs_of(path: &CStr) -> io::Result<Fs> {
        let mut s: libc::statfs = zeroed();
        if unsafe { libc::statfs(path.as_ptr(), &mut s) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(with_used(&s))
    }

    /*
     * The figures of a mount entry brought up to date: getfsstat(MNT_NOWAIT) may
     * hand out the numbers it cached. If statfs on the mount point fails, or
     * answers for another file system (one mounted over it), the cached ones stay.
     */
    fn refreshed(cached: &libc::statfs) -> libc::statfs {
        let root = crate::cstr(&name_bytes(&cached.f_mntonname));
        let mut fresh: libc::statfs = zeroed();
        if unsafe { libc::statfs(root.as_ptr(), &mut fresh) } == 0 && fresh.f_mntfromname == cached.f_mntfromname {
            fresh
        } else {
            *cached
        }
    }

    /* Every mounted file system (MNT_NOWAIT: the list itself is not waited for). */
    pub fn mounts() -> io::Result<Vec<Fs>> {
        let size = std::mem::size_of::<libc::statfs>();
        let mut cap = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
        if cap < 0 {
            return Err(io::Error::last_os_error());
        }
        loop {
            cap += 16;
            let mut buf: Vec<libc::statfs> = vec![zeroed(); cap as usize];
            let n = unsafe { libc::getfsstat(buf.as_mut_ptr(), (cap as usize * size) as _, libc::MNT_NOWAIT) };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            if n < cap {
                return Ok(buf[..n as usize].iter().map(|c| with_used(&refreshed(c))).collect());
            }
            cap *= 2;
        }
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "freebsd")))]
mod imp {
    use super::*;
    use crate::sys::{self, zeroed};
    use std::ffi::CString;

    /* One mount: the file system name and the mount point. */
    struct Mount {
        source: Vec<u8>,
        target: Vec<u8>,
    }

    /*
     * The mount table through getmntent(3), which also undoes the \040-style
     * escapes of the file. The kernel's own table comes first; /etc/mtab is the
     * traditional name and on most systems a link to it.
     */
    #[cfg(target_os = "linux")]
    fn mount_list() -> io::Result<Vec<Mount>> {
        let open = |path: &CStr| {
            let f = unsafe { libc::setmntent(path.as_ptr(), c"r".as_ptr()) };
            if f.is_null() { None } else { Some(f) }
        };
        let Some(table) = open(c"/proc/self/mounts").or_else(|| open(c"/etc/mtab")) else {
            return Err(io::Error::last_os_error());
        };
        let mut out = Vec::new();
        loop {
            let e = unsafe { libc::getmntent(table) };
            if e.is_null() {
                break;
            }
            let (source, target) = unsafe { (CStr::from_ptr((*e).mnt_fsname), CStr::from_ptr((*e).mnt_dir)) };
            out.push(Mount { source: source.to_bytes().to_vec(), target: target.to_bytes().to_vec() });
        }
        unsafe { libc::endmntent(table) };
        Ok(out)
    }

    /* No way to list the mounts is known for this system. */
    #[cfg(not(target_os = "linux"))]
    fn mount_list() -> io::Result<Vec<Mount>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn statvfs(path: &CStr) -> io::Result<libc::statvfs> {
        let mut sv: libc::statvfs = zeroed();
        if unsafe { libc::statvfs(path.as_ptr(), &mut sv) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(sv)
    }

    fn fs_from(sv: &libc::statvfs, name: Vec<u8>, root: Vec<u8>) -> Fs {
        /* the counts are in units of the fragment size */
        let bsize = if sv.f_frsize != 0 { sv.f_frsize } else { sv.f_bsize };
        Fs {
            name,
            root,
            bsize: bsize as u64,
            blocks: sv.f_blocks as u64,
            bfree: sv.f_bfree as u64,
            bavail: sv.f_bavail as u64,
            spaceused: None,
        }
    }

    /* Is dir the path itself or a directory above it? */
    pub(super) fn is_prefix(dir: &[u8], path: &[u8]) -> bool {
        dir == b"/" || (path.starts_with(dir) && (path.len() == dir.len() || path[dir.len()] == b'/'))
    }

    /* The file system a file is on: the deepest mount point above it that is on the same device. */
    pub fn fs_of(path: &CStr) -> io::Result<Fs> {
        let st = sys::stat(path)?;
        let sv = statvfs(path)?;
        let real = sys::realpath(path).unwrap_or_else(|| path.to_bytes().to_vec());
        let mut best: Option<Mount> = None;
        for m in mount_list().unwrap_or_default() {
            if !is_prefix(&m.target, &real) {
                continue;
            }
            let Ok(c) = CString::new(m.target.clone()) else { continue };
            if sys::stat(&c).is_ok_and(|ms| ms.st_dev == st.st_dev) && best.as_ref().is_none_or(|b| m.target.len() >= b.target.len()) {
                best = Some(m);
            }
        }
        Ok(match best {
            Some(m) => fs_from(&sv, m.source, m.target),
            None => fs_from(&sv, b"-".to_vec(), real),
        })
    }

    /*
     * Every mounted file system that has space to report. The kernel's own
     * (proc, sysfs, cgroup and so on) have none and are left out, as df of
     * GNU does by default.
     */
    pub fn mounts() -> io::Result<Vec<Fs>> {
        let mut out = Vec::new();
        for m in mount_list()? {
            let Ok(c) = CString::new(m.target.clone()) else { continue };
            let Ok(sv) = statvfs(&c) else { continue };
            if sv.f_blocks == 0 {
                continue;
            }
            out.push(fs_from(&sv, m.source, m.target));
        }
        Ok(out)
    }
}

#[cfg(all(test, not(any(target_vendor = "apple", target_os = "freebsd"))))]
mod tests {
    use super::*;

    #[test]
    fn a_mount_point_is_above_a_path_only_at_a_component_boundary() {
        assert!(imp::is_prefix(b"/", b"/anything"));
        assert!(imp::is_prefix(b"/home", b"/home"));
        assert!(imp::is_prefix(b"/home", b"/home/user/x"));
        assert!(!imp::is_prefix(b"/home", b"/homework"));
        assert!(!imp::is_prefix(b"/home/user", b"/home"));
    }

    #[test]
    fn a_file_has_a_file_system_even_when_the_mounts_are_unknown() {
        let fs = fs_of(c"/").unwrap();
        assert!(fs.bsize > 0 && !fs.root.is_empty());
    }
}
