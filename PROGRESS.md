# zstd-rs: progress

Pure-Rust, `#![no_std]` + `alloc`, `#![forbid(unsafe_code)]` Zstandard (RFC 8878)
compressor and decompressor with dictionary support. Crate: `zstd-rs/`.

## Status (updated each iteration)

- **Done**
  - Decoder: all of RFC 8878. That covers raw, RLE and compressed blocks; the four literal modes (raw, RLE, Huffman, treeless); and every sequence table mode (predefined, RLE, FSE, repeat). It also handles repeat offsets, zstd-format and raw dictionaries, skippable frames, multiple frames, XXH64 checksum checks, a caller-supplied output cap, and malformed input without panicking.
  - Encoder: levels -7..=22 using the reference level tables and size adjustment.
    - Match finders: fast, dfast, greedy/lazy/lazy2 (hash chain), and an optimal parser (btopt/btultra/btultra2-style price DP).
    - Entropy stage: Huffman literals (new or repeat table, 1 or 4 streams) and FSE sequences, with cost-based mode selection.
    - Frames: optional checksum, content size, dictionary ID, attached dictionaries (tables built once, never copied per frame), and dictionary entropy tables.
  - Interop:
    - C encoder → our decoder at levels -5..19, sizes 0 B..3 MB, streaming frames without content size, ZDICT-trained and raw dictionaries, multi-frame and skippable frames.
    - Our encoder → C bulk *and* streaming decoders (and ours) at 17 levels × the same sizes, with trained and raw dictionaries.
    - The vendored reference golden vectors (decompression, decompression-errors, compression inputs).
- **In progress:** benchmark harness on the ILM corpus.
- **Next:** small-frame speed, fuzzing, the wasm32 build plus V8 bench, the executor API, and the dictionary trainer.

## Early ratio check (synthetic corpus, vs C at the same level)

Size ratio is ours / C; below 1 means ours is smaller.

| input | L1 | L3 | L5 | L9 | L19 |
|---|---|---|---|---|---|
| text 100 KB | 1.024 | 1.006 | 0.997 | 0.996 | 1.046 |
| mixed 1 MB | 1.012 | 1.003 | 0.997 | 0.998 | 1.024 |
| text 600 B | 0.965 | 0.962 | 0.993 | 0.989 | 1.008 |
