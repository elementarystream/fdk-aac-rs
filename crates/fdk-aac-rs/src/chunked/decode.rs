// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: chunked parallel decoding of a whole input with the safe API.

//! [`decode_chunked`]: the decoding of a whole input with the [`Decoder`] of the safe API, split
//! into chunks (see the [module documentation](super)).

use super::{run_chunked, ChunkJob, ChunkedOptions, ChunkedRunError, ChunkedStats, Step};
use crate::decoder::{DecodeFlags, Decoder, DecoderError, DecoderParam, StreamInfo, Transport};
use fdk_aac_rs_aacdec::digest::{HazardProbe, TRACE_DEPTH};
use fdk_aac_rs_fdk::digest::{Digest, Digester};

/// Bytes of input a speculative run of a byte stream decodes before its chunk: one and a half
/// times the input buffer of the transport decoder, whose content (in its logical order) is part
/// of the state.
pub const WARMUP_BYTES: usize = 48 << 10;
/// Access units a speculative run of a packet input decodes before its chunk (at least, and at
/// least [`WARMUP_BYTES`]).
pub const WARMUP_ACCESS_UNITS: usize = 32;
/// The decoding stops after this many decoder calls in a row that return an error other than
/// [`DecoderError::NotEnoughBits`] (as the command-line decoder).
pub const MAX_ERRORS_IN_ROW: usize = 1000;
/// The size of the output buffer of each decoder call, in samples.
const OUT_SIZE: usize = 65536;

/// How the decoders of [`decode_chunked`] are set up (the same for every run).
#[derive(Clone, Debug)]
pub struct DecoderSetup {
    /// The transport type ([`Decoder::open`]).
    pub transport: Transport,
    /// The number of layers ([`Decoder::open`], usually 1).
    pub layers: u32,
    /// A configuration for [`Decoder::config_raw`] (required for [`Transport::Raw`]).
    pub config: Option<Vec<u8>>,
    /// Parameters set with [`Decoder::set`] after the configuration (errors are ignored, as
    /// for a sequential decoder that sets them).
    pub params: Vec<DecoderParam>,
    /// The size of the ancillary data buffer ([`Decoder::set_ancillary_buffer`]); 0: none.
    pub ancillary_buffer: usize,
}

impl DecoderSetup {
    /// A setup with the transport type, one layer, no configuration and no parameters.
    pub fn new(transport: Transport) -> DecoderSetup {
        DecoderSetup { transport, layers: 1, config: None, params: Vec::new(), ancillary_buffer: 0 }
    }

    fn open(&self) -> Result<Decoder, DecoderError> {
        let mut dec = Decoder::open(self.transport, self.layers)?;
        if let Some(c) = &self.config {
            dec.config_raw(c)?;
        }
        for &p in &self.params {
            let _ = dec.set(p);
        }
        if self.ancillary_buffer > 0 {
            dec.set_ancillary_buffer(self.ancillary_buffer)?;
        }
        Ok(dec)
    }
}

/// The input of [`decode_chunked`].
#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    /// A byte stream (ADTS, ADIF, LATM, LOAS): the sequential decoding fills `fill_size` bytes
    /// at a time ([`Decoder::fill`]; the next fill starts after the bytes consumed) and decodes
    /// until [`DecoderError::NotEnoughBits`] after each fill.
    Stream {
        /// The whole stream.
        data: &'a [u8],
        /// The bytes per fill (at least 1).
        fill_size: usize,
    },
    /// Access units (for example of an MP4 track): the sequential decoding fills each access
    /// unit and decodes once after each fill (repeating both while the access unit is not
    /// consumed completely).
    Packets(&'a [&'a [u8]]),
}

/// An output of [`decode_chunked`]: the result of a decoder call that did not return
/// [`DecoderError::NotEnoughBits`].
#[derive(Debug)]
pub struct OutputFrame<'a> {
    /// The samples (interleaved; empty without valid output).
    pub samples: &'a [i16],
    /// The error: a decode error with valid (concealed) output, or an error without output.
    pub error: Option<DecoderError>,
    /// The stream information after the call ([`Decoder::stream_info`]).
    pub info: &'a StreamInfo,
    /// The ancillary data elements of the frame ([`Decoder::ancillary_elements`]).
    pub ancillary: &'a [Vec<u8>],
}

