// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not a port: the reference encoder is single-threaded.

//! Output delay and SBR frame pipelining of the opt-in multi-threading (PORTING.md,
//! "Optimization rules").
//!
//! # The output delay
//!
//! The first `aacEncEncode()` call with output arguments made while the thread count is above
//! 1 and SBR is active (after the call's initialization: HE-AAC, HE-AAC v2, ELD with SBR)
//! switches the encoder to *delayed output* for the rest of its life (also if it is later
//! reconfigured without SBR or to one thread). Without SBR there is nothing to pipeline and the
//! output is not delayed. From then on every such call returns the *result of the previous
//! one*: its return code, `numOutBytes`,
//! `numAncBytes`, `bitResState` and the contents of the output buffers (the access unit and its
//! size), while `numInSamples` is that of the call itself (the caller needs it to advance its
//! input). The first delayed call returns an empty result (`AACENC_OK`, no access unit). The
//! sequence of results is therefore that of the single-threaded encoder, one call later;
//! `aacEncEncode()` returns `AACENC_ENCODE_EOF` one call after the single-threaded encoder,
//! so the usual flush loop (until `AACENC_ENCODE_EOF`) returns every access unit. Calls without
//! output arguments (the initialization call) return their own result at once.
//!
//! Every operation of the encoder runs in the order of the single-threaded encoder: the delay
//! only changes when results are returned, so the state of the encoder, and every later
//! result, is the same. The one exception is the frame pipelining below, which undoes its
//! speculative work when it cannot prove it equal.
//!
//! Other functions:
//! - `aacEncoder_SetParam()` and `aacEncInfo()` first run the core of the pending frame (they
//!   read or change state the core uses).
//! - `aacEncoder_GetParam(AACENC_CONTROL_STATE)` does not show the reinitialization request of
//!   an encoding error before the call that returns that error.
//! - The output buffers of a call must hold the access unit it returns (the previous call's):
//!   with the same buffer for every call, as usual, this holds whenever the single-threaded
//!   encoder succeeds. Otherwise the call returns `AACENC_ENCODE_ERROR`.
//! - The input buffers must not overlap the output buffers.
//!
//! # The SBR frame pipelining
//!
//! With SBR, the SBR encoder of a frame (downsampler, QMF analysis, envelope, tonality and
//! transient estimation, parametric stereo) runs before the core, and its payload size enters
//! the core's bit budget. The pipelining runs the core of frame N-1 on the calling thread (with
//! the core's own pool, `min(threads - 1, channels)` threads) at the same time as everything
//! of the call of frame N before its core (the end of frame N-1: `nSamplesRead` and
//! `sbrEncoder_UpdateBuffers()`; then the input, the metadata encoder, the SBR encoder and the
//! payloads) on a second thread. Both parts are independent:
//!
//! - The core of a pending frame reads its own copy of its input (the core's part of the input
//!   buffer, taken after the SBR encoder downsampled it) and of its extension payloads (SBR,
//!   metadata and ancillary data), and writes only the core, the transport encoder (and its
//!   output buffer) and the core's pool. The second part never touches those.
//! - The transport encoder calls back into the SBR encoder when it writes an in-band
//!   configuration (LATM/LOAS): it writes the SBR header and switches off the headers in the
//!   SBR payload. The core gets a copy of the SBR headers instead (the SBR encoder of frame N
//!   can change them), and records the switch-off. If it happened and was not a no-op, the
//!   speculative part is undone (below).
//! - Not pipelined (the core of frame N-1 runs first): calls that initialize, MPEG Surround
//!   (ELDv2), calls that do not complete a frame, a thread count of 1.
//!
//! The second part is speculative: it assumes that the core of frame N-1 succeeds. Before it
//! changes anything it takes a snapshot of everything it can change (the SBR encoder, the input
//! buffer, the metadata encoder, the sample counters, the initialization flags and user
//! parameters, and the thread-local test counters). If the core fails, its output does not fit
//! the call's output buffer, or the configuration call-back above was not a no-op, the
//! snapshot is restored and the call continues sequentially from the state the single-threaded
//! encoder has at that point.

use super::*;
use crate::par::{self, join2, AssertSend, ThreadPool};
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_sbrenc::bit_sbr::{encode_sbr_header_data, SbrHeaderData};
use fdk_aac_rs_sbrenc::sbr::MAX_SBR_ELEMENTS;

