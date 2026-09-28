//! The job API: frames assembled from independently compressed jobs are valid
//! (C and ours decode them) and byte-identical for every executor.
#![cfg(not(target_arch = "wasm32"))]
mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use zstd_rs::par::{self, Executor, Sequential};
use zstd_rs::{CompressionConfig, DecoderDictionary, Decompressor, Dictionary, DictionaryFormat, EncoderDictionary};

/// Test-only executor: several threads pull jobs in a scrambled order.
struct Scrambled(usize);

impl Executor for Scrambled {
    fn map<T: Sync, R: Send>(&self, items: &[T], f: &(dyn Fn(&T) -> R + Sync)) -> Vec<R> {
        let next = AtomicUsize::new(0);
        let slots: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
        std::thread::scope(|s| {
            for _ in 0..self.0 {
                s.spawn(|| loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    if k >= items.len() {
                        break;
                    }
                    let i = items.len() - 1 - k;
                    *slots[i].lock().unwrap() = Some(f(&items[i]));
                });
            }
        });
        slots.into_iter().map(|m| m.into_inner().unwrap().unwrap()).collect()
    }
}

#[test]
fn jobs_are_executor_independent_and_valid() {
    let data = common::mixed(900_000, 3);
    for level in [1, 3, 7, 19] {
        let cfg = CompressionConfig { level, checksum: true, ..CompressionConfig::DEFAULT };
        for (job, overlap) in [(128 * 1024, 0), (100_000, 32 * 1024), (300_000, 1 << 20)] {
            let jobs = par::plan(data.len(), job, overlap).unwrap();
            let mut a = Vec::new();
            let mut b = Vec::new();
            par::compress_frame(&Sequential, &cfg, &data, None, &jobs, &mut a).unwrap();
            par::compress_frame(&Scrambled(4), &cfg, &data, None, &jobs, &mut b).unwrap();
            assert_eq!(a, b, "L{level} job {job}: executors disagree");
            assert_eq!(zstd::bulk::decompress(&a, data.len()).unwrap(), data, "C L{level} job {job}");
            let mut out = Vec::new();
            Decompressor::new().decompress(&a, None, data.len(), &mut out).unwrap();
            assert_eq!(out, data);
        }
    }
}

#[test]
fn first_job_uses_the_dictionary() {
    let samples = common::samples();
    let raw = zstd::dict::from_samples(&samples, 8192).unwrap();
    let dict = Dictionary::new(&raw, DictionaryFormat::Zstd).unwrap();
    let edict = EncoderDictionary::new(&dict, 3).unwrap();
    let ddict = DecoderDictionary::new(dict).unwrap();
    let data = common::text(50_000, 99);
    let jobs = par::plan(data.len(), 4096, 2048).unwrap();
    let mut a = Vec::new();
    par::compress_frame(&Scrambled(3), &CompressionConfig::DEFAULT, &data, Some(&edict), &jobs, &mut a).unwrap();
    let c = zstd::bulk::Decompressor::with_dictionary(&raw).unwrap().decompress(&a, data.len()).unwrap();
    assert_eq!(c, data);
    let mut out = Vec::new();
    Decompressor::new().decompress(&a, Some(&ddict), data.len(), &mut out).unwrap();
    assert_eq!(out, data);
}

#[test]
fn independent_frames_decode_in_parallel() {
    let data = common::mixed(700_000, 8);
    let mut frames = Vec::new();
    par::compress_frames(&Scrambled(4), &CompressionConfig::FAST, &data, None, 64 * 1024, &mut frames).unwrap();
    let mut seq = Vec::new();
    par::compress_frames(&Sequential, &CompressionConfig::FAST, &data, None, 64 * 1024, &mut seq).unwrap();
    assert_eq!(frames, seq);
    let mut out = Vec::new();
    par::decompress_frames(&Scrambled(4), &frames, None, data.len(), &mut out).unwrap();
    assert_eq!(out, data);
    assert_eq!(par::decompress_frames(&Sequential, &frames, None, data.len() - 1, &mut Vec::new()), Err(zstd_rs::Error::OutputLimit));
}

#[test]
fn bad_plans_are_rejected() {
    let data = common::text(10_000, 1);
    let mut jobs = par::plan(data.len(), 4096, 0).unwrap();
    jobs.swap(0, 1);
    let r = par::compress_frame(&Sequential, &CompressionConfig::FAST, &data, None, &jobs, &mut Vec::new());
    assert!(matches!(r, Err(zstd_rs::Error::Parameter(_))));
    assert!(par::plan(10, 100, 0).is_err());
    let empty = par::plan(0, 4096, 0).unwrap();
    let mut f = Vec::new();
    par::compress_frame(&Sequential, &CompressionConfig::FAST, &[], None, &empty, &mut f).unwrap();
    assert_eq!(zstd::bulk::decompress(&f, 0).unwrap(), b"");
}
