//! V8 benchmark module: zstd-rs and the pure-Rust baselines behind one raw ABI.
//! JavaScript writes the input (and dictionaries) into module buffers, then asks
//! for `iters` compressions or decompressions in one call and times the call.
#![allow(clippy::missing_safety_doc)]
mod codecs;

use std::cell::RefCell;

struct State {
    input: Vec<u8>,
    comp: Vec<u8>,
    dec: Vec<u8>,
    zdict: Vec<u8>,
    codecs: Vec<codecs::Codec>,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State { input: Vec::new(), comp: Vec::new(), dec: Vec::new(), zdict: Vec::new(), codecs: Vec::new() });
}

/// Resizes the input buffer to `len` and returns its address.
#[no_mangle]
pub extern "C" fn input_ptr(len: u32) -> u32 {
    S.with(|s| {
        let mut s = s.borrow_mut();
        s.input.resize(len as usize, 0);
        s.input.as_ptr() as usize as u32
    })
}

/// Resizes the dictionary buffer to `len` and returns its address.
#[no_mangle]
pub extern "C" fn dict_ptr(len: u32) -> u32 {
    S.with(|s| {
        let mut s = s.borrow_mut();
        s.zdict.resize(len as usize, 0);
        s.zdict.as_ptr() as usize as u32
    })
}

/// Builds every codec (after the dictionary was written); returns the codec count.
#[no_mangle]
pub extern "C" fn init() -> u32 {
    S.with(|s| {
        let mut s = s.borrow_mut();
        let d = s.zdict.clone();
        s.codecs = codecs::all(&d);
        s.codecs.len() as u32
    })
}

/// Writes codec `i`'s name into the output buffer; returns its length.
#[no_mangle]
pub extern "C" fn codec_name(i: u32) -> u32 {
    S.with(|s| {
        let mut s = s.borrow_mut();
        let name = s.codecs[i as usize].name.as_bytes().to_vec();
        s.dec = name;
        s.dec.len() as u32
    })
}

/// Compresses the input `iters` times with codec `i`; returns the compressed size.
#[no_mangle]
pub extern "C" fn compress(i: u32, iters: u32) -> u32 {
    S.with(|s| {
        let s = &mut *s.borrow_mut();
        let c = &mut s.codecs[i as usize];
        for _ in 0..iters {
            s.comp.clear();
            (c.c)(&s.input, &mut s.comp);
        }
        s.comp.len() as u32
    })
}

/// Decompresses the last compressed output `iters` times; returns the size, or
/// u32::MAX if the result differs from the input.
#[no_mangle]
pub extern "C" fn decompress(i: u32, iters: u32) -> u32 {
    S.with(|s| {
        let s = &mut *s.borrow_mut();
        let c = &mut s.codecs[i as usize];
        for _ in 0..iters {
            s.dec.clear();
            (c.d)(&s.comp, s.input.len(), &mut s.dec);
        }
        if s.dec == s.input {
            s.dec.len() as u32
        } else {
            u32::MAX
        }
    })
}

/// Address of the scratch output (codec names).
#[no_mangle]
pub extern "C" fn out_ptr() -> u32 {
    S.with(|s| s.borrow().dec.as_ptr() as usize as u32)
}
