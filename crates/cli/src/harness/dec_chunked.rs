// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C (and not in the reference harness): the chunked parallel decoding of
// fdk-aac-rs-dec.

//! The chunked parallel decoding of `fdk-aac-rs-dec` (`fdk_aac_rs::chunked`).
//!
//! The feeding loop of the tool is split into steps: one iteration of the fill loop of a byte
//! stream (a Fill call with `--fill-size` bytes, then the decode loop), or the feeding of one
//! access unit of a packet input. Every run (the true run, which uses the decoder opened by the
//! tool, and the speculative runs, each with its own decoder set up the same way) makes exactly
//! the calls of the sequential tool from its first step on and records the log records of the
//! calls with its own counters (call numbers, samples, stream statistics); the calling thread
//! writes them, with the counters of the sequential tool, and the PCM in order. The state digest
//! of a run is the digest of its decoder and of the tool's loop state (position in the input,
//! error count, zero fills, stop reason, output buffer). After the chunked part, the decoder of
//! the last run continues in the tool's sequential code (the end of the input, `--drain`, or the
//! rest of the input when the chunked decoding was given up, there with the frame pipelining).

use super::*;
use fdk_aac_rs::chunked::{
    run_chunked, ChunkJob, ChunkedOptions, ChunkedRunError, Step, StreamCounters,
};
use fdk_aac_rs_aacdec::digest::{HazardProbe, TRACE_DEPTH};
use fdk_aac_rs_fdk::digest::{Digest, Digester};
use std::sync::Mutex;

/// Bytes of input a speculative run of a byte stream decodes before its chunk (one and a half
/// times the input buffer of the transport decoder, whose content is part of the state).
const WARMUP_BYTES: usize = 48 << 10;
/// Access units a speculative run of a packet input decodes before its chunk (at least; and at
/// least [`WARMUP_BYTES`]).
const WARMUP_ACCESS_UNITS: usize = 32;

/// The chunk options of the command line.
#[derive(Clone, Debug, Default)]
pub(super) struct ChunkArgs {
    /// `--chunks N`: 0 off, else the number of chunks; `None`: automatic.
    pub chunks: Option<usize>,
    /// `--chunk-warmup N`: steps of warm-up (fills of a byte stream, access units of packets).
    pub warmup: Option<usize>,
    /// `--chunk-verify A,B,...`: verification points in steps after a chunk start.
    pub verify: Option<Vec<usize>>,
    /// `--chunk-mem MIB`: buffer limit of the speculative runs.
    pub mem_mib: Option<usize>,
}

/// The input of the chunked decoding.
pub(super) enum ChunkInput<'a> {
    /// A byte stream (`--fill-size` fills).
    Stream(&'a [u8]),
    /// Access units: (offset, size) in the data.
    Packets(&'a [u8], Vec<(usize, usize)>),
}

/// A log record of a run.
enum Rec {
    FreeBytes(AacDecoderError, u32),
    Fill { n: i64, size: u32, ret: AacDecoderError, valid: u32 },
    Decode(Box<DecRec>),
}

struct DecRec {
    call: i64,
    flags: u32,
    ret: AacDecoderError,
    n: i32,
    /// the little-endian PCM (when written or hashed)
    pcm: Vec<u8>,
    hash: Option<String>,
    poison: Option<[i64; 4]>,
    si: Option<CStreamInfo>,
    anc: Vec<AncCall>,
}

/// The counters of a run that are not part of its state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Counters {
    n_dec: i64,
    n_fill: i64,
    samples: u64,
    stats: StreamCounters,
}

impl core::ops::Add for Counters {
    type Output = Counters;
    fn add(self, o: Counters) -> Counters {
        Counters {
            n_dec: self.n_dec.wrapping_add(o.n_dec),
            n_fill: self.n_fill.wrapping_add(o.n_fill),
            samples: self.samples.wrapping_add(o.samples),
            stats: self.stats + o.stats,
        }
    }
}

impl core::ops::Sub for Counters {
    type Output = Counters;
    fn sub(self, o: Counters) -> Counters {
        Counters {
            n_dec: self.n_dec.wrapping_sub(o.n_dec),
            n_fill: self.n_fill.wrapping_sub(o.n_fill),
            samples: self.samples.wrapping_sub(o.samples),
            stats: self.stats - o.stats,
        }
    }
}

