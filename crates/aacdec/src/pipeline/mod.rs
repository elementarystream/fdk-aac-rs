// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: the frame pipeline of the decoder (docs/optimization.md, "Decoder frame
// pipelining"). Stage A and stage B are rearranged copies of aacDecoder_DecodeFrame() and
// CAacDecoder_DecodeFrame() (libAACdec/src/aacdecoder_lib.cpp, aacdecoder.cpp); the sequential
// decoder (one thread, the default) does not use this module.

//! Frame pipelining (opt-in, more than one thread): the end of the decoding of access unit N
//! runs on a second thread while access unit N + 1 is read and parsed.
//!
//! # Stages
//!
//! `aacDecoder_DecodeFrame()` is split into two stages:
//!
//! - **Stage A** (`stage_a`): the transport decoder (with its configuration callbacks), the
//!   element loop of `CAacDecoder_DecodeFrame()` (`CChannelElement_Read()` without the
//!   inverse quantization, extension payloads, data stream elements, the program config
//!   element), the checks after it (element instance tags, byte alignment, CRC, the channel
//!   counters), the channel mapping and `transportDec_EndAccessUnit()`.
//! - **Stage B** (`stage_b`): the inverse quantization of the spectral data (when it cannot
//!   fail), `CChannelElement_Decode()` (stereo tools, TNS, PNS, scaling),
//!   the DRC prolog and epilog, concealment, the inverse transform and the time domain fading,
//!   and everything `aacDecoder_DecodeFrame()` does after the core decoder (QMF domain, SBR, PS,
//!   MPEG Surround, uniDRC, PCM post processing, limiter, output conversion, statistics).
//!
//! The order of all operations within a stage is the order of the sequential decoder; stage B
//! of a frame runs after stage A of the same frame. The two stages touch disjoint state, so
//! stage A of frame N + 1 may run at the same time as stage B of frame N (see the module
//! `state` for the ownership of every member of the decoder instance and why the result is
//! that of the sequential order).
//!
//! A frame is handed over to the worker thread only if it has no errors and stage A did not
//! need state of stage B; every other frame runs stage B in the calling thread, and if its
//! stage A ran while stage B of the previous frame was running, the sequential decoder
//! decodes it again from the state after the transport decoder's part (`state::restart`).
//! After such a restart, the sequential decoder decodes the next calls (`BACKOFF_MIN`).
//!
//! # Delayed output
//!
//! With more than one thread, every call of `aacDecoder_DecodeFrame()` that decodes a frame
//! returns the result of the previous such call (see [`decode_frame`] for the exact contract).
//!
//! # Threads
//!
//! One worker thread per decoder runs stage B. The thread hand-off spins briefly before it
//! parks (`worker`). With more than two threads, stage B on the worker thread decodes the
//! channels of a frame on up to `threads - 1` threads; a frame decoded in the calling thread
//! (stage B or the sequential decoder) on up to `threads` threads ([`chan`]).
//!
//! # Thread modes
//!
//! Whether a frame may be handed over and how many threads the per channel parallelism uses is
//! the *mode* of the call (`MODES_2`, `MODES_3`), chosen by measurement
//! ([`fdk_aac_rs_fdk::par::tune`]): the pipeline and the per channel parallelism cost CPU time
//! in waiting threads, and for some streams and machines they do not make the decoding faster.
//! Every mode decodes exactly as the sequential decoder; a mode without the pipeline runs the
//! call as after a restart (the sequential decoder, which keeps the delayed output).

pub mod chan;
mod stage_a;
mod stage_b;
pub(crate) mod state;
mod worker;

use crate::aac_rom::PcmDec;
use crate::aacdecoder::{AacDecoderInstance, AncBuffer, MAX_ELEMENTS};
use crate::aacdecoder_lib::{
    is_output_valid, CStreamInfo, AACDEC_CONCEAL, AACDEC_FLUSH, AAC_DEC_INVALID_HANDLE,
    AAC_DEC_NOT_ENOUGH_BITS, AAC_DEC_OK, AAC_DEC_SET_PARAM_FAIL,
};
use fdk_aac_rs_fdk::audio::Mp4ElementId;
use fdk_aac_rs_fdk::common_fix::FixpDbl;
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::par::{Tuner, Tuning};
use std::collections::VecDeque;
use std::time::Instant;

use stage_a::{stage_a, AOutcome};
use stage_b::stage_b;
use worker::Worker;

/// `AAC_DECODER_ERROR`
type AacDecoderError = i32;

/// The largest number of output samples of one frame (8 channels of 4096 samples: AAC with
/// 4:1 SBR); the internal output buffers of the pipeline hold this many samples at most.
pub(crate) const MAX_FRAME_SAMPLES: usize = 8 * 4096;

/// The calls decoded by the sequential decoder after a restart ([`Pipeline::backoff`]): a stream
/// whose frames need state of stage B in stage A (the explicit SBR payload of ELD, data stream
/// elements, DRC payloads) or have errors is decoded mostly by the sequential decoder.
pub(crate) const BACKOFF_MIN: u32 = 8;
/// The largest [`Pipeline::backoff`].
pub(crate) const BACKOFF_MAX: u32 = 512;

