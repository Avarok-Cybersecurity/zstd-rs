//! Optimal parsing (btopt / btultra / btultra2 levels): a forward price DP over
//! match candidates with per-node repeat-offset history, then backtracking.
mod matches;
pub mod price;

use super::hc::Chain;
use super::{BlockInput, Tables};
use crate::encode::entropy::DictCTables;
use crate::encode::params::{CParams, Strategy};
use crate::encode::seqstore::{code_offset, Reps, SeqStore};
use alloc::vec;
use alloc::vec::Vec;
use matches::{collect, Cand};
use price::Stats;

const OPT_NUM: usize = 1 << 12;

#[derive(Clone, Copy)]
struct Node {
    price: i32,
    off: u32,
    mlen: u32,
    litlen: u32,
    reps: Reps,
}

const MAX_PRICE: i32 = 1 << 30;
const EMPTY: Node = Node { price: MAX_PRICE, off: 0, mlen: 0, litlen: 1, reps: [0; 3] };

/// Parser state kept across blocks of a frame.
pub struct OptState {
    pub stats: Stats,
    nodes: Vec<Node>,
    cands: Vec<Cand>,
    h3: Vec<u32>,
    /// True until the first block of the current frame has been parsed.
    pub first_block: bool,
}

impl OptState {
    pub fn new() -> OptState {
        OptState { stats: Stats::new(), nodes: vec![EMPTY; OPT_NUM + 8], cands: Vec::new(), h3: Vec::new(), first_block: true }
    }
}

/// Parses one block with the optimal parser. `dict` supplies initial statistics.
pub fn parse(t: &mut Tables, b: &BlockInput<'_>, p: &CParams, store: &mut SeqStore, st: &mut OptState, dict: Option<&DictCTables>) {
    let ultra = p.strategy >= Strategy::BtUltra;
    let first = st.first_block;
    st.first_block = false;
    st.stats.rescale(&b.src[b.start..b.end], first, dict, ultra);
    if p.strategy == Strategy::BtUltra2 && first && dict.is_none() && b.start == b.low && b.end - b.start > 8 {
        let (reps, mark, lit_mark) = (store.reps, store.seqs.len(), store.lits.len());
        run(t, b, p, store, st);
        store.seqs.truncate(mark);
        store.lits.truncate(lit_mark);
        store.reps = reps;
        t.epoch += (b.end + 1) as u32;
        t.next_insert = b.low;
        st.h3.iter_mut().for_each(|x| *x = 0);
    }
    run(t, b, p, store, st);
}

