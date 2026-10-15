// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: per channel parallelism of the decoder (docs/optimization.md, "Per-channel
// parallelism in the decoder"). The functions are rearranged copies of parts of
// CAacDecoder_DecodeFrame() (libAACdec/src/aacdecoder.cpp); the sequential decoder (one thread,
// the default) does not use them.

//! Per channel parallelism of the decoder (opt-in, more than one thread).
//!
//! The decoder's thread pool ([`ChanPar`], owned by stage B like the state it works on, see
//! the module `state` of the pipeline) decodes the channels of a frame at the same time in
//! four places:
//!
//! - the inverse quantization that stage A left to stage B and the element decoding (stereo
//!   tools, PNS, TNS) of stage B, one task per element (`decode_elements_pool`), or the
//!   inverse quantization alone, one task per channel (`inverse_quantize_pool`);
//! - the inverse transform of the channels (`render_channels_pool`): the IMDCT, or the low
//!   delay filter bank of ELD; for these three only frames with at least
//!   `MIN_CORE_CHANNELS` (3) channels;
//! - SBR: the two channels of a stereo element
//!   ([`fdk_aac_rs_sbrdec::sbrdecoder::sbr_decoder_apply_pool`]).
//!
//! The thread count `cap` of a frame is the decoder's thread count when the frame is decoded in
//! the calling thread, and one less when stage B runs on the pipeline's worker thread (stage A
//! of the next frame runs in the calling thread meanwhile). The pool has at most as many
//! threads as the largest `cap` used; a section with `n` tasks uses at most `min(cap, n)`
//! threads.
//!
//! Every function documents why its result is that of the sequential code. Each task of a
//! parallel section has scratch memory of its own where the sequential code shares one buffer
//! between the channels (`pWorkBufferCore1` of the inverse transform, the joint stereo scratch
//! buffer, the QMF work buffer of SBR); buffer poisoning (`FDK_AAC_RS_POISON` of a build with
//! the feature `poison`, bit 8) fills that memory with a pattern before every use, and the
//! shared buffers after the parallel section (`pWorkBufferCore1`) or before every stereo SBR
//! element (the QMF work buffer), so that a channel that read data another channel or an
//! earlier frame left there would change the output. Bit 16 is the negative control: it
//! destroys state the parallel channels read (the overlap buffers of the inverse transform
//! and of the second SBR channel), so every stream that takes a parallel path must change.

use crate::aac_rom::PcmDec;
use crate::aacdec_drc::{aac_decoder_drc_apply, AACDEC_DRC_GAIN_SCALING};
use crate::aacdecoder::{caac_decoder_get_el_channels, AacDecoderInstance, MAX_ELEMENTS, NCH};
use crate::aacdecoder_lib::AAC_DEC_UNKNOWN;
use crate::block::{cblock_frequency_to_time, cblock_frequency_to_time_low_delay};
use crate::channel::{c_channel_element_decode, c_channel_element_inverse_quantize};
use crate::channelinfo::*;
use crate::conceal::{cconcealment_apply, cconcealment_td_fading};
use crate::conceal_types::ConcealState_Mute;
use crate::usacdec_lpd::clpd_render_time_signal;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::common_fix::FixpDbl;
use fdk_aac_rs_fdk::par::ThreadPool;
use fdk_aac_rs_fdk::syslib_channel_map_descr::fdk_ch_map_descr_get_map_value;
use fdk_aac_rs_sbrdec::sbrdecoder::SbrPar;
use std::sync::atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// `AAC_DECODER_ERROR`
type AacDecoderError = i32;

/// `FDK_AAC_RS_POISON` bit: poison the per channel scratch memory (see the module
/// documentation).
pub(crate) const POISON_SCRATCH: u32 = 8;
/// `FDK_AAC_RS_POISON` bit: negative control of [`POISON_SCRATCH`].
pub(crate) const POISON_NEGATIVE: u32 = 16;

/// The length of the spectrum of a channel in `workBufferCore2`.
const SPEC_LEN: usize = 1024;

/// The smallest number of channels of a frame whose core decoding (inverse quantization,
/// element decoding, inverse transform) runs on several threads. The work of a channel is a
/// few microseconds; for two channels the fork, the transfer of the channel data between the
/// cores and the wait for the slower thread cost more than they save (measured on aarch64 and
/// x86-64: stereo AAC-LC, LD and ELD were up to 20% slower), for 5.1 they pay off.
const MIN_CORE_CHANNELS: usize = 3;

