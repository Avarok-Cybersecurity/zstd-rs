//! Candidate collection for the optimal parser: repeat offsets first, then
//! hash-chain matches of strictly increasing length.
use super::super::hc::{search, Chain};
use super::super::{count, count_dict, read32, BlockInput, Tables};
use crate::encode::seqstore::Reps;
use alloc::vec::Vec;

/// A match candidate: raw offset and length.
#[derive(Clone, Copy, Debug)]
pub struct Cand {
    pub off: u32,
    pub len: u32,
}

/// Length of a repeat-offset match at `ip` (0 if shorter than `min`).
fn rep_match(b: &BlockInput<'_>, ip: usize, rep: usize, min: usize) -> usize {
    if rep == 0 {
        return 0;
    }
    let src = b.src;
    let n = if rep <= ip - b.low {
        if rep > b.max_dist {
            return 0;
        }
        let m = ip - rep;
        if src[m] != src[ip] || src[m + 1] != src[ip + 1] || src[m + 2] != src[ip + 2] {
            return 0;
        }
        3 + count(src, m + 3, ip + 3, b.end)
    } else {
        match b.dict.filter(|d| b.low == 0 && rep - ip <= d.content.len()) {
            Some(d) => count_dict(&d.content, d.content.len() - (rep - ip), src, ip, b.end),
            None => 0,
        }
    };
    if n >= min {
        n
    } else {
        0
    }
}

/// Collects candidates at `ip` into `out` (sorted by increasing length).
/// `ll0` is true when no literal precedes this position (changes repeat semantics).
#[allow(clippy::too_many_arguments)]
pub fn collect(
    t: &mut Tables,
    b: &BlockInput<'_>,
    ip: usize,
    reps: &Reps,
    ll0: bool,
    min: usize,
    chain: &Chain,
    h3: Option<&mut [u32]>,
    out: &mut Vec<Cand>,
) {
    out.clear();
    let mut best = min - 1;
    let r = [reps[0] as usize, reps[1] as usize, reps[2] as usize];
    let reps_to_try: [usize; 3] = if ll0 { [r[1], r[2], r[0].saturating_sub(1)] } else { r };
    for &rep in &reps_to_try {
        let n = rep_match(b, ip, rep, min);
        if n > best {
            best = n;
            out.push(Cand { off: rep as u32, len: n as u32 });
            if ip + n >= b.end {
                return;
            }
        }
    }
    if let Some(h3) = h3 {
        let epoch = t.epoch as usize;
        let v = read32(b.src, ip) & 0xFF_FFFF;
        let h = (v.wrapping_mul(506_832_829) >> (32 - h3.len().trailing_zeros())) as usize;
        let e = h3[h] as usize;
        h3[h] = (epoch + ip) as u32;
        if best < 3 && e >= epoch + b.low && e - epoch < ip && ip - (e - epoch) <= b.max_dist.min(1 << 18) {
            let m = e - epoch;
            let n = count(b.src, m, ip, b.end);
            if n >= 3 && n > best {
                best = n;
                out.push(Cand { off: (ip - m) as u32, len: n as u32 });
            }
        }
    }
    search(t, b, ip, chain, best, |len, off| {
        if out.last().is_some_and(|c| c.len as usize == len) {
            return len;
        }
        out.push(Cand { off: off as u32, len: len as u32 });
        if ip + len >= b.end {
            usize::MAX
        } else {
            len
        }
    });
}
