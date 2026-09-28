//! Compress arbitrary input at a fuzzer-chosen level (with/without dictionary,
//! checksum), then decode with ours and with the C library: both must round-trip.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;
use zstd_rs::{
    CompressionConfig, Compressor, DecoderDictionary, Decompressor, Dictionary, DictionaryFormat, EncoderDictionary, FrameFormat,
};

const RAW: &[u8] = include_bytes!("../dict.bin");

fn dicts() -> &'static (Dictionary, DecoderDictionary) {
    static D: OnceLock<(Dictionary, DecoderDictionary)> = OnceLock::new();
    D.get_or_init(|| {
        let d = Dictionary::new(RAW, DictionaryFormat::Zstd).unwrap();
        (d.clone(), DecoderDictionary::new(d).unwrap())
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&sel, input)) = data.split_first() else { return };
    let levels = [-3, 1, 2, 3, 4, 5, 6, 8, 12, 16, 19];
    let level = levels[(sel & 15) as usize % levels.len()];
    let use_dict = sel & 16 != 0;
    let format = if input.len() % 2 == 1 { FrameFormat::Magicless } else { FrameFormat::Standard };
    let cfg =
        CompressionConfig { level, checksum: sel & 32 != 0, window_log: 10 + (sel as u32 >> 6) * 4, format, ..CompressionConfig::DEFAULT };
    let (dict, ddict) = dicts();
    let edict = use_dict.then(|| EncoderDictionary::new(dict, level).unwrap());
    let mut frame = Vec::new();
    Compressor::new(cfg).unwrap().compress(input, edict.as_ref(), &mut frame).unwrap();
    let mut out = Vec::new();
    Decompressor::new().decompress_format(format, &frame, use_dict.then_some(ddict), input.len(), &mut out).unwrap();
    assert_eq!(out, input);
    let mut d = if use_dict { zstd::bulk::Decompressor::with_dictionary(RAW).unwrap() } else { zstd::bulk::Decompressor::new().unwrap() };
    if format == FrameFormat::Magicless {
        d.set_parameter(zstd::zstd_safe::DParameter::Format(zstd::zstd_safe::FrameFormat::Magicless)).unwrap();
    }
    let c = d.decompress(&frame, input.len());
    assert_eq!(c.expect("C decoder rejects our frame"), input);
});
