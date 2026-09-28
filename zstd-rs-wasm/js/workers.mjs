// Web-Worker-style parallel compression with Node worker_threads (the same
// code shape works with browser Web Workers): every worker instantiates its own
// wasm module and runs `zr_compress_job` on the jobs it is handed; the main
// thread writes the frame header and trailer. The core never spawns anything.
// Verifies that the frame is byte-identical to a one-instance run and decodes.
// Usage: node workers.mjs <zstd_rs_wasm.wasm> [workers]
import { readFileSync } from 'node:fs';
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads';
import { ZstdRs, FLAG_CHECKSUM, FLAG_CONTENT_SIZE } from './zstd-rs.mjs';

const LEVEL = 3, WLOG = 22, FLAGS = FLAG_CHECKSUM | FLAG_CONTENT_SIZE;

/** Same plan as zstd_rs::par::plan: fixed-size jobs with an overlap of history. */
function plan(len, jobSize, overlap) {
  const jobs = [];
  for (let start = 0; start < len; start += jobSize) {
    const end = Math.min(start + jobSize, len);
    jobs.push({ start, end, prefix: Math.max(0, start - overlap), first: start === 0, last: end === len });
  }
  return jobs;
}

function compressJobs(z, input, jobs) {
  const comp = z.compressor(LEVEL, WLOG, FLAGS);
  const inH = z.put(input);
  const out = z.x.zr_buf_new(0);
  const parts = jobs.map((j) => {
    z.check(z.x.zr_compress_job(comp, inH, j.start, j.end, j.prefix, (j.first ? 1 : 0) | (j.last ? 2 : 0), 0, out));
    return { index: j.index, bytes: z.get(out) };
  });
  z.free(inH);
  z.free(out);
  return parts;
}

if (!isMainThread) {
  const z = await ZstdRs.load(readFileSync(workerData.wasm));
  parentPort.on('message', ({ input, jobs }) => parentPort.postMessage(compressJobs(z, input, jobs)));
} else {
  const wasm = process.argv[2];
  const n = Number(process.argv[3] ?? 4);
  const words = ['session', 'peer', 'workspace', 'message', '{"type":"chat"}', 'frame', 'offset', '\n'];
  const input = new TextEncoder().encode(Array.from({ length: 400000 }, (_, i) => words[(i * 7 + (i >> 5)) % words.length] + (i % 97)).join(' '));
  const jobs = plan(input.length, 256 * 1024, 64 * 1024).map((j, index) => ({ ...j, index }));
  const z = await ZstdRs.load(readFileSync(wasm));
  const frame = (parts) => {
    const inH = z.put(input);
    const o = z.x.zr_buf_new(0);
    z.check(z.x.zr_frame_begin(LEVEL, WLOG, FLAGS, inH, 0, o));
    const head = z.get(o);
    z.check(z.x.zr_frame_end(LEVEL, WLOG, FLAGS, inH, o));
    const tail = z.get(o);
    z.free(inH);
    z.free(o);
    const body = parts.sort((a, b) => a.index - b.index).map((p) => p.bytes);
    return Buffer.concat([head, ...body, tail]);
  };
  const t0 = performance.now();
  const sequential = frame(compressJobs(z, input, jobs));
  const t1 = performance.now();
  const workers = Array.from({ length: n }, () => new Worker(new URL(import.meta.url), { workerData: { wasm } }));
  const shares = workers.map((_, w) => jobs.filter((j) => j.index % n === w));
  const t2 = performance.now();
  const results = await Promise.all(workers.map((w, i) => new Promise((res) => { w.once('message', res); w.postMessage({ input, jobs: shares[i] }); })));
  const parallel = frame(results.flat());
  const t3 = performance.now();
  await Promise.all(workers.map((w) => w.terminate()));
  if (!sequential.equals(parallel)) throw new Error('worker output differs from the sequential run');
  const back = z.decompress(parallel, input.length);
  if (!Buffer.from(back).equals(Buffer.from(input))) throw new Error('round trip failed');
  console.log(`${jobs.length} jobs, ${n} workers: ${input.length} -> ${parallel.length} bytes, byte-identical; ` +
    `sequential ${(t1 - t0).toFixed(1)} ms, workers ${(t3 - t2).toFixed(1)} ms (incl. transfer)`);
}
