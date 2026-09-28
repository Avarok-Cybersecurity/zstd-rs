// CI smoke test: round-trips data through the wasm module under V8 at several
// levels, with and without a raw dictionary, and checks error reporting.
import { readFileSync } from 'node:fs';
import { ZstdRs, FLAG_CHECKSUM, FLAG_CONTENT_SIZE } from './zstd-rs.mjs';

const z = await ZstdRs.load(readFileSync(process.argv[2]));
const text = new TextEncoder().encode('{"type":"chat","body":"hello from the wasm smoke test"} '.repeat(200));
const dictBytes = new TextEncoder().encode('{"type":"chat","body":"');
for (const level of [1, 3, 9, 19]) {
  const c = z.compressor(level, 22, FLAG_CHECKSUM | FLAG_CONTENT_SIZE);
  const ed = z.encoderDict(dictBytes, level, 1);
  const dd = z.decoderDict(dictBytes, 1);
  for (const [e, d] of [[0, 0], [ed, dd]]) {
    const frame = z.compress(c, text, e);
    const back = z.decompress(frame, text.length, d);
    if (back.length !== text.length || back.some((b, i) => b !== text[i])) throw new Error(`mismatch at level ${level}`);
    if (frame.length >= text.length / 10) throw new Error(`poor ratio at level ${level}: ${frame.length}`);
  }
}
let threw = false;
try {
  z.decompress(new Uint8Array([0x28, 0xb5, 0x2f, 0xfd, 0, 0]), 100);
} catch {
  threw = true;
}
if (!threw) throw new Error('corrupt input was accepted');
console.log('wasm smoke test passed');
