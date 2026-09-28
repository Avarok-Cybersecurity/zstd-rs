//! Compression: configuration, the reusable `Compressor`, and frame assembly.
mod block;
mod cost;
mod dictionary;
mod entropy;
mod literals;
mod matchers;
mod params;
mod seqstore;
mod sequences;

pub use dictionary::EncoderDictionary;
pub use params::{MAX_LEVEL, MIN_LEVEL};

use crate::error::{Error, Result};
use crate::frame::{write_block_header, BlockType, FrameFormat, FrameHeader, BLOCK_MAX};
use crate::xxh64::xxh64;
use alloc::vec::Vec;
use entropy::{EntropyState, PredefinedC};
use matchers::{opt::OptState, BlockInput, Tables};
use params::CParams;
use seqstore::{Reps, SeqStore};
use sequences::Codes;

/// Every compression setting, stated explicitly by the caller. Use one of the
/// named presets or build your own; there is no implicit default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompressionConfig {
    /// zstd level: `MIN_LEVEL..=MAX_LEVEL`, excluding 0. Levels mean what they mean in zstd.
    pub level: i32,
    /// Upper bound on the window (log2 bytes, 10..=30). The window also shrinks to fit the input.
    pub window_log: u32,
    /// Append the XXH64-based content checksum.
    pub checksum: bool,
    /// Record the content size in the frame header.
    pub content_size: bool,
    /// Record the dictionary ID in the frame header (when the dictionary has one).
    pub dict_id: bool,
    /// Standard frames, or magicless ones (4 bytes smaller; the decoder must be told).
    pub format: FrameFormat,
}

impl CompressionConfig {
    /// zstd level 1: fastest.
    pub const FAST: CompressionConfig =
        CompressionConfig { level: 1, window_log: 23, checksum: false, content_size: true, dict_id: true, format: FrameFormat::Standard };
    /// zstd level 3: the reference default.
    pub const DEFAULT: CompressionConfig = CompressionConfig { level: 3, ..Self::FAST };
    /// zstd level 19: optimal parsing.
    pub const HIGH: CompressionConfig = CompressionConfig { level: 19, ..Self::FAST };
    /// Small-message profile: level 3, no dictionary ID (the ID is known out of band).
    pub const MESSAGE: CompressionConfig = CompressionConfig { dict_id: false, ..Self::DEFAULT };

    /// Checks every field's range.
    pub fn validate(&self) -> Result<()> {
        if self.level == 0 || !(MIN_LEVEL..=MAX_LEVEL).contains(&self.level) {
            return Err(Error::Parameter("level must be in MIN_LEVEL..=MAX_LEVEL and not 0"));
        }
        if !(10..=30).contains(&self.window_log) {
            return Err(Error::Parameter("window_log must be in 10..=30"));
        }
        Ok(())
    }
}

/// Reusable per-frame state shared by the block compressor.
pub(crate) struct Workspace {
    pub params: CParams,
    pub tables: Tables,
    pub store: SeqStore,
    pub opt: OptState,
    pub entropy: EntropyState,
    pub pre: PredefinedC,
    pub codes: Codes,
    pub reps: Reps,
}

/// A reusable compression context bound to one configuration.
pub struct Compressor {
    cfg: CompressionConfig,
    w: Workspace,
    prev_len: usize,
}

impl Compressor {
    /// Creates a context for `cfg` (validated here).
    pub fn new(cfg: CompressionConfig) -> Result<Compressor> {
        cfg.validate()?;
        let w = Workspace {
            params: params::lookup(cfg.level, 0),
            tables: Tables::new(),
            store: SeqStore::new(),
            opt: OptState::new(),
            entropy: EntropyState::new(),
            pre: PredefinedC::new()?,
            codes: Codes::default(),
            reps: [1, 4, 8],
        };
        Ok(Compressor { cfg, w, prev_len: 0 })
    }

    /// The configuration this context compresses with.
    pub fn config(&self) -> &CompressionConfig {
        &self.cfg
    }

    /// Compresses `src` into one frame appended to `out`. A dictionary must have
    /// been prepared for the same level.
    pub fn compress(&mut self, src: &[u8], dict: Option<&EncoderDictionary>, out: &mut Vec<u8>) -> Result<()> {
        let p = self.frame_params(src.len(), dict)?;
        write_header(&self.cfg, &p, src.len(), dict, out);
        let whole = Job { start: 0, end: src.len(), prefix_start: 0, first: true, last: true };
        self.blocks(src, &whole, dict, &p, out)?;
        write_checksum(&self.cfg, src, out);
        Ok(())
    }

    /// Compresses one job of a frame (see [`crate::par`]) into complete blocks,
    /// without the frame header or checksum. The output depends only on
    /// (`self.config()`, `src`, `job`, `dict`), never on which context runs it.
    pub fn compress_job(&mut self, src: &[u8], job: &Job, dict: Option<&EncoderDictionary>, out: &mut Vec<u8>) -> Result<()> {
        if job.start > job.end || job.end > src.len() || job.prefix_start > job.start {
            return Err(Error::Parameter("job range outside the input"));
        }
        let p = self.frame_params(src.len(), dict)?;
        let window_start = job.start.saturating_sub(1 << p.window_log);
        let job = Job { prefix_start: job.prefix_start.max(window_start), ..*job };
        self.blocks(src, &job, if job.first { dict } else { None }, &p, out)
    }