/// What a call wrote into its output arguments and buffers besides `numInSamples`: returned
/// with the next call in delayed mode.
#[derive(Clone, Debug, Default)]
pub(super) struct CallResult {
    ret: AacencError,
    /// C cleared the output arguments (else they keep the caller's values).
    cleared: bool,
    num_out_bytes: i32,
    num_anc_bytes: i32,
    bit_res_state: i32,
    /// C cleared the output buffers.
    zeroed: bool,
    /// The access unit size written to `OUT_AU_SIZES`.
    au_size: Option<i32>,
    /// The bytes written to `OUT_BITSTREAM_DATA`.
    bytes: Vec<u8>,
}

impl CallResult {
    /// The result returned by the first delayed call: nothing.
    fn empty() -> Self {
        CallResult { ret: AACENC_OK, cleared: true, zeroed: true, ..Default::default() }
    }

    /// The result of a call that returned before the core.
    fn early(ret: AacencError, opts: &FrontOpts) -> Self {
        CallResult { ret, cleared: opts.cleared, zeroed: opts.zeroed, ..Default::default() }
    }
}

/// What `aacEncEncode()` does with the output after the core, from the descriptor of the call
/// of the frame.
#[derive(Clone, Copy, Debug, Default)]
struct OutDesc {
    /// `outBufDesc != NULL && outBufDesc->numBufs >= 1`
    present: bool,
    /// `bufSizes[bsIdx]`, `None` for `bsIdx == -1`
    bs_size: Option<i32>,
    /// `auIdx != -1`
    au: bool,
}

/// A frame whose core has not run yet, with copies of what the core reads.
pub(super) struct PendingCore {
    /// The core's part of the input buffer (`channels * inputBufferSizePerChannel` samples).
    core_input: Vec<IntPcm>,
    buf_size: u32,
    /// The extension payloads; `p_data` points into the copies below (or, for MPEG Surround,
    /// into its encoder, which does not run before this core).
    ext_payload: [AacencExtPayload; MAX_TOTAL_EXT_PAYLOADS],
    sbr_payload: Box<SbrencExtPayload>,
    md_payload: Box<[u8; PAYLOAD_MEM_SIZE]>,
    anc: Vec<u8>,
    anc_data_ext_idx: i32,
    in_num_anc_bytes: i32,
    out: OutDesc,
    zeroed: bool,
}

// SAFETY: the raw pointers of `ext_payload` point into the buffers of the structure itself or
// into the MPEG Surround encoder of the same `AacEncoder` (see `AacEncoder`).
unsafe impl Send for PendingCore {}

/// The state of the output delay and the pipelining of an encoder (`AacEncoder::pipe`).
#[derive(Default)]
pub struct EncPipe {
    /// Delayed output (see the module documentation).
    pub(super) delayed: bool,
    /// The frame of the previous call, if its core has not run yet.
    pending: Option<Box<PendingCore>>,
    /// The result of the previous call, if it is complete.
    ready: Option<CallResult>,
    /// The pool of the two pipelined parts (2 threads), created on demand.
    pool: Option<ThreadPool>,
    pool_failed: bool,
    /// Frames whose core ran pipelined, and speculative parts undone (for tests).
    pub(super) overlapped: u64,
    pub(super) undone: u64,
}

impl std::fmt::Debug for EncPipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncPipe")
            .field("delayed", &self.delayed)
            .field("pending", &self.pending.is_some())
            .field("ready", &self.ready)
            .field("overlapped", &self.overlapped)
            .field("undone", &self.undone)
            .finish()
    }
}

impl EncPipe {
    /// The two-thread pool of the pipelining (created here; `None` if that fails).
    fn pool(&mut self) -> Option<&ThreadPool> {
        if self.pool.is_none() && !self.pool_failed {
            match ThreadPool::new(2) {
                Ok(p) => self.pool = Some(p),
                Err(_) => self.pool_failed = true,
            }
        }
        self.pool.as_ref()
    }
}

/// Starts the output delay in the call of `front` (the first call with output arguments made
/// with more than one thread and SBR): the call's own result is kept for the next call, and it
/// returns an empty result (the output arguments and buffers as C cleared them).
///
/// # Safety
/// As [`aac_enc_encode`].
pub(super) unsafe fn enter(
    h: &mut AacEncoder,
    f: Front,
    opts: &FrontOpts,
    inargs: Option<&AacencInArgs>,
    out_buf_desc: Option<&AacencBufDesc>,
) -> AacencError {
    h.pipe.delayed = true;
    let next = match f {
        Front::Return(err) => {
            if err == AACENC_ENCODE_ERROR {
                // All encoder modules have to be initialized
                h.init_flags = AACENC_INIT_ALL;
            }
            Next::Ready(CallResult::early(err, opts))
        }
        Front::Frame { anc_data_ext_idx } => {
            let channels = h.h_aac_enc.as_deref().map_or(0, |a| a.channel_mapping.n_channels);
            // SAFETY: the caller's guarantee.
            Next::Pending(unsafe {
                make_pending(h, channels, anc_data_ext_idx, inargs, out_buf_desc, opts.zeroed)
            })
        }
    };
    h.pipe.store(next);
    AACENC_OK
}

