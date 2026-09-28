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
    while ip < ilimit {
        let v = read64(src, ip);
        let h = hash(v, mls, hlog);
        let cand = table[h] as usize;
        table[h] = (epoch + ip) as u32;
        let ml;
        let r0 = store.reps[0] as usize;
        let rl = rep_len(b, ip + 1, r0);
        if rl > 0 {
            ml = rl;
            store.push(&src[anchor..ip + 1], r0 as u32, ml as u32);
            ip += 1 + ml;
        } else {
            let c = cand.wrapping_sub(epoch);
            if cand >= epoch + b.low && c < ip && ip - c <= b.max_dist && read32(src, c) == v as u32 {
                let (s, l) = extend_back(src, ip, ip - c, 4 + count(src, c + 4, ip + 4, b.end), anchor, b.low);
                ml = l;
                store.push(&src[anchor..s], (ip - c) as u32, ml as u32);
                ip = s + ml;
            } else if let Some((s, off, l)) =
                b.dict.and_then(|d| dict_candidate(b, d.hash[hash(v, mls, d.params.hash_log)], ip, v, anchor, 4))
            {
                ml = l;
                store.push(&src[anchor..s], off as u32, ml as u32);
                ip = s + ml;
            } else {
                ip += ((ip - anchor) >> 8) + step;
                continue;
            }
        }
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
    }
    store.tail(&src[anchor..b.end]);
}
