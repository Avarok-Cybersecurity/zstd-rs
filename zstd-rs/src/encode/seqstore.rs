//! Sequences produced by a match finder for one block, with repeat-offset
//! bookkeeping identical to the decoder's (so offsets are coded correctly).
use alloc::vec::Vec;

/// One sequence: `ll` literals, then `ml` bytes copied; `off_base` is the coded
/// offset value (1..=3 = repeat codes, otherwise offset + 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seq {
    pub ll: u32,
    pub ml: u32,
    pub off_base: u32,
}

/// Repeat offsets as the decoder sees them. 0 marks an entry the encoder does not
/// know (block jobs compressed independently); it never equals a real offset.
pub type Reps = [u32; 3];

/// Converts a raw offset to its coded value, updating `reps` like the decoder.
#[inline(always)]
pub fn code_offset(offset: u32, ll: u32, reps: &mut Reps) -> u32 {
    let r = *reps;
    let (code, new) = if ll > 0 {
        if offset == r[0] {
            (1, r)
        } else if offset == r[1] {
            (2, [offset, r[0], r[2]])
        } else if offset == r[2] {
            (3, [offset, r[0], r[1]])
        } else {
            (offset + 3, [offset, r[0], r[1]])
        }
    } else if offset == r[1] {
        (1, [offset, r[0], r[2]])
    } else if offset == r[2] {
        (2, [offset, r[0], r[1]])
    } else if r[0] > 1 && offset == r[0] - 1 {
        (3, [offset, r[0], r[1]])
    } else {
        (offset + 3, [offset, r[0], r[1]])
    };
    *reps = new;
    code
}

/// Literals and sequences of one block, plus the repeat offsets after them.
pub struct SeqStore {
    pub lits: Vec<u8>,
    pub seqs: Vec<Seq>,
    pub reps: Reps,
}

impl SeqStore {
    pub fn new() -> SeqStore {
        SeqStore { lits: Vec::new(), seqs: Vec::new(), reps: [1, 4, 8] }
    }

    pub fn reset(&mut self, reps: Reps) {
        self.lits.clear();
        self.seqs.clear();
        self.reps = reps;
    }

    /// Records `literals` followed by a match of `ml` bytes at distance `offset`.
    #[inline(always)]
    pub fn push(&mut self, literals: &[u8], offset: u32, ml: u32) {
        debug_assert!(ml >= 3 && offset > 0);
        let ll = literals.len() as u32;
        self.lits.extend_from_slice(literals);
        let off_base = code_offset(offset, ll, &mut self.reps);
        self.seqs.push(Seq { ll, ml, off_base });
    }

    /// Records the literals that follow the last sequence of the block.
    pub fn tail(&mut self, literals: &[u8]) {
        self.lits.extend_from_slice(literals);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_codes_follow_the_decoder_rules() {
        let mut r = [1, 4, 8];
        assert_eq!(code_offset(4, 5, &mut r), 2, "rep1 with literals");
        assert_eq!(r, [4, 1, 8]);
        assert_eq!(code_offset(1, 0, &mut r), 1, "ll = 0 shifts codes: 1 means rep1");
        assert_eq!(r, [1, 4, 8]);
        assert_eq!(code_offset(1, 0, &mut r), 4, "rep0 with ll = 0 has no repeat code");
        assert_eq!(r, [1, 1, 4]);
        assert_eq!(code_offset(8, 0, &mut [9, 4, 8]), 2, "ll = 0: 2 means rep2");
        assert_eq!(code_offset(8, 3, &mut [9, 4, 8]), 3, "rep2 with literals");
        assert_eq!(code_offset(8, 0, &mut [9, 4, 7]), 3, "ll = 0: 3 means rep0 - 1");
        assert_eq!(code_offset(100, 1, &mut [9, 4, 8]), 103);
        assert_eq!(code_offset(1, 0, &mut [0, 0, 0]), 4, "unknown repeats never match");
    }
}
