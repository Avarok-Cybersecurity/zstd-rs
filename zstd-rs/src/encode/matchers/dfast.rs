//! The "double fast" strategy: an 8-byte hash table for long matches and an
//! `mls`-byte table for short ones, plus repeat offsets.
use super::{count, dict_candidate, extend_back, hash, read32, read64, rep_len, BlockInput, Tables};
use crate::encode::params::CParams;
use crate::encode::seqstore::SeqStore;

#[inline(always)]
fn valid(stored: usize, epoch: usize, b: &BlockInput<'_>, ip: usize) -> Option<usize> {
    let c = stored.wrapping_sub(epoch);
    (stored >= epoch + b.low && c < ip && ip - c <= b.max_dist).then_some(c)
}

pub fn parse(t: &mut Tables, b: &BlockInput<'_>, p: &CParams, store: &mut SeqStore) {
    let src = b.src;
    let (hl, hs) = (p.hash_log, p.chain_log);
    let mls = p.min_match.clamp(4, 7);
    let epoch = t.epoch as usize;
    let ilimit = b.end.saturating_sub(8);
    let mut ip = b.start;
    let mut anchor = b.start;
    if ip == b.low && b.dict.is_none() {
        ip += 1;
    }
    let long = &mut t.hash[..1 << hl];
    let short = &mut t.hash2[..1 << hs];
    let mut r0 = store.reps[0] as usize;
    while ip < ilimit {
        let v = read64(src, ip);
        let (h8, h4) = (hash(v, 8, hl), hash(v, mls, hs));
        let (cl, cs) = (long[h8] as usize, short[h4] as usize);
        long[h8] = (epoch + ip) as u32;
        short[h4] = (epoch + ip) as u32;
        let found = if r0 != 0 && ip + 1 >= b.low + r0 && r0 <= b.max_dist && read32(src, ip + 1 - r0) == (v >> 8) as u32 {
            Some((ip + 1, r0, 4 + count(src, ip + 5 - r0, ip + 5, b.end)))
        } else if let Some(c) = valid(cl, epoch, b, ip).filter(|&c| read64(src, c) == v) {
            let (s, l) = extend_back(src, ip, ip - c, 8 + count(src, c + 8, ip + 8, b.end), anchor, b.low);
            Some((s, ip - c, l))
        } else if let Some(m) = if b.dict.is_some() { dict_long(b, r0, ip, v, anchor) } else { None } {
            Some(m)
        } else if let Some(c) = valid(cs, epoch, b, ip).filter(|&c| read32(src, c) == v as u32) {
            let v1 = read64(src, ip + 1);
            let h81 = hash(v1, 8, hl);
            let c1 = long[h81] as usize;
            long[h81] = (epoch + ip + 1) as u32;
            let (p, c, l) = match valid(c1, epoch, b, ip + 1).filter(|&c| read64(src, c) == v1) {
                Some(c1) => (ip + 1, c1, 8 + count(src, c1 + 8, ip + 9, b.end)),
                None => (ip, c, 4 + count(src, c + 4, ip + 4, b.end)),
            };
            let (s, l) = extend_back(src, p, p - c, l, anchor, b.low);
            Some((s, p - c, l))
        } else {
            b.dict.and_then(|d| dict_candidate(b, d.hash2[hash(v, mls, d.params.chain_log)], ip, v, anchor, 4))
        };
        let Some((s, off, ml)) = found else {
            ip += ((ip - anchor) >> 8) + 1;
            continue;
        };
        store.push(&src[anchor..s], off as u32, ml as u32);
        ip = s + ml;
        anchor = ip;
        if ip < ilimit {
            let c0 = s + 2;
            if c0 + 8 <= b.end {
                let w = read64(src, c0);
                long[hash(w, 8, hl)] = (epoch + c0) as u32;
                short[hash(w, mls, hs)] = (epoch + c0) as u32;
            }
            long[hash(read64(src, ip - 2), 8, hl)] = (epoch + ip - 2) as u32;
            short[hash(read64(src, ip - 1), mls, hs)] = (epoch + ip - 1) as u32;
            loop {
                let r1 = store.reps[1] as usize;
                let rl = if ip < ilimit { rep_len(b, ip, r1) } else { 0 };
                if rl == 0 {
                    break;
                }
                let w = read64(src, ip);
                long[hash(w, 8, hl)] = (epoch + ip) as u32;
                short[hash(w, mls, hs)] = (epoch + ip) as u32;
                store.push(&[], r1 as u32, rl as u32);
                ip += rl;
                anchor = ip;
            }
        }
        r0 = store.reps[0] as usize;
    }
    store.tail(&src[anchor..b.end]);
}

/// With a dictionary: repeat 0 reaching into it at `ip + 1`, else its 8-byte table at `ip`.
#[inline(always)]
fn dict_long(b: &BlockInput<'_>, r0: usize, ip: usize, v: u64, anchor: usize) -> Option<(usize, usize, usize)> {
    let d = b.dict?;
    if r0 > ip + 1 - b.low {
        let rl = rep_len(b, ip + 1, r0);
        if rl > 0 {
            return Some((ip + 1, r0, rl));
        }
    }
    dict_candidate(b, d.hash[hash(v, 8, d.params.hash_log)], ip, v, anchor, 8)
}
