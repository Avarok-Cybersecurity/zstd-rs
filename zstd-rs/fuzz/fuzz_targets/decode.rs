//! Arbitrary bytes into the decoder, with and without a dictionary: must never
//! panic, and must respect the output cap.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;
use zstd_rs::{DecoderDictionary, Decompressor, Dictionary, DictionaryFormat};

fn dict() -> &'static DecoderDictionary {
    static D: OnceLock<DecoderDictionary> = OnceLock::new();
    D.get_or_init(|| DecoderDictionary::new(Dictionary::new(include_bytes!("../dict.bin"), DictionaryFormat::Zstd).unwrap()).unwrap())
}

fuzz_target!(|data: &[u8]| {
    let Some((&sel, frame)) = data.split_first() else { return };
    let cap = [0usize, 1, 100, 1 << 16, 1 << 20][(sel >> 1) as usize % 5];
    let d = if sel & 1 == 1 { Some(dict()) } else { None };
    let mut out = Vec::new();
    let _ = Decompressor::new().decompress(frame, d, cap, &mut out);
    assert!(out.len() <= cap);
    let _ = Dictionary::new(frame, DictionaryFormat::Zstd).map(DecoderDictionary::new);
});
