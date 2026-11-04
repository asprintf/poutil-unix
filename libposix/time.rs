/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Clocks, time zones and broken-down time.
 */

use std::ffi::CStr;
use std::io;

mod ffi {
    extern "C" {
        /* the libc crate does not declare it on every platform */
        pub fn tzset();
    }
}

/* Apply TZ from the environment. */
pub fn tzset() {
    unsafe { ffi::tzset() }
}

/* Seconds since the Epoch (time(3)). */
pub fn now() -> i64 {
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

/* Seconds of the real-time clock (clock_gettime(2)). */
pub fn realtime() -> i64 {
    let mut ts: libc::timespec = crate::sys::zeroed();
    unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
    ts.tv_sec as i64
}

/* Set the real-time clock to a whole number of seconds. */
pub fn set_realtime(sec: libc::time_t) -> io::Result<()> {
    let ts = libc::timespec { tv_sec: sec, tv_nsec: 0 };
    if unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/* Make UTC the time zone, as if TZ were set to the string UTC0. */
pub fn use_utc() {
    unsafe {
        libc::setenv(c"TZ".as_ptr(), c"UTC0".as_ptr(), 1);
        ffi::tzset();
    }
}

/* The broken-down local time of t, if the library takes the value. */
pub fn localtime(t: libc::time_t) -> Option<libc::tm> {
    let mut tm: libc::tm = crate::sys::zeroed();
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        None
    } else {
        Some(tm)
    }
}

/* The same in UTC. */
pub fn gmtime(t: libc::time_t) -> Option<libc::tm> {
    let mut tm: libc::tm = crate::sys::zeroed();
    if unsafe { libc::gmtime_r(&t, &mut tm) }.is_null() {
        None
    } else {
        Some(tm)
    }
}

/*
 * strftime(3) into buf: the number of bytes written, 0 if the result (or the
 * empty result) did not fit. The tm should come from localtime or gmtime,
 * or be a zeroed one with fields set; its tm_zone must be null or valid.
 */
pub fn strftime(buf: &mut [u8], fmt: &CStr, tm: &libc::tm) -> usize {
    unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), tm) }
}
