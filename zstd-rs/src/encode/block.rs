//! One block: parse, entropy-code, and keep it only if it beats a raw block.
use super::entropy::{DictCTables, EntropyState, Held};
use super::literals;
use super::matchers::{self, BlockInput};
use super::sequences::{self, SeqChoice};
use super::Workspace;
use crate::error::Result;
use crate::frame::{write_block_header, BlockType};
use alloc::vec::Vec;

/// Compresses `b.src[b.start..b.end]` as one block appended to `out`.
/// Entropy tables and repeat offsets are committed only if the block is kept compressed.
pub fn compress(
    w: &mut Workspace,
    b: &BlockInput<'_>,
    dict_tables: Option<&DictCTables>,
    last: bool,
    allow_rle: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    let raw = &b.src[b.start..b.end];
    let p = w.params;
    let sidx = p.strategy as u32 + 1;
    w.store.reset(w.reps);
    matchers::parse(&mut w.tables, b, &p, &mut w.store, &mut w.opt, dict_tables);
    let at = out.len();
    out.extend_from_slice(&[0; 3]);
    let huf = literals::encode(&w.store.lits, &w.entropy, dict_tables, sidx, out)?;
    let seq = sequences::encode(&w.store.seqs, &mut w.codes, &w.entropy, &w.pre, dict_tables, out)?;
    let csize = out.len() - at - 3;
    let min_gain = literals::min_gain(raw.len(), sidx);
    if csize + min_gain >= raw.len() || csize >= crate::frame::BLOCK_MAX {
        out.truncate(at);
        if allow_rle && raw.len() > 1 && raw.iter().all(|&x| x == raw[0]) {
            write_block_header(last, BlockType::Rle, raw.len(), out);
            out.push(raw[0]);
        } else {
            write_block_header(last, BlockType::Raw, raw.len(), out);
            out.extend_from_slice(raw);
        }
        return Ok(());
    }
    let mut hdr = Vec::with_capacity(3);
    write_block_header(last, BlockType::Compressed, csize, &mut hdr);
    out[at..at + 3].copy_from_slice(&hdr);
    commit(&mut w.entropy, huf, seq);
    w.reps = w.store.reps;
    Ok(())
}

fn commit(e: &mut EntropyState, huf: Option<crate::huf::ctable::HufCTable>, seq: [SeqChoice; 3]) {
    if let Some(h) = huf {
        e.huf_local = Some(h);
        e.huf_held = Held::Local;
    }
    for (i, c) in seq.into_iter().enumerate() {
        match c {
            SeqChoice::Predefined => e.seq_held[i] = Held::Predefined,
            SeqChoice::Rle(s) => e.seq_held[i] = Held::Rle(s),
            SeqChoice::New(t) => {
                e.seq_local[i] = Some(*t);
                e.seq_held[i] = Held::Local;
            }
            SeqChoice::Repeat => {}
        }
    }
}
