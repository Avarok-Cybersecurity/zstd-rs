//! Interop: frames produced by our encoder decode with the reference C library
//! (bulk and streaming decoders) and with ours, byte for byte.
#![cfg(not(target_arch = "wasm32"))]
mod common;

use zstd_rs::{CompressionConfig, Compressor, DecoderDictionary, Decompressor, Dictionary, DictionaryFormat, EncoderDictionary};

const LEVELS: &[i32] = &[-5, -1, 1, 2, 3, 4, 5, 6, 7, 9, 12, 13, 16, 17, 18, 19, 22];

fn cfg(level: i32, checksum: bool) -> CompressionConfig {
    CompressionConfig { level, checksum, ..CompressionConfig::DEFAULT }
}

fn check(name: &str, data: &[u8], frame: &[u8], cdict: Option<&[u8]>, ddict: Option<&DecoderDictionary>) {
    let c = match cdict {
        Some(d) => zstd::bulk::Decompressor::with_dictionary(d).unwrap().decompress(frame, data.len()),
        None => zstd::bulk::decompress(frame, data.len()),
    };
    assert_eq!(c.unwrap_or_else(|e| panic!("C rejects {name}: {e}")), data, "C bulk {name}");
    let mut s = match cdict {
        Some(d) => zstd::stream::Decoder::with_dictionary(frame, d).unwrap(),
        None => zstd::stream::Decoder::with_buffer(frame).unwrap(),
    };
    let mut v = Vec::new();
    std::io::Read::read_to_end(&mut s, &mut v).unwrap_or_else(|e| panic!("C stream rejects {name}: {e}"));
    assert_eq!(v, data, "C stream {name}");
    let mut out = Vec::new();
    Decompressor::new().decompress(frame, ddict, data.len(), &mut out).unwrap_or_else(|e| panic!("ours rejects {name}: {e}"));
    assert_eq!(out, data, "ours {name}");
}

#[test]
fn all_levels_all_sizes() {
    for (name, data) in common::corpus() {
        for &level in LEVELS {
            if level >= 13 && data.len() > 500_000 {
                continue;
            }
            let mut c = Compressor::new(cfg(level, level % 2 == 0)).unwrap();
            let mut frame = Vec::new();
            c.compress(&data, None, &mut frame).unwrap();
            check(&format!("{name} L{level}"), &data, &frame, None, None);
        }
    }
}

#[test]
fn context_reuse_across_frames() {
    let mut c = Compressor::new(CompressionConfig::DEFAULT).unwrap();
    for i in 0..300u64 {
        let data = common::text((i as usize * 131) % 5000, i + 11);
        let mut frame = Vec::new();
        c.compress(&data, None, &mut frame).unwrap();
        check(&format!("reuse {i}"), &data, &frame, None, None);
    }
}

#[test]
fn trained_dictionary_all_levels() {
    let samples = common::samples();
    let dict_bytes = zstd::dict::from_samples(&samples, 16 * 1024).unwrap();
    let dict = Dictionary::new(&dict_bytes, DictionaryFormat::Zstd).unwrap();
    let ddict = DecoderDictionary::new(dict.clone()).unwrap();
    for &level in &[1, 3, 5, 9, 19] {
        let edict = EncoderDictionary::new(&dict, level).unwrap();
        let mut c = Compressor::new(cfg(level, true)).unwrap();
        for i in 0..150u64 {
            let data = common::text(5 + (i as usize * 53) % 3000, 9_000_000 + i);
            let mut frame = Vec::new();
            c.compress(&data, Some(&edict), &mut frame).unwrap();
            check(&format!("dict L{level} #{i}"), &data, &frame, Some(&dict_bytes), Some(&ddict));
        }
    }
}

#[test]
fn raw_dictionary() {
    let raw = common::text(30_000, 77);
    let dict = Dictionary::new(&raw, DictionaryFormat::Raw { id: 0 }).unwrap();
    let ddict = DecoderDictionary::new(dict.clone()).unwrap();
    for &level in &[1, 3, 6, 19] {
        let edict = EncoderDictionary::new(&dict, level).unwrap();
        let mut c = Compressor::new(cfg(level, false)).unwrap();
        for i in 0..60u64 {
            let data = common::text(10 + (i as usize * 97) % 9000, 8_000_000 + i);
            let mut frame = Vec::new();
            c.compress(&data, Some(&edict), &mut frame).unwrap();
            check(&format!("raw dict L{level} #{i}"), &data, &frame, Some(&raw), Some(&ddict));
        }
    }
}

