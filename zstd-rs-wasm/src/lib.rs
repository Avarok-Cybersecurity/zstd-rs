//! A raw WebAssembly ABI for zstd-rs, built for `wasm32-unknown-unknown` with no
//! imports. Every object (byte buffer, compressor, dictionary) lives in a registry
//! and is referred to by a nonzero `u32` handle; JavaScript writes and reads
//! buffer bytes at `zr_buf_ptr(handle)` in the module's memory between calls.
//! No unsafe operations are performed. `#[no_mangle]` is flagged by the
//! `unsafe_code` lint, so it is allowed on the exports only (see `abi.rs`).
#![deny(unsafe_code)]
#![warn(missing_docs)]

mod abi;
mod registry;

pub use registry::{io, with, Registry};

/// Negative status codes returned by the ABI (0 or positive means success).
pub mod status {
    /// Unknown or freed handle.
    pub const BAD_HANDLE: i32 = -1;
    /// Invalid parameter (level, window, flags, plan).
    pub const PARAMETER: i32 = -2;
    /// Decompression hit the caller's output limit.
    pub const OUTPUT_LIMIT: i32 = -3;
    /// Malformed or truncated compressed data.
    pub const CORRUPT: i32 = -4;
    /// Dictionary missing, mismatched or malformed.
    pub const DICTIONARY: i32 = -5;
    /// Checksum mismatch.
    pub const CHECKSUM: i32 = -6;
}

/// Maps a library error to its status code.
pub fn code(e: zstd_rs::Error) -> i32 {
    use zstd_rs::Error::*;
    match e {
        Parameter(_) => status::PARAMETER,
        OutputLimit => status::OUTPUT_LIMIT,
        MissingDictionary | WrongDictionary | BadDictionary(_) => status::DICTIONARY,
        Checksum => status::CHECKSUM,
        _ => status::CORRUPT,
    }
}
