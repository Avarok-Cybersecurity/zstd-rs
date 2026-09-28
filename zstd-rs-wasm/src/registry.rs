//! Handle registry: owns every object the JavaScript side refers to.
use std::cell::RefCell;
use zstd_rs::{Compressor, DecoderDictionary, Decompressor, EncoderDictionary};

/// A slot table with 1-based handles (0 means "none").
pub struct Slots<T>(Vec<Option<T>>);

impl<T> Slots<T> {
    fn new() -> Self {
        Slots(Vec::new())
    }

    /// Stores `v` and returns its handle.
    pub fn put(&mut self, v: T) -> u32 {
        if let Some(i) = self.0.iter().position(Option::is_none) {
            self.0[i] = Some(v);
            return i as u32 + 1;
        }
        self.0.push(Some(v));
        self.0.len() as u32
    }

    /// Borrows the object behind `h`.
    pub fn get(&self, h: u32) -> Option<&T> {
        self.0.get((h as usize).checked_sub(1)?)?.as_ref()
    }

    /// Mutably borrows the object behind `h`.
    pub fn get_mut(&mut self, h: u32) -> Option<&mut T> {
        self.0.get_mut((h as usize).checked_sub(1)?)?.as_mut()
    }

    /// Removes and returns the object behind `h`.
    pub fn take(&mut self, h: u32) -> Option<T> {
        self.0.get_mut((h as usize).checked_sub(1)?)?.take()
    }

    /// Puts `v` back under handle `h` (after `take`).
    pub fn restore(&mut self, h: u32, v: T) {
        if let Some(slot) = (h as usize).checked_sub(1).and_then(|i| self.0.get_mut(i)) {
            *slot = Some(v);
        }
    }
}

/// Runs `f` on the bytes of buffer `input` and the (cleared) buffer `out`,
/// with the rest of the registry available. Returns `f`'s status.
pub fn io(input: u32, out: u32, f: impl FnOnce(&mut Registry, &[u8], &mut Vec<u8>) -> i32) -> i32 {
    with(|r| {
        let Some(src) = r.bufs.take(input) else { return crate::status::BAD_HANDLE };
        let Some(mut dst) = r.bufs.take(out) else {
            r.bufs.restore(input, src);
            return crate::status::BAD_HANDLE;
        };
        dst.clear();
        let res = f(r, &src, &mut dst);
        r.bufs.restore(input, src);
        r.bufs.restore(out, dst);
        res
    })
}

/// Everything the module owns on behalf of JavaScript.
pub struct Registry {
    /// Byte buffers.
    pub bufs: Slots<Vec<u8>>,
    /// Compression contexts.
    pub comps: Slots<Compressor>,
    /// Dictionaries prepared for compression.
    pub edicts: Slots<EncoderDictionary>,
    /// Dictionaries prepared for decompression.
    pub ddicts: Slots<DecoderDictionary>,
    /// The (single) decompression context.
    pub dec: Decompressor,
}

thread_local! {
    static REG: RefCell<Registry> = RefCell::new(Registry {
        bufs: Slots::new(),
        comps: Slots::new(),
        edicts: Slots::new(),
        ddicts: Slots::new(),
        dec: Decompressor::new(),
    });
}

/// Runs `f` with exclusive access to the registry.
pub fn with<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    REG.with(|r| f(&mut r.borrow_mut()))
}
