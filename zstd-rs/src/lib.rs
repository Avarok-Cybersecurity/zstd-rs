#![doc = include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate alloc;

mod bits;
mod decode;
mod dict;
mod encode;
mod error;
mod frame;
mod fse;
mod huf;
pub mod par;
mod xxh64;

pub use decode::{DecoderDictionary, Decompressor};
pub use dict::{Dictionary, DictionaryFormat};
pub use encode::{CompressionConfig, Compressor, EncoderDictionary, MAX_LEVEL, MIN_LEVEL};
pub use error::{Error, Result};
pub use frame::{FrameFormat, FrameHeader};
pub use xxh64::{xxh64, Xxh64};
