/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * A C long double, given as the bytes of one in memory, written as
 * printf("%.*Lg") would. Rust has no such type and the libc crate cannot
 * pass one to snprintf, so the digits are worked out here, exactly.
 *
 * The layout is that of the target: the x87 extended format on x86, IEEE
 * quadruple precision on 64-bit ARM (not Apple's) and a plain double
 * elsewhere.
 */

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    X87,
    Quad,
    Double,
}

const LAYOUT: Layout = if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
    Layout::X87
} else if cfg!(all(target_arch = "aarch64", not(target_vendor = "apple"))) {
    Layout::Quad
} else {
    Layout::Double
};

/* sizeof (long double) */
pub const SIZE: usize = if cfg!(target_arch = "x86") {
    12
} else if matches!(LAYOUT, Layout::Double) {
    8
} else {
    16
};

/* LDBL_DIG + 2: the digits that od writes */
pub const DIGITS: usize = match LAYOUT {
    Layout::X87 => 20,
    Layout::Quad => 35,
    Layout::Double => 17,
};

/* The widest text of format(): a sign, the digits, a point, "e+" and the exponent. */
pub const WIDTH: usize = match LAYOUT {
    Layout::X87 | Layout::Quad => DIGITS + 1 + 1 + 2 + 4,
    Layout::Double => DIGITS + 1 + 1 + 2 + 3,
};

enum Num {
    Zero(bool),
    Inf(bool),
    Nan(bool),
    /* value = mant * 2^exp */
    Finite(bool, u128, i32),
}

fn decode(layout: Layout, b: &[u8]) -> Num {
    match layout {
        Layout::X87 => {
            let mut m = [0u8; 8];
            m.copy_from_slice(&b[..8]);
            let mant = u64::from_le_bytes(m);
            let se = u16::from_le_bytes([b[8], b[9]]);
            let (neg, e) = (se & 0x8000 != 0, (se & 0x7fff) as i32);
            if e == 0x7fff {
                return if mant << 1 == 0 { Num::Inf(neg) } else { Num::Nan(neg) };
            }
            /* a number with a clear integer bit and a non-zero exponent is no number: glibc writes nan */
            if e != 0 && mant >> 63 == 0 {
                return Num::Nan(neg);
            }
            if mant == 0 {
                return Num::Zero(neg);
            }
            let e = if e == 0 { 1 } else { e };
            Num::Finite(neg, mant as u128, e - 16383 - 63)
        }
        Layout::Quad => {
            let mut m = [0u8; 16];
            m.copy_from_slice(&b[..16]);
            let bits = u128::from_ne_bytes(m);
            let neg = bits >> 127 != 0;
            let e = ((bits >> 112) & 0x7fff) as i32;
            let frac = bits & ((1u128 << 112) - 1);
            if e == 0x7fff {
                return if frac == 0 { Num::Inf(neg) } else { Num::Nan(neg) };
            }
            if e == 0 {
                return if frac == 0 { Num::Zero(neg) } else { Num::Finite(neg, frac, 1 - 16383 - 112) };
            }
            Num::Finite(neg, frac | (1u128 << 112), e - 16383 - 112)
        }
        Layout::Double => {
            let mut m = [0u8; 8];
            m.copy_from_slice(&b[..8]);
            let bits = u64::from_ne_bytes(m);
            let neg = bits >> 63 != 0;
            let e = ((bits >> 52) & 0x7ff) as i32;
            let frac = bits & ((1u64 << 52) - 1);
            if e == 0x7ff {
                return if frac == 0 { Num::Inf(neg) } else { Num::Nan(neg) };
            }
            if e == 0 {
                return if frac == 0 { Num::Zero(neg) } else { Num::Finite(neg, frac as u128, 1 - 1023 - 52) };
            }
            Num::Finite(neg, (frac | (1u64 << 52)) as u128, e - 1023 - 52)
        }
    }
}

const BASE: u64 = 1_000_000_000;

/* A natural number in base 10^9, least significant limb first. */
struct Big(Vec<u32>);

impl Big {
    fn from_u128(mut n: u128) -> Big {
        let mut v = Vec::new();
        while n > 0 {
            v.push((n % BASE as u128) as u32);
            n /= BASE as u128;
        }
        Big(v)
    }

