//! Compression parameters: the reference level tables (so levels mean what they
//! mean in zstd) and the reference rules for shrinking them to the input size.

/// Match-finding strategy, in increasing order of effort.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strategy {
    Fast,
    DFast,
    Greedy,
    Lazy,
    Lazy2,
    BtLazy2,
    BtOpt,
    BtUltra,
    BtUltra2,
}

/// Resolved parameters for one frame (or one dictionary's tables).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CParams {
    pub window_log: u32,
    pub chain_log: u32,
    pub hash_log: u32,
    pub search_log: u32,
    pub min_match: u32,
    pub target_length: u32,
    pub strategy: Strategy,
}

use Strategy::*;
type Row = (u32, u32, u32, u32, u32, u32, Strategy);

#[rustfmt::skip]
const TABLES: [[Row; 23]; 4] = [
    [(19,12,13,1,6,1,Fast),(19,13,14,1,7,0,Fast),(20,15,16,1,6,0,Fast),(21,16,17,1,5,0,DFast),(21,18,18,1,5,0,DFast),
     (21,18,19,3,5,2,Greedy),(21,18,19,3,5,4,Lazy),(21,19,20,4,5,8,Lazy),(21,19,20,4,5,16,Lazy2),(22,20,21,4,5,16,Lazy2),
     (22,21,22,5,5,16,Lazy2),(22,21,22,6,5,16,Lazy2),(22,22,23,6,5,32,Lazy2),(22,22,22,4,5,32,BtLazy2),(22,22,23,5,5,32,BtLazy2),
     (22,23,23,6,5,32,BtLazy2),(22,22,22,5,5,48,BtOpt),(23,23,22,5,4,64,BtOpt),(23,23,22,6,3,64,BtUltra),(23,24,22,7,3,256,BtUltra2),
     (25,25,23,7,3,256,BtUltra2),(26,26,24,7,3,512,BtUltra2),(27,27,25,9,3,999,BtUltra2)],
    [(18,12,13,1,5,1,Fast),(18,13,14,1,6,0,Fast),(18,14,14,1,5,0,DFast),(18,16,16,1,4,0,DFast),(18,16,17,3,5,2,Greedy),
     (18,17,18,5,5,2,Greedy),(18,18,19,3,5,4,Lazy),(18,18,19,4,4,4,Lazy),(18,18,19,4,4,8,Lazy2),(18,18,19,5,4,8,Lazy2),
     (18,18,19,6,4,8,Lazy2),(18,18,19,5,4,12,BtLazy2),(18,19,19,7,4,12,BtLazy2),(18,18,19,4,4,16,BtOpt),(18,18,19,4,3,32,BtOpt),
     (18,18,19,6,3,128,BtOpt),(18,19,19,6,3,128,BtUltra),(18,19,19,8,3,256,BtUltra),(18,19,19,6,3,128,BtUltra2),(18,19,19,8,3,256,BtUltra2),
     (18,19,19,10,3,512,BtUltra2),(18,19,19,12,3,512,BtUltra2),(18,19,19,13,3,999,BtUltra2)],
    [(17,12,12,1,5,1,Fast),(17,12,13,1,6,0,Fast),(17,13,15,1,5,0,Fast),(17,15,16,2,5,0,DFast),(17,17,17,2,4,0,DFast),
     (17,16,17,3,4,2,Greedy),(17,16,17,3,4,4,Lazy),(17,16,17,3,4,8,Lazy2),(17,16,17,4,4,8,Lazy2),(17,16,17,5,4,8,Lazy2),
     (17,16,17,6,4,8,Lazy2),(17,17,17,5,4,8,BtLazy2),(17,18,17,7,4,12,BtLazy2),(17,18,17,3,4,12,BtOpt),(17,18,17,4,3,32,BtOpt),
     (17,18,17,6,3,256,BtOpt),(17,18,17,6,3,128,BtUltra),(17,18,17,8,3,256,BtUltra),(17,18,17,10,3,512,BtUltra),(17,18,17,5,3,256,BtUltra2),
     (17,18,17,7,3,512,BtUltra2),(17,18,17,9,3,512,BtUltra2),(17,18,17,11,3,999,BtUltra2)],
    [(14,12,13,1,5,1,Fast),(14,14,15,1,5,0,Fast),(14,14,15,1,4,0,Fast),(14,14,15,2,4,0,DFast),(14,14,14,4,4,2,Greedy),
     (14,14,14,3,4,4,Lazy),(14,14,14,4,4,8,Lazy2),(14,14,14,6,4,8,Lazy2),(14,14,14,8,4,8,Lazy2),(14,15,14,5,4,8,BtLazy2),
     (14,15,14,9,4,8,BtLazy2),(14,15,14,3,4,12,BtOpt),(14,15,14,4,3,24,BtOpt),(14,15,14,5,3,32,BtUltra),(14,15,15,6,3,64,BtUltra),
     (14,15,15,7,3,256,BtUltra),(14,15,15,5,3,48,BtUltra2),(14,15,15,6,3,128,BtUltra2),(14,15,15,7,3,256,BtUltra2),(14,15,15,8,3,256,BtUltra2),
     (14,15,15,8,3,512,BtUltra2),(14,15,15,9,3,512,BtUltra2),(14,15,15,10,3,999,BtUltra2)],
];

