//! Fixed-point bit costs (1/256 bit units) for mode selection and optimal parsing.
use crate::fse::Norm;

/// `LOG2_FRAC[m]` = round(256 * log2(1 + m/256)), computed at compile time.
const LOG2_FRAC: [u32; 256] = build_frac();

const fn build_frac() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut m = 0;
    while m < 256 {
        let mut y: u64 = (256 + m as u64) << 8;
        let mut r = 0u32;
        let mut i = 0;
        while i < 16 {
            y = (y * y) >> 16;
            r <<= 1;
            if y >= 2 << 16 {
                y >>= 1;
                r |= 1;
            }
            i += 1;
        }
        t[m] = (r + 128) >> 8;
        m += 1;
    }
    t
}

/// 256 * log2(x) for x >= 1.
#[inline(always)]
pub fn log2_q8(x: u32) -> u32 {
    let hb = 31 - x.leading_zeros();
    let m = if hb >= 8 { (x >> (hb - 8)) & 255 } else { (x << (8 - hb)) & 255 };
    hb * 256 + LOG2_FRAC[m as usize]
}

/// Cost of coding `count` with distribution `norm` (None if a used symbol is absent).
pub fn norm_cost(norm: &Norm, count: &[u32]) -> Option<u64> {
    let table = norm.log * 256;
    let mut total = 0u64;
    for (s, &c) in count.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let p = *norm.counts.get(s).filter(|_| s < norm.symbols)?;
        if p == 0 {
            return None;
        }
        let p = if p < 0 { 1 } else { p as u32 };
        total += c as u64 * (table - log2_q8(p)) as u64;
    }
    Some(total)
}

/// Shannon cost of `count` (summing to `total`) in 1/256 bits.
pub fn entropy_cost(count: &[u32], total: u32) -> u64 {
    let lt = log2_q8(total.max(1));
    count.iter().filter(|&&c| c > 0).map(|&c| c as u64 * (lt - log2_q8(c)) as u64).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log2_is_accurate() {
        assert_eq!(log2_q8(1), 0);
        assert_eq!(log2_q8(2), 256);
        assert_eq!(log2_q8(1024), 2560);
        let l3 = log2_q8(3) as i32;
        assert!((l3 - 406).abs() <= 1, "{l3}");
        let l1000 = log2_q8(1000) as i32;
        assert!((l1000 - 2551).abs() <= 2, "{l1000}");
    }
}
