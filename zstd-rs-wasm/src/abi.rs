//! Exported functions. `#[no_mangle]` is the only reason `unsafe_code` is allowed
//! here: the functions themselves perform no unsafe operations.
#![allow(unsafe_code)]

use crate::{code, io, status, with};
use zstd_rs::par::Job;
use zstd_rs::{CompressionConfig, Compressor, DecoderDictionary, Dictionary, DictionaryFormat, EncoderDictionary};

fn config(level: i32, window_log: u32, flags: u32) -> CompressionConfig {
    CompressionConfig { level, window_log, checksum: flags & 1 != 0, content_size: flags & 2 != 0, dict_id: flags & 4 != 0 }
}

fn format(f: u32) -> DictionaryFormat {
    match f {
        0 => DictionaryFormat::Zstd,
        1 => DictionaryFormat::Raw { id: 0 },
        _ => DictionaryFormat::Detect,
    }
}

/// Allocates a zero-filled buffer of `len` bytes; returns its handle.
#[no_mangle]
pub extern "C" fn zr_buf_new(len: u32) -> u32 {
    with(|r| r.bufs.put(vec![0; len as usize]))
}

/// Address of a buffer's bytes in linear memory (valid until the next call that resizes it).
#[no_mangle]
pub extern "C" fn zr_buf_ptr(h: u32) -> u32 {
    with(|r| r.bufs.get(h).map_or(0, |b| b.as_ptr() as usize as u32))
}

/// Length of a buffer in bytes.
#[no_mangle]
pub extern "C" fn zr_buf_len(h: u32) -> u32 {
    with(|r| r.bufs.get(h).map_or(0, |b| b.len() as u32))
}

/// Frees a buffer.
#[no_mangle]
pub extern "C" fn zr_buf_free(h: u32) {
    with(|r| drop(r.bufs.take(h)));
}

/// Creates a compressor; flags: bit0 checksum, bit1 content size, bit2 dictionary ID.
/// Returns 0 if the configuration is invalid.
#[no_mangle]
pub extern "C" fn zr_compressor_new(level: i32, window_log: u32, flags: u32) -> u32 {
    match Compressor::new(config(level, window_log, flags)) {
        Ok(c) => with(|r| r.comps.put(c)),
        Err(_) => 0,
    }
}

/// Frees a compressor.
#[no_mangle]
pub extern "C" fn zr_compressor_free(h: u32) {
    with(|r| drop(r.comps.take(h)));
}

/// Prepares the dictionary in buffer `buf` for compression at `level` (format 0 zstd, 1 raw, 2 detect).
#[no_mangle]
pub extern "C" fn zr_encoder_dict_new(buf: u32, level: i32, fmt: u32) -> u32 {
    with(|r| {
        let Some(bytes) = r.bufs.get(buf) else { return 0 };
        match Dictionary::new(bytes, format(fmt)).and_then(|d| EncoderDictionary::new(&d, level)) {
            Ok(d) => r.edicts.put(d),
            Err(_) => 0,
        }
    })
}

/// Prepares the dictionary in buffer `buf` for decompression.
#[no_mangle]
pub extern "C" fn zr_decoder_dict_new(buf: u32, fmt: u32) -> u32 {
    with(|r| {
        let Some(bytes) = r.bufs.get(buf) else { return 0 };
        match Dictionary::new(bytes, format(fmt)).and_then(DecoderDictionary::new) {
            Ok(d) => r.ddicts.put(d),
            Err(_) => 0,
        }
    })
}

/// Resolves an optional handle: 0 means none; an unknown handle is an error.
fn opt<T>(h: u32, get: impl FnOnce(u32) -> Option<T>) -> Result<Option<T>, i32> {
    if h == 0 {
        return Ok(None);
    }
    get(h).map(Some).ok_or(status::BAD_HANDLE)
}

/// Compresses buffer `input` into buffer `out` (replacing its contents). Returns the frame length or a status.
#[no_mangle]
pub extern "C" fn zr_compress(comp: u32, input: u32, edict: u32, out: u32) -> i32 {
    io(input, out, |r, src, dst| {
        let d = match opt(edict, |h| r.edicts.get(h)) {
            Ok(d) => d,
            Err(e) => return e,
        };
        match r.comps.get_mut(comp) {
            Some(c) => c.compress(src, d, dst).map(|_| dst.len() as i32).unwrap_or_else(code),
            None => status::BAD_HANDLE,
        }
    })
}

/// Decompresses buffer `input` into buffer `out`, never producing more than `max_out` bytes.
#[no_mangle]
pub extern "C" fn zr_decompress(input: u32, ddict: u32, max_out: u32, out: u32) -> i32 {
    io(input, out, |r, src, dst| {
        let d = match opt(ddict, |h| r.ddicts.get(h)) {
            Ok(d) => d,
            Err(e) => return e,
        };
        r.dec.decompress(src, d, max_out as usize, dst).map(|n| n as i32).unwrap_or_else(code)
    })
}

/// Job API: writes the header of a frame holding buffer `input`'s bytes into `out`.
#[no_mangle]
pub extern "C" fn zr_frame_begin(level: i32, window_log: u32, flags: u32, input: u32, edict: u32, out: u32) -> i32 {
    io(input, out, |r, src, dst| {
        let d = match opt(edict, |h| r.edicts.get(h)) {
            Ok(d) => d,
            Err(e) => return e,
        };
        zstd_rs::par::frame_header(&config(level, window_log, flags), src.len(), d, dst).map(|_| dst.len() as i32).unwrap_or_else(code)
    })
}

/// Job API: compresses one job of buffer `input` into `out` (bit0 of `first_last`: first, bit1: last).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn zr_compress_job(
    comp: u32,
    input: u32,
    start: u32,
    end: u32,
    prefix_start: u32,
    first_last: u32,
    edict: u32,
    out: u32,
) -> i32 {
    io(input, out, |r, src, dst| {
        let d = match opt(edict, |h| r.edicts.get(h)) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let job = Job {
            start: start as usize,
            end: end as usize,
            prefix_start: prefix_start as usize,
            first: first_last & 1 != 0,
            last: first_last & 2 != 0,
        };
        match r.comps.get_mut(comp) {
            Some(c) => c.compress_job(src, &job, d, dst).map(|_| dst.len() as i32).unwrap_or_else(code),
            None => status::BAD_HANDLE,
        }
    })
}

/// Job API: writes the frame trailer (the checksum, if `flags` bit0) for buffer `input` into `out`.
#[no_mangle]
pub extern "C" fn zr_frame_end(level: i32, window_log: u32, flags: u32, input: u32, out: u32) -> i32 {
    io(input, out, |_, src, dst| {
        zstd_rs::par::frame_trailer(&config(level, window_log, flags), src, dst);
        dst.len() as i32
    })
}