/// The thread pool of the per channel parallelism and its scratch memory (owned by stage B).
#[derive(Default)]
pub struct ChanPar {
    /// The pool (created on first use).
    pool: Option<Arc<ThreadPool>>,
    /// `pWorkBufferCore1` of each task of [`render_channels_pool`].
    wb1: Vec<Box<CWorkBufferCore1>>,
    /// The joint stereo scratch buffer of each task of [`decode_elements_pool`].
    scratch: Vec<Vec<FixpDbl>>,
    /// The poisoning mode (`FDK_AAC_RS_POISON` of a test build).
    poison: u32,
    /// The state of the poisoning pattern.
    seed: u64,
}

impl ChanPar {
    /// The pool for `items` tasks with at most `cap` threads (`None`: run them sequentially).
    fn pool(&mut self, cap: usize, items: usize) -> Option<Arc<ThreadPool>> {
        let want = cap.min(items).min(NCH);
        if want < 2 {
            return None;
        }
        if self.pool.as_ref().is_none_or(|p| p.threads() < want) {
            // (a pool that cannot be created leaves the decoder sequential)
            self.pool = Some(Arc::new(ThreadPool::new(want).ok()?));
            self.poison = super::poison_mode();
            self.seed = 0x2545_f491_4f6c_dd1d;
        }
        self.pool.clone()
    }

    /// The next seed of a poisoning pattern.
    fn next_seed(&mut self) -> u64 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.seed | 1
    }
}

/// Frames whose channels [`render_channels_pool`] transformed in parallel (all decoders of
/// the process; for tests).
static PAR_RENDER_FRAMES: AtomicU64 = AtomicU64::new(0);

/// The number of frames whose inverse transform ran on several threads in this process.
pub fn par_render_frames() -> u64 {
    PAR_RENDER_FRAMES.load(Ordering::Relaxed)
}

/// Frames whose elements [`decode_elements_pool`] decoded in parallel (all decoders of the
/// process; for tests).
static PAR_ELEMENT_FRAMES: AtomicU64 = AtomicU64::new(0);

/// The number of frames whose element decoding ran on several threads in this process.
pub fn par_element_frames() -> u64 {
    PAR_ELEMENT_FRAMES.load(Ordering::Relaxed)
}

/// The arguments of [`fdk_aac_rs_sbrdec::sbrdecoder::sbr_decoder_apply_pool`] for a frame
/// with the thread count `cap`, with the pool kept alive by the returned handle.
pub(crate) fn sbr_par(chan_par: &mut ChanPar, cap: usize) -> Option<(Arc<ThreadPool>, u64, bool)> {
    let pool = chan_par.pool(cap, 2)?;
    let poison = if chan_par.poison & POISON_SCRATCH != 0 { chan_par.next_seed() } else { 0 };
    Some((pool, poison, chan_par.poison & POISON_NEGATIVE != 0))
}

/// [`SbrPar`] of the result of [`sbr_par`].
pub(crate) fn as_sbr_par(p: &Option<(Arc<ThreadPool>, u64, bool)>) -> Option<SbrPar<'_>> {
    p.as_ref().map(|(pool, poison, neg)| SbrPar { pool, poison: *poison, poison_overlap: *neg })
}

/// Fills a buffer with a pattern derived from `seed`.
fn poison(buf: &mut [FixpDbl], seed: u64) {
    let mut x = seed | 1;
    for v in buf.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *v = x as FixpDbl;
    }
}

/// The inverse quantization that stage A left to stage B (`iq`: the elements as (first
/// channel, number of channels)) on several threads; `false` without any effect if the frame
/// does not qualify (the caller runs the sequential loop).
///
/// `CChannelElement_InverseQuantize()` of an element reads and writes, per channel, only the
/// channel info and the spectrum of that channel (1024 values from `channel * 1024` in
/// `workBufferCore2`), and reads the sampling rate info: the channels are independent. Every
/// channel of the elements must exist (the element function pairs the present channel infos
/// with the spectra in order, which differs for a missing one).
pub(crate) fn inverse_quantize_pool(
    self_: &mut AacDecoderInstance,
    cap: usize,
    iq: &[(u8, u8)],
) -> bool {
    if cap < 2 {
        return false;
    }
    let mut chans = [0u8; NCH];
    let mut n = 0usize;
    for &(a, k) in iq {
        for c in a as usize..a as usize + k as usize {
            if c >= NCH || n >= NCH || chans[..n].contains(&(c as u8)) {
                return false;
            }
            if self_.p_aac_decoder_channel_info.get(c).is_none_or(|ci| ci.is_none()) {
                return false;
            }
            chans[n] = c as u8;
            n += 1;
        }
    }
    if n < MIN_CORE_CHANNELS {
        return false;
    }
    let Some(wb2) = self_.work_buffer_core2.as_deref_mut() else { return false };
    if chans[..n].iter().any(|&c| (c as usize + 1) * SPEC_LEN > wb2.len()) {
        return false;
    }
    let Some(pool) = self_.chan_par.pool(cap, n) else { return false };

    let mut cis: Vec<Option<&mut CAacDecoderChannelInfo>> =
        self_.p_aac_decoder_channel_info.iter_mut().map(|c| c.as_deref_mut()).collect();
    let mut specs: Vec<Option<&mut [FixpDbl]>> = wb2.chunks_mut(SPEC_LEN).map(Some).collect();
    let mut jobs: Vec<(&mut CAacDecoderChannelInfo, &mut [FixpDbl])> = Vec::with_capacity(n);
    for &c in &chans[..n] {
        let c = c as usize;
        match (cis[c].take(), specs[c].take()) {
            (Some(ci), Some(spec)) => jobs.push((ci, spec)),
            _ => unreachable!("checked above"),
        }
    }
    let sri = &self_.sampling_rate_info[0];
    run_jobs(&pool, cap, &mut [(); NCH], jobs, |_, _, (ci, spec)| {
        c_channel_element_inverse_quantize(&mut [ci], sri, spec);
    });
    true
}