/// The threads of the core's pool kept for the second part of the pipelining.
fn reserve(h: &AacEncoder) -> u32 {
    (h.pipe.delayed && is_sbr_active(&h.aac_config) != 0) as u32
}

/// `aacEncEncode()` in delayed mode.
///
/// # Safety
/// As [`aac_enc_encode`].
pub(super) unsafe fn encode_delayed(
    h: &mut AacEncoder,
    in_buf_desc: Option<&AacencBufDesc>,
    out_buf_desc: Option<&AacencBufDesc>,
    inargs: Option<&AacencInArgs>,
    outargs: Option<&mut AacencOutArgs>,
) -> AacencError {
    if h.threads.get() < 2 {
        h.pipe.pool = None;
    }
    let Some(outargs) = outargs else {
        // Without output arguments the call has no frame and returns its own result. It may
        // initialize, which comes after the core of the pending frame.
        if h.init_flags != 0 {
            drain(h);
        }
        let mut opts = FrontOpts::default();
        // SAFETY: the caller's guarantee.
        let err = match unsafe { front(h, in_buf_desc, out_buf_desc, inargs, None, &mut opts) } {
            Front::Return(err) => err,
            Front::Frame { .. } => AACENC_UNSUPPORTED_PARAMETER, // (not reached)
        };
        if err == AACENC_ENCODE_ERROR {
            h.init_flags = AACENC_INIT_ALL;
        }
        return err;
    };

    // SAFETY: the caller's guarantee.
    let result = if unsafe { can_pipeline(h, in_buf_desc, out_buf_desc, inargs) } {
        // SAFETY: the caller's guarantee.
        unsafe { pipelined(h, in_buf_desc, out_buf_desc, inargs, outargs) }
    } else {
        drain(h);
        let result = h.pipe.ready.take().unwrap_or_else(CallResult::empty);
        // SAFETY: the caller's guarantee.
        let (next, num_in) = unsafe { front_delayed(h, in_buf_desc, out_buf_desc, inargs, None) };
        if let Some(n) = num_in {
            outargs.num_in_samples = n;
        }
        h.pipe.store(next);
        result
    };
    // SAFETY: the caller's guarantee.
    unsafe { deliver(result, out_buf_desc, outargs) }
}

/// What a call leaves for the next one.
enum Next {
    Ready(CallResult),
    Pending(Box<PendingCore>),
}

impl EncPipe {
    fn store(&mut self, next: Next) {
        match next {
            Next::Ready(r) => self.ready = Some(r),
            Next::Pending(p) => self.pending = Some(p),
        }
    }
}

/// The front part of a delayed call: returns what it leaves for the next call and the
/// `numInSamples` to return (`None`: C did not write the output arguments).
///
/// # Safety
/// As [`aac_enc_encode`].
unsafe fn front_delayed(
    h: &mut AacEncoder,
    in_buf_desc: Option<&AacencBufDesc>,
    out_buf_desc: Option<&AacencBufDesc>,
    inargs: Option<&AacencInArgs>,
    cm: Option<ChannelMapping>,
) -> (Next, Option<i32>) {
    let mut local = AacencOutArgs::default();
    let mut opts = FrontOpts { cm, defer_zero: true, ..Default::default() };
    // SAFETY: the caller's guarantee.
    let f = unsafe { front(h, in_buf_desc, out_buf_desc, inargs, Some(&mut local), &mut opts) };
    let num_in = opts.cleared.then_some(local.num_in_samples);
    let next = match f {
        Front::Return(err) => {
            if err == AACENC_ENCODE_ERROR {
                h.init_flags = AACENC_INIT_ALL;
            }
            Next::Ready(CallResult::early(err, &opts))
        }
        Front::Frame { anc_data_ext_idx } => {
            let cm = cm.or_else(|| h.h_aac_enc.as_deref().map(|a| a.channel_mapping));
            // SAFETY: the caller's guarantee.
            Next::Pending(unsafe {
                make_pending(
                    h,
                    cm.map_or(0, |c| c.n_channels),
                    anc_data_ext_idx,
                    inargs,
                    out_buf_desc,
                    opts.zeroed,
                )
            })
        }
    };
    (next, num_in)
}

