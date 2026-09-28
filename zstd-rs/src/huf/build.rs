//! Length-limited Huffman code construction (Moffat–Katajainen in-place
//! minimum redundancy, then a Kraft-sum repair to the depth limit).
use super::HufWeights;
use crate::fse::normalize::optimal_table_log;

/// Depth limit the reference uses for `total` literals over `max_symbol + 1` symbols.
pub fn depth_limit(total: usize, max_symbol: usize) -> u32 {
    optimal_table_log(super::MAX_BITS, total, max_symbol, 1).min(super::MAX_BITS)
}

/// Builds weights for `count` (at least two present symbols) with codes of at most `limit` bits.
pub fn build(count: &[u32; 256], limit: u32) -> HufWeights {
    let mut order = [0u8; 256];
    let mut n = 0;
    for (s, &c) in count.iter().enumerate() {
        if c > 0 {
            order[n] = s as u8;
            n += 1;
        }
    }
    debug_assert!(n >= 2);
    let order = &mut order[..n];
    order.sort_unstable_by_key(|&s| (count[s as usize], core::cmp::Reverse(s)));
    let mut a = [0u32; 256];
    for (i, &s) in order.iter().enumerate() {
        a[i] = count[s as usize];
    }
    minimum_redundancy(&mut a[..n]);
    let mut num = [0u32; 33];
    for &d in &a[..n] {
        num[(d as usize).min(32)] += 1;
    }
    enforce_limit(&mut num, limit);
    let mut lens = [0u8; 256];
    let mut j = n;
    for (len, &k) in num.iter().enumerate().skip(1) {
        for _ in 0..k {
            j -= 1;
            lens[order[j] as usize] = len as u8;
        }
    }
    HufWeights::from_lengths(&lens)
}

fn enforce_limit(num: &mut [u32; 33], limit: u32) {
    let limit = limit as usize;
    for i in limit + 1..33 {
        num[limit] += num[i];
        num[i] = 0;
    }
    let mut total: u64 = 0;
    for (i, &k) in num.iter().enumerate().take(limit + 1).skip(1) {
        total += (k as u64) << (limit - i);
    }
    while total > 1u64 << limit {
        num[limit] -= 1;
        for i in (1..limit).rev() {
            if num[i] > 0 {
                num[i] -= 1;
                num[i + 1] += 2;
                break;
            }
        }
        total -= 1;
    }
}

/// In-place minimum-redundancy code lengths for ascending frequencies `a`.
fn minimum_redundancy(a: &mut [u32]) {
    let n = a.len();
    if n == 0 {
        return;
    }
    if n == 1 {
        a[0] = 1;
        return;
    }
    a[0] += a[1];
    let (mut root, mut leaf) = (0usize, 2usize);
    for next in 1..n - 1 {
        if leaf >= n || a[root] < a[leaf] {
            a[next] = a[root];
            a[root] = next as u32;
            root += 1;
        } else {
            a[next] = a[leaf];
            leaf += 1;
        }
        if leaf >= n || (root < next && a[root] < a[leaf]) {
            a[next] += a[root];
            a[root] = next as u32;
            root += 1;
        } else {
            a[next] += a[leaf];
            leaf += 1;
        }
    }
    a[n - 2] = 0;
    for next in (0..n.saturating_sub(2)).rev() {
        a[next] = a[a[next] as usize] + 1;
    }
    let (mut avbl, mut used, mut dpth) = (1i64, 0i64, 0u32);
    let (mut root, mut next) = (n as i64 - 2, n as i64 - 1);
    while avbl > 0 {
        while root >= 0 && a[root as usize] == dpth {
            used += 1;
            root -= 1;
        }
        while avbl > used {
            a[next as usize] = dpth;
            next -= 1;
            avbl -= 1;
        }
        avbl = 2 * used;
        dpth += 1;
        used = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kraft(hw: &HufWeights) -> u64 {
        (0..256).filter(|&s| hw.w[s] > 0).map(|s| 1u64 << (hw.max_bits - hw.len(s))).sum()
    }

    #[test]
    fn complete_and_limited() {
        let mut count = [0u32; 256];
        for (i, c) in count.iter_mut().enumerate() {
            *c = if i < 40 { 1 << (i % 20) } else { (i % 3) as u32 };
        }
        for limit in [8, 9, 11] {
            let hw = build(&count, limit);
            assert!(hw.max_bits <= limit);
            assert_eq!(kraft(&hw), 1 << hw.max_bits, "limit {limit}");
            for (c, w) in count.iter().zip(hw.w.iter()) {
                assert_eq!(*c > 0, *w > 0);
            }
        }
    }

    #[test]
    fn two_symbols() {
        let mut count = [0u32; 256];
        count[3] = 5;
        count[200] = 1;
        let hw = build(&count, 11);
        assert_eq!((hw.len(3), hw.len(200)), (1, 1));
    }
}