/// Runs `f(i, scratch, job)` for every job `i` on at most `min(cap, scratch.len())` threads of
/// the pool (the calling thread included), each with its own scratch memory; the threads take
/// the jobs in their order.
fn run_jobs<J: Send, S: Send>(
    pool: &ThreadPool,
    cap: usize,
    scratch: &mut [S],
    jobs: Vec<J>,
    f: impl Fn(usize, &mut S, J) + Sync,
) {
    let n = jobs.len();
    let k = n.min(cap).min(scratch.len()).min(pool.threads());
    let jobs: Vec<Mutex<Option<J>>> = jobs.into_iter().map(|j| Mutex::new(Some(j))).collect();
    let next = AtomicUsize::new(0);
    pool.for_each_mut(&mut scratch[..k], |_, s| loop {
        let i = next.fetch_add(1, Ordering::Relaxed);
        if i >= n {
            break;
        }
        let job = jobs[i].lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(job) = job {
            f(i, s, job);
        }
    });
}

/// One element of [`decode_elements_pool`].
struct ElementJob<'a> {
    p: Vec<&'a mut CAacDecoderChannelInfo>,
    ps: Vec<&'a mut CAacDecoderStaticChannelInfo>,
    spec: &'a mut [FixpDbl],
    com: &'a mut CAacDecoderCommonData,
    com_r: Option<&'a mut CAacDecoderCommonData>,
    com_static: &'a mut CAacDecoderCommonStaticData,
    cpe: Option<&'a mut CpePersistentData>,
    el: usize,
    n: usize,
    iq: bool,
    seed: i32,
}