/// A run: a decoder with the state of the tool's feeding loop.
pub(super) struct Run {
    h: Box<AacDecoderInstance>,
    out: Vec<IntPcm>,
    /// the next step (`fill_idx` of a byte stream, the access unit of a packet input)
    step: usize,
    /// the position in a byte stream
    pos: usize,
    n_dec: i64,
    n_fill: i64,
    samples: u64,
    errors_in_row: i64,
    zero_fills: u32,
    stop: Option<&'static str>,
}

/// The settings of the runs (the options of the tool) and the input.
struct Job<'a> {
    input: &'a ChunkInput<'a>,
    /// byte stream: the start position of every step when every fill is consumed completely
    stream_pos: Vec<usize>,
    fill_sizes: Vec<u32>,
    transport: i32,
    layers: u32,
    asc: Vec<Vec<u8>>,
    init_params: Vec<(i32, i32)>,
    anc_size: i32,
    out_size: i32,
    poison: bool,
    free_bytes: bool,
    decode_once: bool,
    max_errors: i64,
    log: bool,
    pcm: bool,
    /// `--chunk-warmup` (steps); default: [`WARMUP_BYTES`] of a byte stream, and
    /// [`WARMUP_ACCESS_UNITS`] and [`WARMUP_BYTES`] of packets
    warmup: Option<usize>,
    /// the decoder of the tool (the true run)
    true_dec: Mutex<Option<Box<AacDecoderInstance>>>,
}

impl Job<'_> {
    fn finished(&self, r: &Run) -> bool {
        r.stop.is_some()
            || match self.input {
                ChunkInput::Stream(d) => r.pos >= d.len(),
                ChunkInput::Packets(_, p) => r.step >= p.len(),
            }
    }

    /// `fill()` of the tool.
    fn fill(&self, r: &mut Run, data: &[u8], recs: &mut Vec<Rec>) -> u32 {
        let size = data.len() as u32;
        let mut valid = [size];
        if self.free_bytes {
            let mut free: u32 = 0;
            let ret = aac_decoder_get_free_bytes(Some(&mut r.h), &mut free);
            recs.push(Rec::FreeBytes(ret, free));
        }
        let ret = aac_decoder_fill(&mut r.h, &[data], &[size], &mut valid);
        recs.push(Rec::Fill { n: r.n_fill, size, ret, valid: valid[0] });
        r.n_fill += 1;
        size.wrapping_sub(valid[0])
    }

    /// `decode()` of the tool with one thread and without events, and `log_decode()`,
    /// `account()`. Returns the result and whether the call was a hazard.
    fn decode(&self, r: &mut Run, recs: &mut Vec<Rec>) -> (AacDecoderError, bool) {
        let flags = 0;
        if self.poison {
            for i in 0..self.out_size {
                r.out[i as usize] = poison_value(i);
            }
        }
        let before = r.h.hazard_probe();
        let ret = aac_decoder_decode_frame(Some(&mut r.h), &mut r.out[..], self.out_size, flags);
        let hazard = HazardProbe::hazard(&before, &r.h.hazard_probe(), ret);

        let si = aac_decoder_get_stream_info(Some(&r.h)).copied();
        let valid = is_output_valid(ret) != 0;
        let n = output_samples(ret, si.as_ref(), self.out_size);
        let mut pcm = Vec::new();
        let mut hash = None;
        if n > 0 && (self.pcm || self.log) {
            pcm.reserve(n as usize * 2);
            for s in &r.out[..n as usize] {
                pcm.extend_from_slice(&s.to_le_bytes());
            }
            if self.log {
                hash = Some(sha256_hex(&pcm));
            }
            if !self.pcm {
                pcm = Vec::new();
            }
        }
        let poison = self.poison.then(|| poison_info(&r.out, self.out_size, n));
        let anc = if valid && self.anc_size > 0 { anc_calls(Some(&r.h)) } else { Vec::new() };
        recs.push(Rec::Decode(Box::new(DecRec {
            call: r.n_dec,
            flags,
            ret,
            n,
            pcm,
            hash,
            poison,
            si,
            anc,
        })));
        if n > 0 {
            r.samples = r.samples.wrapping_add(n as u64);
        }
        r.n_dec += 1;
        /* account() */
        if ret == AAC_DEC_OK || ret == AAC_DEC_NOT_ENOUGH_BITS {
            r.errors_in_row = 0;
        } else {
            r.errors_in_row += 1;
            if self.max_errors > 0 && r.errors_in_row >= self.max_errors {
                r.stop = Some("errors");
            }
        }
        (ret, hazard)
    }

    /// `decode_loop()` of the tool without `--max-calls`.
    fn decode_loop(&self, r: &mut Run, recs: &mut Vec<Rec>) -> bool {
        let mut hazard = false;
        loop {
            let (ret, hz) = self.decode(r, recs);
            hazard |= hz;
            if r.stop.is_some() || self.decode_once || ret == AAC_DEC_NOT_ENOUGH_BITS {
                return hazard;
            }
        }
    }

    fn open(&self) -> Option<Box<AacDecoderInstance>> {
        let mut h = aac_decoder_open(self.transport, self.layers)?;
        if !self.asc.is_empty() {
            let conf: Vec<&[u8]> = self.asc.iter().map(|a| a.as_slice()).collect();
            let len: Vec<u32> = self.asc.iter().map(|a| a.len() as u32).collect();
            aac_decoder_config_raw(&mut h, &conf, &len);
        }
        for &(id, value) in &self.init_params {
            aac_decoder_set_param(Some(&mut h), id, value);
        }
        if self.anc_size > 0 {
            let buf = vec![0u8; self.anc_size as usize].into_boxed_slice();
            aac_decoder_anc_data_init(&mut h, Some(buf), self.anc_size);
        }
        Some(h)
    }

    /// The state of a run (see the module documentation).
    fn feed(&self, r: &Run, d: &mut Digester) {
        let Run {
            h,
            out,
            step: _,
            pos,
            n_dec: _,
            n_fill: _,
            samples: _,
            errors_in_row,
            zero_fills,
            stop,
        } = r;
        d.field("decoder", &**h);
        d.field("out", out);
        d.field("pos", pos);
        d.field("errors_in_row", errors_in_row);
        d.field("zero_fills", zero_fills);
        d.field("stop", &stop.map(|s| s.as_bytes().to_vec()));
    }
}

