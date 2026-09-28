//! Encoder-side entropy state: which tables the decoder currently holds (so
//! "repeat" modes can be chosen), and the tables a dictionary provides.
use crate::decode::tables::*;
use crate::dict::DictTables;
use crate::error::Result;
use crate::fse::ctable::CTable;
use crate::fse::Norm;
use crate::huf::ctable::HufCTable;

/// A sequence table: the distribution (for coverage checks and costs) and its encoder.
#[derive(Clone, Debug)]
pub struct SeqCTable {
    pub norm: Norm,
    pub ct: CTable,
}

impl SeqCTable {
    pub fn new(norm: Norm) -> Result<SeqCTable> {
        let ct = CTable::build(&norm)?;
        Ok(SeqCTable { norm, ct })
    }
}

/// Which table the decoder will use if a block says "repeat".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    None,
    Predefined,
    Dict,
    Local,
    Rle(u8),
}

/// The three predefined tables, built once per compressor.
pub struct PredefinedC {
    pub t: [SeqCTable; 3],
}

impl PredefinedC {
    pub fn new() -> Result<PredefinedC> {
        Ok(PredefinedC {
            t: [
                SeqCTable::new(Norm::from_slice(&LL_DEFAULT, LL_DEFAULT_LOG))?,
                SeqCTable::new(Norm::from_slice(&OF_DEFAULT, OF_DEFAULT_LOG))?,
                SeqCTable::new(Norm::from_slice(&ML_DEFAULT, ML_DEFAULT_LOG))?,
            ],
        })
    }
}

/// Encoder tables built from a zstd-format dictionary.
#[derive(Clone, Debug)]
pub struct DictCTables {
    pub huf: HufCTable,
    pub seq: [SeqCTable; 3],
}

impl DictCTables {
    pub fn new(t: &DictTables) -> Result<DictCTables> {
        Ok(DictCTables {
            huf: HufCTable::new(&t.huf),
            seq: [SeqCTable::new(t.ll.clone())?, SeqCTable::new(t.of.clone())?, SeqCTable::new(t.ml.clone())?],
        })
    }
}

/// What the decoder holds after the blocks emitted so far in this frame.
pub struct EntropyState {
    pub huf_held: Held,
    pub huf_local: Option<HufCTable>,
    pub seq_held: [Held; 3],
    pub seq_local: [Option<SeqCTable>; 3],
}

impl EntropyState {
    pub fn new() -> EntropyState {
        EntropyState { huf_held: Held::None, huf_local: None, seq_held: [Held::None; 3], seq_local: [None, None, None] }
    }

    /// Resets for a new frame (or an independent job).
    pub fn reset(&mut self, dict: Option<&DictCTables>) {
        let h = if dict.is_some() { Held::Dict } else { Held::None };
        self.huf_held = h;
        self.seq_held = [h; 3];
    }

    /// The Huffman table the decoder holds, if any.
    pub fn huf<'a>(&'a self, dict: Option<&'a DictCTables>) -> Option<&'a HufCTable> {
        match self.huf_held {
            Held::Dict => dict.map(|d| &d.huf),
            Held::Local => self.huf_local.as_ref(),
            _ => None,
        }
    }

    /// The sequence table the decoder holds for stream `i`, if any (RLE is reported separately).
    pub fn seq<'a>(&'a self, i: usize, pre: &'a PredefinedC, dict: Option<&'a DictCTables>) -> Option<&'a SeqCTable> {
        match self.seq_held[i] {
            Held::Predefined => Some(&pre.t[i]),
            Held::Dict => dict.map(|d| &d.seq[i]),
            Held::Local => self.seq_local[i].as_ref(),
            _ => None,
        }
    }
}
