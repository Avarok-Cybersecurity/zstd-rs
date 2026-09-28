//! Huffman tree description (RFC 8878 §4.2.1): reading and writing.
use super::{HufWeights, MAX_BITS};
use crate::bits::{BitWriter, Refill, RevReader};
use crate::error::{Error, Result};
use crate::fse::ctable::{CState, CTable};
use crate::fse::dtable::{self, DState};
use crate::fse::{highbit, ncount, normalize};
use alloc::vec::Vec;

/// Reads a tree description; returns the weights and the bytes consumed.
pub fn read(src: &[u8]) -> Result<(HufWeights, usize)> {
    let header = *src.first().ok_or(Error::Truncated)? as usize;
    let mut w = [0u8; 256];
    let (count, used) = if header >= 128 {
        let n = header - 127;
        let bytes = n.div_ceil(2);
        let body = src.get(1..1 + bytes).ok_or(Error::Truncated)?;
        for i in 0..n {
            let b = body[i / 2];
            w[i] = if i % 2 == 0 { b >> 4 } else { b & 15 };
        }
        (n, 1 + bytes)
    } else {
        let body = src.get(1..1 + header).ok_or(Error::Truncated)?;
        (decode_fse_weights(body, &mut w)?, 1 + header)
    };
    Ok((complete(w, count)?, used))
}

fn decode_fse_weights(body: &[u8], w: &mut [u8; 256]) -> Result<usize> {
    if body.is_empty() {
        return Err(Error::Corrupt("empty Huffman weight stream"));
    }
    let (norm, used) = ncount::read(body, 16, 6)?;
    let mut table = [dtable::DEntry::default(); 64];
    let mut n_entries = 0;
    dtable::build_with(&norm, |e| {
        table[n_entries] = e;
        n_entries += 1;
    })?;
    let table = &table[..n_entries];
    let mut r = RevReader::new(&body[used..])?;
    let mut s1 = DState::init(&mut r, norm.log);
    let mut s2 = DState::init(&mut r, norm.log);
    let mut n = 0usize;
    loop {
        if n > 253 {
            return Err(Error::Corrupt("too many Huffman weights"));
        }
        r.refill();
        w[n] = s1.symbol(table);
        n += 1;
        s1.update(table, &mut r);
        if r.refill() == Refill::Overflow {
            w[n] = s2.symbol(table);
            return Ok(n + 1);
        }
        if n > 253 {
            return Err(Error::Corrupt("too many Huffman weights"));
        }
        w[n] = s2.symbol(table);
        n += 1;
        s2.update(table, &mut r);
        if r.refill() == Refill::Overflow {
            w[n] = s1.symbol(table);
            return Ok(n + 1);
        }
    }
}

/// Validates explicit weights and appends the implied last one.
fn complete(mut w: [u8; 256], count: usize) -> Result<HufWeights> {
    let mut total: u32 = 0;
    let mut rank1 = 0;
    for &x in &w[..count] {
        if x as u32 > MAX_BITS {
            return Err(Error::Corrupt("Huffman weight too large"));
        }
        if x > 0 {
            total += 1 << (x - 1);
        }
        rank1 += (x == 1) as u32;
    }
    if total == 0 || count >= 256 {
        return Err(Error::Corrupt("invalid Huffman weights"));
    }
    let max_bits = highbit(total) + 1;
    if max_bits > MAX_BITS {
        return Err(Error::Corrupt("Huffman table too deep"));
    }
    let rest = (1u32 << max_bits) - total;
    if !rest.is_power_of_two() {
        return Err(Error::Corrupt("Huffman weights not completable"));
    }
    let last = highbit(rest) + 1;
    w[count] = last as u8;
    rank1 += (last == 1) as u32;
    if rank1 < 2 || rank1 & 1 == 1 {
        return Err(Error::Corrupt("Huffman rank-1 count invalid"));
    }
    Ok(HufWeights { w, symbols: count + 1, max_bits })
}

/// Appends the description of `hw`. Uses FSE-compressed weights when smaller.
pub fn write(hw: &HufWeights, out: &mut Vec<u8>) -> Result<()> {
    let n = hw.symbols - 1;
    let weights = &hw.w[..n];
    let start = out.len();
    out.push(0);
    if let Some(()) = compress_weights(weights, out) {
        let size = out.len() - start - 1;
        if size > 1 && size < n / 2 {
            out[start] = size as u8;
            return Ok(());
        }
    }
    out.truncate(start);
    if n > 128 {
        return Err(Error::Parameter("Huffman table needs FSE weight compression"));
    }
    out.push((127 + n) as u8);
    for pair in weights.chunks(2) {
        out.push((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
    }
    Ok(())
}

/// FSE-compresses the weights with two interleaved states (the reference layout).
fn compress_weights(weights: &[u8], out: &mut Vec<u8>) -> Option<()> {
    let n = weights.len();
    if n <= 2 {
        return None;
    }
    let mut count = [0u32; 13];
    for &x in weights {
        count[x as usize] += 1;
    }
    let max_symbol = count.iter().rposition(|&c| c > 0)?;
    let max_count = *count.iter().max()?;
    if max_count as usize == n || max_count == 1 {
        return None;
    }
    let log = normalize::optimal_table_log(6, n, max_symbol, 2);
    let norm = normalize::normalize(&count[..=max_symbol], n, log, false)?;
    ncount::write(&norm, out);
    let ct = CTable::build(&norm).ok()?;
    let mut w = BitWriter::new(out);
    let (last_even, last_odd) = if n % 2 == 1 { (n - 1, n - 2) } else { (n - 2, n - 1) };
    let mut c1 = CState::init(&ct, weights[last_even] as usize);
    let mut c2 = CState::init(&ct, weights[last_odd] as usize);
    for i in (0..last_even.min(last_odd)).rev() {
        let st = if i % 2 == 0 { &mut c1 } else { &mut c2 };
        st.encode(&ct, &mut w, weights[i] as usize);
    }
    c2.flush(&ct, &mut w);
    c1.flush(&ct, &mut w);
    w.finish();
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(lens: &[u8; 256]) {
        let hw = HufWeights::from_lengths(lens);
        let mut out = Vec::new();
        write(&hw, &mut out).unwrap();
        let (back, used) = read(&out).unwrap();
        assert_eq!(used, out.len());
        assert_eq!(back, hw);
    }

    #[test]
    fn roundtrips_small_and_large_alphabets() {
        let mut lens = [0u8; 256];
        lens[b'a' as usize] = 1;
        lens[b'b' as usize] = 2;
        lens[b'c' as usize] = 3;
        lens[b'd' as usize] = 3;
        roundtrip(&lens);
        let mut mixed = [0u8; 256];
        mixed[..2].fill(7);
        mixed[2..252].fill(8);
        mixed[252..].fill(9);
        roundtrip(&mixed);
        let mut direct = [0u8; 256];
        direct[..16].fill(4);
        roundtrip(&direct);
    }

    #[test]
    fn rejects_truncated_and_invalid() {
        assert!(read(&[]).is_err());
        assert!(read(&[200]).is_err());
        assert!(read(&[128, 0x20]).is_err());
    }
}
