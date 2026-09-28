//! Hash-chain match search over the frame tables and the attached dictionary.
use super::{count, count_dict, hash, read64, BlockInput, Tables};

/// Hash-chain geometry for one search.
#[derive(Clone, Copy)]
pub struct Chain {
    pub hash_log: u32,
    pub chain_log: u32,
    pub mls: u32,
    pub attempts: u32,
}

/// Inserts every position below `target` not yet in the chain.
#[inline(always)]
pub fn insert_upto(t: &mut Tables, src: &[u8], target: usize, c: &Chain) {
    let epoch = t.epoch as usize;
    let mask = (1usize << c.chain_log) - 1;
    let lim = target.min(src.len().saturating_sub(7));
    let mut i = t.next_insert;
    while i < lim {
        let h = hash(read64(src, i), c.mls, c.hash_log);
        t.chain[(epoch + i) & mask] = t.hash[h];
        t.hash[h] = (epoch + i) as u32;
        i += 1;
    }
    t.next_insert = t.next_insert.max(target);
}

/// Calls `f(len, offset)` for candidates at `ip` in chain order (most recent first),
/// for every candidate at least as long as `min_len`. `f` returns the new minimum
/// length to report (or `usize::MAX` to stop).
#[inline(always)]
pub fn search(t: &mut Tables, b: &BlockInput<'_>, ip: usize, c: &Chain, min_len: usize, mut f: impl FnMut(usize, usize) -> usize) {
    insert_upto(t, b.src, ip, c);
    let src = b.src;
    let epoch = t.epoch as usize;
    let mask = (1usize << c.chain_log) - 1;
    let chain_size = mask + 1;
    let v = read64(src, ip);
    let mut best = min_len;
    let mut cur = t.hash[hash(v, c.mls, c.hash_log)] as usize;
    let mut attempts = c.attempts;
    let floor = epoch + b.low.max(ip.saturating_sub(b.max_dist));
    while cur >= floor && attempts > 0 {
        let m = cur - epoch;
        if m >= ip || ip - m >= chain_size {
            break;
        }
        if ip + best < b.end && src[m + best] == src[ip + best] {
            let len = count(src, m, ip, b.end);
            if len > best {
                best = f(len, ip - m);
                if best == usize::MAX {
                    return;
                }
            }
        }
        cur = t.chain[cur & mask] as usize;
        attempts -= 1;
    }
    let Some(d) = b.dict.filter(|_| b.low == 0) else { return };
    let dp = &d.params;
    let dmask = (1usize << dp.chain_log) - 1;
    let dlen = d.content.len();
    let mut e = d.hash[hash(v, c.mls, dp.hash_log)] as usize;
    let mut attempts = c.attempts;
    while e > 0 && attempts > 0 {
        let m = e - 1;
        if m + best < dlen && ip + best < b.end && d.content[m + best] == src[ip + best] {
            let len = count_dict(&d.content, m, src, ip, b.end);
            if len > best {
                best = f(len, ip + dlen - m);
                if best == usize::MAX {
                    return;
                }
            }
        }
        let next = d.chain[m & dmask] as usize;
        if next >= e {
            break;
        }
        e = next;
        attempts -= 1;
    }
}

/// Longest match at `ip` of at least `min_len + 1` bytes: (length, offset), or (0, 0).
#[inline(always)]
pub fn best(t: &mut Tables, b: &BlockInput<'_>, ip: usize, c: &Chain, min_len: usize) -> (usize, usize) {
    let mut out = (0, 0);
    search(t, b, ip, c, min_len, |len, off| {
        out = (len, off);
        if ip + len >= b.end {
            usize::MAX
        } else {
            len
        }
    });
    out
}

/// Bytes hashed by hash-chain strategies (the frame and its dictionary must agree).
pub fn chain_mls(p: &crate::encode::params::CParams) -> u32 {
    p.min_match.clamp(4, 6)
}
