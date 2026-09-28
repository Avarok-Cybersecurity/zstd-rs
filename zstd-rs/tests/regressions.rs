//! Inputs found by fuzzing, kept as regression tests. Each must be rejected, as
//! the reference decoder rejects it.
use zstd_rs::Decompressor;

#[test]
fn fuzz_findings_are_rejected() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regressions");
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        let data = std::fs::read(&p).unwrap();
        let r = Decompressor::new().decompress(&data, None, 1 << 20, &mut Vec::new());
        assert!(r.is_err(), "{} decoded", p.display());
        n += 1;
    }
    assert!(n > 0);
}
