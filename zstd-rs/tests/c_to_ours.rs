//! Interop: frames produced by the reference C library decode with our decoder.
#![cfg(not(target_arch = "wasm32"))]
mod common;

use zstd_rs::{DecoderDictionary, Decompressor, Dictionary, DictionaryFormat};

fn c_compress(data: &[u8], level: i32, checksum: bool, dict: Option<&[u8]>) -> Vec<u8> {
    let mut c = match dict {
        Some(d) => zstd::bulk::Compressor::with_dictionary(level, d).unwrap(),
        None => zstd::bulk::Compressor::new(level).unwrap(),
    };
    c.set_parameter(zstd::zstd_safe::CParameter::ChecksumFlag(checksum)).unwrap();
    c.compress(data).unwrap()
}

fn ours(frame: &[u8], dict: Option<&DecoderDictionary>, cap: usize) -> Vec<u8> {
    let mut out = Vec::new();
    Decompressor::new().decompress(frame, dict, cap, &mut out).unwrap_or_else(|e| panic!("decode failed: {e}"));
    out
}

#[test]
fn decodes_c_frames_all_levels() {
    for (name, data) in common::corpus() {
        for level in [-5, 1, 3, 7, 19] {
            if level == 19 && data.len() > 500_000 {
                continue;
            }
            let frame = c_compress(&data, level, level % 2 == 1, None);
            assert_eq!(ours(&frame, None, data.len()), data, "{name} level {level}");
        }
    }
}

#[test]
fn decodes_c_streaming_frames_without_content_size() {
    for (name, data) in common::corpus().into_iter().filter(|(_, d)| d.len() <= 400_000) {
        let frame = zstd::stream::encode_all(&data[..], 3).unwrap();
        let mut enc = zstd::stream::Encoder::new(Vec::new(), 5).unwrap();
        enc.include_contentsize(false).unwrap();
        std::io::Write::write_all(&mut enc, &data).unwrap();
        let frame2 = enc.finish().unwrap();
        assert_eq!(ours(&frame, None, data.len()), data, "{name}");
        assert_eq!(ours(&frame2, None, data.len()), data, "{name} no-fcs");
    }
}

#[test]
fn decodes_c_frames_with_trained_dictionary() {
    let samples = common::samples();
    let dict_bytes = zstd::dict::from_samples(&samples, 16 * 1024).unwrap();
    let dict = DecoderDictionary::new(Dictionary::new(&dict_bytes, DictionaryFormat::Zstd).unwrap()).unwrap();
    for i in 0..200u64 {
        let data = common::text(10 + (i as usize * 53) % 3000, 5_000_000 + i);
        for level in [1, 3, 19] {
            let frame = c_compress(&data, level, i % 2 == 0, Some(&dict_bytes));
            assert_eq!(ours(&frame, Some(&dict), data.len()), data, "sample {i} level {level}");
        }
    }
}

#[test]
fn decodes_c_frames_with_raw_dictionary() {
    let raw = common::text(20_000, 42);
    let dict = DecoderDictionary::new(Dictionary::new(&raw, DictionaryFormat::Raw { id: 0 }).unwrap()).unwrap();
    for i in 0..100u64 {
        let data = common::text(10 + (i as usize * 97) % 5000, 7_000_000 + i);
        let mut c = zstd::bulk::Compressor::new(3).unwrap();
        c.set_dictionary(3, &raw).unwrap();
        let frame = c.compress(&data).unwrap();
        assert_eq!(ours(&frame, Some(&dict), data.len()), data, "sample {i}");
    }
}

#[test]
fn multiple_and_skippable_frames() {
    let a = common::text(5000, 1);
    let b = common::runs(9000, 2);
    let mut stream = c_compress(&a, 3, true, None);
    stream.extend_from_slice(&0x184D_2A53u32.to_le_bytes());
    stream.extend_from_slice(&5u32.to_le_bytes());
    stream.extend_from_slice(b"skip!");
    stream.extend(c_compress(&b, 1, false, None));
    let mut want = a.clone();
    want.extend_from_slice(&b);
    assert_eq!(ours(&stream, None, want.len()), want);
}

#[test]
fn output_limit_is_enforced() {
    let data = common::runs(100_000, 9);
    let frame = c_compress(&data, 3, false, None);
    let mut out = Vec::new();
    let err = Decompressor::new().decompress(&frame, None, data.len() - 1, &mut out).unwrap_err();
    assert_eq!(err, zstd_rs::Error::OutputLimit);
    let mut enc = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
    enc.include_contentsize(false).unwrap();
    std::io::Write::write_all(&mut enc, &data).unwrap();
    let frame = enc.finish().unwrap();
    let err = Decompressor::new().decompress(&frame, None, 1000, &mut out).unwrap_err();
    assert_eq!(err, zstd_rs::Error::OutputLimit);
}