/// Errors of [`decode_chunked`] and [`decode_sequential`], returned after the frames before
/// them.
#[derive(Debug)]
pub enum ChunkedError<E> {
    /// A decoder could not be opened or configured ([`DecoderSetup`]), or a fill failed.
    Decoder(DecoderError),
    /// The sink returned an error.
    Sink(E),
}

impl<E: core::fmt::Display> core::fmt::Display for ChunkedError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ChunkedError::Decoder(e) => write!(f, "decoder: {e}"),
            ChunkedError::Sink(e) => write!(f, "{e}"),
        }
    }
}

impl<E: core::fmt::Debug + core::fmt::Display> std::error::Error for ChunkedError<E> {}

/// The statistics of the stream information that a run reports relative to its start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamCounters {
    pub total_bytes: i64,
    pub bad_bytes: i64,
    pub total_access_units: i64,
    pub bad_access_units: i64,
}

impl core::ops::Add for StreamCounters {
    type Output = StreamCounters;
    fn add(self, o: StreamCounters) -> StreamCounters {
        StreamCounters {
            total_bytes: self.total_bytes.wrapping_add(o.total_bytes),
            bad_bytes: self.bad_bytes.wrapping_add(o.bad_bytes),
            total_access_units: self.total_access_units.wrapping_add(o.total_access_units),
            bad_access_units: self.bad_access_units.wrapping_add(o.bad_access_units),
        }
    }
}

impl core::ops::Sub for StreamCounters {
    type Output = StreamCounters;
    fn sub(self, o: StreamCounters) -> StreamCounters {
        StreamCounters {
            total_bytes: self.total_bytes.wrapping_sub(o.total_bytes),
            bad_bytes: self.bad_bytes.wrapping_sub(o.bad_bytes),
            total_access_units: self.total_access_units.wrapping_sub(o.total_access_units),
            bad_access_units: self.bad_access_units.wrapping_sub(o.bad_access_units),
        }
    }
}

impl StreamCounters {
    /// The counters of a stream information.
    pub fn of(si: &fdk_aac_rs_aacdec::aacdecoder_lib::CStreamInfo) -> StreamCounters {
        StreamCounters {
            total_bytes: si.num_total_bytes,
            bad_bytes: si.num_bad_bytes,
            total_access_units: si.num_total_access_units,
            bad_access_units: si.num_bad_access_units,
        }
    }

    /// Adds the counters to a stream information.
    pub fn add_to(&self, si: &mut fdk_aac_rs_aacdec::aacdecoder_lib::CStreamInfo) {
        si.num_total_bytes = si.num_total_bytes.wrapping_add(self.total_bytes);
        si.num_bad_bytes = si.num_bad_bytes.wrapping_add(self.bad_bytes);
        si.num_total_access_units = si.num_total_access_units.wrapping_add(self.total_access_units);
        si.num_bad_access_units = si.num_bad_access_units.wrapping_add(self.bad_access_units);
    }
}

/// A frame stored by a run.
struct Frame {
    samples: Vec<i16>,
    error: Option<DecoderError>,
    info: StreamInfo,
    ancillary: Vec<Vec<u8>>,
}

/// A decoder of [`decode_chunked`] with its position in the input.
struct DecRun {
    dec: Decoder,
    /// The next step (fill of a byte stream, access unit of a packet input).
    step: usize,
    /// The position in a byte stream.
    pos: usize,
    errors_in_row: usize,
    zero_fills: u32,
    stopped: bool,
    pcm: Vec<i16>,
}

struct DecodeJob<'a> {
    setup: &'a DecoderSetup,
    input: Input<'a>,
}

