//! Bit streams. zstd entropy streams are written forwards (LSB first) and read
//! backwards starting from the final byte, whose highest set bit is a marker.
use crate::error::{Error, Result};
use alloc::vec::Vec;

/// Forward bit writer producing a stream that `RevReader` reads back in reverse.
/// It stores whole 8-byte words into `out` (grown ahead of the write position)
/// and trims the spare bytes in `finish`.
pub struct BitWriter<'a> {
    out: &'a mut Vec<u8>,
    pos: usize,
    acc: u64,
    n: u32,
}

impl<'a> BitWriter<'a> {
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        let pos = out.len();
        out.resize(pos + 64, 0);
        BitWriter { out, pos, acc: 0, n: 0 }
    }

    /// Appends the low `nb` bits of `v` (`nb` <= 32; `v` must fit in `nb` bits).
    #[inline(always)]
    pub fn add(&mut self, v: u64, nb: u32) {
        debug_assert!(nb <= 32 && (nb == 32 || v >> nb == 0));
        self.acc |= v << self.n;
        self.n += nb;
        if self.n >= 32 {
            self.spill();
        }
    }

    /// Appends bits without flushing; the caller keeps the total under 64
    /// between [`BitWriter::flush`] calls (after a flush at most 7 bits are pending).
    #[inline(always)]
    pub fn add_unflushed(&mut self, v: u64, nb: u32) {
        self.acc |= v << self.n;
        self.n += nb;
    }

    /// Stores the pending whole bytes, leaving at most 7 bits pending.
    #[inline(always)]
    pub fn flush(&mut self) {
        if self.pos + 8 > self.out.len() {
            let len = self.out.len();
            self.out.resize(len * 2, 0);
        }
        self.out[self.pos..self.pos + 8].copy_from_slice(&self.acc.to_le_bytes());
        let bytes = self.n >> 3;
        self.pos += bytes as usize;
        self.acc = if bytes == 8 { 0 } else { self.acc >> (bytes * 8) };
        self.n &= 7;
    }

    #[inline(always)]
    fn spill(&mut self) {
        if self.pos + 8 > self.out.len() {
            let len = self.out.len();
            self.out.resize(len * 2, 0);
        }
        self.out[self.pos..self.pos + 8].copy_from_slice(&self.acc.to_le_bytes());
        self.pos += 4;
        self.acc >>= 32;
        self.n -= 32;
    }

    /// Writes the end marker and pads to a byte boundary.
    pub fn finish(mut self) {
        self.add(1, 1);
        self.finish_plain();
    }

    /// Pads to a byte boundary without an end marker (forward-read structures).
    pub fn finish_plain(self) {
        let bytes = self.n.div_ceil(8) as usize;
        if self.pos + 8 > self.out.len() {
            self.out.resize(self.pos + 8, 0);
        }
        self.out[self.pos..self.pos + 8].copy_from_slice(&self.acc.to_le_bytes());
        self.out.truncate(self.pos + bytes);
    }
}

/// Outcome of a refill, mirroring the reference decoder's states.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refill {
    Unfinished,
    EndOfBuffer,
    Completed,
    Overflow,
}

/// Backward bit reader. Bits past the start of the stream read as zero; callers
/// that must not rely on that check `finished()` / `overflowed()`.
pub struct RevReader<'a> {
    src: &'a [u8],
    ptr: usize,
    container: u64,
    consumed: u32,
}

impl<'a> RevReader<'a> {
    pub fn new(src: &'a [u8]) -> Result<Self> {
        let last = *src.last().ok_or(Error::Corrupt("empty bitstream"))?;
        if last == 0 {
            return Err(Error::Corrupt("bitstream missing end marker"));
        }
        let skip = last.leading_zeros() + 1;
        if src.len() >= 8 {
            let ptr = src.len() - 8;
            Ok(RevReader { src, ptr, container: load(src, ptr), consumed: skip })
        } else {
            let mut a = [0u8; 8];
            a[..src.len()].copy_from_slice(src);
            let pad = (8 - src.len() as u32) * 8;
            Ok(RevReader { src, ptr: 0, container: u64::from_le_bytes(a), consumed: skip + pad })
        }
    }

    /// Resumes reading at byte window `ptr` with `consumed` bits of it used
    /// (the state a fast loop hands over; requires `ptr + 8 <= src.len()`).
    pub fn resume(src: &'a [u8], ptr: usize, consumed: u32) -> Self {
        RevReader { src, ptr, container: load(src, ptr), consumed }
    }

    /// Reads `nb` bits (0..=57 after a refill); past-the-start bits are zero.
    #[inline(always)]
    pub fn read(&mut self, nb: u32) -> u64 {
        let v = self.peek(nb);
        self.consumed += nb;
        v
    }

    /// Returns the next `nb` bits without consuming them.
    #[inline(always)]
    pub fn peek(&self, nb: u32) -> u64 {
        let shifted = if self.consumed < 64 { self.container << self.consumed } else { 0 };
        (shifted >> 1) >> (63 - nb)
    }

    #[inline(always)]
    pub fn skip(&mut self, nb: u32) {
        self.consumed += nb;
    }

    /// Reloads the container so at least 57 bits are available when possible.
    #[inline(always)]
    pub fn refill(&mut self) -> Refill {
        if self.consumed > 64 {
            return Refill::Overflow;
        }
        let want = (self.consumed >> 3) as usize;
        if self.ptr >= 8 {
            self.ptr -= want;
            self.consumed &= 7;
            self.container = load(self.src, self.ptr);
            return Refill::Unfinished;
        }
        if self.ptr == 0 {
            return if self.consumed == 64 { Refill::Completed } else { Refill::EndOfBuffer };
        }
        let step = want.min(self.ptr);
        self.ptr -= step;
        self.consumed -= (step as u32) * 8;
        self.container = load(self.src, self.ptr);
        Refill::EndOfBuffer
    }

    /// True when every bit of the stream was consumed exactly.
    pub fn finished(&self) -> bool {
        self.ptr == 0 && self.consumed == 64
    }
}

#[inline(always)]
fn load(src: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&src[at..at + 8]);
    u64::from_le_bytes(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_mixed_widths() {
        for count in [0usize, 1, 3, 17, 200] {
            let vals: Vec<(u64, u32)> =
                (0..count).map(|i| (((i * 2654435761) as u64) & ((1 << (i % 31 + 1)) - 1), (i % 31 + 1) as u32)).collect();
            let mut out = Vec::new();
            let mut w = BitWriter::new(&mut out);
            for &(v, nb) in vals.iter().rev() {
                w.add(v, nb);
            }
            w.finish();
            let mut r = RevReader::new(&out).unwrap();
            for &(v, nb) in &vals {
                r.refill();
                assert_eq!(r.read(nb), v);
            }
            r.refill();
            assert!(r.finished(), "count {count}");
        }
    }

    #[test]
    fn rejects_missing_marker() {
        assert!(RevReader::new(&[1, 0]).is_err());
        assert!(RevReader::new(&[]).is_err());
    }

    #[test]
    fn overflow_detected() {
        let mut r = RevReader::new(&[0b10]).unwrap();
        assert_eq!(r.read(1), 0);
        r.refill();
        assert!(r.finished());
        r.read(1);
        assert_eq!(r.refill(), Refill::Overflow);
    }
}