fn run(t: &mut Tables, b: &BlockInput<'_>, p: &CParams, store: &mut SeqStore, st: &mut OptState) {
    let src = b.src;
    let ultra = p.strategy >= Strategy::BtUltra;
    let min = if p.min_match <= 3 { 3 } else { 4 };
    let chain = Chain { hash_log: p.hash_log, chain_log: p.chain_log, mls: super::hc::chain_mls(p), attempts: 1 << p.search_log };
    let sufficient = (p.target_length as usize).clamp(8, OPT_NUM - 1);
    if min == 3 {
        let log = p.window_log.min(17);
        if st.h3.len() != 1 << log {
            st.h3 = vec![0; 1 << log];
        }
    }
    let ilimit = b.end.saturating_sub(8);
    let mut ip = b.start;
    let mut anchor = b.start;
    if ip == b.low && b.dict.is_none() {
        ip += 1;
    }
    t.next_insert = t.next_insert.max(b.low);
    let OptState { stats, nodes, cands, h3, .. } = st;
    let mut h3 = if min == 3 { Some(&mut h3[..]) } else { None };
    let mut path: Vec<(usize, Cand)> = Vec::new();
    while ip < ilimit {
        let litlen = (ip - anchor) as u32;
        let reps = store.reps;
        collect(t, b, ip, &reps, litlen == 0, min, &chain, h3.as_deref_mut(), cands);
        let Some(&longest) = cands.last() else {
            ip += 1;
            continue;
        };
        if longest.len as usize > sufficient {
            emit(store, stats, src, &mut anchor, ip, &[(0, longest)]);
            ip = anchor;
            continue;
        }
        let ll0_price = stats.lit_len(0) as i32;
        nodes[0] = Node { price: stats.lit_len(litlen) as i32, off: 0, mlen: 0, litlen, reps };
        let mut last = longest.len as usize;
        for n in nodes[1..=last + 1].iter_mut() {
            *n = EMPTY;
        }
        let base0 = nodes[0].price + ll0_price;
        fill(nodes, stats, cands, 0, base0, litlen, &reps, min, ultra, &mut last);
        let mut stretch: Option<(usize, Cand)> = None;
        let mut cur = 1;
        while cur <= last {
            let inr = ip + cur;
            let prev = nodes[cur - 1];
            let ll = prev.litlen + 1;
            let lit_price = prev.price + stats.lit(src[inr - 1]) as i32 + stats.lit_len(ll) as i32 - stats.lit_len(ll - 1) as i32;
            if lit_price <= nodes[cur].price {
                nodes[cur] = Node { price: lit_price, off: 0, mlen: 0, litlen: ll, reps: prev.reps };
            } else if nodes[cur].litlen == 0 {
                let from = cur - nodes[cur].mlen as usize;
                let mut r = nodes[from].reps;
                code_offset(nodes[cur].off, nodes[from].litlen, &mut r);
                nodes[cur].reps = r;
            }
            if inr >= ilimit || cur == last {
                cur += 1;
                continue;
            }
            let here = nodes[cur];
            collect(t, b, inr, &here.reps, here.litlen == 0, min, &chain, h3.as_deref_mut(), cands);
            if let Some(&lg) = cands.last() {
                if lg.len as usize > sufficient || cur + lg.len as usize >= OPT_NUM || inr + lg.len as usize >= b.end {
                    stretch = Some((cur, lg));
                    break;
                }
                fill(nodes, stats, cands, cur, here.price + ll0_price, here.litlen, &here.reps, min, ultra, &mut last);
            }
            cur += 1;
        }
        path.clear();
        let (mut end, reach) = match stretch {
            Some((at, lg)) => {
                path.push((at, lg));
                (at, at + lg.len as usize)
            }
            None => (last, last),
        };
        while end > 0 {
            let n = nodes[end];
            if n.litlen != 0 {
                end -= 1;
            } else {
                end -= n.mlen as usize;
                path.push((end, Cand { off: n.off, len: n.mlen }));
            }
        }
        path.reverse();
        emit(store, stats, src, &mut anchor, ip, &path);
        ip += reach.max(1);
    }
    store.tail(&src[anchor..b.end]);
}

/// Records the prices of every match length reachable from node `cur`.
#[allow(clippy::too_many_arguments)]
fn fill(
    nodes: &mut [Node],
    stats: &Stats,
    cands: &[Cand],
    cur: usize,
    base: i32,
    litlen: u32,
    reps: &Reps,
    min: usize,
    ultra: bool,
    last: &mut usize,
) {
    let mut start = min;
    for c in cands {
        let ob = code_offset(c.off, litlen, &mut reps.clone());
        for len in start..=c.len as usize {
            let pos = cur + len;
            let price = base + stats.matched(ob, len as u32, ultra) as i32;
            while *last < pos {
                *last += 1;
                nodes[*last] = EMPTY;
            }
            if price < nodes[pos].price {
                nodes[pos] = Node { price, off: c.off, mlen: len as u32, litlen: 0, reps: *reps };
            }
        }
        start = c.len as usize + 1;
    }
    nodes[*last + 1] = EMPTY;
}

/// Stores the chosen matches (positions relative to `ip`) and updates statistics.
fn emit(store: &mut SeqStore, stats: &mut Stats, src: &[u8], anchor: &mut usize, ip: usize, path: &[(usize, Cand)]) {
    for &(at, c) in path {
        let s = ip + at;
        let lits = &src[*anchor..s];
        let before = store.seqs.len();
        store.push(lits, c.off, c.len);
        let ob = store.seqs[before].off_base;
        stats.update(lits, ob, c.len);
        *anchor = s + c.len as usize;
    }
    stats.commit();
}
