//! Greedy (depth 0), lazy (1) and lazy2 (2) parsing over hash chains, with the
//! reference's gain heuristics for deferring a match by one or two positions.
use super::hc::{best, Chain};
use super::{extend_back, rep_len, BlockInput, Tables};
use crate::encode::params::CParams;
use crate::encode::seqstore::SeqStore;

#[inline(always)]
fn hb(v: usize) -> i64 {
    (usize::BITS - 1 - v.leading_zeros()) as i64
}

/// Current best candidate: start, length, raw offset, and whether it is repeat 0.
#[derive(Clone, Copy)]
struct Cand {
    start: usize,
    len: usize,
    off: usize,
    rep: bool,
}

impl Cand {
    fn off_base(&self) -> usize {
        if self.rep {
            1
        } else {
            self.off + 3
        }
    }
}

pub fn parse(t: &mut Tables, b: &BlockInput<'_>, p: &CParams, store: &mut SeqStore, depth: u32) {
    let src = b.src;
    let chain = Chain { hash_log: p.hash_log, chain_log: p.chain_log, mls: super::hc::chain_mls(p), attempts: 1 << p.search_log };
    let ilimit = b.end.saturating_sub(8);
    let mut ip = b.start;
    let mut anchor = b.start;
    if ip == b.low && b.dict.is_none() {
        ip += 1;
    }
    t.next_insert = t.next_insert.max(b.low);
    while ip < ilimit {
        let r0 = store.reps[0] as usize;
        let mut c = Cand { start: ip + 1, len: 0, off: r0, rep: true };
        let rl = rep_len(b, ip + 1, r0);
        if rl > 0 {
            c.len = rl;
        }
        if depth > 0 || rl == 0 {
            let (l, o) = best(t, b, ip, &chain, 3);
            if l > c.len {
                c = Cand { start: ip, len: l, off: o, rep: false };
            }
        }
        if c.len < 4 {
            ip += ((ip - anchor) >> 8) + 1;
            continue;
        }
        let mut step = 0;
        while depth > 0 && ip < ilimit {
            ip += 1;
            step += 1;
            let rl = rep_len(b, ip, r0);
            if rl >= 4 {
                let (g2, g1) = if step == 1 {
                    (rl as i64 * 3, c.len as i64 * 3 - hb(c.off_base()) + 1)
                } else {
                    (rl as i64 * 4, c.len as i64 * 4 - hb(c.off_base()) + 1)
                };
                if g2 > g1 {
                    c = Cand { start: ip, len: rl, off: r0, rep: true };
                }
            }
            let (l, o) = best(t, b, ip, &chain, 3);
            let bonus = if step == 1 { 4 } else { 7 };
            if l >= 4 && (l as i64 * 4 - hb(o + 3)) > (c.len as i64 * 4 - hb(c.off_base()) + bonus) {
                c = Cand { start: ip, len: l, off: o, rep: false };
                step = 0;
                continue;
            }
            if depth == 2 && step == 1 {
                continue;
            }
            break;
        }
        let (start, len) =
            if c.rep || c.off > c.start - b.low { (c.start, c.len) } else { extend_back(src, c.start, c.off, c.len, anchor, b.low) };
        store.push(&src[anchor..start], c.off as u32, len as u32);
        ip = start + len;
        anchor = ip;
        while ip < ilimit {
            let r1 = store.reps[1] as usize;
            let rl = rep_len(b, ip, r1);
            if rl == 0 {
                break;
            }
            store.push(&[], r1 as u32, rl as u32);
            ip += rl;
            anchor = ip;
        }
    }
    store.tail(&src[anchor..b.end]);
}