impl ChunkJob for Job<'_> {
    type Run = Run;
    type Out = Vec<Rec>;
    type Counters = Counters;
    type Error = String;

    fn steps(&self) -> usize {
        match self.input {
            ChunkInput::Stream(_) => self.stream_pos.len() - 1,
            ChunkInput::Packets(_, p) => p.len(),
        }
    }

    fn warmup_start(&self, step: usize) -> Option<usize> {
        if let Some(n) = self.warmup {
            return step.checked_sub(n);
        }
        match self.input {
            ChunkInput::Stream(_) => {
                let target = self.stream_pos[step].checked_sub(WARMUP_BYTES)?;
                // the latest step that starts at least the warm-up before
                let w = self.stream_pos.partition_point(|&p| p <= target);
                w.checked_sub(1)
            }
            ChunkInput::Packets(_, p) => {
                let mut bytes = 0;
                let mut w = step;
                while w > 0 && (step - w < WARMUP_ACCESS_UNITS || bytes < WARMUP_BYTES) {
                    w -= 1;
                    bytes += p[w].1;
                }
                (step - w >= WARMUP_ACCESS_UNITS && bytes >= WARMUP_BYTES).then_some(w)
            }
        }
    }

    fn start(&self, step: usize) -> Result<Run, String> {
        let h = if step == 0 {
            self.true_dec.lock().unwrap().take().ok_or("no decoder")?
        } else {
            self.open().ok_or("aacDecoder_Open failed")?
        };
        let pos = match self.input {
            ChunkInput::Stream(_) => self.stream_pos[step],
            ChunkInput::Packets(..) => 0,
        };
        Ok(Run {
            h,
            out: vec![0; self.out_size as usize + 1],
            step,
            pos,
            n_dec: 0,
            n_fill: 0,
            samples: 0,
            errors_in_row: 0,
            zero_fills: 0,
            stop: None,
        })
    }

    fn position(&self, r: &Run) -> usize {
        r.step
    }

    fn step(&self, r: &mut Run) -> Result<Step<Vec<Rec>>, String> {
        let mut recs = Vec::new();
        let mut hazard = false;
        match self.input {
            ChunkInput::Stream(data) => {
                /* one iteration of feed() */
                let mut chunk = data.len() - r.pos;
                let fs = self.fill_sizes[r.step % self.fill_sizes.len()] as usize;
                chunk = chunk.min(fs);
                let consumed = self.fill(r, &data[r.pos..r.pos + chunk], &mut recs) as usize;
                r.pos += consumed;
                let mut stalled = false;
                if chunk > 0 && consumed == 0 {
                    r.zero_fills += 1;
                    if r.zero_fills >= 2 {
                        r.stop = Some("stall");
                        stalled = true;
                    }
                } else {
                    r.zero_fills = 0;
                }
                if !stalled {
                    hazard = self.decode_loop(r, &mut recs);
                }
            }
            ChunkInput::Packets(data, p) => {
                /* feed() of one access unit */
                let (o, size) = p[r.step];
                let au = &data[o..o + size];
                let mut q = 0;
                let mut zero_fills = 0;
                loop {
                    let chunk = size - q;
                    let consumed = self.fill(r, &au[q..], &mut recs) as usize;
                    q += consumed;
                    if chunk > 0 && consumed == 0 {
                        zero_fills += 1;
                        if zero_fills >= 2 {
                            r.stop = Some("stall");
                            break;
                        }
                    } else {
                        zero_fills = 0;
                    }
                    hazard |= self.decode_loop(r, &mut recs);
                    if r.stop.is_some() || q >= size {
                        break;
                    }
                }
            }
        }
        r.step += 1;
        Ok(Step { out: recs, hazard, finished: self.finished(r) })
    }

    fn digest(&self, r: &Run) -> Digest {
        let mut d = Digester::new();
        self.feed(r, &mut d);
        d.finish()
    }

    fn trace(&self, r: &Run) -> Vec<(String, Digest)> {
        let mut d = Digester::traced(TRACE_DEPTH);
        self.feed(r, &mut d);
        d.finish_traced().1
    }

    fn counters(&self, r: &Run) -> Counters {
        Counters {
            n_dec: r.n_dec,
            n_fill: r.n_fill,
            samples: r.samples,
            stats: StreamCounters::of(&r.h.stream_info),
        }
    }

    fn out_bytes(out: &Vec<Rec>) -> usize {
        out.iter()
            .map(|r| match r {
                Rec::Decode(d) => d.pcm.len() + 256,
                _ => 64,
            })
            .sum()
    }

    fn poison(&self, r: &mut Run, seed: u64) {
        r.h.poison_unfed_state(seed);
    }

    fn converges_rarely(&self, r: &Run) -> bool {
        r.h.converges_rarely()
    }
}

