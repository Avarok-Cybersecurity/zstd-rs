//! Huffman encoding tables and 1-/4-stream literal encoding.
use super::HufWeights;
use crate::bits::BitWriter;
use crate::error::{Error, Result};
use alloc::vec::Vec;

/// Canonical codes matching `HufDTable`'s layout for the same weights.
#[derive(Clone, Debug)]
pub struct HufCTable {
    codes: [u16; 256],
    lens: [u8; 256],
}

impl HufCTable {
    pub fn new(hw: &HufWeights) -> HufCTable {
        let mut rank = [0usize; 13];
        for &w in &hw.w[..hw.symbols] {
            rank[w as usize] += 1;
        }
        let mut start = [0usize; 13];
        let mut next = 0usize;
        for w in 1..=hw.max_bits as usize {
            start[w] = next;
            next += rank[w] << (w - 1);
        }
        let mut codes = [0u16; 256];
        let mut lens = [0u8; 256];
        for s in 0..hw.symbols {
            let w = hw.w[s] as usize;
            if w > 0 {
                codes[s] = (start[w] >> (w - 1)) as u16;
                lens[s] = hw.len(s) as u8;
                start[w] += 1 << (w - 1);
            }
        }
        HufCTable { codes, lens }
    }

    /// Total encoded bits for `count` over the symbols in `present` (those with a
    /// nonzero count), or `None` if one of them has no code.
    pub fn cost_bits(&self, count: &[u32; 256], present: &[u8]) -> Option<u64> {
        let mut bits = 0u64;
        for &s in present {
            let len = self.lens[s as usize];
            if len == 0 {
                return None;
            }
            bits += count[s as usize] as u64 * len as u64;
        }
        Some(bits)
    }

    /// Code length of `s` in bits (0 when absent).
    #[inline(always)]
    pub fn len(&self, s: u8) -> u32 {
        self.lens[s as usize] as u32
    }

    fn stream(&self, src: &[u8], out: &mut Vec<u8>) {
        let mut w = BitWriter::new(out);
        let tail = src.len() % 4;
        for &b in src[src.len() - tail..].iter().rev() {
            w.add_unflushed(self.codes[b as usize] as u64, self.lens[b as usize] as u32);
        }
        w.flush();
        // Four 11-bit codes (44 bits) plus at most 7 pending bits fit in 64.
        for q in src[..src.len() - tail].rchunks_exact(4) {
            for &b in q.iter().rev() {
                w.add_unflushed(self.codes[b as usize] as u64, self.lens[b as usize] as u32);
            }
            w.flush();
        }
        w.finish();
    }

    /// One stream (used for fewer than 256 literals, as the reference does).
    pub fn encode_1x(&self, src: &[u8], out: &mut Vec<u8>) {
        self.stream(src, out);
    }

    /// Four streams with a jump table. Fails if a stream would not fit a u16 size.
    pub fn encode_4x(&self, src: &[u8], out: &mut Vec<u8>) -> Result<()> {
        let seg = src.len().div_ceil(4);
        if src.len() < 6 {
            return Err(Error::Parameter("4-stream literals need at least 6 bytes"));
        }
        let jump = out.len();
        out.extend_from_slice(&[0; 6]);
        let mut sizes = [0usize; 3];
        for i in 0..4 {
            let part = &src[i * seg..if i == 3 { src.len() } else { (i + 1) * seg }];
            let before = out.len();
            self.stream(part, out);
            if i < 3 {
                sizes[i] = out.len() - before;
            }
        }
        for (i, &s) in sizes.iter().enumerate() {
            let v = u16::try_from(s).map_err(|_| Error::Parameter("Huffman stream too large"))?;
            out[jump + 2 * i..jump + 2 * i + 2].copy_from_slice(&v.to_le_bytes());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{build, dtable::HufDTable};
    use super::*;

    #[test]
    fn encode_decode_both_layouts() {
        let text = b"the quick brown fox jumps over the lazy dog; zstd literals, huffman-coded ".repeat(20);
        let mut count = [0u32; 256];
        for &b in &text {
            count[b as usize] += 1;
        }
        let hw = build::build(&count, 11);
        let ct = HufCTable::new(&hw);
        let dt = HufDTable::new(&hw);
        let mut one = Vec::new();
        ct.encode_1x(&text[..200], &mut one);
        let mut back = alloc::vec![0u8; 200];
        dt.decode_1x(&one, &mut back).unwrap();
        assert_eq!(&back[..], &text[..200]);
        let mut four = Vec::new();
        ct.encode_4x(&text, &mut four).unwrap();
        let mut back = alloc::vec![0u8; text.len()];
        dt.decode_4x(&four, &mut back).unwrap();
        assert_eq!(back, text);
        let present: Vec<u8> = (0..=255u8).filter(|&b| count[b as usize] > 0).collect();
        assert!(ct.cost_bits(&count, &present).unwrap() / 8 < text.len() as u64);
    }

    #[test]
    fn streams_must_be_consumed_exactly() {
        let text = b"exactness matters: every bit of a Huffman stream is accounted for".repeat(8);
        let mut count = [0u32; 256];
        for &b in &text {
            count[b as usize] += 1;
        }
        let hw = build::build(&count, 11);
        let (ct, dt) = (HufCTable::new(&hw), HufDTable::new(&hw));
        for n in [5usize, 40, 200] {
            let mut s = Vec::new();
            ct.encode_1x(&text[..n], &mut s);
            let mut back = alloc::vec![0u8; n];
            dt.decode_1x(&s, &mut back).unwrap();
            let mut padded = alloc::vec![0xA5u8; 9];
            padded.extend_from_slice(&s);
            assert!(dt.decode_1x(&padded, &mut back).is_err(), "unread bits accepted (n = {n})");
            assert!(dt.decode_1x(&s, &mut alloc::vec![0u8; n + 3]).is_err(), "over-read accepted (n = {n})");
        }
    }
}