/// The inverse quantization that stage A left to stage B (`iq`: the elements as (first
/// channel, number of channels), either empty or the elements of `dl`) and the element decode
/// loop (`dl`: (element index, first channel, number of channels)) of stage B, one task per
/// element; `false` without any effect if the frame does not qualify (the caller runs the
/// sequential loops).
///
/// `CChannelElement_Decode()` of an element (stereo tools, PNS, TNS, scaling; the inverse
/// quantization comes first, see [`inverse_quantize_pool`]) reads and writes the channel
/// infos, static channel infos and spectra of the element's channels, the element's
/// `pComData` (one, or one per channel), `pComStaticData`, `pCpeStaticData` and the PNS seed
/// `*currentSeed` (the `pnsCurrentSeed` of a static channel info, see
/// [`crate::channelinfo::aacdec_pns_types`], copied in and out by the caller), and reads the
/// sampling rate info and the flags. The joint stereo scratch buffer (`pTimeData`) is used
/// only by complex stereo prediction (USAC, not decoded here). The elements are independent if
/// these are different for every element (the channels are: the elements of a frame have
/// disjoint channels); the seeds are read before and written after the tasks in element order.
pub(crate) fn decode_elements_pool(
    self_: &mut AacDecoderInstance,
    cap: usize,
    iq: &[(u8, u8)],
    dl: &[(u8, u8, u8)],
) -> bool {
    let ne = dl.len();
    if cap < 2 || ne < 2 || ne > MAX_ELEMENTS {
        return false;
    }
    if dl.iter().map(|d| d.2 as usize).sum::<usize>() < MIN_CORE_CHANNELS {
        return false;
    }
    if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA | AC_DRM) != 0 {
        return false;
    }
    let fused = !iq.is_empty();
    if fused && (iq.len() != ne || iq.iter().zip(dl).any(|(q, d)| q.0 != d.1 || q.1 != d.2)) {
        return false;
    }
    let wb2_len = self_.work_buffer_core2.as_deref().map_or(0, |w| w.len());
    // (seed channel, CPE data, common data 0, common data 1, common static data) per element
    let mut res = [(None::<usize>, None::<usize>, 0usize, None::<usize>, 0usize); MAX_ELEMENTS];
    let mut used_ch = 0u32;
    let (mut used_seed, mut used_cpe, mut used_com, mut used_cs) = (0u64, 0u64, 0u64, 0u64);
    let mark = |set: &mut u64, i: usize| -> bool {
        if i >= 64 || *set & (1 << i) != 0 {
            return false;
        }
        *set |= 1 << i;
        true
    };
    for (e, &(el, a, n)) in dl.iter().enumerate() {
        let (el, a, n) = (el as usize, a as usize, n as usize);
        if el >= MAX_ELEMENTS || n == 0 || n > 2 || a + n > NCH || (a + n) * SPEC_LEN > wb2_len {
            return false;
        }
        for c in a..a + n {
            if used_ch & (1 << c) != 0
                || self_.p_aac_decoder_channel_info[c].is_none()
                || self_.p_aac_decoder_static_channel_info.get(c).is_none_or(|s| s.is_none())
            {
                return false;
            }
            used_ch |= 1 << c;
        }
        let seed = self_.p_aac_decoder_channel_info[a]
            .as_deref()
            .and_then(|c| c.data.aac.pns_data.current_seed)
            .filter(|&s| {
                self_.p_aac_decoder_static_channel_info.get(s).is_some_and(|s| s.is_some())
            });
        if let Some(s) = seed {
            if !mark(&mut used_seed, s) {
                return false;
            }
        }
        let cpe = self_.p_aac_decoder_static_channel_info[a]
            .as_deref()
            .and_then(|s| s.p_cpe_static_data)
            .filter(|&i| self_.cpe_static_data.get(i).is_some_and(|c| c.is_some()));
        if let Some(i) = cpe {
            if !mark(&mut used_cpe, i) {
                return false;
            }
        }
        let Some(i0) = self_.p_com_data[a].filter(|&i| i < self_.com_data.len()) else {
            return false;
        };
        let i1 = if n == 2 { self_.p_com_data[a + 1] } else { None }.filter(|&j| j != i0);
        if !mark(&mut used_com, i0) {
            return false;
        }
        if let Some(j) = i1 {
            if j >= self_.com_data.len() || !mark(&mut used_com, j) {
                return false;
            }
        }
        let Some(cs) = self_.p_com_static_data[a].filter(|&i| i < self_.com_static_data.len())
        else {
            return false;
        };
        if !mark(&mut used_cs, cs) {
            return false;
        }
        res[e] = (seed, cpe, i0, i1, cs);
    }
    let Some(pool) = self_.chan_par.pool(cap, ne) else { return false };

    let mut cis: Vec<Option<&mut CAacDecoderChannelInfo>> =
        self_.p_aac_decoder_channel_info.iter_mut().map(|c| c.as_deref_mut()).collect();
    let mut scis: Vec<Option<&mut CAacDecoderStaticChannelInfo>> =
        self_.p_aac_decoder_static_channel_info.iter_mut().map(|c| c.as_deref_mut()).collect();
    let mut coms: Vec<Option<&mut CAacDecoderCommonData>> =
        self_.com_data.iter_mut().map(|c| Some(&mut **c)).collect();
    let mut css: Vec<Option<&mut CAacDecoderCommonStaticData>> =
        self_.com_static_data.iter_mut().map(|c| Some(&mut **c)).collect();
    let mut cpes: Vec<Option<&mut CpePersistentData>> =
        self_.cpe_static_data.iter_mut().map(|c| c.as_deref_mut()).collect();
    let seeds: Vec<i32> = res[..ne]
        .iter()
        .map(|r| r.0.and_then(|s| scis[s].as_deref()).map_or(0, |s| s.pns_current_seed))
        .collect();
    // The spectra of the elements' channels (disjoint ranges of workBufferCore2).
    let wb2 = self_.work_buffer_core2.as_deref_mut().expect("checked above");
    let mut specs: Vec<Option<&mut [FixpDbl]>> = (0..ne).map(|_| None).collect();
    {
        let mut order: Vec<usize> = (0..ne).collect();
        order.sort_by_key(|&e| dl[e].1);
        let mut rest: &mut [FixpDbl] = wb2;
        let mut pos = 0usize;
        for e in order {
            let (start, len) = (dl[e].1 as usize * SPEC_LEN, dl[e].2 as usize * SPEC_LEN);
            let r = core::mem::take(&mut rest);
            let (_, r) = r.split_at_mut(start - pos);
            let (s, r) = r.split_at_mut(len);
            specs[e] = Some(s);
            rest = r;
            pos = start + len;
        }
    }
    let mut jobs: Vec<ElementJob> = Vec::with_capacity(ne);
    for (e, &(el, a, n)) in dl.iter().enumerate() {
        let (el, a, n) = (el as usize, a as usize, n as usize);
        let (_, cpe, i0, i1, cs) = res[e];
        let spec = specs[e].take().expect("checked above");
        jobs.push(ElementJob {
            p: (a..a + n).map(|c| cis[c].take().expect("checked above")).collect(),
            ps: (a..a + n).map(|c| scis[c].take().expect("checked above")).collect(),
            spec,
            com: coms[i0].take().expect("checked above"),
            com_r: i1.map(|j| coms[j].take().expect("checked above")),
            com_static: css[cs].take().expect("checked above"),
            cpe: cpe.map(|i| cpes[i].take().expect("checked above")),
            el,
            n,
            iq: fused,
            seed: seeds[e],
        });
    }
    let cp = &mut self_.chan_par;
    while cp.scratch.len() < ne.min(cap) {
        cp.scratch.push(vec![0; 2 * SPEC_LEN]);
    }
    let poison_seed = if cp.poison & POISON_SCRATCH != 0 { Some(cp.next_seed()) } else { None };
    let out_seeds: Vec<AtomicI32> = (0..ne).map(|_| AtomicI32::new(0)).collect();
    let sri = &self_.sampling_rate_info[0];
    let flags = self_.flags[0];
    let el_flags = &self_.el_flags;
    run_jobs(&pool, cap, &mut cp.scratch[..], jobs, |i, scratch, j| {
        let ElementJob { mut p, mut ps, spec, com, com_r, com_static, cpe, el, n, iq, mut seed } =
            j;
        if let Some(s) = poison_seed {
            poison(scratch, s.wrapping_add(i as u64));
        }
        if iq {
            c_channel_element_inverse_quantize(&mut p, sri, spec);
        }
        c_channel_element_decode(
            &mut p,
            &mut ps,
            sri,
            flags,
            el_flags[el],
            n as i32,
            spec,
            com,
            com_r,
            com_static,
            cpe,
            scratch,
            &mut seed,
        );
        out_seeds[i].store(seed, Ordering::Relaxed);
    });
    for e in 0..ne {
        if let Some(s) = res[e].0 {
            if let Some(s) = self_.p_aac_decoder_static_channel_info[s].as_deref_mut() {
                s.pns_current_seed = out_seeds[e].load(Ordering::Relaxed);
            }
        }
    }
    PAR_ELEMENT_FRAMES.fetch_add(1, Ordering::Relaxed);
    true
}

