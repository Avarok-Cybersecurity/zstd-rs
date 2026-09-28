# zstd-rs

A pure-Rust implementation of [Zstandard](https://www.rfc-editor.org/rfc/rfc8878) (RFC 8878). It includes both the compressor and the decompressor.

- **Runs in the browser.** The core is `#![no_std]` + `alloc` and `#![forbid(unsafe_code)]`, with no C and no `std`. It builds for `wasm32-unknown-unknown`, and the resulting module has **zero imports**.
- **Interoperates with the C library in both directions.** Our frames decode with the reference `zstd`, and its frames decode with ours. This is tested at every level, from 0 bytes to 3 MB, with and without dictionaries. The official golden vectors also pass.
- **Has real compression levels.** It implements the reference's level table from -7 to 22:
  - the fast and dfast strategies;
  - greedy, lazy and lazy2 on hash chains;
  - an optimal parser (btopt/btultra/btultra2-style price DP).

  Compression ratio is within about 2-5% of C at the same level, and better than C on small inputs.
- **Supports dictionaries.** It loads zstd-format dictionaries (`zstd --train` / ZDICT output, with entropy tables and repeat offsets) as well as raw-content dictionaries.
  - Dictionary match tables are built once and *attached* to each frame, never copied. This is what makes small messages fast.
- **Safe to decode untrusted data.** Malformed input returns an error and never panics, and every decode call takes a caller-supplied output cap.
- **Parallel without owning threads.** The core only plans and runs independent jobs. You bring the `Executor` (threads, Web Workers, or sequential), and the output is byte-identical whichever one you use.

## Crates

| crate | what |
|---|---|
| `zstd-rs` | the codec (`no_std` + `alloc`) |
| `zstd-rs-exec` | `ThreadPool`: a native `std::thread` executor for the job API |
| `zstd-rs-wasm` | raw wasm ABI (handle registry, no imports) plus a JS wrapper (`js/zstd-rs.mjs`) |

## Usage

```rust
use zstd_rs::{CompressionConfig, Compressor, Decompressor};

let data = b"hello hello hello hello".repeat(100);

// Every setting is explicit; presets are named constants (FAST = 1, DEFAULT = 3, HIGH = 19).
let mut c = Compressor::new(CompressionConfig::DEFAULT)?;
let mut frame = Vec::new();
c.compress(&data, None, &mut frame)?;

// The decoder never writes more than the cap you give it.
let mut out = Vec::new();
Decompressor::new().decompress(&frame, None, 1 << 20, &mut out)?;
assert_eq!(out, data);
# Ok::<(), zstd_rs::Error>(())
```

To use a dictionary, load it once and share it across frames and threads:

```rust
use zstd_rs::*;
# let dict_bytes = b"...trained with zstd --train...".to_vec();
# let msg = b"payload";
let dict = Dictionary::new(&dict_bytes, DictionaryFormat::Detect)?;
let enc = EncoderDictionary::new(&dict, 3)?;          // bound to a level, like a CDict
let dec = DecoderDictionary::new(dict)?;
let mut c = Compressor::new(CompressionConfig::MESSAGE)?; // level 3, no dict ID in the header
let mut frame = Vec::new();
c.compress(msg, Some(&enc), &mut frame)?;
let mut out = Vec::new();
Decompressor::new().decompress(&frame, Some(&dec), 4096, &mut out)?;
# Ok::<(), zstd_rs::Error>(())
```

To compress large inputs in parallel, split one frame into jobs and supply an executor:

```rust
use zstd_rs::{par, CompressionConfig};
# let data = vec![0u8; 1 << 20];
let jobs = par::plan(data.len(), 256 * 1024, 64 * 1024)?; // job size, overlap
let mut frame = Vec::new();
par::compress_frame(&par::Sequential, &CompressionConfig::DEFAULT, &data, None, &jobs, &mut frame)?;
// With zstd-rs-exec: par::compress_frame(&ThreadPool::new(8)?, ...) gives identical bytes.
# Ok::<(), zstd_rs::Error>(())
```

Other parallel entry points:
- `par::compress_frames` / `par::decompress_frames` work on independent frames.
- In the browser, the same pure functions are exported to JS: `zr_frame_begin`, `zr_compress_job`, `zr_frame_end`. A Web Worker pool can run the jobs.

## Interoperability and testing

- `tests/c_to_ours.rs` decodes frames from the reference C library:
  - levels -5..19, sizes 0 B to 3 MB, text, random, runs and mixed data;
  - streaming frames without a content size;
  - ZDICT-trained and raw dictionaries;
  - multiple frames and skippable frames.
- `tests/ours_to_c.rs` takes our frames at 17 levels and checks them with both the C **bulk and streaming** decoders, and with ours.
- `tests/golden.rs` runs the reference repository's `golden-decompression`, `golden-decompression-errors` and `golden-compression` vectors.
- `tests/par.rs` checks that frames built from jobs are valid, and identical across executors.
- Fuzzing (`zstd-rs/fuzz`, cargo-fuzz) has three targets:
  - `decode`: arbitrary bytes, with and without a dictionary, never panics and respects the cap.
  - `roundtrip`: every level ± dictionary; our decoder and C's must both reproduce the input.
  - `diff_c`: differential decoding against C.

### Where we are stricter than the reference

The reference's one-shot path accepts some malformed input that its own streaming path, or RFC 8878, rejects. We reject it too:
- raw and RLE blocks larger than `Block_Maximum_Size`;
- Huffman streams that are not consumed exactly, or that lack an end marker. The reference's fast 4-stream loop checks only the output count.

Legacy (pre-v0.8) zstd formats are not supported.

## Benchmarks

The benchmarks run on a real ILM traffic corpus (chat, control, markdown, WebSocket JSON, Yjs updates and snapshots, file chunks).

- Each cell is **ratio / compress µs / decompress µs** per frame. Ratio is the sum of compressed bytes over the sum of raw bytes.
- `-dict` means a 32 KB ZDICT dictionary trained on a *disjoint* training corpus. No dictionary ID is written, for either implementation.
- `dec:X<-zr3` decodes zstd-rs level-3 frames with decoder X, which gives a like-for-like decoder comparison.
- Yjs rows use the current CBOR byte-string wire form. `-legacy` rows use the old `Array.from` integer arrays.

### V8 (Node 22, wasm32-unknown-unknown)

zstd-C is absent here because it does not build for this target.

| class (n) | zr-1 | zr-3 | zr-3-dict | zr-19-dict | ruzstd-fastest | dec:ruzstd<-zr3 | brotli-q4 | deflate-l1 | lz4_flex | rill |
|---|---|---|---|---|---|---|---|---|---|---|
| chat (49) | 0.856 / 6.0 / 2.52 | 0.851 / 6.4 / 2.61 | 0.357 / 1.5 / 0.55 | 0.345 / 25.3 / 0.52 | 1.036 / 11.0 / 0.85 | 0.851 / 6.4 / 5.00 | 0.803 / 20.3 / 6.85 | 0.829 / 5.5 / 3.67 | 0.936 / 0.4 / 0.09 | 0.224 / 6.2 / 1.89 |
| ctrl (5) | 0.917 / 5.2 / 1.47 | 0.914 / 5.4 / 1.47 | 0.355 / 1.1 / 0.40 | 0.339 / 19.0 / 0.36 | 1.094 / 10.3 / 0.73 | 0.914 / 5.4 / 2.86 | 0.848 / 17.2 / 6.08 | 0.886 / 5.0 / 2.24 | 0.931 / 0.3 / 0.07 | 0.196 / 2.5 / 0.76 |
| markdown (5) | 0.495 / 15.4 / 5.37 | 0.482 / 16.4 / 5.55 | 0.416 / 12.3 / 3.20 | 0.375 / 180.1 / 4.40 | 0.623 / 25.8 / 7.98 | 0.482 / 16.5 / 13.89 | 0.461 / 49.3 / 13.25 | 0.499 / 10.2 / 7.25 | 0.647 / 2.3 / 0.57 | 0.324 / 109.9 / 26.17 |
| ws-json (9) | 0.195 / 35.5 / 9.74 | 0.196 / 40.9 / 9.86 | 0.185 / 50.6 / 9.64 | 0.173 / 2212 / 10.01 | 0.249 / 77.7 / 37.26 | 0.196 / 41.0 / 37.18 | 0.194 / 123.2 / 32.22 | 0.238 / 23.6 / 17.70 | 0.328 / 6.8 / 2.14 | 0.136 / 159.3 / 38.14 |
| yjs-inc (30) | 0.805 / 6.7 / 2.44 | 0.796 / 6.9 / 2.47 | 0.429 / 2.4 / 0.90 | 0.411 / 31.6 / 0.85 | 0.960 / 11.9 / 1.00 | 0.796 / 7.1 / 5.31 | 0.755 / 19.9 / 7.02 | 0.749 / 5.7 / 3.79 | 0.905 / 0.5 / 0.09 | 0.298 / 7.6 / 2.41 |
| yjs-snap (3) | 0.129 / 60.3 / 19.83 | 0.122 / 64.0 / 20.46 | 0.116 / 70.4 / 21.03 | 0.108 / 5042 / 23.46 | 0.151 / 164.4 / 57.98 | 0.122 / 63.6 / 54.93 | 0.115 / 200.8 / 52.72 | 0.228 / 58.1 / 41.45 | 0.170 / 15.4 / 4.53 | 0.099 / 658.7 / 154.24 |
| file-chunk (18) | 0.935 / 117.9 / 6.08 | 0.930 / 137.7 / 5.24 | 0.930 / 189.7 / 5.37 | 0.922 / 1794 / 11.16 | 0.937 / 564.9 / 18.54 | 0.930 / 137.8 / 16.34 | 0.930 / 320.9 / 172.82 | 0.932 / 169.8 / 124.55 | 0.944 / 11.0 / 2.36 | 0.955 / 283.0 / 103.69 |

Native results (Apple M-series, compared with zstd-C) are in [PROGRESS.md](PROGRESS.md).

The wasm module (encoder plus decoder, `zstd-rs-wasm`) is 164 KB, or 64 KB gzipped.

## Limitations

- The API is one-shot: whole input in, whole frame out. There is no streaming `Read`/`Write` adapter yet. The job API covers large inputs.
- Levels 13-22 use hash chains rather than the reference's binary trees. Their ratio is within about 2-5% of C, and they are slower on large inputs.
- On large inputs, speed is about 55-75% of the C library natively. Small frames with a dictionary are closer.
- There is no built-in dictionary trainer. Train with `zstd --train` (ZDICT); both formats are loaded.
- Legacy zstd formats (pre-v0.8) are not supported.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
The test vectors in `zstd-rs/tests/golden` come from the reference repository (BSD-3-Clause / GPL-2.0).
