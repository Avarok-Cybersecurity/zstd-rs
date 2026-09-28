//! Dictionaries: zstd-format (magic, ID, entropy tables, repeat offsets, content,
//! as produced by `zstd --train` / ZDICT) and raw-content dictionaries.
use crate::error::{Error, Result};
use crate::fse::{ncount, Norm};
use crate::huf::{weights, HufWeights};
use alloc::vec::Vec;

pub const DICT_MAGIC: u32 = 0xEC30_A437;

/// Entropy tables carried by a zstd-format dictionary.
#[derive(Clone, Debug)]
pub struct DictTables {
    pub huf: HufWeights,
    pub of: Norm,
    pub ml: Norm,
    pub ll: Norm,
}

/// A parsed dictionary, usable by both the decoder and (via `EncoderDictionary`) the encoder.
#[derive(Clone, Debug)]
pub struct Dictionary {
    pub(crate) id: u32,
    pub(crate) content: Vec<u8>,
    pub(crate) tables: Option<DictTables>,
    pub(crate) reps: [u32; 3],
}

/// How to interpret dictionary bytes. There is deliberately no auto-detection default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DictionaryFormat {
    /// Must start with the zstd dictionary magic; entropy tables and repeat offsets are loaded.
    Zstd,
    /// The bytes are history only; frames reference it with dictionary ID 0 unless `id` is given.
    Raw {
        /// ID to write in frame headers (0 = none).
        id: u32,
    },
    /// Zstd format if the magic is present, else raw with ID 0 (the reference library's behaviour).
    Detect,
}

impl Dictionary {
    /// Parses `bytes` in the given format.
    pub fn new(bytes: &[u8], format: DictionaryFormat) -> Result<Dictionary> {
        let has_magic = bytes.len() >= 8 && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == DICT_MAGIC;
        match format {
            DictionaryFormat::Zstd if has_magic => Self::parse_zstd(bytes),
            DictionaryFormat::Zstd => Err(Error::BadDictionary("missing zstd dictionary magic")),
            DictionaryFormat::Raw { id } => Ok(Self::raw(bytes, id)),
            DictionaryFormat::Detect if has_magic => Self::parse_zstd(bytes),
            DictionaryFormat::Detect => Ok(Self::raw(bytes, 0)),
        }
    }

    fn raw(bytes: &[u8], id: u32) -> Dictionary {
        Dictionary { id, content: bytes.to_vec(), tables: None, reps: [1, 4, 8] }
    }

    fn parse_zstd(bytes: &[u8]) -> Result<Dictionary> {
        let id = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let mut at = 8;
        let bad = |e: Error| match e {
            Error::Corrupt(why) => Error::BadDictionary(why),
            Error::Truncated => Error::BadDictionary("truncated entropy tables"),
            other => other,
        };
        let (huf, used) = weights::read(&bytes[at..]).map_err(bad)?;
        at += used;
        let (of, used) = ncount::read(&bytes[at..], 32, 8).map_err(bad)?;
        at += used;
        let (ml, used) = ncount::read(&bytes[at..], 53, 9).map_err(bad)?;
        at += used;
        let (ll, used) = ncount::read(&bytes[at..], 36, 9).map_err(bad)?;
        at += used;
        let rep_bytes = bytes.get(at..at + 12).ok_or(Error::BadDictionary("truncated repeat offsets"))?;
        let mut reps = [0u32; 3];
        for (i, r) in reps.iter_mut().enumerate() {
            let b = &rep_bytes[4 * i..4 * i + 4];
            *r = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        }
        let content = bytes[at + 12..].to_vec();
        for &r in &reps {
            if r == 0 || r as usize > content.len() {
                return Err(Error::BadDictionary("repeat offset outside content"));
            }
        }
        Ok(Dictionary { id, content, tables: Some(DictTables { huf, of, ml, ll }), reps })
    }

    /// Dictionary ID written to / expected in frame headers (0 = none).
    pub fn id(&self) -> u32 {
        self.id
    }

    /// The history bytes matches may reference.
    pub fn content(&self) -> &[u8] {
        &self.content
    }

    /// True when the dictionary carries entropy tables.
    pub fn has_entropy_tables(&self) -> bool {
        self.tables.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_and_detect() {
        let d = Dictionary::new(b"hello world", DictionaryFormat::Detect).unwrap();
        assert_eq!((d.id(), d.content(), d.has_entropy_tables()), (0, &b"hello world"[..], false));
        assert!(Dictionary::new(b"hello world", DictionaryFormat::Zstd).is_err());
        let r = Dictionary::new(b"x", DictionaryFormat::Raw { id: 9 }).unwrap();
        assert_eq!(r.id(), 9);
    }

    #[test]
    fn truncated_zstd_dict_is_error() {
        let mut b = DICT_MAGIC.to_le_bytes().to_vec();
        b.extend_from_slice(&[1, 0, 0, 0, 0x80]);
        assert!(matches!(Dictionary::new(&b, DictionaryFormat::Zstd), Err(Error::BadDictionary(_))));
    }
}
