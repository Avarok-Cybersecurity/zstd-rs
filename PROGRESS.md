# zstd-rs: progress

A pure-Rust Zstandard (RFC 8878) compressor and decompressor with dictionary support. It is `#![no_std]` + `alloc` and `#![forbid(unsafe_code)]`.

The repository is a Cargo workspace:

- `zstd-rs/` (core)
- `zstd-rs-exec/` (thread executor)
- `zstd-rs-wasm/` (raw wasm ABI + JS)
- `bench/` (native and V8 harnesses; its own workspace, and needs the local rill checkout)

## Status

### Done

- **Decoder:** all of RFC 8878.
  - Blocks: raw, RLE and compressed.
  - Literals: all four modes. Sequences: all four table modes.
  - Repeat offsets; zstd-format and raw dictionaries; skippable and concatenated frames.
  - Checks the XXH64 checksum and enforces Block_Maximum_Size.
  - Takes a caller-supplied output cap, and returns errors instead of panicking.
  - Also decodes the magicless format.
- **Encoder:** levels -7..=22, using the reference's level tables and size adjustment.
  - Strategies: fast and dfast, greedy/lazy/lazy2 over hash chains, and an optimal parser.
  - Entropy: Huffman literals and FSE sequences, chosen by estimated cost (with the reference's heuristics for small blocks at fast levels).
  - Frame options: checksum, content size, dictionary ID, magicless.
  - Dictionaries are attached (built once per level, shared, never copied per frame).
- **Parallelism boundary (SBIO):**
  - `par::plan` / `compress_frame` / `compress_frames` / `decompress_frames` over an `Executor` trait.
  - Executors: `Sequential` in the core, `ThreadPool` in zstd-rs-exec, and a JS worker pool (`zstd-rs-wasm/js/workers.mjs`).
  - Output is byte-identical for every executor. This is tested in Rust and in Node.
- **wasm32-unknown-unknown:**
  - The module has **0 imports**.
  - Size is 164 KB raw, 64 KB gzipped (encoder + decoder).
  - V8 smoke, magicless and worker tests run in CI.
- **Tests:**
  - Interop with the C library in both directions: 17 levels, 0 B–3 MB, with and without dictionaries, bulk and streaming C decoders, magicless.
  - The reference golden vectors, and fuzz regressions.
  - Negative controls (`scripts/negative_controls.sh`): six injected defects, each caught by its test and green again after restore.
- **Quality:** clippy `-D warnings`, rustfmt, every file ≤ 250 lines, and README examples run as doctests.
- **Repo:** README, both licenses, crate metadata, CI workflow, local commits; `cargo publish -p zstd-rs --dry-run` passes.

### Not done / limitations

- No streaming Read/Write API (one-shot plus the job API).
- Hash chains are used where the reference uses binary trees (levels 13–22).
- Native speed on large inputs is below C (see the tables).
- No built-in dictionary trainer (use `zstd --train`).

## Fuzzing (cargo-fuzz / libFuzzer, macOS arm64)

| target | what it checks | runs | findings |
|---|---|---|---|
| `decode` | arbitrary bytes ± dictionary; never panics, respects the cap | 75.7 M (30 min) | 0 |
| `roundtrip` | our encoder at 11 levels ± dictionary, checksum, window, magicless; our decoder and C must both reproduce the input | 3.7 M (30 min) | 0 |
| `diff_c` | differential decoding against C | 21.6 M + 30 min | 1 (fixed) |

The one `diff_c` finding was real, and is fixed and kept in `tests/regressions`: literal sections and block output are now limited to the *frame's* Block_Maximum_Size (min(window, 128 KiB)), not just to 128 KiB.

`diff_c` also documents intentional strictness, where we follow the reference's streaming decoder and RFC 8878 rather than its one-shot fast paths:
- raw and RLE blocks above Block_Maximum_Size are rejected;
- Huffman streams must be consumed exactly and must carry an end marker;
- legacy (pre-v0.8) formats are out of scope.

## Benchmarks

The final tables are in README.md. The raw CSVs are local only (`results/`, gitignored, because the corpus is private).
