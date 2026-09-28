//! Quick ratio/speed comparison against the C library on the test corpus.
#[path = "../tests/common/mod.rs"]
mod common;
use std::time::Instant;
use zstd_rs::{CompressionConfig, Compressor};

fn main() {
    let inputs = [("text-100k", common::text(100_000, 5)), ("mixed-1M", common::mixed(1_000_000, 6)), ("text-600", common::text(600, 7))];
    for (name, data) in &inputs {
        for level in [1, 2, 3, 5, 7, 9, 12, 16, 19] {
            let mut c = Compressor::new(CompressionConfig { level, ..CompressionConfig::DEFAULT }).unwrap();
            let mut out = Vec::new();
            let t = Instant::now();
            let reps = if data.len() > 100_000 { 3 } else { 20 };
            for _ in 0..reps {
                out.clear();
                c.compress(data, None, &mut out).unwrap();
            }
            let ours_t = t.elapsed().as_secs_f64() / reps as f64;
            let t = Instant::now();
            let mut cz = zstd::bulk::Compressor::new(level).unwrap();
            let mut cout = Vec::new();
            for _ in 0..reps {
                cout = cz.compress(data).unwrap();
            }
            let c_t = t.elapsed().as_secs_f64() / reps as f64;
            println!(
                "{name:10} L{level:2}: ours {:8} ({:6.1} MB/s)  C {:8} ({:6.1} MB/s)  size ratio {:.3}",
                out.len(),
                data.len() as f64 / ours_t / 1e6,
                cout.len(),
                data.len() as f64 / c_t / 1e6,
                out.len() as f64 / cout.len() as f64
            );
        }
    }
}
