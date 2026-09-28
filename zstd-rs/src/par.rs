//! The parallelism boundary. The core stays pure and single-threaded: it plans
//! independent jobs, compresses or decompresses one job at a time, and assembles
//! results. *How* jobs run (native threads, Web Workers, or one after another) is
//! an [`Executor`] the caller supplies; nothing here picks one implicitly.
//! Output is byte-identical for every executor, because each job's bytes depend
//! only on (config, input, job, dictionary).
use crate::decode::{DecoderDictionary, Decompressor};
pub use crate::encode::Job;
use crate::encode::{begin_frame, write_checksum, CompressionConfig, Compressor, EncoderDictionary};
use crate::error::{Error, Result};
use crate::frame::{parse_block_header, BlockType, FrameHeader, SKIPPABLE_BASE, SKIPPABLE_MASK};
use alloc::vec::Vec;

/// Runs `f` over `items` and returns the results in input order.
pub trait Executor {
    /// Applies `f` to every item. Implementations may run calls concurrently.
    fn map<T: Sync, R: Send>(&self, items: &[T], f: &(dyn Fn(&T) -> R + Sync)) -> Vec<R>;
}

/// Runs jobs one after another on the calling thread (an explicit choice, not a default).
#[derive(Clone, Copy, Debug)]
pub struct Sequential;

impl Executor for Sequential {
    fn map<T: Sync, R: Send>(&self, items: &[T], f: &(dyn Fn(&T) -> R + Sync)) -> Vec<R> {
        items.iter().map(f).collect()
    }
}

/// Splits `len` input bytes into jobs of `job_size` bytes, each allowed to
/// reference up to `overlap` bytes before its start (clamped to the frame
/// window when the job runs).
pub fn plan(len: usize, job_size: usize, overlap: usize) -> Result<Vec<Job>> {
    if job_size < 1024 {
        return Err(Error::Parameter("job_size must be at least 1024"));
    }
    if len == 0 {
        return Ok(alloc::vec![Job { start: 0, end: 0, prefix_start: 0, first: true, last: true }]);
    }
    let mut jobs = Vec::with_capacity(len.div_ceil(job_size));
    let mut start = 0;
    while start < len {
        let end = (start + job_size).min(len);
        jobs.push(Job { start, end, prefix_start: start.saturating_sub(overlap), first: start == 0, last: end == len });
        start = end;
    }
    Ok(jobs)
}

fn check_plan(len: usize, jobs: &[Job]) -> Result<()> {
    let mut at = 0;
    for (i, j) in jobs.iter().enumerate() {
        if j.start != at || j.end < j.start || j.first != (i == 0) || j.last != (i + 1 == jobs.len()) || j.prefix_start > j.start {
            return Err(Error::Parameter("jobs must tile the input in order"));
        }
        at = j.end;
    }
    if at != len || jobs.is_empty() {
        return Err(Error::Parameter("jobs must cover the whole input"));
    }
    Ok(())
}

/// Writes the header of a frame holding `len` bytes, for callers that run jobs
/// themselves (for example on Web Workers): header, then every job's
/// [`Compressor::compress_job`] output in order, then [`frame_trailer`].
pub fn frame_header(cfg: &CompressionConfig, len: usize, dict: Option<&EncoderDictionary>, out: &mut Vec<u8>) -> Result<()> {
    begin_frame(cfg, len, dict, out)
}

/// Writes the end of a frame over `src` (the checksum, when configured).
pub fn frame_trailer(cfg: &CompressionConfig, src: &[u8], out: &mut Vec<u8>) {
    write_checksum(cfg, src, out);
}

/// Compresses `src` into one frame whose jobs run on `exec`.
pub fn compress_frame<E: Executor>(
    exec: &E,
    cfg: &CompressionConfig,
    src: &[u8],
    dict: Option<&EncoderDictionary>,
    jobs: &[Job],
    out: &mut Vec<u8>,
) -> Result<()> {
    check_plan(src.len(), jobs)?;
    begin_frame(cfg, src.len(), dict, out)?;
    let parts = exec.map(jobs, &|job: &Job| -> Result<Vec<u8>> {
        let mut c = Compressor::new(*cfg)?;
        let mut o = Vec::new();
        c.compress_job(src, job, dict, &mut o)?;
        Ok(o)
    });
    for p in parts {
        out.extend_from_slice(&p?);
    }
    write_checksum(cfg, src, out);
    Ok(())
}

