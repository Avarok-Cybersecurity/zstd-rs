// Runs the wasm benchmark under V8 (Node) over a corpus directory and prints a
// CSV with the same columns as the native harness.
// Usage: node run.mjs <module.wasm> <corpus_dir> <zdict> [budget_ms]
import { readFileSync, readdirSync } from 'node:fs';
import { performance } from 'node:perf_hooks';

const [, , wasmPath, dir, dictPath, budgetArg] = process.argv;
const budget = Number(budgetArg ?? 60);
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const x = instance.exports;
const put = (ptrFn, bytes) => {
  const ptr = ptrFn(bytes.length); // may grow memory: read .buffer only afterwards
  new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
};
put(x.dict_ptr, readFileSync(dictPath));
const n = x.init();
const names = [];
for (let i = 0; i < n; i++) {
  const len = x.codec_name(i);
  const p = x.out_ptr();
  names.push(new TextDecoder().decode(new Uint8Array(x.memory.buffer, p, len).slice()));
}
const only = process.env.CODECS?.split(',');

function time(fn) {
  fn(1);
  let iters = 1;
  for (;;) {
    const t0 = performance.now();
    fn(iters);
    const dt = performance.now() - t0;
    if (dt >= budget || iters >= 1 << 20) return (dt * 1000) / iters;
    iters = Math.min(iters * Math.max(2, Math.ceil(budget / Math.max(dt, 0.01))), 1 << 20);
  }
}

console.log('class,file,codec,raw,compressed,ratio,comp_us,decomp_us,roundtrip_ok');
for (const f of readdirSync(dir).sort()) {
  const data = readFileSync(`${dir}/${f}`);
  const cls = f.split('__')[0];
  for (let i = 0; i < n; i++) {
    if (only && !only.includes(names[i])) continue;
    put(x.input_ptr, data);
    let comp = 0;
    const cus = time((k) => (comp = x.compress(i, k)));
    let ok = true;
    const dus = time((k) => (ok = x.decompress(i, k) === data.length));
    console.log([cls, f, names[i], data.length, comp, (comp / data.length).toFixed(4), cus.toFixed(3), dus.toFixed(3), ok].join(','));
  }
}
