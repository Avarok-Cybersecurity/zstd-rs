//! Literals section encoding: raw, RLE, Huffman with a new table, or Huffman
//! reusing the table the decoder already holds ("treeless"), chosen by size.
use super::entropy::{DictCTables, EntropyState};
use crate::error::Result;
use crate::huf::build::{build, depth_limit};
use crate::huf::ctable::HufCTable;
use crate::huf::weights;
use alloc::vec::Vec;

fn raw_header(ty: u8, n: usize, out: &mut Vec<u8>) {
    if n < 32 {
        out.push(ty | ((n as u8) << 3));
    } else if n < 4096 {
        out.extend_from_slice(&((ty as u16) | (1 << 2) | ((n as u16) << 4)).to_le_bytes());
    } else {
        let v = (ty as u32) | (3 << 2) | ((n as u32) << 4);
        out.extend_from_slice(&v.to_le_bytes()[..3]);
    }
}

/// Appends raw literals.
pub fn write_raw(lits: &[u8], out: &mut Vec<u8>) {
    raw_header(0, lits.len(), out);
    out.extend_from_slice(lits);
}

/// Minimum literal count worth Huffman-coding (reference `ZSTD_minLiteralsToCompress`).
fn min_to_compress(strategy_index: u32, have_table: bool) -> usize {
    if have_table {
        6
    } else {
        8 << (9u32.saturating_sub(strategy_index)).min(3)
    }
}

/// Minimum saving a compressed section must achieve (reference `ZSTD_minGain`).
pub fn min_gain(n: usize, strategy_index: u32) -> usize {
    let min_log = if strategy_index >= 8 { strategy_index - 1 } else { 6 };
    (n >> min_log) + 2
}

enum Choice {
    Raw,
    Repeat,
    New,
}

/// Appends the literals section. Returns a newly built table when the section
/// installs one in the decoder (the caller commits it only if the block is kept).
pub fn encode(
    lits: &[u8],
    st: &EntropyState,
    dict: Option<&DictCTables>,
    strategy_index: u32,
    out: &mut Vec<u8>,
) -> Result<Option<HufCTable>> {
    let n = lits.len();
    let prev = st.huf(dict);
    if n < min_to_compress(strategy_index, prev.is_some()) {
        write_raw(lits, out);
        return Ok(None);
    }
    let mut count = [0u32; 256];
    let mut present = [0u8; 257];
    let mut distinct = 0;
    for &b in lits {
        let c = &mut count[b as usize];
        present[distinct] = b;
        distinct += (*c == 0) as usize;
        *c += 1;
    }
    let present = &present[..distinct];
    let max_symbol = present.iter().copied().max().unwrap_or(0) as usize;
    if distinct == 1 {
        raw_header(1, n, out);
        out.push(lits[0]);
        return Ok(None);
    }
    let four = n >= 256;
    let lh = 3 + (n >= 1024) as usize + (n >= 16384) as usize;
    let stream_overhead = if four { 6 + 4 } else { 1 };
    let raw_cost = n + if n < 32 {
        1
    } else if n < 4096 {
        2
    } else {
        3
    };
    let repeat_cost = prev.and_then(|t| t.cost_bits(&count, present)).map(|b| lh + stream_overhead + (b as usize).div_ceil(8));
    if repeat_cost.is_some() && strategy_index < 4 && n <= 1024 {
        let table = prev.unwrap_or_else(|| unreachable!("repeat cost implies a table"));
        return emit(lits, table, None, 3, lh, four, raw_cost, strategy_index, out).map(|_| None);
    }
    let hw = build(&count, depth_limit(n, max_symbol));
    let fresh = HufCTable::new(&hw);
    let mut desc = Vec::new();
    let new_cost = match weights::write(&hw, &mut desc) {
        Ok(()) => fresh.cost_bits(&count, present).map(|b| lh + desc.len() + stream_overhead + (b as usize).div_ceil(8)),
        Err(_) => None,
    };
    let mut choice = Choice::Raw;
    let mut best = raw_cost;
    if let Some(c) = repeat_cost.filter(|&c| c < best) {
        choice = Choice::Repeat;
        best = c;
    }
    if new_cost.is_some_and(|c| c < best) {
        choice = Choice::New;
    }
    match choice {
        Choice::Raw => {
            write_raw(lits, out);
            Ok(None)
        }
        Choice::Repeat => {
            emit(lits, prev.unwrap_or(&fresh), None, 3, lh, four, raw_cost, strategy_index, out)?;
            Ok(None)
        }
        Choice::New => {
            let kept = emit(lits, &fresh, Some(&desc), 2, lh, four, raw_cost, strategy_index, out)?;
            Ok(if kept { Some(fresh) } else { None })
        }
    }
}

/// Writes a Huffman-coded section; falls back to raw (returning false) if it does not pay.
#[allow(clippy::too_many_arguments)]
fn emit(
    lits: &[u8],
    table: &HufCTable,
    desc: Option<&[u8]>,
    ty: u8,
    lh: usize,
    four: bool,
    raw_cost: usize,
    strategy_index: u32,
    out: &mut Vec<u8>,
) -> Result<bool> {
    let n = lits.len();
    let start = out.len();
    out.resize(start + lh, 0);
    if let Some(d) = desc {
        out.extend_from_slice(d);
    }
    if four {
        table.encode_4x(lits, out)?;
    } else {
        table.encode_1x(lits, out);
    }
    let comp = out.len() - start - lh;
    if comp + lh + min_gain(n, strategy_index) > raw_cost {
        out.truncate(start);
        write_raw(lits, out);
        return Ok(false);
    }
    let sf: u64 = match (four, lh) {
        (false, _) => 0,
        (true, 3) => 1,
        (true, 4) => 2,
        _ => 3,
    };
    let bits = [0, 0, 0, 10, 14, 18][lh];
    let v = ty as u64 | (sf << 2) | ((n as u64) << 4) | ((comp as u64) << (4 + bits));
    out[start..start + lh].copy_from_slice(&v.to_le_bytes()[..lh]);
    Ok(true)
}