/// Compresses `src` as independent frames of `frame_size` bytes (each decodable
/// on its own, and in parallel by [`decompress_frames`]).
pub fn compress_frames<E: Executor>(
    exec: &E,
    cfg: &CompressionConfig,
    src: &[u8],
    dict: Option<&EncoderDictionary>,
    frame_size: usize,
    out: &mut Vec<u8>,
) -> Result<()> {
    if frame_size == 0 {
        return Err(Error::Parameter("frame_size must be positive"));
    }
    let chunks: Vec<&[u8]> = if src.is_empty() { alloc::vec![src] } else { src.chunks(frame_size).collect() };
    let parts = exec.map(&chunks, &|chunk: &&[u8]| -> Result<Vec<u8>> {
        let mut o = Vec::new();
        Compressor::new(*cfg)?.compress(chunk, dict, &mut o)?;
        Ok(o)
    });
    for p in parts {
        out.extend_from_slice(&p?);
    }
    Ok(())
}

/// Location of one frame inside a buffer of concatenated frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSpan {
    /// Offset of the frame's first byte.
    pub start: usize,
    /// Offset just past the frame.
    pub end: usize,
    /// Declared content size (`None` for skippable frames or frames without one).
    pub content_size: Option<u64>,
    /// True for skippable frames (no content).
    pub skippable: bool,
}

/// Finds frame boundaries by reading headers and block headers only.
pub fn split_frames(src: &[u8]) -> Result<Vec<FrameSpan>> {
    let mut spans = Vec::new();
    let mut at = 0;
    while at < src.len() {
        let rest = &src[at..];
        let magic = u32::from_le_bytes(rest.get(..4).ok_or(Error::Truncated)?.try_into().map_err(|_| Error::Truncated)?);
        let (len, content_size, skippable) = if magic & SKIPPABLE_MASK == SKIPPABLE_BASE {
            let n = u32::from_le_bytes(rest.get(4..8).ok_or(Error::Truncated)?.try_into().map_err(|_| Error::Truncated)?);
            (8 + n as usize, None, true)
        } else {
            let h = FrameHeader::parse(rest)?;
            let mut p = h.header_len;
            loop {
                let (last, ty, size) = parse_block_header(rest.get(p..).ok_or(Error::Truncated)?)?;
                p += 3 + if ty == BlockType::Rle { 1 } else { size };
                if last {
                    break;
                }
            }
            (p + if h.checksum { 4 } else { 0 }, h.content_size, false)
        };
        if at + len > src.len() {
            return Err(Error::Truncated);
        }
        spans.push(FrameSpan { start: at, end: at + len, content_size, skippable });
        at += len;
    }
    Ok(spans)
}

/// Decodes concatenated frames on `exec`, appending to `out`. Every frame must
/// declare its content size (so each job's output is bounded before it runs);
/// the total may not exceed `max_output`.
pub fn decompress_frames<E: Executor>(
    exec: &E,
    src: &[u8],
    dict: Option<&DecoderDictionary>,
    max_output: usize,
    out: &mut Vec<u8>,
) -> Result<usize> {
    let spans: Vec<FrameSpan> = split_frames(src)?.into_iter().filter(|s| !s.skippable).collect();
    let mut total: u64 = 0;
    for s in &spans {
        total += s.content_size.ok_or(Error::Parameter("parallel decoding needs every frame's content size"))?;
    }
    if total > max_output as u64 {
        return Err(Error::OutputLimit);
    }
    let parts = exec.map(&spans, &|s: &FrameSpan| -> Result<Vec<u8>> {
        let cs = s.content_size.unwrap_or(0) as usize;
        let mut o = Vec::with_capacity(cs);
        Decompressor::new().decompress_frame(&src[s.start..s.end], dict, cs, &mut o)?;
        Ok(o)
    });
    let before = out.len();
    for p in parts {
        out.extend_from_slice(&p?);
    }
    Ok(out.len() - before)
}