/// The pending frame of a frame [`front`] just set up: copies of the core's input and of the
/// extension payloads.
///
/// # Safety
/// The ancillary data payload (`h.ext_payload[anc_data_ext_idx]`) must be readable for its
/// `data_size` bits (the caller's guarantee of `aacEncEncode()`).
unsafe fn make_pending(
    h: &mut AacEncoder,
    core_channels: i32,
    anc_data_ext_idx: i32,
    inargs: Option<&AacencInArgs>,
    out_buf_desc: Option<&AacencBufDesc>,
    zeroed: bool,
) -> Box<PendingCore> {
    let buf_size = h.input_buffer_size_per_channel.max(0) as usize;
    let n = (core_channels.clamp(0, 64) as usize * buf_size).min(h.input_buffer.len());
    let mut p = Box::new(PendingCore {
        core_input: h.input_buffer[..n].to_vec(),
        buf_size: h.input_buffer_size_per_channel as u32,
        ext_payload: h.ext_payload,
        sbr_payload: Box::default(),
        md_payload: Box::new([0; PAYLOAD_MEM_SIZE]),
        anc: Vec::new(),
        anc_data_ext_idx,
        in_num_anc_bytes: inargs.map_or(0, |a| a.num_anc_bytes),
        out: OutDesc::default(),
        zeroed,
    });

    // The payloads of the SBR and metadata encoders, which their next frame overwrites.
    let sbr = h.p_sbr_payload.as_deref().map(|pl| {
        let base = pl.data.as_ptr() as usize;
        (base, base + core::mem::size_of_val(&pl.data))
    });
    if let Some(pl) = h.p_sbr_payload.as_deref() {
        *p.sbr_payload = pl.clone();
    }
    let md = h.h_metadata_enc.as_deref().map(|m| {
        let base = m.payload_mem.as_ptr() as usize;
        (base, base + PAYLOAD_MEM_SIZE)
    });
    if let Some(m) = h.h_metadata_enc.as_deref() {
        *p.md_payload = m.payload_mem;
    }
    let sbr_copy = p.sbr_payload.data.as_ptr() as usize;
    let md_copy = p.md_payload.as_ptr() as usize;
    for (i, x) in p.ext_payload.iter_mut().enumerate() {
        let a = x.p_data as usize;
        if x.p_data.is_null() {
            continue;
        }
        if i as i32 == anc_data_ext_idx {
            // the caller's ancillary data (read for data_size bits, see bitenc)
            let bytes = (x.data_size as usize).div_ceil(8);
            // SAFETY: the function's guarantee.
            p.anc = unsafe { core::slice::from_raw_parts(x.p_data, bytes) }.to_vec();
            x.p_data = if p.anc.is_empty() {
                core::ptr::NonNull::<u8>::dangling().as_ptr()
            } else {
                p.anc.as_ptr()
            };
        } else if let Some((lo, hi)) = sbr.filter(|&(lo, hi)| a >= lo && a < hi) {
            let _ = hi;
            x.p_data = (sbr_copy + (a - lo)) as *const u8;
        } else if let Some((lo, _)) = md.filter(|&(lo, hi)| a >= lo && a < hi) {
            x.p_data = (md_copy + (a - lo)) as *const u8;
        }
    }

    if let Some(d) = out_buf_desc {
        if d.num_bufs >= 1 {
            // SAFETY: the caller's guarantee (the descriptor was validated by front()).
            let bs_idx = unsafe { get_buf_desc_idx(out_buf_desc, OUT_BITSTREAM_DATA) };
            // SAFETY: as above.
            let au_idx = unsafe { get_buf_desc_idx(out_buf_desc, OUT_AU_SIZES) };
            p.out = OutDesc {
                present: true,
                // SAFETY: as above.
                bs_size: (bs_idx != -1).then(|| unsafe { *d.buf_sizes.add(bs_idx as usize) }),
                au: au_idx != -1,
            };
        }
    }
    p
}

/// Runs the core of `p` (`FDKaacEnc_EncodeFrame()`): returns its error and `nOutBytes[0]`.
fn run_core(
    aac: &mut AacEnc,
    tp: &mut TransportEnc,
    aac_config: &AacencConfig,
    p: &mut PendingCore,
    cb: &mut dyn TpEncCallbacks,
    par: Option<crate::par::EncPar<'_>>,
) -> (AacEncoderError, i32) {
    // hAacEnc->config points to aacConfig (see crate::aacenc)
    aac.config = *aac_config;
    let mut out_bytes = [0i32; 1];
    // SAFETY: the payloads point into `p` (or the MPEG Surround encoder, see PendingCore).
    let r = unsafe {
        fdk_aac_enc_encode_frame_pool(
            aac,
            tp,
            &p.core_input,
            p.buf_size,
            &mut out_bytes,
            &mut p.ext_payload,
            cb,
            par,
        )
    };
    (r, out_bytes[0])
}