/// A thread mode of a call: whether stage B may run on the worker thread, the thread count of
/// the per channel parallelism of a frame decoded in the calling thread (`0`: the decoder's
/// thread count) and of stage B on the worker thread (`0`: one less), and whether stage A does
/// the inverse quantization (else stage B: the balance of the two stages).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mode {
    pub pipeline: bool,
    pub inline_cap: u32,
    pub worker_cap: u32,
    pub iq_in_a: bool,
}

const fn mode(pipeline: bool, inline_cap: u32, worker_cap: u32, iq_in_a: bool) -> Mode {
    Mode { pipeline, inline_cap, worker_cap, iq_in_a }
}

/// The thread modes with two threads, cheapest first: sequential; sequential with the channels
/// on two threads; the pipeline (stage B on the worker thread, a frame decoded in the calling
/// thread with the channels on two threads) with the inverse quantization in stage A, and in
/// stage B.
pub(crate) const MODES_2: [Mode; 4] = [
    mode(false, 1, 1, false),
    mode(false, 0, 1, false),
    mode(true, 0, 1, true),
    mode(true, 0, 1, false),
];

/// The thread modes with more than two threads, cheapest first: sequential; the pipeline with
/// one thread for stage B (and two for a frame decoded in the calling thread) and the inverse
/// quantization in stage A; sequential with the channels on all threads; the pipeline with the
/// channels of stage B on all threads but one (all for a frame decoded in the calling thread),
/// with the inverse quantization in stage A, and in stage B.
pub(crate) const MODES_3: [Mode; 5] = [
    mode(false, 1, 1, false),
    mode(true, 2, 1, true),
    mode(false, 0, 0, false),
    mode(true, 0, 0, true),
    mode(true, 0, 0, false),
];

/// The thread modes of a decoder with `threads` threads (at least 2).
pub(crate) fn modes(threads: u32) -> &'static [Mode] {
    if threads <= 2 {
        &MODES_2
    } else {
        &MODES_3
    }
}

/// The default [`Tuning`] of a decoder: [`fdk_aac_rs_fdk::par::tune::TUNE_ENV`], else
/// adaptive (in poisoning builds the cycle through all modes, so that the poisoning covers
/// every mode and every switch).
fn default_tuning() -> Tuning {
    let fallback = if cfg!(feature = "poison") { Tuning::Cycle } else { Tuning::Adaptive };
    Tuning::from_env(fallback)
}

/// The number of bytes beyond an SBR payload that its copy holds (see [`SbrParse`]).
///
/// `sbrDecoder_Parse()` of a fill element reads at most: the CRC (10 bits), the header (about
/// 50 bits), per channel the grid (at most 5 envelopes: 30 bits), the time/frequency direction
/// flags, the inverse filtering modes (5 x 2 bits), the envelope data (at most 5 envelopes of
/// 48 bands of Huffman codewords of at most 20 bits: 4800 bits), the noise floor data (2 x 5
/// codewords) and the sinusoidal flags (48 bits), and the extended data (at most 270 bytes,
/// whose parametric stereo parser reads at most its own data plus 34 x 4 x 2 codewords of at
/// most 20 bits beyond a damaged length). This is less than 3 KB beyond the payload start in
/// every case; the copy holds 8 KB more than the payload, and stage B checks that the parser
/// stayed within the copy.
pub(crate) const SBR_COPY_MARGIN: usize = 8192;

