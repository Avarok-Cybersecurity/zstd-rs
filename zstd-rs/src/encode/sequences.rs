//! Sequences section encoding with per-stream table selection (predefined,
//! RLE, new FSE table, or repeat), chosen by estimated size.
use super::cost::{entropy_cost, norm_cost};
use super::entropy::{DictCTables, EntropyState, Held, PredefinedC, SeqCTable};
use super::seqstore::Seq;
use crate::bits::BitWriter;
use crate::decode::tables::{ll_code, ml_code, of_code, LL_CODES, ML_CODES};
use crate::error::Result;
use crate::fse::ctable::CState;
use crate::fse::{ncount, normalize};
use alloc::vec::Vec;

/// The table decision for one stream, applied to the entropy state on commit.
#[derive(Clone, Debug)]
pub enum SeqChoice {
    Predefined,
    Rle(u8),
    New(alloc::boxed::Box<SeqCTable>),
    Repeat,
}

const MAX_LOG: [u32; 3] = [9, 8, 9];
const DEFAULT_MAX: [usize; 3] = [35, 28, 52];

/// Scratch buffers for the symbol codes of one block.
#[derive(Default)]
pub struct Codes {
    pub c: [Vec<u8>; 3],
}

fn header(n: usize, out: &mut Vec<u8>) {
    if n < 128 {
        out.push(n as u8);
    } else if n < 0x7F00 {
        out.extend_from_slice(&[((n >> 8) + 128) as u8, n as u8]);
    } else {
        let v = (n - 0x7F00) as u16;
        out.push(255);
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Picks and writes the table for stream `i`; returns the mode bits and the choice.
fn choose(
    i: usize,
    codes: &[u8],
    st: &EntropyState,
    pre: &PredefinedC,
    dict: Option<&DictCTables>,
    out: &mut Vec<u8>,
) -> Result<(u8, SeqChoice)> {
    let n = codes.len();
    let mut count = [0u32; 64];
    for &c in codes {
        count[c as usize] += 1;
    }
    let max = count.iter().rposition(|&c| c > 0).unwrap_or(0);
    let count = &count[..=max];
    if count[max] as usize == n && (n > 2 || max > DEFAULT_MAX[i]) {
        if st.seq_held[i] == Held::Rle(max as u8) {
            return Ok((3, SeqChoice::Repeat));
        }
        out.push(max as u8);
        return Ok((1, SeqChoice::Rle(max as u8)));
    }
    let basic = if max <= DEFAULT_MAX[i] { norm_cost(&pre.t[i].norm, count) } else { None };
    let repeat = match st.seq_held[i] {
        Held::Rle(s) => (count.len() == s as usize + 1 && count[s as usize] as usize == n).then_some(0),
        _ => st.seq(i, pre, dict).and_then(|t| norm_cost(&t.norm, count)),
    };
    let mut best = (u64::MAX, 0u8);
    if let Some(c) = basic {
        best = (c, 0);
    }
    if let Some(c) = repeat.filter(|&c| c < best.0) {
        best = (c, 3);
    }
    let fresh = if n > 1 && entropy_cost(count, n as u32) + (24 << 8) < best.0 {
        let mut adj = [0u32; 64];
        adj[..count.len()].copy_from_slice(count);
        let last = codes[n - 1] as usize;
        let mut total = n;
        if adj[last] > 1 {
            adj[last] -= 1;
            total -= 1;
        }
        let log = normalize::optimal_table_log(MAX_LOG[i], total, max, 2);
        normalize::normalize(&adj[..=max], total, log, total >= 2048)
    } else {
        None
    };
    if let Some(norm) = fresh {
        let mut desc = Vec::new();
        ncount::write(&norm, &mut desc);
        if let Some(c) = norm_cost(&norm, count).map(|c| c + ((desc.len() as u64) << 11)) {
            if c < best.0 {
                out.extend_from_slice(&desc);
                return Ok((2, SeqChoice::New(alloc::boxed::Box::new(SeqCTable::new(norm)?))));
            }
        }
    }
    match best.1 {
        3 => Ok((3, SeqChoice::Repeat)),
        _ if basic.is_some() => Ok((0, SeqChoice::Predefined)),
        _ => {
            let norm = normalize::normalize(count, n, normalize::optimal_table_log(MAX_LOG[i], n, max, 2), n >= 2048)
                .ok_or(crate::Error::Parameter("sequence distribution not normalizable"))?;
            ncount::write(&norm, out);
            Ok((2, SeqChoice::New(alloc::boxed::Box::new(SeqCTable::new(norm)?))))
        }
    }
}

/// Appends the sequences section; returns the per-stream choices to commit.
pub fn encode(
    seqs: &[Seq],
    codes: &mut Codes,
    st: &EntropyState,
    pre: &PredefinedC,
    dict: Option<&DictCTables>,
    out: &mut Vec<u8>,
) -> Result<[SeqChoice; 3]> {
    let n = seqs.len();
    header(n, out);
    if n == 0 {
        return Ok([SeqChoice::Repeat, SeqChoice::Repeat, SeqChoice::Repeat]);
    }
    for c in codes.c.iter_mut() {
        c.clear();
    }
    for s in seqs {
        codes.c[0].push(ll_code(s.ll));
        codes.c[1].push(of_code(s.off_base));
        codes.c[2].push(ml_code(s.ml));
    }
    let mode_at = out.len();
    out.push(0);
    let mut modes = 0u8;
    let mut choices: [SeqChoice; 3] = [SeqChoice::Repeat, SeqChoice::Repeat, SeqChoice::Repeat];
    for (i, slot) in choices.iter_mut().enumerate() {
        let (m, ch) = choose(i, &codes.c[i], st, pre, dict, out)?;
        modes |= m << (6 - 2 * i);
        *slot = ch;
    }
    out[mode_at] = modes;
    let table = |i: usize| -> Option<&SeqCTable> {
        match &choices[i] {
            SeqChoice::Predefined => Some(&pre.t[i]),
            SeqChoice::New(t) => Some(&**t),
            SeqChoice::Repeat => st.seq(i, pre, dict),
            SeqChoice::Rle(_) => None,
        }
    };
    let (ll_t, of_t, ml_t) = (table(0), table(1), table(2));
    let mut w = BitWriter::new(out);
    let init = |t: Option<&SeqCTable>, c: u8| t.map(|t| CState::init(&t.ct, c as usize));
    let last = n - 1;
    let (mut ll_s, mut of_s, mut ml_s) = (init(ll_t, codes.c[0][last]), init(of_t, codes.c[1][last]), init(ml_t, codes.c[2][last]));
    for k in (0..n).rev() {
        let (llc, ofc, mlc) = (codes.c[0][k], codes.c[1][k], codes.c[2][k]);
        if k != last {
            if let (Some(s), Some(t)) = (of_s.as_mut(), of_t) {
                s.encode(&t.ct, &mut w, ofc as usize);
            }
            if let (Some(s), Some(t)) = (ml_s.as_mut(), ml_t) {
                s.encode(&t.ct, &mut w, mlc as usize);
            }
            if let (Some(s), Some(t)) = (ll_s.as_mut(), ll_t) {
                s.encode(&t.ct, &mut w, llc as usize);
            }
        }
        let s = &seqs[k];
        let (llb, lln) = LL_CODES[llc as usize];
        let (mlb, mln) = ML_CODES[mlc as usize];
        w.add((s.ll - llb) as u64, lln as u32);
        w.add((s.ml - mlb) as u64, mln as u32);
        w.add((s.off_base - (1 << ofc)) as u64, ofc as u32);
    }
    for (s, t) in [(ml_s, ml_t), (of_s, of_t), (ll_s, ll_t)] {
        if let (Some(s), Some(t)) = (s, t) {
            s.flush(&t.ct, &mut w);
        }
    }
    w.finish();
    Ok(choices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_header_sizes() {
        for (n, len) in [(0usize, 1usize), (127, 1), (128, 2), (0x7EFF, 2), (0x7F00, 3), (0x17EFF, 3)] {
            let mut v = Vec::new();
            header(n, &mut v);
            assert_eq!(v.len(), len, "{n}");
        }
    }
}