/// Writes the records of a step with the counters of the sequential tool.
fn write_records(st: &mut St, recs: Vec<Rec>, base: &Counters) -> Result<(), Die> {
    for rec in recs {
        match rec {
            Rec::FreeBytes(ret, free) => write_free_bytes(&mut st.j, ret, free),
            Rec::Fill { n, size, ret, valid } => {
                write_fill(&mut st.j, n.wrapping_add(base.n_fill), size, ret, valid)
            }
            Rec::Decode(d) => {
                let mut si = d.si;
                if let Some(si) = si.as_mut() {
                    base.stats.add_to(si);
                }
                write_decode(
                    &mut st.j,
                    d.call.wrapping_add(base.n_dec),
                    d.flags,
                    d.ret,
                    d.n,
                    d.hash.as_deref(),
                    d.poison.as_ref(),
                    si.as_ref(),
                );
                if !d.pcm.is_empty() {
                    if let Some(p) = st.pcm.as_mut() {
                        if p.write_all(&d.pcm).is_err() {
                            return die("cannot write PCM");
                        }
                    }
                }
                for (idx, r, size, data) in &d.anc {
                    write_anc(&mut st.j, *idx, *r, *size, data.as_ref().map(|(o, v)| (*o, &v[..])));
                }
            }
        }
    }
    Ok(())
}

/// Whether the options allow the chunked decoding (every call of the sequential tool must be
/// a function of the input and the decoder state, not of the call number).
pub(super) fn eligible(st: &St, args: &ChunkArgs) -> bool {
    args.chunks != Some(0)
        && (st.threads > 1 || args.chunks.is_some_and(|c| c > 1))
        && st.events.is_empty()
        && st.conceal_every == 0
        && st.max_calls == 0
}

/// Where the sequential tool continues after the chunked decoding.
pub(super) struct Resume {
    /// the next step (fill index of a byte stream, access unit of a packet input)
    pub step: usize,
    /// the position in a byte stream
    pub pos: usize,
}

