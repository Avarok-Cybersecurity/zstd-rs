# zstd-rs

A pure-Rust implementation of [Zstandard](https://www.rfc-editor.org/rfc/rfc8878) (RFC 8878). It includes both the compressor and the decompressor.

- **Runs in the browser.** The core is `#![no_std]` + `alloc` and `#![forbid(unsafe_code)]`, with no C and no `std`. It builds for `wasm32-unknown-unknown`, and the resulting module has **zero imports**.
- **Interoperates with the C library in both directions.** Our frames decode with the reference `zstd`, and its frames decode with ours. This is tested at every level, from 0 bytes to 3 MB, with and without dictionaries. The official golden vectors also pass.
- **Has real compression levels.** It implements the reference's level table from -7 to 22:
  - the fast and dfast strategies;
  - greedy, lazy and lazy2 on hash chains;
  - an optimal parser (btopt/btultra/btultra2-style price DP).

  Compression ratio is within about 1% of C at the same level, and sometimes better on small inputs.
- **Magicless frames.** The reference's `ZSTD_f_zstd1_magicless` format is supported, which saves 4 bytes per frame.
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

| class (n) | zstd-rs-1 | zstd-rs-3 | zstd-rs-3-dict | zstd-rs-19-dict | ruzstd-fastest | ruzstd decoding zstd-rs-3 frames | brotli-q1-w16 | brotli-q4-w18 | deflate-l1 | lz4_flex |
|---|---|---|---|---|---|---|---|---|---|---|
| chat (49) | 0.856 / 5.2 / 2.49 | 0.851 / 5.5 / 2.56 | 0.357 / 1.4 / 0.54 | 0.345 / 25.1 / 0.51 | 1.036 / 11.0 / 0.85 | 0.851 / 5.5 / 4.99 | 0.934 / 7.0 / 8.39 | 0.803 / 20.2 / 6.81 | 0.829 / 5.7 / 3.75 | 0.936 / 0.4 / 0.10 |
| ctrl (5) | 0.916 / 4.6 / 1.48 | 0.914 / 4.8 / 1.47 | 0.355 / 1.0 / 0.39 | 0.339 / 18.9 / 0.36 | 1.094 / 10.4 / 0.74 | 0.914 / 4.8 / 2.85 | 0.977 / 6.2 / 5.87 | 0.848 / 17.3 / 6.10 | 0.886 / 5.2 / 2.27 | 0.931 / 0.3 / 0.07 |
| markdown (5) | 0.495 / 13.3 / 5.09 | 0.482 / 14.7 / 5.23 | 0.416 / 11.1 / 2.93 | 0.375 / 180.0 / 4.09 | 0.623 / 26.1 / 8.02 | 0.482 / 14.8 / 13.68 | 0.521 / 20.0 / 17.61 | 0.461 / 49.8 / 13.46 | 0.499 / 10.5 / 7.43 | 0.647 / 2.3 / 0.58 |
| ws-json (9) | 0.195 / 27.7 / 9.38 | 0.196 / 34.1 / 9.48 | 0.185 / 44.6 / 9.20 | 0.173 / 2253.0 / 9.59 | 0.249 / 78.3 / 36.85 | 0.196 / 34.1 / 36.47 | 0.370 / 88.9 / 68.52 | 0.194 / 123.9 / 31.79 | 0.238 / 23.9 / 17.88 | 0.328 / 6.8 / 2.19 |
| yjs-inc (30) | 0.811 / 6.3 / 2.58 | 0.804 / 6.4 / 2.49 | 0.429 / 2.3 / 0.98 | 0.411 / 35.6 / 0.98 | 0.960 / 13.8 / 1.18 | 0.804 / 6.7 / 6.20 | 0.837 / 8.7 / 9.81 | 0.755 / 22.2 / 8.21 | 0.749 / 7.0 / 4.64 | 0.905 / 0.6 / 0.11 |
| yjs-snap (3) | 0.129 / 56.8 / 19.66 | 0.122 / 66.9 / 21.92 | 0.116 / 79.3 / 23.94 | 0.108 / 6089.2 / 24.82 | 0.151 / 181.4 / 62.12 | 0.122 / 62.9 / 60.55 | 0.474 / 355.0 / 290.96 | 0.115 / 240.8 / 64.63 | 0.228 / 79.5 / 61.42 | 0.170 / 23.0 / 6.69 |
| file-chunk (18) | 0.932 / 95.2 / 5.88 | 0.930 / 107.7 / 4.84 | 0.930 / 154.0 / 4.99 | 0.922 / 1791.9 / 10.28 | 0.937 / 556.3 / 18.65 | 0.930 / 106.8 / 16.46 | 0.949 / 294.7 / 100.32 | 0.930 / 322.6 / 175.23 | 0.932 / 170.7 / 125.05 | 0.944 / 10.9 / 2.41 |

### Native (Apple M-series arm64), compared with the C library (zstd 1.5.7) at the same level

| class (n) | zstd-rs-1 | zstd-c-1 | zstd-rs-3 | zstd-c-3 | zstd-rs-3-dict | zstd-c-3-dict | zstd-rs-19-dict | zstd-c-19-dict | ruzstd-fastest |
|---|---|---|---|---|---|---|---|---|---|
| chat (49) | 0.856 / 4.0 / 2.13 | 0.858 / 4.0 / 2.13 | 0.851 / 4.3 / 2.21 | 0.851 / 4.2 / 2.22 | 0.357 / 1.0 / 0.50 | 0.349 / 0.7 / 0.34 | 0.345 / 17.0 / 0.48 | 0.354 / 19.8 / 0.33 | 1.036 / 13.9 / 0.65 |
| ctrl (5) | 0.916 / 3.5 / 1.24 | 0.919 / 3.9 / 1.26 | 0.914 / 3.7 / 1.25 | 0.915 / 4.0 / 1.25 | 0.355 / 0.7 / 0.38 | 0.351 / 0.5 / 0.24 | 0.339 / 12.6 / 0.34 | 0.357 / 16.1 / 0.21 | 1.094 / 13.2 / 0.55 |
| markdown (5) | 0.495 / 9.7 / 3.95 | 0.494 / 6.9 / 3.15 | 0.482 / 10.9 / 4.04 | 0.482 / 7.4 / 3.24 | 0.416 / 7.7 / 2.26 | 0.412 / 5.5 / 2.56 | 0.375 / 128.4 / 3.13 | 0.372 / 113.8 / 3.10 | 0.623 / 27.2 / 5.71 |
| ws-json (9) | 0.195 / 19.4 / 7.45 | 0.195 / 10.6 / 4.71 | 0.196 / 23.5 / 7.59 | 0.196 / 15.8 / 4.91 | 0.185 / 29.6 / 7.36 | 0.188 / 28.9 / 4.24 | 0.173 / 1477.3 / 7.59 | 0.172 / 1181.4 / 4.60 | 0.249 / 68.0 / 22.87 |
| yjs-inc (30) | 0.811 / 4.1 / 1.85 | 0.815 / 4.1 / 1.87 | 0.804 / 4.4 / 1.88 | 0.808 / 4.2 / 1.87 | 0.429 / 1.4 / 0.79 | 0.452 / 1.0 / 0.50 | 0.411 / 20.5 / 0.75 | 0.414 / 27.5 / 0.43 | 0.960 / 14.1 / 0.76 |
| yjs-snap (3) | 0.129 / 36.5 / 14.55 | 0.128 / 21.7 / 9.11 | 0.122 / 40.8 / 14.93 | 0.122 / 26.4 / 9.31 | 0.116 / 46.4 / 14.57 | 0.117 / 32.8 / 9.02 | 0.108 / 3518.8 / 15.83 | 0.107 / 3638.6 / 11.43 | 0.151 / 133.9 / 38.83 |
| file-chunk (18) | 0.932 / 67.2 / 5.60 | 0.938 / 29.0 / 3.34 | 0.930 / 73.5 / 4.71 | 0.931 / 45.7 / 2.80 | 0.930 / 109.6 / 4.86 | 0.931 / 120.6 / 2.60 | 0.922 / 1251.3 / 8.72 | 0.922 / 1130.2 / 5.81 | 0.937 / 465.9 / 15.30 |

### Decoder speed on identical frames (the C library's level-3 output)

| class (n) | zstd-rs decoding C-3 | ruzstd decoding C-3 | zstd-c-3 | zstd-rs decoding C-3-dict | zstd-c-3-dict |
|---|---|---|---|---|---|
| chat (49) | 0.851 / 4.2 / 2.22 | 0.851 / 4.2 / 3.47 | 0.851 / 4.2 / 2.22 | 0.349 / 0.7 / 0.49 | 0.349 / 0.7 / 0.34 |
| markdown (5) | 0.482 / 7.3 / 4.08 | 0.482 / 7.3 / 9.13 | 0.482 / 7.4 / 3.24 | 0.412 / 5.5 / 3.17 | 0.412 / 5.5 / 2.56 |
| ws-json (9) | 0.196 / 15.9 / 7.52 | 0.196 / 15.8 / 22.85 | 0.196 / 15.8 / 4.91 | 0.188 / 28.7 / 7.03 | 0.188 / 28.9 / 4.24 |
| yjs-inc (30) | 0.808 / 4.2 / 1.88 | 0.808 / 4.2 / 3.66 | 0.808 / 4.2 / 1.87 | 0.452 / 1.0 / 0.83 | 0.452 / 1.0 / 0.50 |
| yjs-snap (3) | 0.122 / 26.4 / 14.76 | 0.122 / 26.3 / 37.42 | 0.122 / 26.4 / 9.31 | 0.117 / 31.9 / 14.46 | 0.117 / 32.8 / 9.02 |
| file-chunk (18) | 0.931 / 45.8 / 5.12 | 0.931 / 45.8 / 13.93 | 0.931 / 45.7 / 2.80 | 0.931 / 120.7 / 4.77 | 0.931 / 120.6 / 2.60 |

ruzstd 0.8 could not decode any dictionary frame in this corpus (`UninitializedHuffmanTable`), so it has no dictionary column.

### Summary

- **Against ruzstd** (the existing pure-Rust crate, `Fastest` is its only level):
  - Our level 1 compresses better on every class, and 2.8-7x faster. For example, chat is 0.856 vs 1.036, which means ruzstd *grows* chat.
  - Our decoder is 1.6-3x faster on the same frames.
  - We also decode dictionary frames, which ruzstd cannot.
- **Against zstd-C at the same level:**
  - Ratio is equal to within ±1% at levels 1 and 3. With dictionaries it is better on Yjs (0.429 vs 0.452) and within 2% on chat.
  - Small-frame speed is close: chat with a dictionary takes 1.0 µs vs 0.7 µs to compress, and 0.50 vs 0.34 µs to decode.
  - Large frames are 1.5-2.3x slower to compress and about 1.6x slower to decode.
  - Level 19 ratio is within 1-3%.
- **The wasm module (encoder + decoder, `zstd-rs-wasm`)** is 164 KB, or 64 KB gzipped. It has 0 imports.

## Limitations

- The API is one-shot: whole input in, whole frame out. There is no streaming `Read`/`Write` adapter yet. The job API covers large inputs.
- Levels 13-22 use hash chains rather than the reference's binary trees. Their ratio is within about 1-3% of C, and they are slower on large inputs.
- On large inputs, native speed is about 45-70% of the C library for compression and about 60% for decompression. Small frames with a dictionary are closer.
- There is no built-in dictionary trainer. Train with `zstd --train` (ZDICT); both formats are loaded.
- Legacy zstd formats (pre-v0.8) are not supported.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
The test vectors in `zstd-rs/tests/golden` come from the reference repository (BSD-3-Clause / GPL-2.0).