#[test]
fn golden_compression_inputs() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/golden-compression");
    for e in std::fs::read_dir(root).unwrap() {
        let p = e.unwrap().path();
        let data = std::fs::read(&p).unwrap();
        for &level in LEVELS {
            let mut frame = Vec::new();
            Compressor::new(cfg(level, true)).unwrap().compress(&data, None, &mut frame).unwrap();
            check(&format!("{} L{level}", p.display()), &data, &frame, None, None);
        }
    }
}

#[test]
fn dictionary_actually_helps_small_messages() {
    let samples = common::samples();
    let dict_bytes = zstd::dict::from_samples(&samples, 16 * 1024).unwrap();
    let dict = Dictionary::new(&dict_bytes, DictionaryFormat::Zstd).unwrap();
    for &level in &[1, 3, 19] {
        let edict = EncoderDictionary::new(&dict, level).unwrap();
        let cfg = CompressionConfig { level, dict_id: false, ..CompressionConfig::DEFAULT };
        let mut c = Compressor::new(cfg).unwrap();
        let mut cz = zstd::bulk::Compressor::with_dictionary(level, &dict_bytes).unwrap();
        cz.set_parameter(zstd::zstd_safe::CParameter::DictIdFlag(false)).unwrap();
        let (mut ours, mut plain, mut theirs, mut raw) = (0, 0, 0, 0);
        for i in 0..100u64 {
            let msg = common::text(60 + (i as usize * 13) % 400, 3_000_000 + i);
            let (mut a, mut b) = (Vec::new(), Vec::new());
            c.compress(&msg, Some(&edict), &mut a).unwrap();
            c.compress(&msg, None, &mut b).unwrap();
            ours += a.len();
            plain += b.len();
            theirs += cz.compress(&msg).unwrap().len();
            raw += msg.len();
        }
        assert!(ours * 10 < plain * 7, "L{level}: dictionary saves too little: {ours} vs {plain} without (raw {raw})");
        assert!(ours * 100 < theirs * 110, "L{level}: {ours} bytes vs C's {theirs} with the same dictionary");
    }
}

#[test]
fn magicless_frames_interoperate() {
    use zstd::zstd_safe::{CParameter, DParameter, FrameFormat as CFormat};
    use zstd_rs::FrameFormat;
    for (i, level) in [1, 3, 19].into_iter().enumerate() {
        let data = common::text(50 + i * 700, 40 + i as u64);
        let cfg = CompressionConfig { level, checksum: true, format: FrameFormat::Magicless, ..CompressionConfig::DEFAULT };
        let mut ours = Vec::new();
        Compressor::new(cfg).unwrap().compress(&data, None, &mut ours).unwrap();
        let mut standard = Vec::new();
        Compressor::new(CompressionConfig { format: FrameFormat::Standard, ..cfg }).unwrap().compress(&data, None, &mut standard).unwrap();
        assert_eq!(ours.len() + 4, standard.len());
        let mut d = zstd::bulk::Decompressor::new().unwrap();
        d.set_parameter(DParameter::Format(CFormat::Magicless)).unwrap();
        assert_eq!(d.decompress(&ours, data.len()).unwrap(), data, "C decodes our magicless frame");
        let mut c = zstd::bulk::Compressor::new(level).unwrap();
        c.set_parameter(CParameter::Format(CFormat::Magicless)).unwrap();
        let theirs = c.compress(&data).unwrap();
        let mut out = Vec::new();
        Decompressor::new().decompress_format(FrameFormat::Magicless, &theirs, None, data.len(), &mut out).unwrap();
        assert_eq!(out, data, "we decode C's magicless frame");
        assert!(Decompressor::new().decompress(&theirs, None, data.len(), &mut Vec::new()).is_err());
    }
}