/// The chunked decoding of the input with the decoder of the tool (`st.h`): writes the log and
/// the PCM of the steps it decodes and leaves the decoder of the last run and the counters in
/// `st` (see the module documentation).
pub(super) fn decode(st: &mut St, args: &ChunkArgs, input: &ChunkInput) -> Result<Resume, Die> {
    let mut stream_pos = vec![0usize];
    if let ChunkInput::Stream(data) = input {
        let mut p = 0usize;
        let mut i = 0usize;
        while p < data.len() {
            p += st.fill_sizes[i % st.fill_sizes.len()] as usize;
            stream_pos.push(p.min(data.len()));
            i += 1;
        }
    }
    let warmup = args.warmup;
    let job = Job {
        input,
        stream_pos,
        fill_sizes: st.fill_sizes.clone(),
        transport: st.transport,
        layers: st.layers as u32,
        asc: st.asc.clone(),
        init_params: st.init_params.iter().map(|e| (e.id as i32, e.value as i32)).collect(),
        anc_size: st.anc_size,
        out_size: st.out_size,
        poison: st.poison,
        free_bytes: st.free_bytes,
        decode_once: st.decode_once,
        max_errors: st.max_errors,
        log: st.j.enabled(),
        pcm: st.pcm.is_some(),
        warmup,
        true_dec: Mutex::new(st.h.take()),
    };
    let mut opts = ChunkedOptions {
        threads: st.threads.max(2) as usize,
        chunks: args.chunks.unwrap_or(0),
        ..Default::default()
    };
    if let Some(v) = &args.verify {
        opts.verify_offsets = v.clone();
    }
    if let Some(m) = args.mem_mib {
        opts.max_buffered_bytes = m << 20;
    }
    if let ChunkInput::Stream(_) = input {
        // chunks of at least 512 KiB of input (128 fills of 4096 bytes), verification points
        // after 0, 8 and 32 KiB (0, 2, 8 fills of 4096 bytes)
        let per_step =
            (st.fill_sizes.iter().map(|&f| f as usize).sum::<usize>() / st.fill_sizes.len()).max(1);
        opts.min_chunk_len = ((512 << 10) / per_step).max(16);
        if args.verify.is_none() {
            let k = ((8 << 10) / per_step).max(1);
            opts.verify_offsets = vec![0, k, 4 * k];
        }
    } else {
        // chunks of at least 512 access units (12 s at 44.1 kHz), verification points after 0,
        // 8 and 32 access units
        opts.min_chunk_len = 512;
        if args.verify.is_none() {
            opts.verify_offsets = vec![0, 8, 32];
        }
    }
    let bytes_per_step = match input {
        ChunkInput::Stream(_) => st.fill_sizes[0] as usize * 16,
        ChunkInput::Packets(..) => 8192,
    };
    let mut sink = |recs: Vec<Rec>, base: &Counters| write_records(st, recs, base);
    let end = match run_chunked(&job, &opts, bytes_per_step, &mut sink) {
        Ok(end) => end,
        Err(ChunkedRunError::Run(_, e)) => return die(e),
        Err(ChunkedRunError::Sink(e)) => return Err(e),
    };
    let (mut r, base) = (end.run, end.base);
    if let Some(path) = std::env::var_os("FDK_AAC_RS_CHUNK_STATS") {
        // One line per run (tests): input, AOT, extension AOT, chunks, handoffs, rejected,
        // fallbacks, the step from which on the decoding was sequential, steps.
        let s = &end.stats;
        let line = format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            st.input.as_deref().unwrap_or(""),
            r.h.stream_info.aot,
            r.h.stream_info.ext_aot,
            s.chunks,
            s.handoffs,
            s.rejected,
            s.fallbacks,
            s.sequential_from.map_or(-1, |p| p as i64),
            job.steps()
        );
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
    base.stats.add_to(&mut r.h.stream_info);
    st.n_dec = r.n_dec.wrapping_add(base.n_dec);
    // (the call numbers of the frame pipeline, which may continue)
    st.n_started = st.n_dec;
    st.n_fill = r.n_fill.wrapping_add(base.n_fill);
    st.samples_total = r.samples.wrapping_add(base.samples);
    st.errors_in_row = r.errors_in_row;
    st.stop = r.stop;
    st.zero_fills = r.zero_fills;
    st.out = std::mem::take(&mut r.out);
    st.h = Some(r.h);
    Ok(Resume { step: r.step, pos: r.pos })
}
