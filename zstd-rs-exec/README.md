# zstd-rs-exec

A native thread executor for the parallel job API of [`zstd-rs`](../zstd-rs).
The core crate stays `no_std` and single-threaded; this crate supplies
`ThreadPool`, an implementation of `zstd_rs::par::Executor` on `std::thread::scope`.
The thread count is always explicit, and the output is byte-identical to `Sequential`.

```rust
use zstd_rs::{par, CompressionConfig};
use zstd_rs_exec::ThreadPool;

let data = vec![7u8; 1 << 20];
let pool = ThreadPool::new(4).unwrap();
let jobs = par::plan(data.len(), 256 * 1024, 64 * 1024).unwrap();
let mut frame = Vec::new();
par::compress_frame(&pool, &CompressionConfig::DEFAULT, &data, None, &jobs, &mut frame).unwrap();
```
