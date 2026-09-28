//! Sequence execution: literal copies and matches into the output, with the
//! caller's output limit and history bounds (frame output plus dictionary) enforced.
//!
//! The output vector is pre-sized with `SLACK` spare bytes past the logical end,
//! so short copies can move fixed 16-byte chunks ("wild copies") without per-byte
//! bounds logic; the caller truncates it to the logical length afterwards.
use crate::error::{Error, Result};
#[cfg(test)]
use crate::frame::BLOCK_MAX;
use alloc::vec::Vec;

/// Spare bytes kept past the logical end of the output (and of literal buffers).
pub const SLACK: usize = 32;

/// Execution context for one block.
pub struct Exec<'a> {
    buf: &'a mut Vec<u8>,
    /// Logical end of the output (next write position).
    pub pos: usize,
    frame_start: usize,
    dict: &'a [u8],
    /// Largest `pos` the caller allows.
    limit: usize,
    /// Largest `pos` this block may reach (Block_Maximum_Size past its start).
    block_end: usize,
    /// Literals consumed so far in this block.
    lit_pos: usize,
}

impl<'a> Exec<'a> {
    /// Starts a block at `pos`. `buf` grows on demand (doubling, never past the
    /// block or caller cap), keeping `SLACK` writable bytes past the logical end.
    pub fn new(buf: &'a mut Vec<u8>, pos: usize, frame_start: usize, dict: &'a [u8], limit: usize, block_max: usize) -> Self {
        Exec { buf, pos, frame_start, dict, limit, block_end: pos + block_max, lit_pos: 0 }
    }

    #[inline(always)]
    fn room(&mut self, n: usize) -> Result<()> {
        let end = self.pos + n;
        if end > self.limit {
            return Err(Error::OutputLimit);
        }
        if end > self.block_end {
            return Err(Error::Corrupt("block regenerates more than Block_Maximum_Size"));
        }
        if end + SLACK > self.buf.len() {
            self.grow(end);
        }
        Ok(())
    }

    #[cold]
    fn grow(&mut self, end: usize) {
        let cap = self.limit.min(self.block_end);
        let want = (self.buf.len() * 2).max(end).min(cap).max(end);
        self.buf.resize(want + SLACK, 0);
    }

    /// Executes one sequence. `lits` may be longer than `lits_len` (spare bytes).
    #[inline(always)]
    pub fn sequence(&mut self, lits: &[u8], lits_len: usize, ll: usize, ml: usize, offset: usize) -> Result<()> {
        self.room(ll + ml)?;
        let lit_end = self.lit_pos + ll;
        if lit_end > lits_len {
            return Err(Error::Corrupt("sequence uses more literals than decoded"));
        }
        let (p, lp) = (self.pos, self.lit_pos);
        if ll <= 16 && lp + 16 <= lits.len() {
            self.buf[p..p + 16].copy_from_slice(&lits[lp..lp + 16]);
        } else {
            self.buf[p..p + ll].copy_from_slice(&lits[lp..lit_end]);
        }
        self.pos += ll;
        self.lit_pos = lit_end;
        let produced = self.pos - self.frame_start;
        if offset > produced {
            return self.dict_match(offset, ml, produced);
        }
        copy_match(self.buf, self.pos, offset, ml);
        self.pos += ml;
        Ok(())
    }

    #[cold]
    fn dict_match(&mut self, offset: usize, ml: usize, produced: usize) -> Result<()> {
        let back = offset - produced;
        if back > self.dict.len() {
            return Err(Error::Corrupt("match offset beyond history"));
        }
        let start = self.dict.len() - back;
        let take = back.min(ml);
        let p = self.pos;
        self.buf[p..p + take].copy_from_slice(&self.dict[start..start + take]);
        self.pos += take;
        if ml > take {
            copy_match(self.buf, self.pos, offset, ml - take);
            self.pos += ml - take;
        }
        Ok(())
    }

    /// Appends the literals left after the last sequence.
    pub fn finish(&mut self, lits: &[u8], lits_len: usize) -> Result<()> {
        let rest = lits_len.checked_sub(self.lit_pos).ok_or(Error::Corrupt("literal cursor past end"))?;
        self.room(rest)?;
        let (p, lp) = (self.pos, self.lit_pos);
        self.buf[p..p + rest].copy_from_slice(&lits[lp..lits_len]);
        self.pos += rest;
        self.lit_pos = lits_len;
        Ok(())
    }
}

/// Copies `len` bytes from `offset` back to `pos` (overlap repeats the pattern).
/// Requires `SLACK` writable bytes past `pos + len`.
#[inline(always)]
fn copy_match(buf: &mut [u8], pos: usize, offset: usize, len: usize) {
    let src = pos - offset;
    if offset >= 16 {
        let mut i = 0;
        while i < len {
            buf.copy_within(src + i..src + i + 16, pos + i);
            i += 16;
        }
    } else if offset >= 8 {
        let mut i = 0;
        while i < len {
            buf.copy_within(src + i..src + i + 8, pos + i);
            i += 8;
        }
    } else {
        // Seed 8 bytes one at a time (they overlap the source), then copy 8-byte
        // chunks from `d` back, the smallest multiple of `offset` that is >= 8:
        // the output is periodic, so the chunks repeat the pattern exactly.
        for k in 0..len.min(8) {
            buf[pos + k] = buf[src + k];
        }
        let d = offset * 8usize.div_ceil(offset);
        let mut i = 8;
        while i < len {
            buf.copy_within(pos + i - d..pos + i - d + 8, pos + i);
            i += 8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dict: &[u8], lits: &[u8], seqs: &[(usize, usize, usize)], limit: usize) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        let mut ex = Exec::new(&mut buf, 0, 0, dict, limit, BLOCK_MAX);
        for &(ll, ml, off) in seqs {
            ex.sequence(lits, lits.len(), ll, ml, off)?;
        }
        ex.finish(lits, lits.len())?;
        let end = ex.pos;
        buf.truncate(end);
        Ok(buf)
    }

    #[test]
    fn overlapping_and_dictionary_matches() {
        assert_eq!(run(b"", b"ab", &[(2, 5, 2)], 100).unwrap(), b"abababa");
        assert_eq!(run(b"XYZ", b"q", &[(1, 4, 3)], 100).unwrap(), b"qYZqY");
        let long: Vec<u8> = (0..40u8).collect();
        let out = run(b"", &long, &[(40, 50, 17), (0, 30, 9)], 1000).unwrap();
        let mut want = long.clone();
        for _ in 0..50 {
            want.push(want[want.len() - 17]);
        }
        for _ in 0..30 {
            want.push(want[want.len() - 9]);
        }
        assert_eq!(out, want);
    }

    #[test]
    fn limits_enforced() {
        assert_eq!(run(b"", b"abcd", &[(2, 3, 1)], 4), Err(Error::OutputLimit));
        assert!(run(b"", b"ab", &[(2, 3, 3)], 100).is_err());
        assert!(run(b"", b"ab", &[(3, 3, 1)], 100).is_err());
    }
}
