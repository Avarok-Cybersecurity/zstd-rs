//! Match finders. All share one model: the frame input `src`, a block range
//! `[start, end)`, the lowest usable history index `low`, a maximum distance,
//! and an optional attached dictionary whose tables were built in advance.
pub mod dfast;
pub mod fast;
pub mod hc;
pub mod lazy;
pub mod opt;
mod prim;

pub use prim::{count, count_dict, hash, read32, read64};

use super::dictionary::DictMatchState;
use super::seqstore::SeqStore;

/// Everything a match finder needs to parse one block.
pub struct BlockInput<'a> {
    pub src: &'a [u8],
    pub start: usize,
    pub end: usize,
    /// Lowest index of `src` usable as match history.
    pub low: usize,
    /// Largest match distance allowed within `src`.
    pub max_dist: usize,
    /// Attached dictionary (only when `low == 0`: the dictionary precedes `src[0]`).
    /// As in the reference, every dictionary position stays referenceable while the
    /// block ends within the window, so the caller drops it for later blocks.
    pub dict: Option<&'a DictMatchState>,
}

/// Frame-local match tables. Stored positions are `epoch + index`, so tables
/// never need clearing between frames: entries below `epoch` are stale.
pub struct Tables {
    pub hash: alloc::vec::Vec<u32>,
    pub hash2: alloc::vec::Vec<u32>,
    pub chain: alloc::vec::Vec<u32>,
    pub epoch: u32,
    /// Next index to insert into the hash chain (lazy/opt strategies).
    pub next_insert: usize,
}

impl Tables {
    pub fn new() -> Tables {
        Tables { hash: alloc::vec::Vec::new(), hash2: alloc::vec::Vec::new(), chain: alloc::vec::Vec::new(), epoch: 1, next_insert: 0 }
    }

    /// Prepares for a new frame of `len` bytes with the given table sizes.
    pub fn start_frame(&mut self, len: usize, hash_log: u32, hash2_log: u32, chain_log: u32, prev_len: usize) {
        let need = |v: &mut alloc::vec::Vec<u32>, log: u32| {
            if log > 0 && v.len() < 1 << log {
                v.clear();
                v.resize(1 << log, 0);
            }
        };
        let next = self.epoch as u64 + prev_len as u64 + (1 << 18);
        if next + len as u64 + 1 >= u32::MAX as u64 - (1 << 20) {
            self.hash.iter_mut().for_each(|x| *x = 0);
            self.hash2.iter_mut().for_each(|x| *x = 0);
            self.chain.iter_mut().for_each(|x| *x = 0);
            self.epoch = 1;
        } else {
            self.epoch = next as u32;
        }
        need(&mut self.hash, hash_log);
        need(&mut self.hash2, hash2_log);
        need(&mut self.chain, chain_log);
        self.next_insert = 0;
    }
}

/// Tries the repeat offset `rep` at `ip` (within `src` or reaching into the dictionary).
#[inline(always)]
pub fn rep_len(b: &BlockInput<'_>, ip: usize, rep: usize) -> usize {
    if rep == 0 {
        return 0;
    }
    if rep <= ip - b.low {
        if rep > b.max_dist {
            return 0;
        }
        let m = ip - rep;
        if read32(b.src, m) == read32(b.src, ip) {
            return 4 + count(b.src, m + 4, ip + 4, b.end);
        }
        return 0;
    }
    match b.dict {
        Some(d) if b.low == 0 && rep - ip <= d.content.len() => {
            let at = d.content.len() - (rep - ip);
            let n = count_dict(&d.content, at, b.src, ip, b.end);
            if n >= 4 {
                n
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// A dictionary table entry checked against `ip` (`min` = 4 or 8 bytes compared);
/// returns (start, offset, length) after extending the match backwards.
#[inline(always)]
pub fn dict_candidate(b: &BlockInput<'_>, entry: u32, ip: usize, v: u64, anchor: usize, min: usize) -> Option<(usize, usize, usize)> {
    let d = b.dict.filter(|_| b.low == 0 && entry != 0)?;
    let m = entry as usize - 1;
    let dlen = d.content.len();
    if m + min > dlen {
        return None;
    }
    let eq = if min == 8 { read64(&d.content, m) == v } else { read32(&d.content, m) == v as u32 };
    if !eq {
        return None;
    }
    let mut ml = count_dict(&d.content, m, b.src, ip, b.end);
    let (mut s, mut mm) = (ip, m);
    while s > anchor && mm > 0 && b.src[s - 1] == d.content[mm - 1] {
        s -= 1;
        mm -= 1;
        ml += 1;
    }
    Some((s, s + dlen - mm, ml))
}

/// Extends a within-`src` match at `s` with distance `off` backwards to `anchor`.
#[inline(always)]
pub fn extend_back(src: &[u8], mut s: usize, off: usize, mut ml: usize, anchor: usize, low: usize) -> (usize, usize) {
    while s > anchor && s - off > low && src[s - 1] == src[s - off - 1] {
        s -= 1;
        ml += 1;
    }
    (s, ml)
}

/// Inserts `src[low..start]` (history from an earlier job) into the tables of the
/// hash-table strategies; hash-chain strategies insert lazily from `low`.
pub fn load_prefix(t: &mut Tables, src: &[u8], low: usize, start: usize, p: &super::params::CParams) {
    use super::params::Strategy::*;
    t.next_insert = low;
    let epoch = t.epoch as usize;
    let mls = p.min_match.clamp(4, 7);
    let end = start.min(src.len().saturating_sub(8));
    for i in low..end {
        let v = read64(src, i);
        match p.strategy {
            Fast => t.hash[hash(v, mls, p.hash_log)] = (epoch + i) as u32,
            DFast => {
                t.hash[hash(v, 8, p.hash_log)] = (epoch + i) as u32;
                t.hash2[hash(v, mls, p.chain_log)] = (epoch + i) as u32;
            }
            _ => return,
        }
    }
}

/// Runs the configured strategy over one block, appending to `store`.
pub fn parse(
    t: &mut Tables,
    b: &BlockInput<'_>,
    p: &super::params::CParams,
    store: &mut SeqStore,
    opt: &mut opt::OptState,
    stats_from: Option<&super::entropy::DictCTables>,
) {
    use super::params::Strategy::*;
    match p.strategy {
        Fast => fast::parse(t, b, p, store),
        DFast => dfast::parse(t, b, p, store),
        Greedy => lazy::parse(t, b, p, store, 0),
        Lazy => lazy::parse(t, b, p, store, 1),
        Lazy2 | BtLazy2 => lazy::parse(t, b, p, store, 2),
        BtOpt | BtUltra | BtUltra2 => opt::parse(t, b, p, store, opt, stats_from),
    }
}
