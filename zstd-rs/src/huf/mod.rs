//! Huffman coding of literals: tree descriptions, decoding and encoding.
pub mod build;
pub mod ctable;
pub mod dtable;
mod fast;
pub mod weights;

/// zstd limits Huffman codes to 11 bits.
pub const MAX_BITS: u32 = 11;

/// A Huffman tree as zstd describes it: per-symbol weights, where weight `w > 0`
/// means a code of `max_bits + 1 - w` bits and weight 0 means "absent".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HufWeights {
    pub w: [u8; 256],
    /// Highest present symbol + 1 (its weight is implied on the wire).
    pub symbols: usize,
    pub max_bits: u32,
}

impl HufWeights {
    /// Derives weights from code lengths (0 = absent); `lens` must form a complete code.
    pub fn from_lengths(lens: &[u8; 256]) -> HufWeights {
        let max_bits = lens.iter().copied().max().unwrap_or(0) as u32;
        let mut w = [0u8; 256];
        let mut symbols = 0;
        for (s, &l) in lens.iter().enumerate() {
            if l > 0 {
                w[s] = (max_bits + 1 - l as u32) as u8;
                symbols = s + 1;
            }
        }
        HufWeights { w, symbols, max_bits }
    }

    /// Code length for symbol `s` (0 when absent).
    #[inline(always)]
    pub fn len(&self, s: usize) -> u32 {
        match self.w[s] {
            0 => 0,
            w => self.max_bits + 1 - w as u32,
        }
    }
}
