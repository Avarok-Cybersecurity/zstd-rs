//! The reference repository's golden vectors (tests/golden-*), vendored under tests/golden.
#![cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use zstd_rs::{Decompressor, Error};

fn files(dir: &str) -> Vec<(String, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(dir);
    let mut v: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap()))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "{dir} is empty");
    v
}

#[test]
fn golden_decompression_matches_reference() {
    for (name, frame) in files("golden-decompression") {
        let want = zstd::stream::decode_all(&frame[..]).unwrap();
        let mut out = Vec::new();
        Decompressor::new().decompress(&frame, None, 1 << 24, &mut out).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(out, want, "{name}");
    }
}

#[test]
fn golden_decompression_errors_are_rejected() {
    for (name, frame) in files("golden-decompression-errors") {
        assert!(zstd::stream::decode_all(&frame[..]).is_err(), "reference accepts {name}");
        let mut out = Vec::new();
        let r: Result<usize, Error> = Decompressor::new().decompress(&frame, None, 1 << 24, &mut out);
        assert!(r.is_err(), "{name} decoded to {} bytes", out.len());
    }
}