/// One channel of [`render_channels_pool`].
struct RenderJob<'a> {
    ci: &'a mut CAacDecoderChannelInfo,
    sci: &'a mut CAacDecoderStaticChannelInfo,
    out: &'a mut [PcmDec],
    spec: &'a mut [FixpDbl],
    frame_ok: i32,
    el_flags: u32,
    el_ch: i32,
}

/// The inverse transform of one channel (the `switch (renderMode)` of the channel loop of
/// `CAacDecoder_DecodeFrame()`); returns the `extGainDelay` it sets, `None` for an unknown
/// render mode (`AAC_DEC_UNKNOWN`).
#[allow(clippy::too_many_arguments)]
fn render_one(
    sci: &mut CAacDecoderStaticChannelInfo,
    ci: &mut CAacDecoderChannelInfo,
    out: &mut [PcmDec],
    spf: i32,
    frame_ok: i32,
    wb1: &mut CWorkBufferCore1,
    headroom: i32,
    el_flags: u32,
    el_ch: i32,
    spec: &mut [FixpDbl],
    sri: &SamplingRateInfo,
    flags: u32,
    ac_flags: u32,
) -> Option<u32> {
    match ci.render_mode {
        AACDEC_RENDER_IMDCT => {
            cblock_frequency_to_time(
                sci,
                ci,
                out,
                spf as i16,
                frame_ok,
                &mut wb1.buf[..],
                headroom,
                el_flags,
                el_ch,
                spec,
            );
            Some(spf as u32)
        }
        AACDEC_RENDER_ELDFB => {
            cblock_frequency_to_time_low_delay(sci, ci, out, spf as i16, spec);
            Some(((spf * 2 - spf / 2 - 1) / 2) as u32)
        }
        AACDEC_RENDER_LPD => {
            clpd_render_time_signal(
                sci,
                ci,
                out,
                spf,
                sri,
                frame_ok as u32,
                headroom,
                flags,
                ac_flags,
                spec,
                wb1,
            );
            Some(spf as u32)
        }
        _ => None,
    }
}