/// The number of deferred operations that read more than they were given (must stay 0; see
/// [`SBR_COPY_MARGIN`]).
static MISMATCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Counts a pipeline mismatch (a deferred SBR parse that read beyond its copy).
pub(crate) fn note_mismatch() {
    MISMATCHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// The number of pipeline mismatches so far in this process (see `SBR_COPY_MARGIN`).
pub fn mismatches() -> u64 {
    MISMATCHES.load(std::sync::atomic::Ordering::Relaxed)
}

/// An operation of stage A on state owned by stage B, executed by stage B of the same frame
/// (or at a join of the pipeline), in the order of stage A, while stage B owns the state.
pub(crate) enum BOp {
    /// `CAacDecoder_SyncQmfMode(self)` with the decoder flags and `aacNumChannels` of the call.
    SyncQmfMode { flags: u32, aac_num_channels: i32 },
    /// `sbrDecoder_Parse()` of an SBR fill element.
    Parse(Box<SbrParse>),
    /// The DRC and downmix settings of `aacDecoder_setMetadataExpiry()` (a configuration
    /// callback) with this number of frames.
    MetadataExpiry(i32),
}

/// A write of stage A to state that stage B writes (the stream information of the output,
/// `flags`, `mpsEnableCurr`), done while stage B of the previous frame runs: it is done on the
/// main instance (stage A reads its own writes) and repeated after the join has taken over
/// stage B's state (it comes after stage B in the sequential order).
#[derive(Clone, Copy)]
pub(crate) enum BWrite {
    /// `streamInfo.bitRate`
    BitRate(i32),
    /// `streamInfo.flags`
    SiFlags(u32),
    /// the bits [`state::B_FLAGS`] of `flags[0]`
    FlagsB(u32),
    /// `mpsEnableCurr`
    MpsEnableCurr(u8),
    /// `aacDecoder_UpdateBitStreamCounters()` with these consumed bits and this return value
    /// (a call that found not enough bits), and the consistency check after it.
    Counters(i32, AacDecoderError),
}

/// The arguments of a deferred `sbrDecoder_Parse()` call of an SBR fill element payload
/// (`EXT_SBR_DATA`, `EXT_SBR_DATA_CRC`): the bits of the transport decoder's bit buffer from
/// the payload on (the payload and [`SBR_COPY_MARGIN`] bytes beyond it, which covers every bit
/// the parser can read), and the number of valid bits the transport decoder had (the parser
/// compares it with the length of its extension data).
pub(crate) struct SbrParse {
    /// The copied bytes; the payload starts at bit `bit_offset` of `data[0]`. The length is a
    /// power of two (a bit buffer requirement).
    pub data: Vec<u8>,
    /// The number of copied bytes.
    pub data_len: usize,
    /// The bit position of the payload in `data[0]`.
    pub bit_offset: u32,
    /// `FDKgetValidBits()` of the transport bitstream at the payload.
    pub valid_bits: u32,
    /// `count`, `bsPayLen`, `crcFlag`, `prevElement`, `elementIndex`, `self->flags[0]`
    pub count: i32,
    pub bs_pay_len: i32,
    pub crc_flag: i32,
    pub prev_element: Mp4ElementId,
    pub element_index: i32,
    pub ac_flags: u32,
}

/// What stage A hands over to stage B of the same frame (the locals of
/// `aacDecoder_DecodeFrame()` and `CAacDecoder_DecodeFrame()` that stage B reads).
pub(crate) struct HandOver {
    /// The `flags` of `CAacDecoder_DecodeFrame()` (the call's flags with `AACDEC_CONCEAL` for a
    /// transport CRC error).
    pub flags: u32,
    /// The `flags` of the call.
    pub call_flags: u32,
    /// `timeDataSize` of the call.
    pub time_data_size: i32,
    /// The bits the transport decoder consumed in the call (`nBits - FDKgetValidBits()` of
    /// `aacDecoder_UpdateBitStreamCounters()`).
    pub n_bits_consumed: i32,
    /// The error status of `CAacDecoder_DecodeFrame()` at the end of stage A.
    pub core_err: AacDecoderError,
    /// `self->frameOK` at the element decode loop (`CChannelElement_Decode()`).
    pub decode_frame_ok: bool,
    /// The elements decoded by the element decode loop: (element index, first channel,
    /// number of channels).
    pub decode_list: [(u8, u8, u8); MAX_ELEMENTS],
    /// The number of entries of `decode_list`.
    pub decode_count: usize,
    /// `element_count` after the element decode loop.
    pub element_count: i32,
    /// `channel_elements[]` after the element decode loop.
    pub channel_elements: [Mp4ElementId; MAX_ELEMENTS],
    /// `aacChannels` (local of `CAacDecoder_DecodeFrame()`) at the inverse transform.
    pub aac_channels: i32,
    /// `drcChMap[]`
    pub drc_ch_map: [u8; 8],
    /// `reverseChMapping[]`
    pub reverse_ch_mapping: [u8; 16],
    /// The SBR operations of stage A (executed first by stage B).
    pub b_ops: Vec<BOp>,
    /// The elements whose inverse quantization stage A left to stage B: (first channel,
    /// number of channels), in the order of the element loop.
    pub iq_list: [(u8, u8); MAX_ELEMENTS],
    /// The number of entries of `iq_list`.
    pub iq_count: usize,
}

impl HandOver {
    pub(crate) fn new() -> HandOver {
        HandOver {
            flags: 0,
            call_flags: 0,
            time_data_size: 0,
            n_bits_consumed: 0,
            core_err: AAC_DEC_OK,
            decode_frame_ok: false,
            decode_list: [(0, 0, 0); MAX_ELEMENTS],
            decode_count: 0,
            element_count: 0,
            channel_elements: [0; MAX_ELEMENTS],
            aac_channels: 0,
            drc_ch_map: [0; 8],
            reverse_ch_mapping: [0; 16],
            b_ops: Vec::new(),
            iq_list: [(0, 0); MAX_ELEMENTS],
            iq_count: 0,
        }
    }
}

/// The ancillary data of a frame (`CAncData` without the buffer, and the bytes it used).
#[derive(Clone, Default)]
pub(crate) struct AncSnapshot {
    offset: [i32; 8],
    nr_elements: i32,
    bytes: Vec<u8>,
}

/// The result of one call of the sequential decoder.
pub(crate) struct FrameResult {
    /// The return value.
    pub err: AacDecoderError,
    /// The output samples (valid output; at most [`MAX_FRAME_SAMPLES`]).
    pub pcm: Vec<IntPcm>,
    /// The number of valid samples in `pcm`.
    pub n: usize,
    /// The stream information after the call.
    pub info: CStreamInfo,
    /// The ancillary data after the call (`None` without an ancillary data buffer).
    pub anc: Option<AncSnapshot>,
}

/// A result of the pipeline that is not returned yet.
#[allow(clippy::large_enum_variant)] // one per decoder; boxing would allocate per frame
pub(crate) enum Pending {
    /// Stage B runs on the worker thread.
    InFlight,
    /// Complete.
    Ready(FrameResult),
}

/// The state of the frame pipeline of a decoder ([`AacDecoderInstance::pipe`]).
pub struct Pipeline {
    /// The thread count of the decoder (at least 2).
    pub(crate) threads: u32,
    /// The worker thread (started with the first deferred stage B).
    pub(crate) worker: Option<Worker>,
    /// The instance that runs stage B on the worker; `None` while it is there.
    pub(crate) shadow: Option<Box<AacDecoderInstance>>,
    /// The `d_gen` of the second set of per frame channel memory in `shadow` (`None`: no valid
    /// set).
    pub(crate) spare_gen: Option<u32>,
    /// The frames not returned yet, oldest first (at most two, inside a call).
    pub(crate) queue: VecDeque<Pending>,
    /// The ancillary data of the frame in flight.
    pub(crate) anc_in_flight: Option<AncSnapshot>,
    /// The stream information returned by `aacDecoder_GetStreamInfo()` (`None`: the
    /// decoder's).
    pub(crate) presented: Option<CStreamInfo>,
    /// The last call returned `AAC_DEC_NOT_ENOUGH_BITS` (for the drain rule).
    pub(crate) last_neb: bool,
    /// `aacDecoder_Fill()` calls (for the drain rule).
    pub(crate) fills: u64,
    /// `fills` at the last `AAC_DEC_NOT_ENOUGH_BITS` return.
    pub(crate) fills_at_neb: u64,
    /// Stage A of the current frame joined the pipeline in a configuration callback (stage B
    /// of the frame runs at once).
    pub(crate) cb_join: bool,
    /// Stage A of the current frame read a program config element.
    pub(crate) pce_in_frame: bool,
    /// The SBR operations of the current stage A that wait for the SBR decoder.
    pub(crate) b_ops: Vec<BOp>,
    /// Writes of the current stage A to state written by stage B ([`BWrite`]).
    pub(crate) b_writes: Vec<BWrite>,
    /// The state of the SBR decoder that stage A needs to know without it.
    pub(crate) sbr_cache: stage_a::SbrCache,
    /// The entries of `streamInfo.pChannelType` / `pChannelIndices` that the current stage A
    /// wrote while stage B of the previous frame was running (bit `i`: entry `i`).
    pub(crate) t_mask: u8,
    /// Stage A stored `channelTypePrev` / `channelIndicesPrev` while stage B of the previous
    /// frame was running; `t_prev_mask` is `t_mask` at that moment.
    pub(crate) t_prev_stored: bool,
    pub(crate) t_prev_mask: u8,
    /// Spare output buffers.
    pcm_pool: Vec<Vec<IntPcm>>,
    /// Spare SBR payload copies.
    pub(crate) sbr_pool: Vec<Vec<u8>>,
    /// Buffer poisoning (test builds, `FDK_AAC_RS_POISON`).
    pub(crate) poison: u32,
    /// Stage A of the current frame writes the per frame channel memory the main instance got
    /// at the last hand-over, while stage B of the previous frame runs ([`state::take_snapshot`]).
    pub(crate) touched: bool,
    /// Stage A of the current frame needs stage B of the previous frame after it began to
    /// write that memory: the frame is decoded again ([`state::restart`]).
    pub(crate) restart: bool,
    /// The state after the transport decoder's part of the current call.
    pub(crate) snap: Option<Box<state::Snapshot>>,
    /// Frames decoded again by the sequential decoder ([`state::restart`]).
    pub(crate) restarts: u64,
    /// Restarts since the last frame whose stage A ran while stage B of the previous frame
    /// was running and that was handed over.
    pub(crate) fail_streak: u32,
    /// Calls the sequential decoder decodes before the pipeline is tried again (after a
    /// restart: [`BACKOFF_MIN`] frames, doubled with every further restart in a row up to
    /// [`BACKOFF_MAX`]).
    pub(crate) backoff: u32,
    /// Frames whose stage B ran on the worker thread.
    pub(crate) frames_deferred: u64,
    /// Frames (results other than `AAC_DEC_NOT_ENOUGH_BITS`) whose stage B or whole call ran in
    /// the calling thread.
    pub(crate) frames_inline: u64,
    /// The choice of the thread mode ([`modes`]).
    pub(crate) tuner: Tuner,
    /// The thread count of the per channel parallelism of a frame decoded in the calling
    /// thread, and of stage B on the worker thread (of the current call's mode).
    pub(crate) inline_cap: u32,
    pub(crate) worker_cap: u32,
    /// Stage A does the inverse quantization it can leave to stage B (of the current call's
    /// mode; see `stage_a::read_channel_element_a`).
    pub(crate) iq_in_a: bool,
    /// Frames per mode (for tests).
    pub(crate) mode_frames: [u64; fdk_aac_rs_fdk::par::tune::MAX_MODES],
}

impl Pipeline {
    fn new(threads: u32) -> Box<Pipeline> {
        let mut tuner = Tuner::new(default_tuning());
        tuner.set_modes(modes(threads).len());
        Box::new(Pipeline {
            threads,
            worker: None,
            shadow: None,
            spare_gen: None,
            queue: VecDeque::new(),
            anc_in_flight: None,
            presented: None,
            last_neb: false,
            fills: 0,
            fills_at_neb: 0,
            cb_join: false,
            pce_in_frame: false,
            b_ops: Vec::new(),
            b_writes: Vec::new(),
            sbr_cache: stage_a::SbrCache::default(),
            t_mask: 0,
            t_prev_stored: false,
            t_prev_mask: 0,
            pcm_pool: Vec::new(),
            sbr_pool: Vec::new(),
            poison: poison_mode(),
            touched: false,
            restart: false,
            snap: None,
            restarts: 0,
            fail_streak: 0,
            backoff: 0,
            frames_deferred: 0,
            frames_inline: 0,
            tuner,
            inline_cap: threads,
            worker_cap: threads - 1,
            iq_in_a: false,
            mode_frames: Default::default(),
        })
    }

    /// Sets the caps of the mode of the next call; returns whether it allows the pipeline.
    fn apply_mode(&mut self) -> bool {
        let ms = modes(self.threads);
        self.tuner.set_modes(ms.len());
        let i = self.tuner.mode().min(ms.len() - 1);
        let m = ms[i];
        self.inline_cap = if m.inline_cap == 0 { self.threads } else { m.inline_cap };
        self.worker_cap = if m.worker_cap == 0 { self.threads - 1 } else { m.worker_cap };
        self.iq_in_a = m.iq_in_a;
        m.pipeline
    }

    /// Stage B of a frame runs on the worker thread.
    pub(crate) fn away(&self) -> bool {
        matches!(self.queue.back(), Some(Pending::InFlight))
    }

    fn take_pcm(&mut self) -> Vec<IntPcm> {
        self.pcm_pool.pop().unwrap_or_default()
    }

    fn give_pcm(&mut self, v: Vec<IntPcm>) {
        if self.pcm_pool.len() < 4 {
            self.pcm_pool.push(v);
        }
    }
}

/// The thread count of the per channel parallelism of a frame decoded in the calling thread
/// (1 for the sequential decoder; see [`chan`]).
pub(crate) fn inline_cap(self_: &AacDecoderInstance) -> usize {
    self_.pipe.as_deref().map_or(1, |p| p.inline_cap as usize)
}

/// The poisoning mode of a test build (`FDK_AAC_RS_POISON`, see [`state::poison_stage_a`] and
/// [`chan`]).
fn poison_mode() -> u32 {
    #[cfg(feature = "poison")]
    {
        std::env::var("FDK_AAC_RS_POISON").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(0)
    }
    #[cfg(not(feature = "poison"))]
    {
        0
    }
}

/// Sets the number of threads of the decoder (1: the sequential decoder; more: frame
/// pipelining). Changing it to 1 is only possible while no frame is pending
/// (`AAC_DEC_SET_PARAM_FAIL` otherwise; drain the decoder first).
pub fn set_threads(self_: &mut AacDecoderInstance, threads: u32) -> AacDecoderError {
    let threads = threads.max(1);
    match self_.pipe.as_deref_mut() {
        None => {
            if threads > 1 {
                self_.pipe = Some(Pipeline::new(threads));
            }
            AAC_DEC_OK
        }
        Some(p) => {
            if threads > 1 {
                p.threads = threads;
                return AAC_DEC_OK;
            }
            join(self_);
            if pending_frames(self_) > 0 {
                return AAC_DEC_SET_PARAM_FAIL;
            }
            close(self_);
            AAC_DEC_OK
        }
    }
}

/// The thread count of the decoder.
pub fn threads(self_: &AacDecoderInstance) -> u32 {
    self_.pipe.as_deref().map_or(1, |p| p.threads)
}

/// Sets how a decoder with more than one thread chooses its thread mode (tests; the default is
/// [`fdk_aac_rs_fdk::par::tune::TUNE_ENV`] or adaptive), until the decoder goes back to one
/// thread. Without effect on a decoder with one thread.
#[doc(hidden)]
pub fn set_tuning(self_: &mut AacDecoderInstance, tuning: Tuning) {
    if let Some(p) = self_.pipe.as_deref_mut() {
        p.tuner.set_tuning(tuning);
    }
}

/// The frames a decoder decoded in each thread mode (tests; see [`modes`]).
#[doc(hidden)]
pub fn mode_frames(self_: &AacDecoderInstance) -> [u64; fdk_aac_rs_fdk::par::tune::MAX_MODES] {
    self_.pipe.as_deref().map_or([0; fdk_aac_rs_fdk::par::tune::MAX_MODES], |p| p.mode_frames)
}

/// The number of frames decoded but not returned yet (0 or 1 between calls).
pub fn pending_frames(self_: &AacDecoderInstance) -> u32 {
    self_.pipe.as_deref().map_or(0, |p| p.queue.len() as u32)
}

/// Statistics of the pipeline: (frames whose stage B ran on the worker thread, frames whose
/// stage B or whole call ran in the calling thread, frames among the latter whose stage A was
/// redone by the sequential decoder).
pub fn stats(self_: &AacDecoderInstance) -> (u64, u64, u64) {
    self_.pipe.as_deref().map_or((0, 0, 0), |p| (p.frames_deferred, p.frames_inline, p.restarts))
}

/// Statistics of the per channel parallelism ([`chan`]), counted over all decoders of the
/// process (for tests): (frames whose element decoding ran on several threads, frames whose
/// inverse transforms ran on several threads, stereo SBR elements whose channels were decoded
/// on two threads).
pub fn par_stats() -> (u64, u64, u64) {
    (
        chan::par_element_frames(),
        chan::par_render_frames(),
        fdk_aac_rs_sbrdec::sbrdecoder::par_elements(),
    )
}

/// Waits for a running stage B and takes its state back (a join of the pipeline). The SBR
/// operations stage A recorded meanwhile stay recorded ([`stage_a::join_mid_a`] executes them).
///
/// The main instance gets the per frame channel memory of the joined frame back (the state of
/// the sequential decoder), unless the current stage A has written its own set
/// ([`Pipeline::touched`]: the frame is handed over next).
pub(crate) fn join(self_: &mut AacDecoderInstance) {
    let Some(p) = self_.pipe.as_deref_mut() else { return };
    if !p.away() {
        return;
    }
    let done = p.worker.as_mut().expect("worker of a running stage B").wait();
    let worker::Done { mut shadow, ho, out, n, err } = *done;
    drop(ho);
    let info = shadow.stream_info;
    state::return_from_stage_b(self_, &mut shadow);
    let p = self_.pipe.as_deref_mut().expect("pipeline");
    if !core::mem::take(&mut p.touched) {
        state::swap_channel_memory(self_, &mut shadow);
    }
    let p = self_.pipe.as_deref_mut().expect("pipeline");
    p.shadow = Some(shadow);
    let anc = p.anc_in_flight.take();
    let res = FrameResult { err, pcm: out, n, info, anc };
    if let Some(q) = p.queue.back_mut() {
        *q = Pending::Ready(res);
    }
}

/// Ends the pipeline (the decoder becomes sequential): waits for a running stage B; frames
/// not returned yet are dropped.
pub(crate) fn close(self_: &mut AacDecoderInstance) {
    join(self_);
    if let Some(mut p) = self_.pipe.take() {
        if let Some(w) = p.worker.take() {
            w.shutdown();
        }
        p.queue.clear();
    }
    // (the threads of the per channel parallelism end)
    self_.chan_par = Default::default();
}

/// `aacDecoder_Fill()` was called (for the drain rule of [`decode_frame`]).
pub(crate) fn note_fill(self_: &mut AacDecoderInstance) {
    if let Some(p) = self_.pipe.as_deref_mut() {
        p.fills = p.fills.wrapping_add(1);
    }
}

/// The stream information `aacDecoder_GetStreamInfo()` returns.
pub(crate) fn presented_info(self_: &AacDecoderInstance) -> &CStreamInfo {
    match self_.pipe.as_deref().and_then(|p| p.presented.as_ref()) {
        Some(i) => i,
        None => &self_.stream_info,
    }
}

/// Makes the decoder state complete for an API function other than `aacDecoder_DecodeFrame()`
/// (it acts in the sequential order: between the frames of the surrounding calls).
pub(crate) fn sync_for_api(self_: &mut AacDecoderInstance) {
    join(self_);
}

/// The stream information of the decoder itself (not that of the frame the last call returned):
/// that of the sequential decoder after the last call, once a frame still being decoded on the
/// worker thread is joined ([`sync_stream_info`]). After a drain (6 of [`decode_frame`]) it is
/// that of the call, which returned `AAC_DEC_NOT_ENOUGH_BITS` in the sequential decoder.
pub fn decoder_stream_info(self_: &AacDecoderInstance) -> &CStreamInfo {
    &self_.stream_info
}

/// After a call that returned `AAC_DEC_NOT_ENOUGH_BITS` while a frame was being decoded on the
/// worker thread, the stream information of the decoder is complete only after that frame is
/// finished (the call does not wait for it). This waits for it; the stream information
/// `aacDecoder_GetStreamInfo()` returns is then that of the sequential decoder after the call.
/// Without effect otherwise.
pub fn sync_stream_info(self_: &mut AacDecoderInstance) {
    if self_.pipe.as_deref().is_some_and(|p| p.presented.is_none()) {
        join(self_);
    }
}

/// Copies a result into the output buffer of the call that returns it, as
/// `aacDecoder_DecodeFrame()` leaves its output buffer: the valid samples, or `timeDataSize`
/// cleared samples without valid output.
fn deliver(
    self_: &mut AacDecoderInstance,
    r: FrameResult,
    p_time_data: &mut [IntPcm],
    time_data_size: i32,
) -> AacDecoderError {
    if is_output_valid(r.err) != 0 {
        let n = r.n.min(p_time_data.len());
        p_time_data[..n].copy_from_slice(&r.pcm[..n]);
    } else {
        let n = (time_data_size.max(0) as usize).min(p_time_data.len());
        p_time_data[..n].fill(0);
    }
    if let Some(a) = r.anc.as_ref() {
        restore_anc(self_, a);
    }
    let err = r.err;
    let p = self_.pipe.as_deref_mut().expect("pipeline");
    p.presented = Some(r.info);
    p.give_pcm(r.pcm);
    err
}

/// The ancillary data of the current state.
fn snapshot_anc(self_: &AacDecoderInstance) -> Option<AncSnapshot> {
    let a = &self_.anc_data;
    let n = a.offset[(a.nr_elements.clamp(0, 7)) as usize].max(0) as usize;
    let bytes = match &a.buffer {
        AncBuffer::None => return None,
        AncBuffer::Owned(b) => b[..n.min(b.len())].to_vec(),
        AncBuffer::Raw(r) => {
            let b = r.as_slice();
            b[..n.min(a.buffer_size.max(0) as usize).min(b.len())].to_vec()
        }
    };
    Some(AncSnapshot { offset: a.offset, nr_elements: a.nr_elements, bytes })
}

/// Restores the ancillary data of a returned frame.
fn restore_anc(self_: &mut AacDecoderInstance, s: &AncSnapshot) {
    let a = &mut self_.anc_data;
    a.offset = s.offset;
    a.nr_elements = s.nr_elements;
    match &mut a.buffer {
        AncBuffer::None => {}
        AncBuffer::Owned(b) => {
            let n = s.bytes.len().min(b.len());
            b[..n].copy_from_slice(&s.bytes[..n]);
        }
        AncBuffer::Raw(r) => {
            let b = r.as_mut_slice();
            let n = s.bytes.len().min(a.buffer_size.max(0) as usize).min(b.len());
            b[..n].copy_from_slice(&s.bytes[..n]);
        }
    }
}

/// `aacDecoder_DecodeFrame()` of a decoder with more than one thread.
///
/// # Contract
///
/// 1. A call decodes as the call of the sequential decoder would (it reads input and applies
///    its flags).
/// 2. A call that finds not enough input (`AAC_DEC_NOT_ENOUGH_BITS`) returns that at once; it
///    decodes no frame, and a frame pending from an earlier call stays pending (except 6). The
///    stream information after it is that of the sequential decoder after the call.
/// 3. Every other call decodes a frame (concealed, flushed or with an error, as the sequential
///    decoder) and returns the result of the previous such call: its return value, its output
///    samples in the output buffer of this call (or `timeDataSize` cleared samples without
///    valid output), and, until the next call, its stream information
///    (`aacDecoder_GetStreamInfo()`) and ancillary data (`aacDecoder_AncDataGet()`). The
///    result of this call stays pending.
/// 4. If there is no previous result (the first frame, or the first after a drain), the call
///    returns `AAC_DEC_NOT_ENOUGH_BITS` without output.
/// 5. Parameter changes (`aacDecoder_SetParam()`) and the other functions act in the sequential
///    order: after the frames of the calls before them, before the frames of later calls.
/// 6. A call that finds not enough input directly after a call that returned
///    `AAC_DEC_NOT_ENOUGH_BITS`, with no `aacDecoder_Fill()` in between, returns the pending
///    frame if there is one (a *drain*), as in 3.
///
/// So the frames (all results other than `AAC_DEC_NOT_ENOUGH_BITS`) come in the order and
/// with the content of the sequential decoder, one call later; after the last
/// `aacDecoder_Fill()`, calling `aacDecoder_DecodeFrame()` with flags 0 until it returns
/// `AAC_DEC_NOT_ENOUGH_BITS` twice in a row returns all of them (the access units still in the
/// input buffer are decoded with the pipeline, the last frame comes with the drain). [`drain`]
/// returns the pending frame without a call of the sequential decoder.
pub fn decode_frame(
    self_: &mut AacDecoderInstance,
    p_time_data: &mut [IntPcm],
    time_data_size: i32,
    flags: u32,
) -> AacDecoderError {
    let p = self_.pipe.as_deref_mut().expect("pipeline");
    let may_drain = p.last_neb && p.fills == p.fills_at_neb;
    p.last_neb = false;

    if run_frame(self_, time_data_size, flags) {
        let p = self_.pipe.as_deref_mut().expect("pipeline");
        if p.queue.len() >= 2 {
            // 3: the previous frame
            match p.queue.pop_front() {
                Some(Pending::Ready(r)) => deliver(self_, r, p_time_data, time_data_size),
                _ => unreachable!("the previous frame is complete"),
            }
        } else {
            // 4: nothing to return yet (a return of AAC_DEC_NOT_ENOUGH_BITS for 6)
            p.last_neb = true;
            p.fills_at_neb = p.fills;
            let n = (time_data_size.max(0) as usize).min(p_time_data.len());
            p_time_data[..n].fill(0);
            AAC_DEC_NOT_ENOUGH_BITS
        }
    } else {
        // 2: not enough bits (the decoder state is that of the sequential decoder after the
        // call once a frame still being decoded is joined, see sync_stream_info())
        let p = self_.pipe.as_deref_mut().expect("pipeline");
        if may_drain && !p.queue.is_empty() {
            // 6: the pending frame
            return drain(self_, p_time_data, time_data_size);
        }
        p.last_neb = true;
        p.fills_at_neb = p.fills;
        p.presented = None;
        let n = (time_data_size.max(0) as usize).min(p_time_data.len());
        p_time_data[..n].fill(0);
        AAC_DEC_NOT_ENOUGH_BITS
    }
}

/// Returns the pending frame without reading input (see [`decode_frame`]);
/// `AAC_DEC_NOT_ENOUGH_BITS` (and the output buffer cleared) if there is none.
pub fn drain(
    self_: &mut AacDecoderInstance,
    p_time_data: &mut [IntPcm],
    time_data_size: i32,
) -> AacDecoderError {
    join(self_);
    let popped = self_.pipe.as_deref_mut().and_then(|p| p.queue.pop_front());
    match popped {
        Some(Pending::Ready(r)) => deliver(self_, r, p_time_data, time_data_size),
        Some(Pending::InFlight) => unreachable!("joined"),
        None => {
            let _ = AAC_DEC_INVALID_HANDLE;
            let n = (time_data_size.max(0) as usize).min(p_time_data.len());
            p_time_data[..n].fill(0);
            AAC_DEC_NOT_ENOUGH_BITS
        }
    }
}

/// Decodes the frame of one call and pushes its result to the queue (`true`), or returns
/// `false` for `AAC_DEC_NOT_ENOUGH_BITS` (no frame; the pipeline is joined and the decoder
/// state complete).
fn run_frame(self_: &mut AacDecoderInstance, time_data_size: i32, flags: u32) -> bool {
    let start = Instant::now();
    let p = self_.pipe.as_deref_mut().expect("pipeline");
    let pipeline = p.apply_mode();
    let mode = p.tuner.mode();
    let frame = run_frame_in_mode(self_, time_data_size, flags, pipeline);
    if frame {
        let p = self_.pipe.as_deref_mut().expect("pipeline");
        p.mode_frames[mode] += 1;
        p.tuner.tick(start);
    }
    frame
}

/// [`run_frame`] in a mode that allows the pipeline or not.
fn run_frame_in_mode(
    self_: &mut AacDecoderInstance,
    time_data_size: i32,
    flags: u32,
    pipeline: bool,
) -> bool {
    let p = self_.pipe.as_deref_mut().expect("pipeline");
    let mut out = p.take_pcm();
    // (only the first samples of the result are used: the buffer is not cleared)
    let size = (time_data_size.max(0) as usize).min(MAX_FRAME_SAMPLES);
    if out.len() != size {
        out.resize(size, 0);
    }

    let backoff = {
        let p = self_.pipe.as_deref_mut().expect("pipeline");
        let b = p.backoff > 0;
        p.backoff = p.backoff.saturating_sub(1);
        b
    };
    if backoff || !pipeline || !state::eligible_before_a(self_, flags) {
        // The sequential decoder for the whole call.
        join(self_);
        let mut td2 = self_.p_time_data2.take().unwrap_or_default();
        let err = crate::aacdecoder_lib::decode_frame_lib(
            self_,
            &mut out,
            time_data_size,
            flags,
            &mut td2,
        );
        self_.p_time_data2 = Some(td2);
        return finish_inline(self_, err, out);
    }

    state::begin_stage_a(self_);
    let outcome = stage_a(self_, time_data_size, flags, &mut out);
    match outcome {
        AOutcome::Done(err) => finish_inline(self_, err, out),
        AOutcome::Frame(mut ho) => {
            // Stage B of the previous frame; then stage B of this frame on the worker thread
            // or here.
            let touched = self_.pipe.as_deref().is_some_and(|p| p.touched);
            join(self_);
            let anc = snapshot_anc(self_);
            if self_.pipe.as_deref().is_some_and(|p| p.poison & 4 != 0) {
                // Negative control of the poisoning: destroys what stage A handed over.
                state::poison_channel_memory(self_, 0x1234_5678_9abc_def1);
            }
            // (a frame whose stage A wrote its own set of per frame channel memory is handed
            // over: stage A redoes the others, see state::can_defer_a())
            debug_assert!(!touched || state::can_defer(self_, &ho));
            if state::can_defer(self_, &ho) {
                if touched {
                    self_.pipe.as_deref_mut().expect("pipeline").fail_streak = 0;
                }
                state::hand_over(self_, ho, out, anc);
            } else {
                // (the recorded SBR operations of stage A go first)
                state::flush_b_ops(self_);
                let p = self_.pipe.as_deref_mut().expect("pipeline");
                p.frames_inline += 1;
                let mut td2 = self_.p_time_data2.take().unwrap_or_default();
                let mut tp = self_.h_input.take();
                let (err, n) = {
                    let mut bs = tp.as_deref().map(|t| t.bit_stream[0].clone());
                    let cap = inline_cap(self_);
                    let r = stage_b(self_, &mut ho, &mut td2, &mut out, bs.as_mut(), cap);
                    if let (Some(t), Some(b)) = (tp.as_deref_mut(), bs) {
                        t.bit_stream[0] = b;
                    }
                    r
                };
                self_.h_input = tp;
                self_.p_time_data2 = Some(td2);
                let info = self_.stream_info;
                let p = self_.pipe.as_deref_mut().expect("pipeline");
                p.queue.push_back(Pending::Ready(FrameResult { err, pcm: out, n, info, anc }));
            }
            true
        }
    }
}

/// Queues the result of a call that ran in the calling thread (the decoder state is complete);
/// `false` for `AAC_DEC_NOT_ENOUGH_BITS`.
fn finish_inline(self_: &mut AacDecoderInstance, err: AacDecoderError, out: Vec<IntPcm>) -> bool {
    if err == AAC_DEC_NOT_ENOUGH_BITS {
        self_.pipe.as_deref_mut().expect("pipeline").give_pcm(out);
        return false;
    }
    self_.pipe.as_deref_mut().expect("pipeline").frames_inline += 1;
    let n = if is_output_valid(err) != 0 {
        let si = &self_.stream_info;
        (si.frame_size as i64 * si.num_channels as i64).clamp(0, out.len() as i64) as usize
    } else {
        0
    };
    let info = self_.stream_info;
    let anc = snapshot_anc(self_);
    let p = self_.pipe.as_deref_mut().expect("pipeline");
    p.queue.push_back(Pending::Ready(FrameResult { err, pcm: out, n, info, anc }));
    true
}

#[allow(unused_imports)]
use {FixpDbl as _, PcmDec as _, AACDEC_CONCEAL as _, AACDEC_FLUSH as _};