/// The result of the call of `p` after its core returned `r` (the rest of `aacEncEncode()`).
fn frame_result(
    p: &PendingCore,
    r: AacEncoderError,
    out_bytes: i32,
    n_sub_frames: i32,
    out_buffer: &BitMem,
    bit_res_state: i32,
) -> CallResult {
    let mut res =
        CallResult { ret: AACENC_OK, cleared: true, zeroed: p.zeroed, ..Default::default() };
    if r != AAC_ENC_OK {
        res.ret = AACENC_ENCODE_ERROR;
        return res;
    }
    let idx = p.anc_data_ext_idx;
    if idx >= 0 && (idx as usize) < MAX_TOTAL_EXT_PAYLOADS {
        res.num_anc_bytes =
            p.in_num_anc_bytes.wrapping_sub((p.ext_payload[idx as usize].data_size >> 3) as i32);
    }
    if p.out.present {
        let mut n_bs_bytes = 0;
        for _ in 0..n_sub_frames.clamp(0, 1) {
            n_bs_bytes += out_bytes;
            if p.out.au {
                res.au_size = Some(out_bytes);
            }
        }
        match p.out.bs_size {
            Some(size) if size >= n_bs_bytes => {
                let n = (n_bs_bytes.max(0) as usize).min(out_buffer.len());
                res.bytes = vec![0; n];
                out_buffer.read(0, &mut res.bytes);
                res.num_out_bytes = n_bs_bytes;
                res.bit_res_state = bit_res_state;
            }
            // output buffer too small, can't write valid bitstream
            _ => res.ret = AACENC_ENCODE_ERROR,
        }
    }
    res
}

/// The end of a frame after a successful core: samples exhausted, delay balancing buffer.
fn finish_frame(h: &mut AacEncoder) {
    h.n_samples_read -= h.n_samples_to_read;
    if is_sbr_active(&h.aac_config) != 0 {
        if let Some(env) = h.h_env_enc.as_deref_mut() {
            sbr_encoder_update_buffers(
                env,
                &mut h.input_buffer,
                h.input_buffer_size_per_channel as u32,
            );
        }
    }
}

/// Runs the core of the pending frame (if any) and completes its result.
pub(super) fn drain(h: &mut AacEncoder) {
    let Some(mut p) = h.pipe.pending.take() else {
        return;
    };
    let reserve = reserve(h);
    let (Some(aac), Some(tp)) = (h.h_aac_enc.as_deref_mut(), h.h_tp_enc.as_deref_mut()) else {
        // (not reached: front() set up the frame with both)
        h.init_flags = AACENC_INIT_ALL;
        h.pipe.ready = Some(CallResult { ret: AACENC_ENCODE_ERROR, ..CallResult::empty() });
        return;
    };
    let mut cb =
        EncCallbacks { env_enc: h.h_env_enc.as_deref_mut(), mps_enc: h.h_mps_enc.as_deref() };
    let par = h.threads.par_with(aac.channel_mapping.n_channels, reserve);
    let (r, out_bytes) = run_core(aac, tp, &h.aac_config, &mut p, &mut cb, par);
    let res = frame_result(
        &p,
        r,
        out_bytes,
        h.aac_config.n_sub_frames,
        &h.out_buffer,
        fdk_aac_enc_get_bit_reservoir_state(aac),
    );
    if r == AAC_ENC_OK {
        finish_frame(h);
    }
    if res.ret == AACENC_ENCODE_ERROR {
        // All encoder modules have to be initialized
        h.init_flags = AACENC_INIT_ALL;
    }
    h.pipe.ready = Some(res);
}

