//! Adaptive symbol statistics and bit prices for the optimal parser (the
//! reference's `ZSTD_rescaleFreqs` / `ZSTD_updateStats` model, 1/256-bit units).
use crate::decode::tables::{ll_code, ml_code, LL_CODES, ML_CODES};
use crate::encode::entropy::DictCTables;

const BASE_OF: [u32; 32] = [6, 2, 1, 1, 2, 3, 4, 4, 4, 3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1];

/// Frequency statistics carried across the blocks of a frame.
pub struct Stats {
    pub lit: [u32; 256],
    pub ll: [u32; 36],
    pub ml: [u32; 53],
    pub of: [u32; 32],
    pub sums: [u32; 4],
    /// Fixed prices (tiny blocks): no statistics are trustworthy yet.
    pub predef: bool,
    /// Fractional weights (ultra levels) rather than whole-bit weights.
    pub frac: bool,
    base: [u32; 4],
}

#[inline(always)]
fn weight(stat: u32, frac: bool) -> u32 {
    let s = stat + 1;
    let hb = 31 - s.leading_zeros();
    if frac {
        hb * 256 + ((s << 8) >> hb)
    } else {
        hb * 256
    }
}

fn scale(t: &mut [u32], shift: u32) -> u32 {
    t.iter_mut().fold(0, |sum, f| {
        *f = 1 + (*f >> shift);
        sum + *f
    })
}

impl Stats {
    pub fn new() -> Stats {
        Stats { lit: [0; 256], ll: [0; 36], ml: [0; 53], of: [0; 32], sums: [0; 4], predef: false, frac: false, base: [0; 4] }
    }

    /// Prepares prices for a block: fresh statistics at frame start (from the
    /// dictionary's tables or the block's literals), else decayed ones.
    pub fn rescale(&mut self, block: &[u8], first: bool, dict: Option<&DictCTables>, frac: bool) {
        self.frac = frac;
        self.predef = false;
        if first {
            if let Some(d) = dict {
                for (s, f) in self.lit.iter_mut().enumerate() {
                    let bits = d.huf.len(s as u8);
                    *f = if bits == 0 { 1 } else { 1 << (11u32.saturating_sub(bits)).max(1) };
                }
                let fse = |t: &mut [u32], i: usize| {
                    for (s, f) in t.iter_mut().enumerate() {
                        let norm = d.seq[i].norm.counts.get(s).copied().unwrap_or(0);
                        *f = if s < d.seq[i].norm.symbols && norm != 0 {
                            let bits = d.seq[i].ct.cost_q8(s) >> 8;
                            1 << (10u32.saturating_sub(bits)).max(1)
                        } else {
                            1
                        };
                    }
                };
                fse(&mut self.ll, 0);
                fse(&mut self.of, 1);
                fse(&mut self.ml, 2);
            } else {
                if block.len() <= 8 {
                    self.predef = true;
                }
                self.lit = [0; 256];
                for &b in block {
                    self.lit[b as usize] += 1;
                }
                scale(&mut self.lit, 8);
                self.ll = [1; 36];
                self.ml = [1; 53];
                self.of = BASE_OF;
            }
        } else {
            scale(&mut self.lit, 12);
            scale(&mut self.ll, 11);
            scale(&mut self.ml, 11);
            scale(&mut self.of, 11);
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        self.sums = [self.lit.iter().sum(), self.ll.iter().sum(), self.ml.iter().sum(), self.of.iter().sum()];
        self.commit();
    }

    /// Recomputes the cached base prices after a batch of updates.
    pub fn commit(&mut self) {
        let f = self.frac;
        self.base = [weight(self.sums[0], f), weight(self.sums[1], f), weight(self.sums[2], f), weight(self.sums[3], f)];
    }

    /// Price of one literal byte.
    #[inline(always)]
    pub fn lit(&self, b: u8) -> u32 {
        if self.predef {
            return 6 * 256;
        }
        let max = self.base[0].saturating_sub(256);
        self.base[0] - weight(self.lit[b as usize], self.frac).min(max)
    }

    /// Price of a literal-length field of `ll`.
    #[inline(always)]
    pub fn lit_len(&self, ll: u32) -> u32 {
        if self.predef {
            return weight(ll, self.frac);
        }
        let c = ll_code(ll.min(131_071)) as usize;
        LL_CODES[c].1 as u32 * 256 + self.base[1] - weight(self.ll[c], self.frac)
    }

    /// Price of a match of `ml` bytes with coded offset `off_base`.
    #[inline(always)]
    pub fn matched(&self, off_base: u32, ml: u32, ultra: bool) -> u32 {
        let oc = 31 - off_base.leading_zeros();
        if self.predef {
            return weight(ml - 3, self.frac) + (16 + oc) * 256;
        }
        let mut p = oc * 256 + self.base[3] - weight(self.of[oc as usize], self.frac);
        if !ultra && oc >= 20 {
            p += (oc - 19) * 2 * 256;
        }
        let mc = ml_code(ml) as usize;
        p + ML_CODES[mc].1 as u32 * 256 + self.base[2] - weight(self.ml[mc], self.frac) + 51
    }

    /// Records a chosen sequence.
    pub fn update(&mut self, lits: &[u8], off_base: u32, ml: u32) {
        for &b in lits {
            self.lit[b as usize] += 2;
        }
        self.ll[ll_code(lits.len() as u32) as usize] += 1;
        self.of[(31 - off_base.leading_zeros()) as usize] += 1;
        self.ml[ml_code(ml) as usize] += 1;
        self.sums[0] += 2 * lits.len() as u32;
        self.sums[1] += 1;
        self.sums[2] += 1;
        self.sums[3] += 1;
    }
}