/// The channel loop of the inverse transform of `CAacDecoder_DecodeFrame()` (concealment, DRC,
/// inverse transform and time domain fading of every channel) with the inverse transforms on
/// several threads. Returns the error status after the loop, or `None` without any effect if
/// the frame does not qualify (the caller runs the sequential loop).
///
/// The sequential loop does, for channel `c` in order: the check of the ICS info (which can
/// clear `frameOK` for the following channels), the concealment of the spectrum (static
/// channel info `reverseChMapping[c]`), the DRC (which writes `extGain[0]`, the DRC state and
/// the channel's SBR DRC data), the inverse transform (with `frameOK` at that point, static
/// channel info `reverseChMapping[c]`, the shared scratch buffer `pWorkBufferCore1`, output at
/// the channel's offset) and the time domain fading (static channel info `c`, which also
/// copies its noise seed to channel `c + 1`). With the identity channel mapping, every step of
/// channel `c` reads and writes channel `c`'s channel info, static channel info, spectrum and
/// output; the steps before the inverse transform also read and write state shared by the
/// channels (`frameOK`, `extGain`, DRC, SBR) that the inverse transform and the fading do not
/// touch; the fading reads and writes the concealment state of channel `c`, which the steps of
/// the channels after `c` do not touch, except the noise seed, which only the fading reads. So
/// the loop can run as three passes: concealment and DRC of all channels in order (recording
/// the `frameOK` value of each), the inverse transforms (on several threads, each with a
/// scratch buffer of its own: the transforms read only what they wrote in the scratch buffer,
/// checked by poisoning), the fading of all channels in order.
///
/// Qualify: calls without flags, outside the flushing of a configuration change, with the
/// identity channel mapping, at least [`MIN_CORE_CHANNELS`] channels, every channel info
/// present, distinct output offsets of channels that fit into the output buffer, and the inverse
/// transforms IMDCT (without the LPD to FD transition of USAC, whose post filter may write more
/// than one frame) or the ELD filter bank. If a channel turns out not to qualify after the concealment
/// (which may change the render mode), the transforms run one after the other in the calling
/// thread.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_channels_pool(
    self_: &mut AacDecoderInstance,
    cap: usize,
    flags: u32,
    element_count: i32,
    channel_elements: &[Mp4ElementId; MAX_ELEMENTS],
    aac_channels: i32,
    reverse_ch_mapping: &[u8; 16],
    p_time_data: &mut [PcmDec],
    time_data_size: i32,
    time_data_channel_offset: i32,
    td_fading_applied: &mut [i32; NCH],
    mut error_status: AacDecoderError,
) -> Option<AacDecoderError> {
    if cap < 2 || flags != 0 || (self_.flush_status != 0 && self_.flush_cnt > 0) {
        return None;
    }
    let spf = self_.stream_info.aac_samples_per_frame;
    let tdco = time_data_channel_offset;
    if spf <= 0 || spf as usize > SPEC_LEN || tdco < spf {
        return None;
    }
    if time_data_size < tdco.wrapping_mul(self_.aac_channels) {
        return None;
    }
    if element_count < 0 || element_count as usize > MAX_ELEMENTS {
        return None;
    }

    // The channels: (element, channel in the element, output chunk).
    let mut chans = [(0u8, 0u8, 0u8); NCH];
    let mut n = 0usize;
    let mut used_chunks = 0u32;
    let mut el_channels_acc: i32 = 0;
    for el in 0..element_count as usize {
        let type_: Mp4ElementId = if self_.flags[0] & (AC_USAC | AC_RSV603DA | AC_BSAC) != 0 {
            self_.elements[el]
        } else {
            channel_elements[el]
        };
        let n_element_channels =
            caac_decoder_get_el_channels(type_, self_.usac_stereo_config_index[el]);
        el_channels_acc += n_element_channels;
        let mut el_ch = 0u8;
        while (n as i32) < el_channels_acc {
            let c = n;
            if c as i32 >= aac_channels || c >= NCH || reverse_ch_mapping[c] as usize != c {
                return None;
            }
            if self_.p_aac_decoder_channel_info[c].is_none()
                || self_.p_aac_decoder_static_channel_info.get(c).is_none_or(|s| s.is_none())
            {
                return None;
            }
            let m = fdk_ch_map_descr_get_map_value(
                &self_.map_descr,
                c as u8,
                self_.ch_map_index as u32,
            ) as usize;
            if m >= 32
                || used_chunks & (1 << m) != 0
                || m * tdco as usize + spf as usize > p_time_data.len()
            {
                return None;
            }
            used_chunks |= 1 << m;
            chans[c] = (el as u8, el_ch, m as u8);
            n += 1;
            el_ch += 1;
        }
    }
    if n < MIN_CORE_CHANNELS {
        return None;
    }
    let wb2_len = self_.work_buffer_core2.as_deref().map_or(0, |w| w.len());
    if n * SPEC_LEN > wb2_len {
        return None;
    }
    let pool = self_.chan_par.pool(cap, n)?;

    // Pass 1: ICS check, concealment and DRC of every channel, in order.
    let mut frame_ok_c = [0i32; NCH];
    let mut frame_ok_but_conceal = 0;
    for c in 0..n {
        if c == 0 || chans[c].0 != chans[c - 1].0 {
            /* Force frame concealment during mute release active state. */
            frame_ok_but_conceal = 0;
        }
        let (Some(ci), Some(sci)) = (
            self_.p_aac_decoder_channel_info[c].as_deref_mut(),
            self_.p_aac_decoder_static_channel_info[c].as_deref_mut(),
        ) else {
            unreachable!("checked above");
        };
        {
            let h_concealment_info = &sci.concealment_info;
            let mute_release_active = self_.frame_ok != 0
                && ((h_concealment_info.conceal_state >= ConcealState_Mute)
                    && (h_concealment_info.cnt_valid_frames + 1
                        <= self_.conceal_common_data.num_mute_release_frames));
            let ics_is_invalid = get_scale_factor_bands_transmitted(&ci.ics_info)
                > get_scale_factor_bands_total(&ci.ics_info);
            let ics_info_usedin_fade_out =
                !(ci.render_mode == AACDEC_RENDER_LPD && sci.last_lpd_mode == 0);
            if ics_info_usedin_fade_out && ics_is_invalid && !mute_release_active {
                self_.frame_ok = 0;
            }
        }
        let wb2 = self_.work_buffer_core2.as_deref_mut().expect("checked above");
        let spec = &mut wb2[c * SPEC_LEN..];
        let last_lpd_mode = sci.last_lpd_mode;
        let rc = cconcealment_apply(
            ci,
            sci,
            &self_.sampling_rate_info[0],
            spf,
            last_lpd_mode,
            (self_.frame_ok != 0) as i32,
            self_.flags[0],
            &self_.conceal_common_data,
            spec,
        );
        if rc == -1 {
            frame_ok_but_conceal = 1;
        }
        self_.ext_gain[0] = AACDEC_DRC_GAIN_SCALING as FixpDbl;
        if let Some(drc) = self_.h_drc_info.as_deref_mut() {
            let mut drc_data = sci.drc_data;
            aac_decoder_drc_apply(
                drc,
                self_.h_sbr_decoder.as_deref_mut(),
                ci,
                &mut drc_data,
                Some(&mut self_.ext_gain[0]),
                c as i32,
                spf,
                self_.sbr_enabled as i32,
                spec,
            );
            sci.drc_data = drc_data;
        }
        frame_ok_c[c] = (self_.frame_ok != 0 && frame_ok_but_conceal == 0) as i32;
    }

    // Pass 2: the inverse transforms.
    let parallel = (0..n).all(|c| {
        let ci = self_.p_aac_decoder_channel_info[c].as_deref().expect("checked above");
        let sci = self_.p_aac_decoder_static_channel_info[c].as_deref().expect("checked above");
        (ci.render_mode == AACDEC_RENDER_IMDCT && sci.last_core_mode != LPD)
            || ci.render_mode == AACDEC_RENDER_ELDFB
    });
    let headroom = self_.aac_out_data_headroom;
    let ac_flags = self_.flags[0];
    let mut ext_gain_delay: Option<u32> = None;
    if parallel {
        let cp = &mut self_.chan_par;
        let k = n.min(cap).min(pool.threads());
        while cp.wb1.len() < k {
            cp.wb1.push(Box::default());
        }
        let poison_seed = if cp.poison & POISON_SCRATCH != 0 { Some(cp.next_seed()) } else { None };
        let negative = cp.poison & POISON_NEGATIVE != 0;
        let mut cis: Vec<Option<&mut CAacDecoderChannelInfo>> =
            self_.p_aac_decoder_channel_info.iter_mut().map(|c| c.as_deref_mut()).collect();
        let mut scis: Vec<Option<&mut CAacDecoderStaticChannelInfo>> =
            self_.p_aac_decoder_static_channel_info.iter_mut().map(|c| c.as_deref_mut()).collect();
        let wb2 = self_.work_buffer_core2.as_deref_mut().expect("checked above");
        let mut specs: Vec<Option<&mut [FixpDbl]>> = wb2.chunks_mut(SPEC_LEN).map(Some).collect();
        let mut outs: Vec<Option<&mut [PcmDec]>> =
            p_time_data.chunks_mut(tdco as usize).map(Some).collect();
        let mut jobs: Vec<RenderJob> = Vec::with_capacity(n);
        for c in 0..n {
            let (el, el_ch, m) = chans[c];
            let (Some(ci), Some(sci), Some(spec), Some(out)) =
                (cis[c].take(), scis[c].take(), specs[c].take(), outs[m as usize].take())
            else {
                unreachable!("checked above");
            };
            if negative {
                poison(&mut sci.i_mdct.overlap, 0x0bad_5eed);
            }
            jobs.push(RenderJob {
                ci,
                sci,
                out,
                spec,
                frame_ok: frame_ok_c[c],
                el_flags: self_.el_flags[el as usize],
                el_ch: el_ch as i32,
            });
        }
        let sri = &self_.sampling_rate_info[0];
        run_jobs(&pool, cap, &mut cp.wb1[..k], jobs, |i, wb1, job| {
            let RenderJob { ci, sci, out, spec, frame_ok, el_flags, el_ch } = job;
            if let Some(s) = poison_seed {
                poison(&mut wb1.buf, s.wrapping_add(i as u64));
            }
            let r = render_one(
                sci, ci, out, spf, frame_ok, wb1, headroom, el_flags, el_ch, spec, sri, flags,
                ac_flags,
            );
            debug_assert!(r.is_some());
        });
        // (every channel renders with IMDCT or the ELD filter bank, see above)
        for c in 0..n {
            let ci = self_.p_aac_decoder_channel_info[c].as_deref().expect("checked above");
            ext_gain_delay = Some(if ci.render_mode == AACDEC_RENDER_ELDFB {
                ((spf * 2 - spf / 2 - 1) / 2) as u32
            } else {
                spf as u32
            });
        }
        if let (Some(s), Some(w)) = (poison_seed, self_.p_work_buffer_core1.as_deref_mut()) {
            // (the shared buffer the parallel transforms did not use)
            poison(&mut w.buf, s ^ 0x5555);
        }
        PAR_RENDER_FRAMES.fetch_add(1, Ordering::Relaxed);
    } else {
        for c in 0..n {
            let (el, el_ch, m) = chans[c];
            let offset = m as usize * tdco as usize;
            let mut tmp_wb: Option<Box<CWorkBufferCore1>> = None;
            let wb1: &mut CWorkBufferCore1 = match self_.p_work_buffer_core1.as_deref_mut() {
                Some(w) => w,
                None => tmp_wb.insert(Box::<CWorkBufferCore1>::default()),
            };
            let (Some(ci), Some(sci)) = (
                self_.p_aac_decoder_channel_info[c].as_deref_mut(),
                self_.p_aac_decoder_static_channel_info[c].as_deref_mut(),
            ) else {
                unreachable!("checked above");
            };
            let wb2 = self_.work_buffer_core2.as_deref_mut().expect("checked above");
            match render_one(
                sci,
                ci,
                &mut p_time_data[offset..],
                spf,
                frame_ok_c[c],
                wb1,
                headroom,
                self_.el_flags[el as usize],
                el_ch as i32,
                &mut wb2[c * SPEC_LEN..],
                &self_.sampling_rate_info[0],
                flags,
                ac_flags,
            ) {
                Some(d) => ext_gain_delay = Some(d),
                None => error_status = AAC_DEC_UNKNOWN,
            }
        }
    }
    if let Some(d) = ext_gain_delay {
        self_.ext_gain_delay = d;
    }

    // Pass 3: the time domain fading of every channel, in order.
    for c in 0..n {
        if td_fading_applied[c] == 0 {
            let offset = chans[c].2 as usize * tdco as usize;
            if let Some(sci_c) = self_.p_aac_decoder_static_channel_info[c].as_deref_mut() {
                td_fading_applied[c] = cconcealment_td_fading(
                    spf,
                    sci_c,
                    &self_.conceal_common_data,
                    headroom,
                    &mut p_time_data[offset..],
                );
            }
            if c + 1 < 8 && (c as i32) < aac_channels - 1 {
                /* update next TDNoise Seed to avoid muting in case of Parametric Stereo */
                let seed = self_.p_aac_decoder_static_channel_info[c]
                    .as_deref()
                    .map(|s| s.concealment_info.td_noise_seed);
                if let (Some(seed), Some(next)) =
                    (seed, self_.p_aac_decoder_static_channel_info[c + 1].as_deref_mut())
                {
                    next.concealment_info.td_noise_seed = seed;
                }
            }
        }
    }
    Some(error_status)
}