    fn mul(&mut self, m: u32) {
        let mut carry = 0u64;
        for limb in self.0.iter_mut() {
            let t = *limb as u64 * m as u64 + carry;
            *limb = (t % BASE) as u32;
            carry = t / BASE;
        }
        while carry > 0 {
            self.0.push((carry % BASE) as u32);
            carry /= BASE;
        }
    }

    fn decimal(&self) -> Vec<u8> {
        let mut s = Vec::new();
        for (i, limb) in self.0.iter().rev().enumerate() {
            if i == 0 {
                s.extend_from_slice(limb.to_string().as_bytes());
            } else {
                s.extend_from_slice(format!("{limb:09}").as_bytes());
            }
        }
        s
    }
}

/* The exact value as decimal digits (no leading zero) and the number of them after the point. */
fn exact(mant: u128, exp2: i32) -> (Vec<u8>, i32) {
    let mut big = Big::from_u128(mant);
    if exp2 >= 0 {
        let mut n = exp2;
        while n > 0 {
            let k = n.min(29);
            big.mul(1u32 << k);
            n -= k;
        }
        return (big.decimal(), 0);
    }
    let mut n = -exp2;
    while n > 0 {
        let k = n.min(13);
        big.mul(5u32.pow(k as u32));
        n -= k;
    }
    (big.decimal(), -exp2)
}

