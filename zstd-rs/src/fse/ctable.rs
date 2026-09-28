//! FSE encoding tables and the encoder state machine (inverse of `dtable`).
use super::{highbit, spread, Norm, MAX_SYMBOLS};
use crate::bits::BitWriter;
use crate::error::Result;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, Default, Debug)]
struct SymbolTT {
    delta_nb_bits: u32,
    delta_find_state: i32,
}

/// Encoding table for one normalized distribution.
#[derive(Clone, Debug)]
pub struct CTable {
    pub log: u32,
    state_table: Vec<u16>,
    tt: Vec<SymbolTT>,
}

impl CTable {
    pub fn build(norm: &Norm) -> Result<CTable> {
        let size = 1usize << norm.log;
        let mut symbols = vec![0u8; size];
        spread(norm, &mut symbols)?;
        let mut cumul = [0u32; MAX_SYMBOLS + 1];
        for s in 0..norm.symbols {
            let c = norm.counts[s];
            cumul[s + 1] = cumul[s] + if c == -1 { 1 } else { c.max(0) as u32 };
        }
        let mut state_table = vec![0u16; size];
        for (u, &s) in symbols.iter().enumerate() {
            let slot = &mut cumul[s as usize];
            state_table[*slot as usize] = (size + u) as u16;
            *slot += 1;
        }
        let mut tt = vec![SymbolTT::default(); norm.symbols];
        let mut total: i32 = 0;
        for (s, t) in tt.iter_mut().enumerate() {
            let c = norm.counts[s];
            match c {
                0 => t.delta_nb_bits = ((norm.log + 1) << 16).wrapping_sub(size as u32),
                -1 | 1 => {
                    *t = SymbolTT { delta_nb_bits: (norm.log << 16) - size as u32, delta_find_state: total - 1 };
                    total += 1;
                }
                _ => {
                    let c = c as u32;
                    let max_bits_out = norm.log - highbit(c - 1);
                    let min_state_plus = c << max_bits_out;
                    *t = SymbolTT { delta_nb_bits: (max_bits_out << 16).wrapping_sub(min_state_plus), delta_find_state: total - c as i32 };
                    total += c as i32;
                }
            }
        }
        Ok(CTable { log: norm.log, state_table, tt })
    }

    /// Approximate cost of coding `s` once, in 1/256 bit units.
    pub fn cost_q8(&self, s: usize) -> u32 {
        let d = self.tt[s].delta_nb_bits;
        let min_nb = d >> 16;
        let threshold = (min_nb + 1) << 16;
        let size = 1u32 << self.log;
        let delta = threshold.wrapping_sub(d + size);
        let frac = (delta << 8) >> self.log;
        (min_nb + 1) * 256 - frac.min(256)
    }
}

/// An encoding state. Symbols are encoded in reverse order of decoding.
pub struct CState {
    value: u32,
}

impl CState {
    pub fn init(t: &CTable, s: usize) -> CState {
        let tt = t.tt[s];
        let nb_out = (tt.delta_nb_bits + (1 << 15)) >> 16;
        let value = (nb_out << 16).wrapping_sub(tt.delta_nb_bits);
        let idx = ((value >> nb_out) as i32 + tt.delta_find_state) as usize;
        CState { value: t.state_table[idx] as u32 }
    }

    #[inline(always)]
    pub fn encode(&mut self, t: &CTable, w: &mut BitWriter<'_>, s: usize) {
        let tt = t.tt[s];
        let nb_out = (self.value + tt.delta_nb_bits) >> 16;
        w.add((self.value & ((1 << nb_out) - 1)) as u64, nb_out);
        let idx = ((self.value >> nb_out) as i32 + tt.delta_find_state) as usize;
        self.value = t.state_table[idx] as u32;
    }

    pub fn flush(&self, t: &CTable, w: &mut BitWriter<'_>) {
        w.add((self.value & ((1 << t.log) - 1)) as u64, t.log);
    }
}

#[cfg(test)]
mod tests {
    use super::super::dtable::{build, DState};
    use super::*;
    use crate::bits::RevReader;

    #[test]
    fn encode_then_decode_single_state() {
        let n = Norm::from_slice(&crate::decode::tables::ML_DEFAULT, 6);
        let ct = CTable::build(&n).unwrap();
        let dt = build(&n).unwrap();
        let msg: Vec<usize> = (0..500).map(|i| (i * i + 3 * i) % 53).collect();
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        let mut st = CState::init(&ct, *msg.last().unwrap());
        for &s in msg[..msg.len() - 1].iter().rev() {
            st.encode(&ct, &mut w, s);
        }
        st.flush(&ct, &mut w);
        w.finish();
        let mut r = RevReader::new(&out).unwrap();
        let mut d = DState::init(&mut r, 6);
        for (i, &s) in msg.iter().enumerate() {
            assert_eq!(d.symbol(&dt) as usize, s, "at {i}");
            if i + 1 < msg.len() {
                r.refill();
                d.update(&dt, &mut r);
            }
        }
        r.refill();
        assert!(r.finished());
    }
}
