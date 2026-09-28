//! Differential decoding against the C library on arbitrary bytes: whenever C
//! decodes a frame, we must produce the same bytes; when C rejects it, so must we.
//!
//! The reference has two decoders that disagree on some malformed input, and we
//! follow the stricter one (and RFC 8878):
//! - its one-shot path skips the Block_Maximum_Size check for raw/RLE blocks
//!   that its streaming path enforces;
//! - its fast 4-stream Huffman path checks only the output count, so it accepts
//!   streams with unread or over-read bits that its fallback path rejects.
//! A rejection by us is accepted if the C streaming decoder also rejects the input,
//! or if it is one of our Huffman stream framing checks (end marker, exact end).
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::io::Read;
use zstd_rs::{Decompressor, Error};

fn c_streaming_rejects(data: &[u8]) -> bool {
    let Ok(mut d) = zstd::stream::Decoder::with_buffer(data) else { return true };
    let _ = d.window_log_max(27);
    let mut v = Vec::new();
    d.take(1 << 20).read_to_end(&mut v).is_err()
}

fuzz_target!(|data: &[u8]| {
    const CAP: usize = 1 << 20;
    let mut ours = Vec::new();
    let r = Decompressor::new().decompress(data, None, CAP, &mut ours);
    let c = zstd::bulk::decompress(data, CAP);
    match (&r, &c) {
        (Ok(_), Ok(v)) => assert_eq!(&ours, v, "output differs from C"),
        (Err(_), Err(_)) => {}
        (Ok(_), Err(e)) => panic!("we accept what C rejects ({e}); ours = {} bytes", ours.len()),
        (Err(Error::Corrupt(why)), Ok(_)) if why.starts_with("Huffman stream") => {}
        (Err(_), Ok(_)) if c_streaming_rejects(data) => {}
        (Err(e), Ok(v)) => panic!("we reject what both C decoders accept: {e} (C output {} bytes)", v.len()),
    }
});
