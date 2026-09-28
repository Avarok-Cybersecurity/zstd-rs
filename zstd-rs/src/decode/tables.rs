//! Constant tables from RFC 8878 §3.1.1.3.2: code baselines and the predefined
//! (default) distributions for literal lengths, match lengths and offsets.

pub const LL_DEFAULT: [i16; 36] =
    [4, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 1, 1, 1, 1, 1, -1, -1, -1, -1];
pub const LL_DEFAULT_LOG: u32 = 6;

pub const ML_DEFAULT: [i16; 53] = [
    1, 4, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, -1, -1, -1, -1, -1, -1, -1,
];
pub const ML_DEFAULT_LOG: u32 = 6;

pub const OF_DEFAULT: [i16; 29] = [1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1];
pub const OF_DEFAULT_LOG: u32 = 5;

/// Literal length code -> (baseline, extra bits).
pub const LL_CODES: [(u32, u8); 36] = [
    (0, 0),
    (1, 0),
    (2, 0),
    (3, 0),
    (4, 0),
    (5, 0),
    (6, 0),
    (7, 0),
    (8, 0),
    (9, 0),
    (10, 0),
    (11, 0),
    (12, 0),
    (13, 0),
    (14, 0),
    (15, 0),
    (16, 1),
    (18, 1),
    (20, 1),
    (22, 1),
    (24, 2),
    (28, 2),
    (32, 3),
    (40, 3),
    (48, 4),
    (64, 6),
    (128, 7),
    (256, 8),
    (512, 9),
    (1024, 10),
    (2048, 11),
    (4096, 12),
    (8192, 13),
    (16384, 14),
    (32768, 15),
    (65536, 16),
];

/// Match length code -> (baseline, extra bits).
pub const ML_CODES: [(u32, u8); 53] = [
    (3, 0),
    (4, 0),
    (5, 0),
    (6, 0),
    (7, 0),
    (8, 0),
    (9, 0),
    (10, 0),
    (11, 0),
    (12, 0),
    (13, 0),
    (14, 0),
    (15, 0),
    (16, 0),
    (17, 0),
    (18, 0),
    (19, 0),
    (20, 0),
    (21, 0),
    (22, 0),
    (23, 0),
    (24, 0),
    (25, 0),
    (26, 0),
    (27, 0),
    (28, 0),
    (29, 0),
    (30, 0),
    (31, 0),
    (32, 0),
    (33, 0),
    (34, 0),
    (35, 1),
    (37, 1),
    (39, 1),
    (41, 1),
    (43, 2),
    (47, 2),
    (51, 3),
    (59, 3),
    (67, 4),
    (83, 4),
    (99, 5),
    (131, 7),
    (259, 8),
    (515, 9),
    (1027, 10),
    (2051, 11),
    (4099, 12),
    (8195, 13),
    (16387, 14),
    (32771, 15),
    (65539, 16),
];

/// Largest offset code the decoder accepts (offsets up to 2^31).
pub const MAX_OF_CODE: usize = 31;
pub const LL_MAX_LOG: u32 = 9;
pub const ML_MAX_LOG: u32 = 9;
pub const OF_MAX_LOG: u32 = 8;

/// Inverts a baseline table for small values: `out[v]` is the code covering `v + bias`.
const fn invert<const N: usize>(codes: &[(u32, u8)], bias: u32) -> [u8; N] {
    let mut out = [0u8; N];
    let mut v = 0;
    let mut c = 0;
    while v < N {
        while c + 1 < codes.len() && codes[c + 1].0 <= v as u32 + bias {
            c += 1;
        }
        out[v] = c as u8;
        v += 1;
    }
    out
}

const LL_SMALL: [u8; 64] = invert::<64>(&LL_CODES, 0);
const ML_SMALL: [u8; 128] = invert::<128>(&ML_CODES, 3);

/// Literal length -> code (the encoder's inverse of `LL_CODES`).
#[inline(always)]
pub fn ll_code(ll: u32) -> u8 {
    if ll < 64 {
        LL_SMALL[ll as usize]
    } else {
        (crate::fse::highbit(ll) + 19) as u8
    }
}

/// Match length -> code (`ml` >= 3).
#[inline(always)]
pub fn ml_code(ml: u32) -> u8 {
    let v = ml - 3;
    if v < 128 {
        ML_SMALL[v as usize]
    } else {
        (crate::fse::highbit(v) + 36) as u8
    }
}

/// Offset value (offset + 3, or a repeat code 1..=3) -> code.
#[inline(always)]
pub fn of_code(off_value: u32) -> u8 {
    crate::fse::highbit(off_value) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_codes_invert_baselines() {
        for ll in 0..=crate::frame::BLOCK_MAX as u32 - 3 {
            let c = ll_code(ll) as usize;
            let (base, bits) = LL_CODES[c];
            assert!(ll >= base && ll - base < (1 << bits).max(1), "ll {ll} code {c}");
        }
        for ml in 3..=crate::frame::BLOCK_MAX as u32 {
            let c = ml_code(ml) as usize;
            let (base, bits) = ML_CODES[c];
            assert!(ml >= base && ml - base < (1 << bits).max(1), "ml {ml} code {c}");
        }
    }
}