    fn frame_params(&self, len: usize, dict: Option<&EncoderDictionary>) -> Result<CParams> {
        checked_params(&self.cfg, len, dict)
    }

    /// Emits the blocks covering `job`; the first job starts from the dictionary's
    /// state, later ones from none (unknown repeat offsets, no reusable tables).
    fn blocks(&mut self, src: &[u8], job: &Job, dict: Option<&EncoderDictionary>, p: &CParams, out: &mut Vec<u8>) -> Result<()> {
        let w = &mut self.w;
        w.params = *p;
        let hash2_log = if p.strategy == params::Strategy::DFast { p.chain_log } else { 0 };
        let chain_log = if p.strategy >= params::Strategy::Greedy { p.chain_log } else { 0 };
        w.tables.start_frame(src.len(), p.hash_log, hash2_log, chain_log, self.prev_len);
        self.prev_len = src.len();
        matchers::load_prefix(&mut w.tables, src, job.prefix_start, job.start, p);
        let dict_tables = dict.and_then(|d| d.tables.as_ref());
        w.entropy.reset(dict_tables);
        w.reps = match (job.first, dict) {
            (true, Some(d)) => d.reps,
            (true, None) => [1, 4, 8],
            (false, _) => [0, 0, 0],
        };
        w.opt.first_block = true;
        let window = 1usize << p.window_log;
        if job.start == job.end {
            if job.last {
                write_block_header(true, BlockType::Raw, 0, out);
            }
            return Ok(());
        }
        let block = BLOCK_MAX.min(window);
        let mut start = job.start;
        while start < job.end {
            let end = (start + block).min(job.end);
            let attached = dict.filter(|_| end <= window && job.prefix_start == 0).map(|d| &d.matcher);
            let b = BlockInput { src, start, end, low: job.prefix_start, max_dist: window, dict: attached };
            block::compress(w, &b, dict_tables, job.last && end == job.end, start > 0, out)?;
            start = end;
        }
        Ok(())
    }
}

/// Validates a request and resolves its frame parameters.
fn checked_params(cfg: &CompressionConfig, len: usize, dict: Option<&EncoderDictionary>) -> Result<CParams> {
    cfg.validate()?;
    if len > (u32::MAX >> 2) as usize {
        return Err(Error::Parameter("input larger than 1 GiB; split it into frames"));
    }
    if dict.is_some_and(|d| d.level != cfg.level) {
        return Err(Error::Parameter("dictionary prepared for a different level"));
    }
    Ok(frame_params(cfg, len, dict))
}

/// Writes the header of a frame holding `len` bytes (used by the job API).
pub(crate) fn begin_frame(cfg: &CompressionConfig, len: usize, dict: Option<&EncoderDictionary>, out: &mut Vec<u8>) -> Result<()> {
    let p = checked_params(cfg, len, dict)?;
    write_header(cfg, &p, len, dict, out);
    Ok(())
}

/// Writes the frame header for an input of `len` bytes.
pub(crate) fn write_header(cfg: &CompressionConfig, p: &CParams, len: usize, dict: Option<&EncoderDictionary>, out: &mut Vec<u8>) {
    let window = 1u64 << p.window_log;
    let header = FrameHeader {
        window_size: window,
        content_size: cfg.content_size.then_some(len as u64),
        dict_id: if cfg.dict_id { dict.map_or(0, |d| d.id) } else { 0 },
        checksum: cfg.checksum,
        single_segment: cfg.content_size && len as u64 <= window,
        header_len: 0,
    };
    header.write_format(cfg.format, p.window_log, out);
}

/// Appends the content checksum if configured.
pub(crate) fn write_checksum(cfg: &CompressionConfig, src: &[u8], out: &mut Vec<u8>) {
    if cfg.checksum {
        out.extend_from_slice(&(xxh64(src, 0) as u32).to_le_bytes());
    }
}

/// One independently compressible slice of a frame's input: blocks for
/// `src[start..end]`, which may reference `src[prefix_start..start]` as history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Job {
    /// First input byte this job encodes.
    pub start: usize,
    /// One past the last input byte this job encodes.
    pub end: usize,
    /// Lowest input byte the job may reference (the overlap with earlier jobs).
    pub prefix_start: usize,
    /// True for the first job: it may use the dictionary and its repeat offsets.
    pub first: bool,
    /// True for the last job: its final block is marked last.
    pub last: bool,
}

/// Frame parameters: the level's row for this input size, shrunk to fit; with a
/// dictionary, the dictionary's search settings (its tables are shared) and frame-sized tables.
fn frame_params(cfg: &CompressionConfig, len: usize, dict: Option<&EncoderDictionary>) -> CParams {
    let mut p = params::adjust(params::lookup(cfg.level, len as u64), len as u64, 0);
    if let Some(d) = dict {
        let dp = d.matcher.params;
        p.strategy = dp.strategy;
        p.min_match = dp.min_match;
        p.search_log = dp.search_log;
        p.target_length = dp.target_length;
        p.hash_log = p.hash_log.max(6);
        p.chain_log = p.chain_log.max(6);
    }
    p.window_log = p.window_log.min(cfg.window_log);
    p
}
