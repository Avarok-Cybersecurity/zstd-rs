//! FSE table description ("NCount"): reading (RFC 8878 §4.1.1) and writing.
use super::{Norm, MAX_SYMBOLS};
use crate::bits::BitWriter;
use crate::error::{Error, Result};
use alloc::vec::Vec;

/// Forward little-endian bit cursor over a byte slice; bits past the end read as 0.
struct Fwd<'a> {
    src: &'a [u8],
    pos: usize,
}

impl Fwd<'_> {
    fn peek(&self, nb: u32) -> u32 {
        let byte = self.pos >> 3;
        let mut w = [0u8; 4];
        for (i, slot) in w.iter_mut().enumerate() {
            *slot = self.src.get(byte + i).copied().unwrap_or(0);
        }
        let v = u32::from_le_bytes(w) >> (self.pos & 7);
        v & ((1u32 << nb) - 1)
    }

    fn take(&mut self, nb: u32) -> u32 {
        let v = self.peek(nb);
        self.pos += nb as usize;
        v
    }
}

/// Reads a table description. Returns the distribution and the bytes consumed.
pub fn read(src: &[u8], max_symbols: usize, max_log: u32) -> Result<(Norm, usize)> {
    if src.is_empty() {
        return Err(Error::Truncated);
    }
    let mut r = Fwd { src, pos: 0 };
    let log = r.take(4) + 5;
    if log > max_log {
        return Err(Error::Corrupt("FSE accuracy log too large"));
    }
    let mut norm = Norm { counts: [0; MAX_SYMBOLS], symbols: 0, log };
    let mut remaining: i32 = (1 << log) + 1;
    let mut threshold: i32 = 1 << log;
    let mut nb_bits = log + 1;
    let mut sym = 0usize;
    let mut prev0 = false;
    while remaining > 1 && sym < max_symbols {
        if prev0 {
            loop {
                let rep = r.take(2) as usize;
                sym += rep;
                if rep != 3 {
                    break;
                }
                if sym > max_symbols {
                    return Err(Error::Corrupt("FSE zero run past alphabet"));
                }
            }
            if sym >= max_symbols {
                break;
            }
        }
        let max = (2 * threshold - 1) - remaining;
        let low = r.peek(nb_bits - 1) as i32;
        let mut count = if low < max {
            r.pos += (nb_bits - 1) as usize;
            low
        } else {
            let mut c = r.take(nb_bits) as i32;
            if c >= threshold {
                c -= max;
            }
            c
        };
        count -= 1;
        remaining -= count.abs();
        norm.counts[sym] = count as i16;
        sym += 1;
        prev0 = count == 0;
        if remaining < 1 {
            break;
        }
        while remaining < threshold {
            nb_bits -= 1;
            threshold >>= 1;
        }
    }
    if remaining != 1 {
        return Err(Error::Corrupt("FSE probabilities do not sum to table size"));
    }
    let used = r.pos.div_ceil(8);
    if used > src.len() {
        return Err(Error::Truncated);
    }
    norm.symbols = sym;
    Ok((norm, used))
}

/// Appends the description of `norm` to `out`.
pub fn write(norm: &Norm, out: &mut Vec<u8>) {
    let mut w = BitWriter::new(out);
    let size: i32 = 1 << norm.log;
    w.add((norm.log - 5) as u64, 4);
    let mut remaining = size + 1;
    let mut threshold = size;
    let mut nb_bits = norm.log + 1;
    let mut sym = 0usize;
    let mut prev0 = false;
    while sym < norm.symbols && remaining > 1 {
        if prev0 {
            let mut start = sym;
            while sym < norm.symbols && norm.counts[sym] == 0 {
                sym += 1;
            }
            debug_assert!(sym < norm.symbols, "distribution ends in zeros");
            while sym >= start + 3 {
                start += 3;
                w.add(3, 2);
            }
            w.add((sym - start) as u64, 2);
        }
        let mut count = norm.counts[sym] as i32;
        sym += 1;
        let max = (2 * threshold - 1) - remaining;
        remaining -= count.abs();
        count += 1;
        if count >= threshold {
            count += max;
        }
        let bits = if count < max { nb_bits - 1 } else { nb_bits };
        w.add(count as u64, bits);
        prev0 = count == 1;
        while remaining < threshold && remaining >= 1 {
            nb_bits -= 1;
            threshold >>= 1;
        }
    }
    w.finish_plain();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(counts: &[i16], log: u32) {
        let n = Norm::from_slice(counts, log);
        let mut out = Vec::new();
        write(&n, &mut out);
        let (back, used) = read(&out, 64, 9).unwrap();
        assert_eq!(used, out.len());
        let last = counts.iter().rposition(|&c| c != 0).unwrap() + 1;
        assert_eq!(back.symbols, last);
        assert_eq!(&back.counts[..last], &counts[..last]);
    }

    #[test]
    fn roundtrips_predefined_and_sparse() {
        roundtrip(&crate::decode::tables::LL_DEFAULT, 6);
        roundtrip(&crate::decode::tables::ML_DEFAULT, 6);
        roundtrip(&crate::decode::tables::OF_DEFAULT, 5);
        let mut sparse = [0i16; 40];
        sparse[0] = 20;
        sparse[9] = 10;
        sparse[30] = -1;
        sparse[39] = 1;
        roundtrip(&sparse, 5);
    }

    #[test]
    fn rejects_bad_sums_and_logs() {
        assert!(read(&[0x0f], 64, 9).is_err());
        assert!(read(&[], 64, 9).is_err());
        assert!(read(&[0x00, 0x00], 64, 9).is_err());
    }
}