impl DecodeJob<'_> {
    fn finished(&self, r: &DecRun) -> bool {
        r.stopped
            || match self.input {
                Input::Stream { data, .. } => r.pos >= data.len(),
                Input::Packets(p) => r.step >= p.len(),
            }
    }

    /// One decoder call (the frame is pushed unless NotEnoughBits); returns whether it was
    /// NotEnoughBits and whether it was a hazard.
    fn decode_once(&self, r: &mut DecRun, out: &mut Vec<Frame>) -> (bool, bool) {
        let before = r.dec.instance().map(|h| h.hazard_probe());
        let res = r.dec.decode_frame(&mut r.pcm, DecodeFlags::empty());
        let (frame, valid, neb) = match res {
            Ok(f) => (Some((f.samples().to_vec(), f.error())), true, false),
            Err(DecoderError::NotEnoughBits) => (None, false, true),
            Err(e) => (Some((Vec::new(), Some(e))), false, false),
        };
        let after = r.dec.instance().map(|h| h.hazard_probe());
        let code = match &frame {
            None => DecoderError::NotEnoughBits.code(),
            Some((_, e)) => e.map_or(0, |e| e.code()),
        };
        let hazard = match (before, after) {
            (Some(b), Some(a)) => HazardProbe::hazard(&b, &a, code),
            _ => false,
        };
        // (the stop accounting of the command-line decoder: an error is any result other than
        // AAC_DEC_OK and AAC_DEC_NOT_ENOUGH_BITS, also a decode error with output)
        match &frame {
            Some((_, Some(_))) => r.errors_in_row += 1,
            _ => r.errors_in_row = 0,
        }
        if let Some((samples, error)) = frame {
            let ancillary = if self.setup.ancillary_buffer > 0 && valid {
                r.dec.ancillary_elements().map(|a| a.to_vec()).collect()
            } else {
                Vec::new()
            };
            out.push(Frame { samples, error, info: r.dec.stream_info(), ancillary });
        }
        if r.errors_in_row >= MAX_ERRORS_IN_ROW {
            r.stopped = true;
        }
        (neb, hazard)
    }

    fn fill(&self, r: &mut DecRun, data: &[u8]) -> Result<usize, DecoderError> {
        let consumed = r.dec.fill(data)?;
        if !data.is_empty() && consumed == 0 {
            r.zero_fills += 1;
            if r.zero_fills >= 2 {
                r.stopped = true;
            }
        } else {
            r.zero_fills = 0;
        }
        Ok(consumed)
    }
}