/// Lowest (fastest) accepted level.
pub const MIN_LEVEL: i32 = -7;
/// Highest (strongest) accepted level.
pub const MAX_LEVEL: i32 = 22;

fn highbit(v: u64) -> u32 {
    63 - v.leading_zeros()
}

/// Looks up `level` for an input of `size` bytes (the reference `ZSTD_getCParams`).
/// Negative levels use the fast strategy with acceleration `-level` as target length.
pub fn lookup(level: i32, size: u64) -> CParams {
    let table = (size <= 256 << 10) as usize + (size <= 128 << 10) as usize + (size <= 16 << 10) as usize;
    let row = if level < 0 { 0 } else { level.clamp(1, MAX_LEVEL) as usize };
    let (w, c, h, s, m, t, st) = TABLES[table][row];
    let target_length = if level < 0 { (-level) as u32 } else { t };
    CParams { window_log: w, chain_log: c, hash_log: h, search_log: s, min_match: m, target_length, strategy: st }
}

/// Shrinks window, hash and chain sizes to fit `src + dict` bytes (reference
/// `ZSTD_adjustCParams_internal` for a known source size).
pub fn adjust(mut p: CParams, src: u64, dict: u64) -> CParams {
    let total = src + dict;
    let src_log = if total < 64 { 6 } else { highbit(total - 1) + 1 };
    if p.window_log > src_log {
        p.window_log = src_log;
    }
    let dw_log = if dict == 0 {
        p.window_log
    } else {
        let wsize = 1u64 << p.window_log;
        if wsize >= dict + src {
            p.window_log
        } else {
            (highbit(dict + wsize - 1) + 1).min(31)
        }
    };
    let cycle_log = p.chain_log - (p.strategy >= Strategy::BtLazy2) as u32;
    if p.hash_log > dw_log + 1 {
        p.hash_log = dw_log + 1;
    }
    if cycle_log > dw_log {
        p.chain_log -= cycle_log - dw_log;
    }
    p.window_log = p.window_log.max(10);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_inputs_shrink_tables() {
        let p = adjust(lookup(3, 100), 100, 0);
        assert_eq!((p.window_log, p.strategy), (10, Strategy::DFast));
        assert!(p.hash_log <= 8 && p.chain_log <= 7);
        let big = adjust(lookup(19, 10 << 20), 10 << 20, 0);
        assert_eq!((big.window_log, big.strategy), (23, Strategy::BtUltra2));
        assert_eq!(lookup(-3, 1 << 30).target_length, 3);
    }
}