/// Writes the result of the previous call into the output arguments and buffers of this one.
///
/// # Safety
/// As [`aac_enc_encode`].
unsafe fn deliver(
    r: CallResult,
    out_buf_desc: Option<&AacencBufDesc>,
    outargs: &mut AacencOutArgs,
) -> AacencError {
    // SAFETY: the caller's guarantee.
    let valid = unsafe { validate_buf_desc(out_buf_desc) } == AACENC_OK;
    if r.zeroed && valid {
        if let Some(d) = out_buf_desc {
            for i in 0..d.num_bufs.max(0) as usize {
                // SAFETY: the caller's guarantee.
                unsafe {
                    let b = *d.bufs.add(i);
                    if !b.is_null() {
                        let n = *d.buf_sizes.add(i);
                        core::ptr::write_bytes(b as *mut u8, 0, n.max(0) as usize);
                    }
                }
            }
        }
    }
    if r.cleared {
        outargs.num_out_bytes = r.num_out_bytes;
        outargs.num_anc_bytes = r.num_anc_bytes;
        outargs.bit_res_state = r.bit_res_state;
    }
    let d = out_buf_desc.filter(|d| valid && d.num_bufs >= 1);
    if let Some(size) = r.au_size {
        // SAFETY: the caller's guarantee.
        let au_idx = unsafe { get_buf_desc_idx(d, OUT_AU_SIZES) };
        if let (Some(d), true) = (d, au_idx != -1) {
            // SAFETY: the caller's guarantee.
            unsafe { (*d.bufs.add(au_idx as usize) as *mut i32).write_unaligned(size) };
        }
    }
    if !r.bytes.is_empty() {
        // SAFETY: the caller's guarantee.
        let bs_idx = unsafe { get_buf_desc_idx(d, OUT_BITSTREAM_DATA) };
        match d {
            // SAFETY: the caller's guarantee.
            Some(d)
                if bs_idx != -1
                    && unsafe { *d.buf_sizes.add(bs_idx as usize) } >= r.bytes.len() as i32 =>
            {
                // SAFETY: the caller's guarantee (bufSizes[bsIdx] bytes are writable).
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        r.bytes.as_ptr(),
                        *d.bufs.add(bs_idx as usize) as *mut u8,
                        r.bytes.len(),
                    );
                }
            }
            // The access unit of the previous call does not fit this call's buffer (see the
            // module documentation). Not an encoding error: the state is unchanged.
            _ => return AACENC_ENCODE_ERROR,
        }
    }
    r.ret
}

/// Whether this call can run the core of the pending frame at the same time as its front part
/// (see the module documentation).
///
/// # Safety
/// As [`aac_enc_encode`].
unsafe fn can_pipeline(
    h: &mut AacEncoder,
    in_buf_desc: Option<&AacencBufDesc>,
    out_buf_desc: Option<&AacencBufDesc>,
    inargs: Option<&AacencInArgs>,
) -> bool {
    if h.pipe.pending.is_none()
        || h.threads.get() < 2
        || h.init_flags != 0
        || !h.aac_config_bound
        || h.h_aac_enc.is_none()
        || h.h_tp_enc.is_none()
        || h.p_sbr_payload.is_none()
        || is_sbr_active(&h.aac_config) == 0
        || h.h_env_enc.as_deref().is_none_or(|e| e.n_channels == 0)
        || (h.h_mps_enc.is_some() && (h.aac_config.syntax_flags & AC_LD_MPS != 0))
    {
        return false;
    }
    let Some(inargs) = inargs else {
        return false;
    };
    // SAFETY: the caller's guarantee.
    if ((AACENC_OK != unsafe { validate_buf_desc(in_buf_desc) }) && (inargs.num_in_samples > 0))
        || (AACENC_OK != unsafe { validate_buf_desc(out_buf_desc) })
    {
        return false;
    }
    // Does the call complete a frame (after the end of the pending one)? A wrong answer only
    // costs time.
    let ext_n_channels = h.ext_param.n_channels as i32;
    let mut n_read = h.n_samples_read.wrapping_sub(h.n_samples_to_read);
    // SAFETY: the caller's guarantee.
    let idx = unsafe { get_buf_desc_idx(in_buf_desc, IN_AUDIO_DATA) };
    if inargs.num_in_samples > 0 && idx != -1 {
        let mut new_samples =
            fix_max(0, fix_min(inargs.num_in_samples, h.n_samples_to_read.wrapping_sub(n_read)));
        new_samples -= srem_i(new_samples, ext_n_channels);
        n_read += new_samples;
    }
    let frame = n_read >= h.n_samples_to_read
        || (inargs.num_in_samples == -1 && h.n_zeros_appended < h.n_delay);
    // The thread mode of the frame (see crate::par::EncThreads::mode): pipelined or not.
    let channels = h.h_aac_enc.as_deref().map_or(0, |a| a.channel_mapping.n_channels);
    let reserve = reserve(h);
    frame && h.threads.mode(channels, reserve).pipeline && h.pipe.pool().is_some()
}

/// Everything the speculative part of [`pipelined`] can change.
struct Snapshot {
    sbr: Option<SbrEncoder>,
    input_buffer: Vec<IntPcm>,
    metadata: Option<FdkMetadataEncoder>,
    n_samples_read: i32,
    n_zeros_appended: i32,
    init_flags: u32,
    ext_param: UserParam,
}

impl Snapshot {
    fn take(h: &AacEncoder) -> Self {
        Snapshot {
            sbr: h.h_env_enc.as_deref().cloned(),
            input_buffer: h.input_buffer.clone(),
            metadata: h.h_metadata_enc.as_deref().cloned(),
            n_samples_read: h.n_samples_read,
            n_zeros_appended: h.n_zeros_appended,
            init_flags: h.init_flags,
            ext_param: h.ext_param,
        }
    }

