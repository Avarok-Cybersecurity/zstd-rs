//! A native thread executor for zstd-rs's parallel job API.
//!
//! The core crate is `no_std` and single-threaded: it only plans and runs single
//! jobs. This crate supplies [`ThreadPool`], which runs jobs on scoped OS threads.
//! The thread count is always explicit, and results come back in input order, so
//! output is byte-identical to `zstd_rs::par::Sequential`.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use zstd_rs::par::Executor;

/// Runs jobs on `threads` scoped OS threads per call.
#[derive(Clone, Copy, Debug)]
pub struct ThreadPool {
    threads: usize,
}

/// Returned when a pool is requested with zero threads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZeroThreads;

impl core::fmt::Display for ZeroThreads {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("a thread pool needs at least one thread")
    }
}

impl std::error::Error for ZeroThreads {}

impl ThreadPool {
    /// Creates a pool that uses exactly `threads` threads (at least 1).
    pub fn new(threads: usize) -> Result<ThreadPool, ZeroThreads> {
        if threads == 0 {
            return Err(ZeroThreads);
        }
        Ok(ThreadPool { threads })
    }

    /// The configured thread count.
    pub fn threads(&self) -> usize {
        self.threads
    }
}

impl Executor for ThreadPool {
    fn map<T: Sync, R: Send>(&self, items: &[T], f: &(dyn Fn(&T) -> R + Sync)) -> Vec<R> {
        let workers = self.threads.min(items.len());
        if workers <= 1 {
            return items.iter().map(f).collect();
        }
        let next = AtomicUsize::new(0);
        let slots: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    *slots[i].lock().unwrap_or_else(|p| p.into_inner()) = Some(r);
                });
            }
        });
        slots
            .into_iter()
            .map(|m| m.into_inner().unwrap_or_else(|p| p.into_inner()).expect("every job slot is filled before the scope ends"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zstd_rs::par::{self, Sequential};
    use zstd_rs::{CompressionConfig, Decompressor};

    fn sample(n: usize) -> Vec<u8> {
        (0..n).map(|i| ((i * 31) ^ (i >> 7)) as u8 % 97).collect()
    }

    #[test]
    fn matches_sequential_byte_for_byte() {
        let data = sample(1 << 20);
        let jobs = par::plan(data.len(), 128 * 1024, 32 * 1024).unwrap();
        for level in [1, 3, 9] {
            let cfg = CompressionConfig { level, ..CompressionConfig::DEFAULT };
            let (mut a, mut b) = (Vec::new(), Vec::new());
            par::compress_frame(&Sequential, &cfg, &data, None, &jobs, &mut a).unwrap();
            par::compress_frame(&ThreadPool::new(4).unwrap(), &cfg, &data, None, &jobs, &mut b).unwrap();
            assert_eq!(a, b);
            let mut out = Vec::new();
            Decompressor::new().decompress(&b, None, data.len(), &mut out).unwrap();
            assert_eq!(out, data);
        }
    }

    #[test]
    fn zero_threads_is_an_error() {
        assert_eq!(ThreadPool::new(0).unwrap_err(), ZeroThreads);
    }
}
