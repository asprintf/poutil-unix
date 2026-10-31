/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Decimal integers of any length, as expr and test read them: an optional
 * minus sign and digits. They are compared without converting, so a number
 * that does not fit in 64 bits still has the right order.
 */

use std::cmp::Ordering;

/* An integer argument: an optional minus and digits. */
pub struct Int<'a> {
    pub neg: bool,
    /* the digits without leading zeros: empty for zero */
    pub mag: &'a [u8],
}

pub fn int(s: &[u8]) -> Option<Int<'_>> {
    let (neg, digits) = match s.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, s),
    };
    if digits.is_empty() || !digits.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let first = digits.iter().position(|&c| c != b'0').unwrap_or(digits.len());
    let mag = &digits[first..];
    Some(Int { neg: neg && !mag.is_empty(), mag })
}

/* The value of an integer argument, if it fits in 64 bits. */
pub fn value(i: &Int) -> Option<i64> {
    let mut n: i64 = 0;
    for &c in i.mag {
        n = n.checked_mul(10)?.checked_sub((c - b'0') as i64)?;
    }
    if i.neg {
        Some(n)
    } else {
        n.checked_neg()
    }
}

/* Compare two integers of any length. */
pub fn compare_ints(a: &Int, b: &Int) -> Ordering {
    let by_size = |x: &Int, y: &Int| x.mag.len().cmp(&y.mag.len()).then_with(|| x.mag.cmp(y.mag));
    match (a.neg, b.neg) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (false, false) => by_size(a, b),
        (true, true) => by_size(b, a),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_an_integer() {
        assert!(int(b"0").is_some() && int(b"-0").is_some() && int(b"007").is_some());
        for bad in ["", "-", "+1", "1.5", " 1", "1 ", "--1", "0x1", "\u{ff}"] {
            assert!(int(bad.as_bytes()).is_none(), "{bad:?}");
        }
        assert!(int(b"-0").unwrap().mag.is_empty() && !int(b"-0").unwrap().neg);
    }

    #[test]
    fn values_that_fit() {
        assert_eq!(value(&int(b"-9223372036854775808").unwrap()), Some(i64::MIN));
        assert_eq!(value(&int(b"9223372036854775807").unwrap()), Some(i64::MAX));
        assert_eq!(value(&int(b"9223372036854775808").unwrap()), None);
        assert_eq!(value(&int(b"-9223372036854775809").unwrap()), None);
        assert_eq!(value(&int(b"-0000").unwrap()), Some(0));
    }

    #[test]
    fn order_of_any_size() {
        let c = |a: &str, b: &str| compare_ints(&int(a.as_bytes()).unwrap(), &int(b.as_bytes()).unwrap());
        assert_eq!(c("1", "2"), Ordering::Less);
        assert_eq!(c("-1", "-2"), Ordering::Greater);
        assert_eq!(c("-0", "0"), Ordering::Equal);
        assert_eq!(c("100000000000000000000", "99999999999999999999"), Ordering::Greater);
        assert_eq!(c("-100000000000000000000", "-99999999999999999999"), Ordering::Less);
        assert_eq!(c("-5", "3"), Ordering::Less);
    }
}