impl ChunkJob for DecodeJob<'_> {
    type Run = DecRun;
    type Out = Vec<Frame>;
    type Counters = StreamCounters;
    type Error = DecoderError;

    fn steps(&self) -> usize {
        match self.input {
            Input::Stream { data, fill_size } => data.len().div_ceil(fill_size.max(1)),
            Input::Packets(p) => p.len(),
        }
    }

    fn warmup_start(&self, step: usize) -> Option<usize> {
        match self.input {
            Input::Stream { fill_size, .. } => {
                step.checked_sub(WARMUP_BYTES.div_ceil(fill_size.max(1)))
            }
            Input::Packets(p) => {
                let mut bytes = 0;
                let mut w = step;
                while w > 0 && (step - w < WARMUP_ACCESS_UNITS || bytes < WARMUP_BYTES) {
                    w -= 1;
                    bytes += p[w].len();
                }
                (step - w >= WARMUP_ACCESS_UNITS && bytes >= WARMUP_BYTES).then_some(w)
            }
        }
    }

    fn start(&self, step: usize) -> Result<DecRun, DecoderError> {
        let dec = self.setup.open()?;
        let pos = match self.input {
            Input::Stream { fill_size, .. } => step * fill_size.max(1),
            Input::Packets(_) => 0,
        };
        Ok(DecRun {
            dec,
            step,
            pos,
            errors_in_row: 0,
            zero_fills: 0,
            stopped: false,
            pcm: vec![0; OUT_SIZE],
        })
    }

    fn position(&self, r: &DecRun) -> usize {
        r.step
    }

    fn step(&self, r: &mut DecRun) -> Result<Step<Vec<Frame>>, DecoderError> {
        let mut out = Vec::new();
        let mut hazard = false;
        match self.input {
            Input::Stream { data, fill_size } => {
                if r.pos < data.len() && !r.stopped {
                    let chunk = (data.len() - r.pos).min(fill_size.max(1));
                    let consumed = self.fill(r, &data[r.pos..r.pos + chunk])?;
                    r.pos += consumed;
                    if !r.stopped {
                        loop {
                            let (neb, hz) = self.decode_once(r, &mut out);
                            hazard |= hz;
                            if neb || r.stopped {
                                break;
                            }
                        }
                    }
                }
            }
            Input::Packets(p) => {
                if let Some(au) = p.get(r.step).copied() {
                    let mut q = 0;
                    loop {
                        let consumed = self.fill(r, &au[q..])?;
                        q += consumed;
                        if r.stopped {
                            break;
                        }
                        let (_, hz) = self.decode_once(r, &mut out);
                        hazard |= hz;
                        if r.stopped || q >= au.len() {
                            break;
                        }
                    }
                }
            }
        }
        r.step += 1;
        Ok(Step { out, hazard, finished: self.finished(r) })
    }

    fn digest(&self, r: &DecRun) -> Digest {
        let mut d = Digester::new();
        self.feed(r, &mut d);
        d.finish()
    }

    fn trace(&self, r: &DecRun) -> Vec<(String, Digest)> {
        let mut d = Digester::traced(TRACE_DEPTH);
        self.feed(r, &mut d);
        d.finish_traced().1
    }

    fn counters(&self, r: &DecRun) -> StreamCounters {
        r.dec
            .instance()
            .map_or_else(StreamCounters::default, |h| StreamCounters::of(&h.stream_info))
    }

    fn out_bytes(out: &Vec<Frame>) -> usize {
        out.iter().map(|f| f.samples.len() * 2 + 256).sum()
    }

    fn poison(&self, r: &mut DecRun, seed: u64) {
        if let Some(h) = r.dec.instance_mut() {
            h.poison_unfed_state(seed);
        }
    }

    fn converges_rarely(&self, r: &DecRun) -> bool {
        r.dec.instance().is_some_and(|h| h.converges_rarely())
    }
}

impl DecodeJob<'_> {
    /// The state of a run: the decoder, the position in the input, the stop conditions and the
    /// output buffer (only the returned samples of it are output, which the decoder writes; fed
    /// anyway).
    fn feed(&self, r: &DecRun, d: &mut Digester) {
        let DecRun { dec, step: _, pos, errors_in_row, zero_fills, stopped, pcm } = r;
        match dec.instance() {
            Some(h) => d.field("decoder", h),
            None => d.field("decoder", &false),
        }
        d.field("pos", pos);
        d.field("errors_in_row", errors_in_row);
        d.field("zero_fills", zero_fills);
        d.field("stopped", stopped);
        d.field("pcm", pcm);
    }
}

fn emit<E>(
    frames: Vec<Frame>,
    base: &StreamCounters,
    sink: &mut dyn FnMut(&OutputFrame) -> Result<(), E>,
) -> Result<(), E> {
    for mut f in frames {
        base.add_to(f.info.raw_mut());
        sink(&OutputFrame {
            samples: &f.samples,
            error: f.error,
            info: &f.info,
            ancillary: &f.ancillary,
        })?;
    }
    Ok(())
}

