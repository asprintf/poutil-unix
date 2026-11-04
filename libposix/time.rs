/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Clocks, time zones and broken-down time.
 *
 * Single thread only: tzset and use_utc change the time zone (and use_utc
 * the environment) of the whole process, which localtime and strftime read
 * at the same time on another thread. The utilities have one thread; unit
 * tests that call them must hold a lock.
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

/* Seconds since the Epoch, from the real-time clock; an error if it cannot be read (EOVERFLOW: a 32-bit time_t after 2038). */
pub fn now() -> io::Result<i64> {
    let mut ts: libc::timespec = crate::sys::zeroed();
    if unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(ts.tv_sec as i64)
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

/* Make UTC the time zone, as if TZ were set to the string UTC0 (single thread only, see above). */
pub fn use_utc() {
    unsafe {
        libc::setenv(c"TZ".as_ptr(), c"UTC0".as_ptr(), 1);
        ffi::tzset();
    }
}

/*
 * A broken-down time. Its fields can be read (it derefs to libc::tm) but
 * set only through the methods, because strftime follows the tm_zone
 * pointer: it is null, or set by the C library, or a string that lasts as
 * long as the program. The integer fields may hold anything; the C library
 * checks the ones it uses to index its tables.
 */
#[derive(Clone, Copy)]
pub struct Tm(libc::tm);

impl Tm {
    /* Every field zero (Sunday, 1 January 1900, 00:00:00) and no zone name. */
    pub fn zero() -> Tm {
        Tm(crate::sys::zeroed())
    }

    pub fn set_mon(&mut self, mon: i32) {
        self.0.tm_mon = mon;
    }

    pub fn set_mday(&mut self, mday: i32) {
        self.0.tm_mday = mday;
    }

    pub fn set_wday(&mut self, wday: i32) {
        self.0.tm_wday = wday;
    }

    pub fn set_isdst(&mut self, isdst: i32) {
        self.0.tm_isdst = isdst;
    }

    /* The name %Z writes. */
    pub fn set_zone(&mut self, name: &'static CStr) {
        self.0.tm_zone = name.as_ptr() as _;
    }
}

impl std::ops::Deref for Tm {
    type Target = libc::tm;

    fn deref(&self) -> &libc::tm {
        &self.0
    }
}

/* The broken-down local time of t, if the library takes the value. */
pub fn localtime(t: libc::time_t) -> Option<Tm> {
    let mut tm = Tm::zero();
    if unsafe { libc::localtime_r(&t, &mut tm.0) }.is_null() {
        None
    } else {
        Some(tm)
    }
}

/* The same in UTC. */
pub fn gmtime(t: libc::time_t) -> Option<Tm> {
    let mut tm = Tm::zero();
    if unsafe { libc::gmtime_r(&t, &mut tm.0) }.is_null() {
        None
    } else {
        Some(tm)
    }
}

/* strftime(3) into buf: the number of bytes written, 0 if the result (or the empty result) did not fit. */
pub fn strftime(buf: &mut [u8], fmt: &CStr, tm: &Tm) -> usize {
    unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), &tm.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(fmt: &CStr, tm: &Tm) -> Vec<u8> {
        let mut buf = [0u8; 64];
        let n = strftime(&mut buf, fmt, tm);
        buf[..n].to_vec()
    }

    #[test]
    fn a_zero_time_has_no_zone_name_until_one_is_set() {
        let mut tm = Tm::zero();
        tm.set_mon(1);
        tm.set_mday(3);
        assert_eq!(text(c"%m-%d", &tm), b"02-03");
        tm.set_zone(c"PDT");
        assert_eq!(text(c"%Z", &tm), b"PDT");
    }

    #[test]
    fn out_of_range_fields_do_not_index_past_the_tables() {
        let mut tm = Tm::zero();
        tm.set_mon(1000);
        tm.set_wday(-5);
        /* the C library writes "?" or nothing; the point is that it is not a crash */
        let _ = text(c"%B %A %b %a", &tm);
    }

    #[test]
    fn gmtime_of_zero_is_the_epoch() {
        let tm = gmtime(0).unwrap();
        assert_eq!((tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_hour), (70, 0, 1, 0));
    }
}
