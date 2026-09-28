//! Regenerates `fuzz/dict.bin`: a ZDICT dictionary trained on the synthetic test
//! samples in `tests/common` (never on real traffic).
#[path = "../tests/common/mod.rs"]
mod common;

fn main() {
    let dict = zstd::dict::from_samples(&common::samples(), 16 * 1024).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/dict.bin");
    std::fs::write(&path, &dict).unwrap();
    println!("wrote {} bytes to {}", dict.len(), path.display());
}
