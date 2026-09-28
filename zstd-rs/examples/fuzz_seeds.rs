//! Writes seed corpora for the fuzz targets: frames from the C library and from
//! zstd-rs at several levels, with and without the fuzz dictionary.
#[path = "../tests/common/mod.rs"]
mod common;
use zstd_rs::{CompressionConfig, Compressor, Dictionary, DictionaryFormat, EncoderDictionary};

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus");
    let raw = std::fs::read(root.parent().unwrap().join("dict.bin")).unwrap();
    let dict = Dictionary::new(&raw, DictionaryFormat::Zstd).unwrap();
    for t in ["decode", "roundtrip", "diff_c"] {
        std::fs::create_dir_all(root.join(t)).unwrap();
    }
    let mut n = 0;
    let inputs: Vec<Vec<u8>> = (0..24)
        .map(|i| match i % 4 {
            0 => common::text(20 + i * 97, i as u64),
            1 => common::runs(10 + i * 300, i as u64),
            2 => common::mixed(50 + i * 500, i as u64),
            _ => common::random(i * 7, i as u64),
        })
        .collect();
    for (i, data) in inputs.iter().enumerate() {
        for level in [1, 3, 19] {
            let c = zstd::bulk::compress(data, level).unwrap();
            let mut o = Vec::new();
            let edict = EncoderDictionary::new(&dict, level).unwrap();
            Compressor::new(CompressionConfig { level, checksum: i % 2 == 0, ..CompressionConfig::DEFAULT })
                .unwrap()
                .compress(data, Some(&edict), &mut o)
                .unwrap();
            let cd = zstd::bulk::Compressor::with_dictionary(level, &raw).unwrap().compress(data).unwrap();
            for (name, bytes, sel) in [("c", &c, 0u8), ("ours-dict", &o, 1), ("c-dict", &cd, 1)] {
                let mut seed = vec![sel | (3 << 1)];
                seed.extend_from_slice(bytes);
                std::fs::write(root.join("decode").join(format!("{name}-{i}-{level}")), &seed).unwrap();
                std::fs::write(root.join("diff_c").join(format!("{name}-{i}-{level}")), bytes).unwrap();
                n += 1;
            }
        }
        let mut seed = vec![(i % 256) as u8];
        seed.extend_from_slice(&data[..data.len().min(4000)]);
        std::fs::write(root.join("roundtrip").join(format!("in-{i}")), &seed).unwrap();
    }
    println!("wrote {n} seeds");
}
