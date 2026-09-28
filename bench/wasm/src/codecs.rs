//! The codecs under test in wasm (zstd-C is absent: it does not build for wasm32-unknown-unknown).
use zstd_rs::{CompressionConfig, Compressor, DecoderDictionary, Decompressor, Dictionary, DictionaryFormat, EncoderDictionary};

pub type Enc = Box<dyn FnMut(&[u8], &mut Vec<u8>)>;
pub type Dec = Box<dyn FnMut(&[u8], usize, &mut Vec<u8>)>;

pub struct Codec {
    pub name: String,
    pub c: Enc,
    pub d: Dec,
}

fn zr(level: i32, zdict: Option<&[u8]>) -> Codec {
    let mut c = Compressor::new(CompressionConfig { level, ..CompressionConfig::MESSAGE }).unwrap();
    let dict = zdict.map(|d| Dictionary::new(d, DictionaryFormat::Zstd).unwrap());
    let edict = dict.as_ref().map(|d| EncoderDictionary::new(d, level).unwrap());
    let ddict = dict.map(|d| DecoderDictionary::new(d).unwrap());
    let mut dz = Decompressor::new();
    Codec {
        name: format!("zr-{level}{}", if zdict.is_some() { "-dict" } else { "" }),
        c: Box::new(move |b, o| c.compress(b, edict.as_ref(), o).unwrap()),
        d: Box::new(move |b, n, o| {
            dz.decompress(b, ddict.as_ref(), n, o).unwrap();
        }),
    }
}

fn br(q: i32, lgwin: i32) -> Codec {
    Codec {
        name: format!("brotli-q{q}-w{lgwin}"),
        c: Box::new(move |b, o| {
            let p = brotli::enc::BrotliEncoderParams { quality: q, lgwin, size_hint: b.len(), ..Default::default() };
            brotli::BrotliCompress(&mut std::io::Cursor::new(b), o, &p).unwrap();
        }),
        d: Box::new(|b, _, o| {
            brotli::BrotliDecompress(&mut std::io::Cursor::new(b), o).unwrap();
        }),
    }
}

pub fn all(zdict: &[u8]) -> Vec<Codec> {
    let mut v = Vec::new();
    for level in [1, 3, 19] {
        v.push(zr(level, None));
        v.push(zr(level, Some(zdict)));
    }
    let mut fd = ruzstd::decoding::FrameDecoder::new();
    v.push(Codec {
        name: "ruzstd-fastest".into(),
        c: Box::new(|b, o| *o = ruzstd::encoding::compress_to_vec(b, ruzstd::encoding::CompressionLevel::Fastest)),
        d: Box::new(move |b, n, o| {
            o.reserve(n);
            if fd.decode_all_to_vec(b, o).is_err() {
                o.clear();
            }
        }),
    });
    let mut plain = zr(3, None);
    let mut fd2 = ruzstd::decoding::FrameDecoder::new();
    v.push(Codec {
        name: "dec:ruzstd<-zr3".into(),
        c: Box::new(move |b, o| (plain.c)(b, o)),
        d: Box::new(move |b, n, o| {
            o.reserve(n);
            if fd2.decode_all_to_vec(b, o).is_err() {
                o.clear();
            }
        }),
    });
    v.push(br(1, 16));
    v.push(br(4, 18));
    v.push(Codec {
        name: "deflate-l1".into(),
        c: Box::new(|b, o| *o = miniz_oxide::deflate::compress_to_vec(b, 1)),
        d: Box::new(|b, n, o| *o = miniz_oxide::inflate::decompress_to_vec_with_limit(b, n).unwrap()),
    });
    v.push(Codec {
        name: "lz4_flex".into(),
        c: Box::new(|b, o| *o = lz4_flex::block::compress(b)),
        d: Box::new(|b, n, o| *o = lz4_flex::block::decompress(b, n).unwrap()),
    });
    #[cfg(rill_bench)]
    {
        let prof = rill::builtin_v1();
        let mut rc = rill::Compressor::new();
        let mut rd = rill::Decompressor::new();
        v.push(Codec {
            name: "rill".into(),
            c: Box::new(move |b, o| rc.compress_into(&prof, b, &rill::Options::DEFAULT, o)),
            d: Box::new(move |b, n, o| rd.decompress_into(&[prof], b, n, o).unwrap()),
        });
    }
    v
}
