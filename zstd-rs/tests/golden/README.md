# Golden vectors

Copied unmodified from the Zstandard reference repository
(https://github.com/facebook/zstd, tag v1.5.7, directories `tests/golden-*`),
which is dual-licensed BSD-3-Clause / GPL-2.0; they are redistributed here under the BSD-3-Clause
terms, whose copyright notice and text are in `LICENSE-zstd-BSD-3-Clause`. They are used only as
test inputs:

- `golden-decompression/`: valid frames; our output must equal the reference's.
- `golden-decompression-errors/`: invalid frames; both decoders must reject them.
- `golden-compression/`: inputs our encoder must round-trip through both decoders.
- `golden-dictionaries/`: a dictionary with missing symbols (kept for future tests).
