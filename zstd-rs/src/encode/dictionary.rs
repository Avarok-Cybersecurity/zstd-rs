//! Dictionaries prepared for compression: content, match tables built once for
//! a compression level ("attached" to every frame, never copied), entropy tables
//! and repeat offsets.
use super::entropy::DictCTables;
use super::matchers::{hash, read64};
use super::params::{adjust, lookup, CParams, Strategy};
use crate::dict::Dictionary;
use crate::error::{Error, Result};
use alloc::vec;
use alloc::vec::Vec;

/// Read-only match state of a dictionary. Stored positions are `index + 1` (0 = empty).
pub struct DictMatchState {
    pub content: Vec<u8>,
    pub hash: Vec<u32>,
    pub hash2: Vec<u32>,
    pub chain: Vec<u32>,
    pub params: CParams,
}

impl DictMatchState {
    fn build(content: &[u8], p: CParams) -> DictMatchState {
        let n = content.len();
        let mls = p.min_match.clamp(4, 7);
        let mut hash_t = vec![0u32; 1 << p.hash_log];
        let mut hash2_t = Vec::new();
        let mut chain_t = Vec::new();
        let last = n.saturating_sub(8);
        match p.strategy {
            Strategy::Fast => {
                for i in 0..last {
                    hash_t[hash(read64(content, i), mls, p.hash_log)] = i as u32 + 1;
                }
            }
            Strategy::DFast => {
                hash2_t = vec![0u32; 1 << p.chain_log];
                for i in 0..last {
                    let v = read64(content, i);
                    hash_t[hash(v, 8, p.hash_log)] = i as u32 + 1;
                    hash2_t[hash(v, mls, p.chain_log)] = i as u32 + 1;
                }
            }
            _ => {
                chain_t = vec![0u32; 1 << p.chain_log];
                let mask = (1usize << p.chain_log) - 1;
                let hmls = super::matchers::hc::chain_mls(&p);
                for i in 0..last {
                    let h = hash(read64(content, i), hmls, p.hash_log);
                    chain_t[i & mask] = hash_t[h];
                    hash_t[h] = i as u32 + 1;
                }
            }
        }
        DictMatchState { content: content.to_vec(), hash: hash_t, hash2: hash2_t, chain: chain_t, params: p }
    }
}

/// A dictionary prepared for one compression level. Share it across frames and threads.
pub struct EncoderDictionary {
    pub(crate) id: u32,
    pub(crate) reps: [u32; 3],
    pub(crate) level: i32,
    pub(crate) matcher: DictMatchState,
    pub(crate) tables: Option<DictCTables>,
}

impl EncoderDictionary {
    /// Prepares `dict` for compressing at `level` (the level must match the
    /// compressor's; the reference library binds a CDict to a level the same way).
    pub fn new(dict: &Dictionary, level: i32) -> Result<EncoderDictionary> {
        if !(super::params::MIN_LEVEL..=super::params::MAX_LEVEL).contains(&level) || level == 0 {
            return Err(Error::Parameter("compression level out of range"));
        }
        let size = dict.content.len() as u64;
        let p = adjust(lookup(level, size + 500), 513, size);
        let tables = match &dict.tables {
            Some(t) => Some(DictCTables::new(t)?),
            None => None,
        };
        Ok(EncoderDictionary { id: dict.id, reps: dict.reps, level, matcher: DictMatchState::build(&dict.content, p), tables })
    }

    /// The level this dictionary was prepared for.
    pub fn level(&self) -> i32 {
        self.level
    }

    /// Dictionary ID written to frame headers when enabled.
    pub fn id(&self) -> u32 {
        self.id
    }
}
