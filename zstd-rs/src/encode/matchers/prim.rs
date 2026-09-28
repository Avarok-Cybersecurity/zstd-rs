//! Byte-level primitives shared by the match finders: hashing, unaligned
//! little-endian reads and common-prefix counting.

pub const PRIME: [u64; 9] = [0, 0, 0, 0, 0x9E37_79B1, 889_523_592_379, 227_718_039_650_203, 58_295_818_150_454_627, 0xCF1B_BCDC_B7A5_6463];

/// zstd-style multiplicative hash of the low `mls` bytes of `v`.
#[inline(always)]
pub fn hash(v: u64, mls: u32, log: u32) -> usize {
    if mls == 4 {
        (((v as u32).wrapping_mul(0x9E37_79B1)) >> (32 - log)) as usize
    } else {
        ((v << (64 - 8 * mls)).wrapping_mul(PRIME[mls as usize]) >> (64 - log)) as usize
    }
}

#[inline(always)]
pub fn read32(s: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([s[i], s[i + 1], s[i + 2], s[i + 3]])
}

#[inline(always)]
pub fn read64(s: &[u8], i: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&s[i..i + 8]);
    u64::from_le_bytes(a)
}

/// Length of the common prefix of `s[a..]` and `s[b..end]` (a < b).
#[inline(always)]
pub fn count(s: &[u8], mut a: usize, mut b: usize, end: usize) -> usize {
    let start = b;
    while b + 8 <= end {
        let x = read64(s, a) ^ read64(s, b);
        if x != 0 {
            return b - start + (x.trailing_zeros() / 8) as usize;
        }
        a += 8;
        b += 8;
    }
    while b < end && s[a] == s[b] {
        a += 1;
        b += 1;
    }
    b - start
}

/// Match length for a candidate at dictionary index `d` against `src[ip..end]`;
/// a match reaching the dictionary's end continues into `src[0..]`.
#[inline(always)]
pub fn count_dict(dict: &[u8], d: usize, src: &[u8], ip: usize, end: usize) -> usize {
    let dlen = dict.len() - d;
    let limit = end.min(ip + dlen);
    let mut n = 0;
    while ip + n + 8 <= limit {
        let x = read64(dict, d + n) ^ read64(src, ip + n);
        if x != 0 {
            return n + (x.trailing_zeros() / 8) as usize;
        }
        n += 8;
    }
    while ip + n < limit && dict[d + n] == src[ip + n] {
        n += 1;
    }
    if n == dlen && ip + n < end {
        n += count(src, 0, ip + n, end);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts() {
        let s = b"abcdefghabcdefghabcdXfgh";
        assert_eq!(count(s, 0, 8, s.len()), 12);
        assert_eq!(count(s, 0, 16, s.len()), 4);
        let dict = b"__abc";
        let src = b"abcabcabc";
        assert_eq!(count_dict(dict, 2, src, 3, src.len()), 6);
    }
}
