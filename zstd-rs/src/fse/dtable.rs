//! FSE decoding tables.
use super::{highbit, spread, Norm};
use crate::bits::RevReader;
use crate::error::{Error, Result};
#[cfg(test)]
use alloc::vec::Vec;

/// One decoding state: emit `symbol`, then the next state is `base + read(nb_bits)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DEntry {
    pub symbol: u8,
    pub nb_bits: u8,
    pub base: u16,
}

/// Largest table any decoder builds (sequence tables: accuracy log 9).
pub const MAX_LOG: u32 = 9;

/// Calls `emit(state, entry)` for every state of `norm`'s decoding table, in order.
/// Allocation-free (the spread uses a stack buffer).
pub fn build_with(norm: &Norm, mut emit: impl FnMut(DEntry)) -> Result<()> {
    if norm.log > MAX_LOG || norm.symbols > 64 {
        return Err(Error::Corrupt("FSE table too large"));
    }
    let size = 1usize << norm.log;
    let mut symbols = [0u8; 1 << MAX_LOG];
    spread(norm, &mut symbols[..size])?;
    let mut next = [0u32; 64];
    for (n, &c) in next.iter_mut().zip(&norm.counts[..norm.symbols]) {
        *n = if c == -1 { 1 } else { c.max(0) as u32 };
    }
    for &sym in &symbols[..size] {
        let s = sym as usize;
        let state = next[s];
        next[s] += 1;
        let nb = norm.log - highbit(state);
        emit(DEntry { symbol: sym, nb_bits: nb as u8, base: ((state << nb) - size as u32) as u16 });
    }
    Ok(())
}

/// Builds the decoding table for `norm` (`1 << norm.log` entries).
#[cfg(test)]
pub fn build(norm: &Norm) -> Result<Vec<DEntry>> {
    let mut table = Vec::with_capacity(1 << norm.log);
    build_with(norm, |e| table.push(e))?;
    Ok(table)
}

/// A single FSE decoding state over a table.
pub struct DState {
    pub state: usize,
}

impl DState {
    pub fn init(r: &mut RevReader<'_>, log: u32) -> Self {
        DState { state: r.read(log) as usize }
    }

    #[inline(always)]
    pub fn symbol(&self, t: &[DEntry]) -> u8 {
        t[self.state].symbol
    }

    #[inline(always)]
    pub fn update(&mut self, t: &[DEntry], r: &mut RevReader<'_>) {
        let e = t[self.state];
        self.state = e.base as usize + r.read(e.nb_bits as u32) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_cover_table_once() {
        let n = Norm::from_slice(&crate::decode::tables::LL_DEFAULT, 6);
        let t = build(&n).unwrap();
        assert_eq!(t.len(), 64);
        for e in &t {
            assert!((e.base as usize) + (1usize << e.nb_bits) <= 64);
        }
    }
}
