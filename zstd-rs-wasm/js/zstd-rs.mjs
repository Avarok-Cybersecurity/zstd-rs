// Thin JavaScript wrapper over the raw zstd-rs-wasm ABI (works in browsers,
// Web Workers and Node). The module has no imports: instantiate it with {}.
export const FLAG_CHECKSUM = 1;
export const FLAG_CONTENT_SIZE = 2;
export const FLAG_DICT_ID = 4;
export const FLAG_MAGICLESS = 8;
const STATUS = { '-1': 'bad handle', '-2': 'invalid parameter', '-3': 'output limit exceeded', '-4': 'corrupt data', '-5': 'dictionary error', '-6': 'checksum mismatch' };

export class ZstdRs {
  static async load(bytes) {
    const { instance } = await WebAssembly.instantiate(bytes, {});
    return new ZstdRs(instance.exports);
  }

  constructor(exports) {
    this.x = exports;
  }

  check(r) {
    if (r < 0) throw new Error(`zstd-rs: ${STATUS[r] ?? r}`);
    return r;
  }

  /** Copies `data` into a new module buffer and returns its handle. */
  put(data) {
    const h = this.x.zr_buf_new(data.length);
    const ptr = this.x.zr_buf_ptr(h); // memory may have grown: take .buffer afterwards
    new Uint8Array(this.x.memory.buffer, ptr, data.length).set(data);
    return h;
  }

  /** Copies a module buffer out (the view is invalid after the next call). */
  get(h) {
    const ptr = this.x.zr_buf_ptr(h);
    const len = this.x.zr_buf_len(h);
    return new Uint8Array(this.x.memory.buffer, ptr, len).slice();
  }

  free(h) {
    this.x.zr_buf_free(h);
  }

  /** A compressor with explicit level, window log and flags (no defaults). */
  compressor(level, windowLog, flags) {
    const h = this.x.zr_compressor_new(level, windowLog, flags);
    if (h === 0) throw new Error('zstd-rs: invalid compression configuration');
    return h;
  }

  encoderDict(bytes, level, format) {
    const b = this.put(bytes);
    const h = this.x.zr_encoder_dict_new(b, level, format);
    this.free(b);
    if (h === 0) throw new Error('zstd-rs: bad dictionary');
    return h;
  }

  decoderDict(bytes, format) {
    const b = this.put(bytes);
    const h = this.x.zr_decoder_dict_new(b, format);
    this.free(b);
    if (h === 0) throw new Error('zstd-rs: bad dictionary');
    return h;
  }

  compress(comp, data, edict = 0) {
    const i = this.put(data);
    const o = this.x.zr_buf_new(0);
    try {
      this.check(this.x.zr_compress(comp, i, edict, o));
      return this.get(o);
    } finally {
      this.free(i);
      this.free(o);
    }
  }

  /** `flags`: FLAG_MAGICLESS if the frames were made without magic numbers. */
  decompress(frame, maxOut, ddict = 0, flags = 0) {
    const i = this.put(frame);
    const o = this.x.zr_buf_new(0);
    try {
      this.check(this.x.zr_decompress_ex(i, ddict, maxOut, flags, o));
      return this.get(o);
    } finally {
      this.free(i);
      this.free(o);
    }
  }
}
