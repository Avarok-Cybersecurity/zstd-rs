//! The "fast" strategy: one hash table, repeat-offset check, accelerating skip.
use super::{count, dict_candidate, extend_back, hash, read32, read64, rep_len, BlockInput, Tables};
use crate::encode::params::CParams;
use crate::encode::seqstore::SeqStore;

/// Parses `b` with a single hash table of `1 << p.hash_log` entries.
pub fn parse(t: &mut Tables, b: &BlockInput<'_>, p: &CParams, store: &mut SeqStore) {
    let src = b.src;
    let hlog = p.hash_log;
    let mls = p.min_match.clamp(4, 7);
    let step = p.target_length as usize + 1;
    let epoch = t.epoch as usize;
    let ilimit = b.end.saturating_sub(8);
    let mut ip = b.start;
    let mut anchor = b.start;
    if ip == b.low && b.dict.is_none() {
        ip += 1;
    }
    let table = &mut t.hash[..1 << hlog];
    let mut r0 = store.reps[0] as usize;
    while ip + 1 < ilimit {
        // Two positions per round: both hashes and table loads are independent,
        // so their latencies overlap (the reference's fast loop does the same).
        let (v0, v1) = (read64(src, ip), read64(src, ip + 1));
        let (h0, h1) = (hash(v0, mls, hlog), hash(v1, mls, hlog));
        let c0 = table[h0] as usize;
        table[h0] = (epoch + ip) as u32;
        let c1 = table[h1] as usize;
        table[h1] = (epoch + ip + 1) as u32;
        // Candidates in the order a one-position-at-a-time loop would meet them.
        let found = if rep_at(b, r0, ip + 1, v1 as u32) {
            Some((ip + 1, r0, 4 + count(src, ip + 5 - r0, ip + 5, b.end)))
        } else if let Some(m) = own(b, epoch, c0, ip, v0, anchor) {
            Some(m)
        } else if let Some(m) = if b.dict.is_some() { dict_at(b, r0, ip, v0, anchor, mls) } else { None } {
            Some(m)
        } else if rep_at(b, r0, ip + 2, (v1 >> 8) as u32) {
            Some((ip + 2, r0, 4 + count(src, ip + 6 - r0, ip + 6, b.end)))
        } else if let Some(m) = own(b, epoch, c1, ip + 1, v1, anchor) {
            Some(m)
        } else if b.dict.is_some() {
            dict_at(b, r0, ip + 1, v1, anchor, mls)
        } else {
            None
        };
        let Some((start, off, ml)) = found else {
            ip += ((ip - anchor) >> 8) + step + 1;
            continue;
        };
        store.push(&src[anchor..start], off as u32, ml as u32);
        ip = start + ml;
        anchor = ip;
        if ip < ilimit {
            let back = ip - 2;
            table[hash(read64(src, back), mls, hlog)] = (epoch + back) as u32;
            loop {
                let r1 = store.reps[1] as usize;
                let rl = if ip < ilimit { rep_len(b, ip, r1) } else { 0 };
                if rl == 0 {
                    break;
                }
                table[hash(read64(src, ip), mls, hlog)] = (epoch + ip) as u32;
                store.push(&[], r1 as u32, rl as u32);
                ip += rl;
                anchor = ip;
            }
        }
        r0 = store.reps[0] as usize;
    }
    store.tail(&src[anchor..b.end]);
}

/// Repeat offset `r0` matches the four bytes `bytes` at `p` (within `src`).
#[inline(always)]
fn rep_at(b: &BlockInput<'_>, r0: usize, p: usize, bytes: u32) -> bool {
    r0 != 0 && p >= b.low + r0 && r0 <= b.max_dist && read32(b.src, p - r0) == bytes
}

/// A table candidate (stored as `epoch + index`) for position `p` with bytes `v`.
#[inline(always)]
fn own(b: &BlockInput<'_>, epoch: usize, cand: usize, p: usize, v: u64, anchor: usize) -> Option<(usize, usize, usize)> {
    if cand < epoch + b.low || cand - epoch >= p || p + epoch - cand > b.max_dist || read32(b.src, cand - epoch) != v as u32 {
        return None;
    }
    let c = cand - epoch;
    let (s, l) = extend_back(b.src, p, p - c, 4 + count(b.src, c + 4, p + 4, b.end), anchor, b.low);
    Some((s, p - c, l))
}

/// Repeat offset reaching into the dictionary at `p + 1`, else the dictionary table at `p`.
#[inline(always)]
fn dict_at(b: &BlockInput<'_>, r0: usize, p: usize, v: u64, anchor: usize, mls: u32) -> Option<(usize, usize, usize)> {
    let d = b.dict?;
    if r0 > p + 1 - b.low {
        let rl = rep_len(b, p + 1, r0);
        if rl > 0 {
            return Some((p + 1, r0, rl));
        }
    }
    dict_candidate(b, d.hash[hash(v, mls, d.params.hash_log)], p, v, anchor, 4)
}