    /// Restores the snapshot. The boxed encoders keep their addresses (their payloads and
    /// bitstreams point into themselves).
    fn restore(self, h: &mut AacEncoder) {
        if let (Some(d), Some(s)) = (h.h_env_enc.as_deref_mut(), self.sbr) {
            d.restore_from(s);
        }
        h.input_buffer.copy_from_slice(&self.input_buffer);
        if let (Some(d), Some(mut s)) = (h.h_metadata_enc.as_deref_mut(), self.metadata) {
            match (d.h_drc_comp.as_deref_mut(), s.h_drc_comp.take()) {
                (Some(dc), Some(sc)) => {
                    *dc = *sc;
                    s.h_drc_comp = d.h_drc_comp.take();
                }
                (_, sc) => s.h_drc_comp = sc,
            }
            *d = s;
        }
        h.n_samples_read = self.n_samples_read;
        h.n_zeros_appended = self.n_zeros_appended;
        h.init_flags = self.init_flags;
        h.ext_param = self.ext_param;
    }
}

/// The transport encoder call-backs of a pipelined core: the SBR headers come from a copy, the
/// switch-off of the headers in the SBR payload (`sbrEncoder_GetHeader()` with `fSendHeaders
/// == 0`) is recorded.
struct PipelinedCallbacks<'a> {
    headers: &'a [Option<SbrHeaderData>; MAX_SBR_ELEMENTS],
    mps_enc: Option<&'a MpsEncoder>,
    switched_off: [bool; MAX_SBR_ELEMENTS],
}

impl TpEncCallbacks for PipelinedCallbacks<'_> {
    fn has_sbr(&self) -> bool {
        true
    }

    fn sbr(
        &mut self,
        hbs: &mut FdkBitStream,
        _sample_rate_in: i32,
        _sample_rate_out: i32,
        _samples_per_frame: i32,
        _core_codec: AudioObjectType,
        _element_id: Mp4ElementId,
        element_index: i32,
        _harmonic_sbr: u8,
        _stereo_config_index: u8,
        _config_mode: u8,
        _config_changed: Option<&mut u8>,
        _downscale_factor: i32,
    ) -> i32 {
        // sbrEncoder_GetHeader(hEnvEnc, hBs, elementIndex, 0) on the copy
        let i = element_index.max(0) as usize;
        if let Some(Some(header)) = self.headers.get(i) {
            encode_sbr_header_data(Some(header), hbs);
            self.switched_off[i] = true;
        }
        0
    }

    fn has_ssc(&self) -> bool {
        true
    }

    fn ssc(
        &mut self,
        hbs: Option<&mut FdkBitStream>,
        _core_codec: AudioObjectType,
        _sampling_rate: i32,
        _frame_size: i32,
        _num_channels: i32,
        _stereo_config_index: i32,
        _core_sbr_frame_length_index: i32,
        _config_bytes: i32,
        _config_mode: u8,
        _config_changed: Option<&mut u8>,
    ) -> i32 {
        fdk_mpegs_enc_write_spatial_specific_config(self.mps_enc, hbs)
    }
}

/// The SBR headers of the elements, and per element whether the SBR encoder of the next frame
/// is unaffected by switching off its headers in the payload (`sbrEncoder_GetHeader()`):
/// `NrSendHeaderData` and `CountSendHeaderData` are already -1 (`HeaderActive` is set at the
/// start of every frame before it is read), or the element is not encoded (index at or above
/// `noElements`, an element left from an earlier configuration).
fn sbr_headers(
    env: &SbrEncoder,
) -> ([Option<SbrHeaderData>; MAX_SBR_ELEMENTS], [bool; MAX_SBR_ELEMENTS]) {
    let mut headers = [None; MAX_SBR_ELEMENTS];
    let mut independent = [true; MAX_SBR_ELEMENTS];
    for (i, e) in env.sbr_element.iter().enumerate() {
        if let Some(e) = e.as_deref() {
            headers[i] = Some(e.sbr_header_data);
            let b = &e.sbr_bitstream_data;
            independent[i] = (i as i32) >= env.no_elements
                || (b.nr_send_header_data == -1 && b.count_send_header_data == -1);
        }
    }
    (headers, independent)
}

/// `sbrEncoder_GetHeader()` with `fSendHeaders == 0`: prevent header being embedded into the
/// SBR payload of the elements in `switched_off`.
fn switch_off_headers(h: &mut AacEncoder, switched_off: &[bool; MAX_SBR_ELEMENTS]) {
    if let Some(env) = h.h_env_enc.as_deref_mut() {
        for (e, &s) in env.sbr_element.iter_mut().zip(switched_off.iter()) {
            if let (Some(e), true) = (e.as_deref_mut(), s) {
                e.sbr_bitstream_data.nr_send_header_data = -1;
                e.sbr_bitstream_data.header_active = 0;
                e.sbr_bitstream_data.count_send_header_data = -1;
            }
        }
    }
}