/// Decodes the whole `input` with decoders set up by `setup`, with up to `opts.threads` threads,
/// and passes the result of every decoder call other than [`DecoderError::NotEnoughBits`] to
/// `sink` in order: exactly the results of [`decode_sequential`] (the samples, the errors, the
/// stream information including its statistics, and the ancillary data).
///
/// The input is decoded in chunks when [`ChunkedOptions::num_chunks`] gives more than one (by
/// default from 4 threads and long inputs, see the [module documentation](super)); where the
/// decoder states do not become equal (SBR, PS, PNS, ...), the rest of the input is decoded
/// sequentially with one thread. Stops after [`MAX_ERRORS_IN_ROW`] errors in a row.
///
#[cfg_attr(feature = "encoder", doc = "```")]
#[cfg_attr(not(feature = "encoder"), doc = "```ignore")]
/// use fdk_aac_rs::chunked::{decode_chunked, decode_sequential, ChunkedOptions, DecoderSetup, Input};
/// use fdk_aac_rs::decoder::Transport;
/// # use fdk_aac_rs::encoder::{ChannelMode, Encoder};
/// # let mut enc = Encoder::builder().sample_rate(44_100).channel_mode(ChannelMode::Stereo)
/// #     .transport(fdk_aac_rs::encoder::Transport::Adts).build()?;
/// # let mut adts = Vec::new();
/// # let mut out = vec![0u8; 8192];
/// # for _ in 0..8 {
/// #     let r = enc.encode(&vec![0i16; 2 * 1024], &mut out)?;
/// #     adts.extend_from_slice(&out[..r.out_bytes()]);
/// # }
/// // `adts`: an AAC-LC ADTS stream.
/// let setup = DecoderSetup::new(Transport::Adts);
/// let input = Input::Stream { data: &adts, fill_size: 4096 };
/// let opts = ChunkedOptions { threads: 8, ..Default::default() };
/// let mut pcm = Vec::new();
/// decode_chunked(&setup, input, &opts, &mut |f| {
///     pcm.extend_from_slice(f.samples);
///     Ok::<(), std::convert::Infallible>(())
/// })?;
/// let mut expected = Vec::new();
/// decode_sequential(&setup, input, &mut |f| {
///     expected.extend_from_slice(f.samples);
///     Ok::<(), std::convert::Infallible>(())
/// })?;
/// assert_eq!(pcm, expected);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn decode_chunked<E>(
    setup: &DecoderSetup,
    input: Input,
    opts: &ChunkedOptions,
    sink: &mut dyn FnMut(&OutputFrame) -> Result<(), E>,
) -> Result<ChunkedStats, ChunkedError<E>> {
    let job = DecodeJob { setup, input };
    let bytes_per_step = match input {
        Input::Stream { fill_size, .. } => fill_size.max(1) * 16,
        Input::Packets(_) => 8192,
    };
    let mut sink_frames = |o: Vec<Frame>, base: &StreamCounters| emit(o, base, sink);
    let end = match run_chunked(&job, opts, bytes_per_step, &mut sink_frames) {
        Ok(end) => end,
        Err(ChunkedRunError::Run(_, e)) => return Err(ChunkedError::Decoder(e)),
        Err(ChunkedRunError::Sink(e)) => return Err(ChunkedError::Sink(e)),
    };
    let (mut run, base, mut stats) = (end.run, end.base, end.stats);
    if !end.finished {
        /* the rest sequentially */
        while !job.finished(&run) {
            let s = job.step(&mut run).map_err(ChunkedError::Decoder)?;
            stats.steps += 1;
            emit(s.out, &base, sink).map_err(ChunkedError::Sink)?;
            if s.finished {
                break;
            }
        }
    }
    Ok(stats)
}

/// The sequential decoding of [`decode_chunked`] with one decoder and one thread (the reference
/// of its output): see [`Input`] for the calls.
pub fn decode_sequential<E>(
    setup: &DecoderSetup,
    input: Input,
    sink: &mut dyn FnMut(&OutputFrame) -> Result<(), E>,
) -> Result<(), ChunkedError<E>> {
    let job = DecodeJob { setup, input };
    let mut run = job.start(0).map_err(ChunkedError::Decoder)?;
    while !job.finished(&run) {
        let s = job.step(&mut run).map_err(ChunkedError::Decoder)?;
        emit(s.out, &StreamCounters::default(), sink).map_err(ChunkedError::Sink)?;
        if s.finished {
            break;
        }
    }
    Ok(())
}
