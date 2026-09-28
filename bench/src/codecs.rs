//! Codec adapters. Every codec reuses its context across calls where the API allows.
//! zstd-C and zstd-rs share the same ZDICT dictionary and both omit the dictionary
//! ID from frames (the ID is known out of band in ILM), so frame overhead matches.
use std::cell::RefCell;
use std::rc::Rc;
use zstd_rs::{CompressionConfig, Compressor, DecoderDictionary, Decompressor, Dictionary, DictionaryFormat, EncoderDictionary};

pub type Enc = Box<dyn FnMut(&[u8]) -> Vec<u8>>;
pub type Dec = Box<dyn FnMut(&[u8], usize) -> Vec<u8>>;

pub struct Codec {
    pub name: String,
    pub c: Enc,
    pub d: Dec,
}

fn codec(name: &str, c: Enc, d: Dec) -> Codec {
    Codec { name: name.to_string(), c, d }
}

fn zl_decoder(dict: Option<Rc<DecoderDictionary>>) -> Dec {
    let mut dz = Decompressor::new();
    Box::new(move |b, n| {
        let mut o = Vec::with_capacity(n);
        dz.decompress(b, dict.as_deref(), n, &mut o).unwrap();
        o
    })
}

fn zl(level: i32, zdict: Option<&[u8]>, checksum: bool) -> Codec {
    let cfg = CompressionConfig { level, checksum, ..CompressionConfig::MESSAGE };
    let mut c = Compressor::new(cfg).unwrap();
    let dict = zdict.map(|d| Dictionary::new(d, DictionaryFormat::Zstd).unwrap());
    let edict = dict.as_ref().map(|d| EncoderDictionary::new(d, level).unwrap());
    let ddict = dict.map(|d| Rc::new(DecoderDictionary::new(d).unwrap()));
    let name = format!("zl-{level}{}{}", if zdict.is_some() { "-dict" } else { "" }, if checksum { "-ck" } else { "" });
    codec(
        &name,
        Box::new(move |b| {
            let mut o = Vec::with_capacity(b.len() / 2 + 64);
            c.compress(b, edict.as_ref(), &mut o).unwrap();
            o
        }),
        zl_decoder(ddict),
    )
}

fn zstd_c(level: i32, dict: Option<&[u8]>) -> Codec {
    use zstd::zstd_safe::{CParameter, DParameter};
    let mut c = match dict {
        Some(d) => zstd::bulk::Compressor::with_dictionary(level, d).unwrap(),
        None => zstd::bulk::Compressor::new(level).unwrap(),
    };
    c.set_parameter(CParameter::DictIdFlag(false)).unwrap();
    let mut d = match dict {
        Some(d) => zstd::bulk::Decompressor::with_dictionary(d).unwrap(),
        None => zstd::bulk::Decompressor::new().unwrap(),
    };
    d.set_parameter(DParameter::WindowLogMax(31)).unwrap();
    let name = format!("zstd-c-{level}{}", if dict.is_some() { "-dict" } else { "" });
    codec(&name, Box::new(move |b| c.compress(b).unwrap()), Box::new(move |b, n| d.decompress(b, n).unwrap()))
}

fn ruzstd_dec() -> Dec {
    let mut fd = ruzstd::decoding::FrameDecoder::new();
    Box::new(move |b, n| {
        let mut o = Vec::with_capacity(n);
        if fd.decode_all_to_vec(b, &mut o).is_err() {
            o.clear();
        }
        o
    })
}

/// Decoder comparison on identical input: frames made by zstd-C level 3 (with the
/// dictionary if given), decoded by `dec`.
fn decode_only(name: &str, dict: Option<&[u8]>, dec: Dec) -> Codec {
    let enc = Rc::new(RefCell::new(zstd_c(3, dict)));
    let e2 = enc.clone();
    codec(name, Box::new(move |b| (e2.borrow_mut().c)(b)), dec)
}

fn brotli_c(input: &[u8], q: i32, lgwin: i32) -> Vec<u8> {
    let params = brotli::enc::BrotliEncoderParams { quality: q, lgwin, size_hint: input.len(), ..Default::default() };
    let mut out = Vec::with_capacity(input.len() + 64);
    brotli::BrotliCompress(&mut std::io::Cursor::new(input), &mut out, &params).unwrap();
    out
}

fn brotli_d(input: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    brotli::BrotliDecompress(&mut std::io::Cursor::new(input), &mut out).unwrap();
    out
}

pub fn all(zdict: &[u8], rill_dict: &[u8]) -> Vec<Codec> {
    let _ = rill_dict;
    let mut v = Vec::new();
    for level in [1, 3, 19] {
        v.push(zl(level, None, false));
        v.push(zl(level, Some(zdict), false));
    }
    v.push(zl(1, None, true));
    for level in [1, 3, 19] {
        v.push(zstd_c(level, None));
        v.push(zstd_c(level, Some(zdict)));
    }
    v.push(codec(
        "ruzstd-fastest",
        Box::new(|b| ruzstd::encoding::compress_to_vec(b, ruzstd::encoding::CompressionLevel::Fastest)),
        ruzstd_dec(),
    ));
    v.push(decode_only("dec:ruzstd<-c3", None, ruzstd_dec()));
    v.push(decode_only("dec:zl<-c3", None, zl_decoder(None)));
    let dd = Rc::new(DecoderDictionary::new(Dictionary::new(zdict, DictionaryFormat::Zstd).unwrap()).unwrap());
    v.push(decode_only("dec:zl<-c3-dict", Some(zdict), zl_decoder(Some(dd))));
    let rd = ruzstd::decoding::Dictionary::decode_dict(zdict).unwrap();
    let mut fd = ruzstd::decoding::FrameDecoder::new();
    fd.add_dict(rd).unwrap();
    v.push(decode_only(
        "dec:ruzstd<-c3-dict",
        Some(zdict),
        Box::new(move |b, n| {
            let mut o = Vec::with_capacity(n);
            if fd.decode_all_to_vec(b, &mut o).is_err() {
                o.clear();
            }
            o
        }),
    ));
    v.push(codec("brotli-q1-w16", Box::new(|b| brotli_c(b, 1, 16)), Box::new(brotli_d)));
    v.push(codec("brotli-q4-w18", Box::new(|b| brotli_c(b, 4, 18)), Box::new(brotli_d)));
    v.push(codec(
        "deflate-l1",
        Box::new(|b| miniz_oxide::deflate::compress_to_vec(b, 1)),
        Box::new(|b, n| miniz_oxide::inflate::decompress_to_vec_with_limit(b, n).unwrap()),
    ));
    v.push(codec("lz4_flex", Box::new(|b| lz4_flex::block::compress(b)), Box::new(|b, n| lz4_flex::block::decompress(b, n).unwrap())));
    #[cfg(rill_bench)]
    {
        let prof = rill::builtin_v1();
        let mut rc = rill::Compressor::new();
        let mut rd = rill::Decompressor::new();
        v.push(codec(
            "rill",
            Box::new(move |b| {
                let mut o = Vec::new();
                rc.compress_into(&prof, b, &rill::Options::DEFAULT, &mut o);
                o
            }),
            Box::new(move |b, n| {
                let mut o = Vec::new();
                rd.decompress_into(&[prof], b, n, &mut o).unwrap();
                o
            }),
        ));
    }
    v
}