/// A delayed call that runs the core of the pending frame at the same time as its own front
/// part (see the module documentation). Returns the result of the previous call.
///
/// # Safety
/// As [`aac_enc_encode`]; [`can_pipeline`] returned true.
unsafe fn pipelined(
    h: &mut AacEncoder,
    in_buf_desc: Option<&AacencBufDesc>,
    out_buf_desc: Option<&AacencBufDesc>,
    inargs: Option<&AacencInArgs>,
    outargs: &mut AacencOutArgs,
) -> CallResult {
    let (Some(mut p), Some(pool), Some(mut aac), Some(mut tp), Some(env)) = (
        h.pipe.pending.take(),
        h.pipe.pool.take(),
        h.h_aac_enc.take(),
        h.h_tp_enc.take(),
        h.h_env_enc.as_deref(),
    ) else {
        unreachable!("checked by can_pipeline()");
    };
    let (headers, independent) = sbr_headers(env);
    let cm = aac.channel_mapping;
    let aac_config = h.aac_config;
    let reserve = reserve(h);
    let mps = h.h_mps_enc.take();
    let mut threads = core::mem::take(&mut h.threads);

    // Task A (the core of the pending frame) owns `aac`, `tp`, `threads`, `p`, `mps` (shared)
    // and `core_out`; task B (the speculative front part) owns `h` (without those) and the
    // caller's buffers of this call, and writes `spec_out`. join2() returns after both.
    let mut core_out = None;
    let mut spec_out = None;
    {
        let core = AssertSend(|| {
            let mut cb = PipelinedCallbacks {
                headers: &headers,
                mps_enc: mps.as_deref(),
                switched_off: [false; MAX_SBR_ELEMENTS],
            };
            let par = threads.par_with(cm.n_channels, reserve);
            let (r, out_bytes) = run_core(&mut aac, &mut tp, &aac_config, &mut p, &mut cb, par);
            core_out =
                Some((r, out_bytes, fdk_aac_enc_get_bit_reservoir_state(&aac), cb.switched_off));
        });
        let spec = AssertSend(|| {
            let before = par::counters_now();
            let snapshot = Snapshot::take(h);
            // the end of the pending frame, assuming its core succeeds
            finish_frame(h);
            // SAFETY: the caller's guarantee.
            let (next, num_in) =
                unsafe { front_delayed(h, in_buf_desc, out_buf_desc, inargs, Some(cm)) };
            let after = par::counters_now();
            let delta = core::array::from_fn(|k| after[k].wrapping_sub(before[k]));
            spec_out = Some((next, num_in, snapshot, delta));
        });
        join2(Some(&pool), move || core.run(), move || spec.run());
    }
    h.h_aac_enc = Some(aac);
    h.h_tp_enc = Some(tp);
    h.h_mps_enc = mps;
    h.threads = threads;
    h.pipe.pool = Some(pool);
    let (Some((r, out_bytes, bit_res_state, switched_off)), Some((next, num_in, snapshot, delta))) =
        (core_out, spec_out)
    else {
        unreachable!("join2() runs both tasks");
    };

    let result =
        frame_result(&p, r, out_bytes, aac_config.n_sub_frames, &h.out_buffer, bit_res_state);
    let switched = switched_off.iter().any(|&s| s);
    let conflict = switched_off.iter().zip(independent.iter()).any(|(&s, &i)| s && !i);
    if result.ret == AACENC_OK && !conflict {
        // (the switch-off is a no-op, or for elements the front part did not touch)
        if switched {
            switch_off_headers(h, &switched_off);
        }
        if let Some(n) = num_in {
            outargs.num_in_samples = n;
        }
        h.pipe.store(next);
        h.pipe.overlapped += 1;
        return result;
    }

    // Undo the speculative part and continue from the state of the single-threaded encoder.
    drop(next);
    snapshot.restore(h);
    par::counters_sub(&delta);
    if switched {
        switch_off_headers(h, &switched_off);
    }
    if r == AAC_ENC_OK {
        finish_frame(h);
    }
    if result.ret == AACENC_ENCODE_ERROR {
        // All encoder modules have to be initialized
        h.init_flags = AACENC_INIT_ALL;
    }
    // SAFETY: the caller's guarantee.
    let (next, num_in) = unsafe { front_delayed(h, in_buf_desc, out_buf_desc, inargs, None) };
    if let Some(n) = num_in {
        outargs.num_in_samples = n;
    }
    h.pipe.store(next);
    h.pipe.undone += 1;
    result
}
