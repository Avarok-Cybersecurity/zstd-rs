//! Count normalization (the reference algorithm, so tables match zstd's choices).
use super::{highbit, Norm, MAX_SYMBOLS};

const RTB: [u64; 8] = [0, 473195, 504333, 520860, 550000, 700000, 750000, 830000];

/// Smallest table log able to represent `total` samples over `max_symbol + 1` symbols.
pub fn min_table_log(total: usize, max_symbol: usize) -> u32 {
    let src = highbit((total - 1).max(1) as u32) + 1;
    let syms = highbit(max_symbol.max(1) as u32) + 2;
    src.min(syms)
}

/// The reference `FSE_optimalTableLog_internal` heuristic.
pub fn optimal_table_log(max_log: u32, total: usize, max_symbol: usize, minus: u32) -> u32 {
    let max_bits_src = (highbit((total - 1).max(1) as u32)).saturating_sub(minus);
    let min_bits = min_table_log(total, max_symbol);
    let mut log = max_log;
    if max_bits_src < log {
        log = max_bits_src;
    }
    if min_bits > log {
        log = min_bits;
    }
    log.clamp(5, 12)
}

/// Normalizes `count[..=max_symbol]` (summing to `total`) to `1 << log`.
/// Returns `None` if a single symbol holds every sample (use RLE instead).
pub fn normalize(count: &[u32], total: usize, log: u32, low_prob: bool) -> Option<Norm> {
    let symbols = count.len();
    debug_assert!(symbols <= MAX_SYMBOLS && total > 0);
    let mut norm = Norm { counts: [0; MAX_SYMBOLS], symbols, log };
    let low_count: i16 = if low_prob { -1 } else { 1 };
    let scale = 62 - log;
    let step = (1u64 << 62) / total as u64;
    let v_step = 1u64 << (scale - 20);
    let mut still: i64 = 1 << log;
    let low_threshold = (total >> log) as u32;
    let (mut largest, mut largest_p) = (0usize, 0i16);
    for (s, &c) in count.iter().enumerate() {
        if c as usize == total {
            return None;
        }
        if c == 0 {
            continue;
        }
        if c <= low_threshold {
            norm.counts[s] = low_count;
            still -= 1;
        } else {
            let mut p = ((c as u64 * step) >> scale) as i16;
            if p < 8 {
                let rest_to_beat = v_step * RTB[p as usize];
                if (c as u64 * step) - ((p as u64) << scale) > rest_to_beat {
                    p += 1;
                }
            }
            if p > largest_p {
                largest_p = p;
                largest = s;
            }
            norm.counts[s] = p;
            still -= p as i64;
        }
    }
    if -still >= (norm.counts[largest] >> 1) as i64 {
        normalize_m2(&mut norm, count, total, low_count)?;
    } else {
        norm.counts[largest] += still as i16;
    }
    Some(norm)
}

fn normalize_m2(norm: &mut Norm, count: &[u32], total: usize, low_count: i16) -> Option<()> {
    const UNSET: i16 = -2;
    let log = norm.log;
    let mut total = total as u64;
    let mut distributed: u64 = 0;
    let low_threshold = total >> log;
    let mut low_one = (total * 3) >> (log + 1);
    for (s, &c) in count.iter().enumerate() {
        let c = c as u64;
        norm.counts[s] = if c == 0 {
            0
        } else if c <= low_threshold {
            distributed += 1;
            total -= c;
            low_count
        } else if c <= low_one {
            distributed += 1;
            total -= c;
            1
        } else {
            UNSET
        };
    }
    let mut to_distribute = (1u64 << log) - distributed;
    if to_distribute == 0 {
        return Some(());
    }
    if total / to_distribute > low_one {
        low_one = (total * 3) / (to_distribute * 2);
        for (s, &c) in count.iter().enumerate() {
            if norm.counts[s] == UNSET && c as u64 <= low_one {
                norm.counts[s] = 1;
                distributed += 1;
                total -= c as u64;
            }
        }
        to_distribute = (1u64 << log) - distributed;
    }
    if distributed == count.len() as u64 {
        let max_v = (0..count.len()).max_by_key(|&s| (count[s], core::cmp::Reverse(s)))?;
        norm.counts[max_v] += to_distribute as i16;
        return Some(());
    }
    if total == 0 {
        let mut s = 0;
        while to_distribute > 0 {
            if norm.counts[s] > 0 {
                to_distribute -= 1;
                norm.counts[s] += 1;
            }
            s = (s + 1) % count.len();
        }
        return Some(());
    }
    let v_step_log = 62 - log as u64;
    let mid = (1u64 << (v_step_log - 1)) - 1;
    let r_step = (((1u64 << v_step_log) * to_distribute) + mid) / total;
    let mut tmp_total = mid;
    for (s, &c) in count.iter().enumerate() {
        if norm.counts[s] == UNSET {
            let end = tmp_total + c as u64 * r_step;
            let weight = (end >> v_step_log) - (tmp_total >> v_step_log);
            if weight < 1 {
                return None;
            }
            norm.counts[s] = weight as i16;
            tmp_total = end;
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum(n: &Norm) -> i32 {
        n.counts[..n.symbols].iter().map(|&c| if c == -1 { 1 } else { c as i32 }).sum()
    }

    #[test]
    fn sums_to_table_size() {
        let cases: [&[u32]; 4] = [&[10, 20, 30, 1, 0, 5], &[1000, 1, 1, 1, 1, 1], &[1; 40], &[7, 0, 0, 3]];
        for c in cases {
            let total: u32 = c.iter().sum();
            for log in min_table_log(total as usize, c.len() - 1).max(5)..=9 {
                let n = normalize(c, total as usize, log, true).unwrap();
                assert_eq!(sum(&n), 1 << log, "{c:?} log {log}");
                for (s, &v) in c.iter().enumerate() {
                    assert_eq!(v == 0, n.counts[s] == 0);
                }
            }
        }
    }

    #[test]
    fn single_symbol_is_rle() {
        assert!(normalize(&[0, 9, 0], 9, 5, false).is_none());
    }
}