/* printf("%.*g", p, value) for the exact digits: the rounding is to nearest, ties to even. */
fn general(neg: bool, mant: u128, exp2: i32, p: usize) -> String {
    let p = p.max(1);
    let (mut d, scale) = exact(mant, exp2);
    let mut x = d.len() as i32 - 1 - scale;
    if d.len() > p {
        let rest = &d[p..];
        let up = match rest[0] {
            b'0'..=b'4' => false,
            b'5' => rest[1..].iter().any(|&c| c != b'0') || (d[p - 1] - b'0') % 2 == 1,
            _ => true,
        };
        d.truncate(p);
        if up {
            let mut i = p;
            loop {
                if i == 0 {
                    d.insert(0, b'1');
                    d.truncate(p);
                    x += 1;
                    break;
                }
                i -= 1;
                if d[i] == b'9' {
                    d[i] = b'0';
                } else {
                    d[i] += 1;
                    break;
                }
            }
        }
    }
    while d.len() < p {
        d.push(b'0');
    }
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    let strip = |s: &mut String| {
        if s.contains('.') {
            while s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
        }
    };
    let digits = String::from_utf8_lossy(&d).into_owned();
    if x < -4 || x >= p as i32 {
        let mut body = String::new();
        body.push_str(&digits[..1]);
        body.push('.');
        body.push_str(&digits[1..]);
        strip(&mut body);
        out.push_str(&body);
        out.push('e');
        out.push(if x < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", x.abs()));
    } else if x >= 0 {
        let mut body = String::new();
        body.push_str(&digits[..x as usize + 1]);
        body.push('.');
        body.push_str(&digits[x as usize + 1..]);
        strip(&mut body);
        out.push_str(&body);
    } else {
        let mut body = String::from("0.");
        for _ in 0..(-x - 1) {
            body.push('0');
        }
        body.push_str(&digits);
        strip(&mut body);
        out.push_str(&body);
    }
    out
}

fn text(layout: Layout, b: &[u8], p: usize) -> String {
    match decode(layout, b) {
        Num::Zero(neg) => (if neg { "-0" } else { "0" }).to_string(),
        Num::Inf(neg) => (if neg { "-inf" } else { "inf" }).to_string(),
        Num::Nan(neg) => (if neg { "-nan" } else { "nan" }).to_string(),
        Num::Finite(neg, mant, exp2) => general(neg, mant, exp2, p),
    }
}

/* The long double in b (SIZE bytes) as "%.*Lg" with DIGITS digits. */
pub fn format(b: &[u8]) -> String {
    text(LAYOUT, b, DIGITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /* the x87 bytes of a double (padded to 16) */
    fn x87(d: f64) -> Vec<u8> {
        let bits = d.to_bits();
        let neg = bits >> 63 != 0;
        let e = ((bits >> 52) & 0x7ff) as i32;
        let frac = bits & ((1u64 << 52) - 1);
        let (mut m, mut ex) = if e == 0 { (frac, -1074) } else { (frac | (1u64 << 52), e - 1075) };
        let mut out = vec![0u8; 16];
        if m == 0 {
            out[9] = if neg { 0x80 } else { 0 };
            return out;
        }
        while m < (1u64 << 52) {
            m <<= 1;
            ex -= 1;
        }
        let mant = m << 11;
        let field = (ex + 52 + 16383) as u16 | if neg { 0x8000 } else { 0 };
        out[..8].copy_from_slice(&mant.to_le_bytes());
        out[8..10].copy_from_slice(&field.to_le_bytes());
        out
    }

    fn c_format(d: f64, p: usize) -> String {
        let fmt = std::ffi::CString::new(format!("%.{p}g")).unwrap();
        let mut buf = [0u8; 512];
        let n = crate::cfmt::format_f64(&mut buf, &fmt, d);
        String::from_utf8_lossy(&buf[..n as usize]).into_owned()
    }

    #[test]
    fn the_x87_digits_are_those_of_printf() {
        let values = [
            1.0, -1.0, 0.1, 0.5, 1.5, 2.5, 3.5, 0.25, 100.0, 123456789.123, 1e22, 1e23, 1e-5, 1e-4, 9.5, 99999.5, 0.000123456, 5e-324, 2.2250738585072014e-308,
            1.7976931348623157e308, 3.141592653589793, 1e15, 1e16, 1e17, 4503599627370496.5, 0.3, 255.0, 65536.0,
        ];
        for &d in &values {
            for p in 1..=25usize {
                assert_eq!(text(Layout::X87, &x87(d), p), c_format(d, p), "{d:e} with {p} digits");
            }
        }
    }

    #[test]
    fn special_values() {
        let mut inf = x87(1.0);
        inf[..8].copy_from_slice(&(1u64 << 63).to_le_bytes());
        inf[8..10].copy_from_slice(&0x7fffu16.to_le_bytes());
        assert_eq!(text(Layout::X87, &inf, 20), "inf");
        inf[9] |= 0x80;
        assert_eq!(text(Layout::X87, &inf, 20), "-inf");
        let mut nan = inf.clone();
        nan[..8].copy_from_slice(&(3u64 << 62).to_le_bytes());
        assert_eq!(text(Layout::X87, &nan, 20), "-nan");
        assert_eq!(text(Layout::X87, &x87(0.0), 20), "0");
        assert_eq!(text(Layout::X87, &x87(-0.0), 20), "-0");
    }

    #[test]
    fn a_long_double_that_a_double_cannot_hold() {
        /* 1 + 2^-63, the least step above 1 in the x87 format */
        let mut b = vec![0u8; 16];
        b[..8].copy_from_slice(&((1u64 << 63) | 1).to_le_bytes());
        b[8..10].copy_from_slice(&16383u16.to_le_bytes());
        assert_eq!(text(Layout::X87, &b, 20), "1.0000000000000000001");
        /* 2^-16445, the least denormal */
        let mut d = vec![0u8; 16];
        d[..8].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(text(Layout::X87, &d, 5), "3.6452e-4951");
    }

    #[test]
    fn quad_layout() {
        /* 1.0 and -2.0 in IEEE binary128, little endian bytes */
        let one = (16383u128) << 112;
        assert_eq!(text(Layout::Quad, &one.to_ne_bytes(), 35), "1");
        let m2 = (1u128 << 127) | ((16384u128) << 112);
        assert_eq!(text(Layout::Quad, &m2.to_ne_bytes(), 35), "-2");
        /* 1 + 2^-112 */
        let b = ((16383u128) << 112) | 1;
        assert_eq!(text(Layout::Quad, &b.to_ne_bytes(), 35), "1.0000000000000000000000000000000002");
    }

    #[test]
    fn double_layout_is_the_double() {
        for &d in &[0.1f64, 1e300, -2.5, 5e-324] {
            for p in [1usize, 6, 17] {
                assert_eq!(text(Layout::Double, &d.to_ne_bytes(), p), c_format(d, p), "{d:e}");
            }
        }
    }
}
