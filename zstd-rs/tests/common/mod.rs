//! Deterministic test inputs shared by the interop tests.
#![allow(dead_code)]

/// Small deterministic PRNG (xorshift64*), so tests need no dependencies.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const WORDS: &[&str] = &[
    "the",
    "zstd",
    "frame",
    "block",
    "literal",
    "match",
    "offset",
    "window",
    "dictionary",
    "entropy",
    "huffman",
    "sequence",
    "compress",
    "message",
    "session",
    "peer",
    "workspace",
    "{\"type\":\"chat\",",
    "\"id\":",
    "\n",
    ", ",
];

/// Text-like data: words from a small vocabulary with random numbers mixed in.
pub fn text(len: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed | 1);
    let mut v = Vec::with_capacity(len + 16);
    while v.len() < len {
        if r.below(9) == 0 {
            v.extend_from_slice(r.below(100000).to_string().as_bytes());
        } else {
            v.extend_from_slice(WORDS[r.below(WORDS.len() as u64) as usize].as_bytes());
        }
        v.push(b' ');
    }
    v.truncate(len);
    v
}

/// Incompressible bytes.
pub fn random(len: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed | 1);
    (0..len).map(|_| r.next() as u8).collect()
}

/// Long runs and short-period repeats (exercises RLE blocks and overlapping matches).
pub fn runs(len: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed | 1);
    let mut v = Vec::with_capacity(len);
    while v.len() < len {
        let period = 1 + r.below(7) as usize;
        let pat: Vec<u8> = (0..period).map(|_| r.next() as u8).collect();
        let n = r.below(3000) as usize;
        for i in 0..n {
            v.push(pat[i % period]);
        }
    }
    v.truncate(len);
    v
}

/// Mixed content: text, then random, then runs, repeated with small edits.
pub fn mixed(len: usize, seed: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(len);
    let mut s = seed;
    while v.len() < len {
        let chunk = 1 + (s as usize * 7919) % 20000;
        match s % 3 {
            0 => v.extend(text(chunk, s)),
            1 => v.extend(random(chunk / 4, s)),
            _ => v.extend(runs(chunk, s)),
        }
        s += 1;
    }
    v.truncate(len);
    v
}

pub const SIZES: &[usize] = &[0, 1, 2, 3, 7, 16, 31, 64, 100, 255, 256, 1000, 4096, 20_000, 131_072, 131_073, 400_000, 3_000_000];

pub fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut v = Vec::new();
    for &n in SIZES {
        v.push((format!("text-{n}"), text(n, n as u64 + 1)));
        v.push((format!("mixed-{n}"), mixed(n, n as u64 + 2)));
        if n <= 400_000 {
            v.push((format!("random-{n}"), random(n, n as u64 + 3)));
            v.push((format!("runs-{n}"), runs(n, n as u64 + 4)));
        }
    }
    v
}

/// Training samples for dictionary tests (disjoint seeds from `corpus`).
pub fn samples() -> Vec<Vec<u8>> {
    (0..400).map(|i| text(60 + (i * 37) % 700, 1_000_000 + i as u64)).collect()
}
