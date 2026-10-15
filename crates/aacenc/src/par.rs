// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not a port: the reference encoder is single-threaded.

//! Opt-in multi-threading of the encoder (PORTING.md, "Optimization rules").
//!
//! An encoder with a thread count above 1 ([`crate::aacenc_lib::aac_enc_set_threads`]) owns a
//! [`ThreadPool`] (created with the first frame) and runs independent per-channel work of a
//! frame on it. With one thread (the default) the sequential code runs unchanged. The output is
//! identical for every thread count; each parallel variant documents why its tasks are
//! independent:
//!
//! - [`crate::qc_main::fdk_aac_enc_qc_main_pool`]: the scale factor estimation, the first
//!   quantization and the first (speculative) bit count of every channel.
//! - [`crate::psy_main::fdk_aac_enc_psy_main_pool`]: the per-channel parts of the psychoacoustic
//!   model of a channel pair.
//! - [`crate::aacenc::fdk_aac_enc_encode_frame_pool`]: with more than one audio element, the
//!   psychoacoustic model and `FDKaacEnc_QCMainPrepare()` of every element as one task (then
//!   the per-channel parts of a pair run sequentially within the task: nested use of the pool
//!   runs on the calling thread).
//!
//! With SBR, the frame pipelining of [`crate::aacenc_lib`] (module `pipeline`) runs the core of
//! the previous frame and the front part of the call (with the SBR encoder) at the same time
//! (`join2` on a pool of two threads), and the core's pool gets one thread less.
//!
//! Whether a frame uses the pool and the SBR pipelining is the *mode* of the frame
//! (`EncThreads::mode`), chosen by measurement ([`fdk_aac_rs_fdk::par::tune`]): the pool's
//! workers spin between the parallel sections, and for some configurations and machines the
//! parallel sections are too small to make the encoding faster (stereo AAC-LC without the
//! afterburner on x86-64 was slower with the pool). Every mode encodes exactly as the
//! single-threaded encoder; a frame without the pipelining is encoded as a call that cannot be
//! pipelined (the output delay stays).
//!
//! `EncCarry` carries the thread-local counters of this crate (and of `fdk`) and the spectrum
//! hook of the psychoacoustic model between the calling thread and the workers.

use crate::dyn_bits::{BitcntrState, SectionData};
use crate::psy_main::{self, PsyDynamic};
use fdk_aac_rs_fdk::common_fix::FixpDbl;
pub use fdk_aac_rs_fdk::par::ThreadPool;
use fdk_aac_rs_fdk::par::{Carry, CounterCarry, FdkCarry, Tuner, Tuning};
use std::time::Instant;

/// A thread-local counter: `(get, add)`.
type Counter = (fn() -> u64, fn(u64));

/// The thread-local counters of this crate.
const COUNTERS: [Counter; 11] = [
    (crate::bit_cnt::huff_rom_outside_reads, crate::bit_cnt::add_huff_rom_outside_reads),
    (crate::qc_main::channel_element_write_calls, crate::qc_main::add_channel_element_write_calls),
    (
        crate::metadata_main::metadata_input_overruns,
        crate::metadata_main::add_metadata_input_overruns,
    ),
    (psy_main::psy_main_stale_element_mappings, psy_main::add_psy_main_stale_element_mappings),
    (psy_main::psy_main_negative_lowpass_clears, psy_main::add_psy_main_negative_lowpass_clears),
    (psy_main::psy_main_negative_final_shifts, psy_main::add_psy_main_negative_final_shifts),
    (psy_main::psy_main_stale_short_blocks, psy_main::add_psy_main_stale_short_blocks),
    (psy_main::psy_main_mixed_block_pairs, psy_main::add_psy_main_mixed_block_pairs),
    (psy_main::psy_main_undefined_spectrum_reads, psy_main::add_psy_main_undefined_spectrum_reads),
    (psy_main::psy_main_lfe_flag_pairs, psy_main::add_psy_main_lfe_flag_pairs),
    (crate::adj_thr::adj_thr_stale_ah_flag_reads, crate::adj_thr::add_adj_thr_stale_ah_flag_reads),
];

/// The [`Carry`] of the encoder: the counters of `fdk` ([`FdkCarry`]) and of this crate, and the
/// spectrum hook of the calling thread ([`psy_main::psy_main_set_spectrum_hook`]), which is
/// installed on the worker for the task.
pub(crate) struct EncCarry {
    fdk: FdkCarry,
    counters: [CounterCarry; COUNTERS.len()],
    hook: Option<fn(&mut [FixpDbl])>,
}

impl EncCarry {
    /// The carry of the calling thread.
    pub(crate) fn new() -> Self {
        EncCarry {
            fdk: FdkCarry::default(),
            counters: Default::default(),
            hook: psy_main::psy_main_spectrum_hook(),
        }
    }
}

/// Restores the worker's spectrum hook (also when the task panics).
struct HookGuard(Option<fn(&mut [FixpDbl])>);

impl Drop for HookGuard {
    fn drop(&mut self) {
        psy_main::psy_main_set_spectrum_hook(self.0);
    }
}

impl Carry for EncCarry {
    fn run_on_worker(&self, task: &mut dyn FnMut()) {
        let _hook = HookGuard(psy_main::psy_main_spectrum_hook());
        psy_main::psy_main_set_spectrum_hook(self.hook);
        let before = COUNTERS.map(|(get, _)| get());
        self.fdk.run_on_worker(task);
        for (k, (get, _)) in COUNTERS.iter().enumerate() {
            let added = get().wrapping_sub(before[k]);
            if added != 0 {
                self.counters[k].add(added);
            }
        }
    }

    fn finish(&self) {
        self.fdk.finish();
        for (k, (_, add)) in COUNTERS.iter().enumerate() {
            self.counters[k].finish(*add);
        }
    }
}

/// The values of the thread-local counters of this crate and of `fdk` on this thread (see
/// [`counters_sub`]).
pub(crate) fn counters_now() -> [u64; COUNTERS.len() + 4] {
    let mut v = [0u64; COUNTERS.len() + 4];
    for (k, (get, _)) in COUNTERS.iter().enumerate() {
        v[k] = get();
    }
    let n = COUNTERS.len();
    v[n] = fdk_aac_rs_fdk::fdk_assert::violations();
    let (w, r, s) = fdk_aac_rs_fdk::mdct::mdct_block_dropped_accesses();
    v[n + 1] = w;
    v[n + 2] = r;
    v[n + 3] = s;
    v
}

/// Takes `delta` (a difference of two [`counters_now`]) off the counters of this thread: the
/// frame pipelining undoes what a discarded speculative step counted.
pub(crate) fn counters_sub(delta: &[u64; COUNTERS.len() + 4]) {
    let neg = |x: u64| 0u64.wrapping_sub(x);
    for (k, (_, add)) in COUNTERS.iter().enumerate() {
        if delta[k] != 0 {
            add(neg(delta[k]));
        }
    }
    let n = COUNTERS.len();
    if delta[n] != 0 {
        fdk_aac_rs_fdk::fdk_assert::add_violations(neg(delta[n]));
    }
    if delta[n + 1..].iter().any(|&x| x != 0) {
        fdk_aac_rs_fdk::mdct::add_mdct_block_dropped_accesses((
            neg(delta[n + 1]),
            neg(delta[n + 2]),
            neg(delta[n + 3]),
        ));
    }
}

/// Marks a value as safe to move to the other task of [`join2`]. The frame pipelining moves
/// structures that hold raw pointers (to buffers they own, or to the caller's buffers of the
/// running call) to one task for the duration of a fork; no other thread accesses them then.
pub(crate) struct AssertSend<T>(pub T);

// SAFETY: see the type documentation; every use states which task owns the value.
unsafe impl<T> Send for AssertSend<T> {}

impl<F: FnOnce()> AssertSend<F> {
    /// Calls the wrapped task (a method, so that a closure calling it captures the wrapper).
    pub(crate) fn run(self) {
        (self.0)()
    }
}

/// Runs `a` and `b`, on `pool` (two threads) at the same time if given, otherwise `a` and then
/// `b` on the calling thread, and returns when both returned. The thread-local counters and the
/// spectrum hook are carried ([`EncCarry`]); a panic is propagated after both finished.
pub(crate) fn join2<A: FnOnce() + Send, B: FnOnce() + Send>(pool: Option<&ThreadPool>, a: A, b: B) {
    match pool {
        None => {
            a();
            b();
        }
        Some(pool) => {
            let a = std::sync::Mutex::new(Some(a));
            let b = std::sync::Mutex::new(Some(b));
            pool.for_each_carry(&EncCarry::new(), 2, |i| {
                if i == 0 {
                    let f = a.lock().unwrap_or_else(|e| e.into_inner()).take();
                    if let Some(f) = f {
                        f();
                    }
                } else {
                    let f = b.lock().unwrap_or_else(|e| e.into_inner()).take();
                    if let Some(f) = f {
                        f();
                    }
                }
            });
        }
    }
}

/// Runs `f(i, &mut items[i])` for every item: on `pool` if given (with [`EncCarry`]), otherwise
/// sequentially in index order.
#[inline]
pub(crate) fn for_each_mut<T: Send, F: Fn(usize, &mut T) + Sync>(
    pool: Option<&ThreadPool>,
    items: &mut [T],
    f: F,
) {
    match pool {
        Some(pool) => pool.for_each_mut_carry(&EncCarry::new(), items, f),
        None => {
            for (i, item) in items.iter_mut().enumerate() {
                f(i, item);
            }
        }
    }
}

/// Scratch memory of one task of [`crate::qc_main::fdk_aac_enc_qc_main_pool`].
#[derive(Debug)]
pub struct ChScratch {
    /// The bit counter state of the task (scratch, see `fdk_aac_enc_qc_main_pool`).
    pub(crate) bit_counter: BitcntrState,
    /// The section data of a speculative bit count.
    pub(crate) section_data: SectionData,
    /// `quantSpec` and `maxValueInSfb` of the first quantization.
    pub(crate) quant_spec: [i16; 1024],
    pub(crate) max_value_in_sfb: [u32; crate::psy_const::MAX_GROUPED_SFB],
}

impl Default for ChScratch {
    fn default() -> Self {
        ChScratch {
            bit_counter: Default::default(),
            section_data: Default::default(),
            quant_spec: [0; 1024],
            max_value_in_sfb: [0; crate::psy_const::MAX_GROUPED_SFB],
        }
    }
}

/// What the parallel variants of a frame use: the pool and scratch memory.
#[derive(Debug)]
pub struct EncPar<'a> {
    pub pool: &'a ThreadPool,
    pub scratch: &'a mut Vec<ChScratch>,
    /// The `PSY_DYNAMIC` scratch of the elements of [`crate::aacenc`]'s parallel
    /// psychoacoustics.
    pub psy_dynamic: &'a mut Vec<Box<PsyDynamic>>,
}

/// A thread mode of the encoder: the SBR frame pipelining of a call (with SBR and the output
/// delay), the pool of the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EncMode {
    pub pipeline: bool,
    pub pool: bool,
}

const SEQ: EncMode = EncMode { pipeline: false, pool: false };
const PIPE: EncMode = EncMode { pipeline: true, pool: false };
const POOL: EncMode = EncMode { pipeline: false, pool: true };
const BOTH: EncMode = EncMode { pipeline: true, pool: true };

/// The thread modes (see [`fdk_aac_rs_fdk::par::tune`]), cheapest first, of an encoder without
/// SBR pipelining: without a pool for the core, one mode.
const MODES_SEQ: [EncMode; 1] = [SEQ];
/// ... with a pool for the core: sequential, the pool.
const MODES_POOL: [EncMode; 2] = [SEQ, POOL];
/// ... with SBR pipelining, without a pool for the core: sequential, the pipelining.
const MODES_SBR: [EncMode; 2] = [SEQ, PIPE];
/// ... with SBR pipelining and a pool for the core: sequential, the pipelining, the pool, both.
const MODES_SBR_POOL: [EncMode; 4] = [SEQ, PIPE, POOL, BOTH];

/// The thread count and the pool of an encoder instance.
#[derive(Debug, Default)]
pub struct EncThreads {
    /// The thread count (0 and 1: the sequential code).
    threads: u32,
    /// The pool, created on demand by [`EncThreads::par`].
    pool: Option<ThreadPool>,
    /// The thread count whose pool could not be created (not retried).
    failed: u32,
    /// The scratch memory of the parallel variants.
    scratch: Vec<ChScratch>,
    psy_dynamic: Vec<Box<PsyDynamic>>,
    /// The choice of the thread mode (created with the first frame with more than one thread).
    tuner: Option<Tuner>,
    /// The tuning of [`EncThreads::set_tuning`] (else the default).
    tuning: Option<Tuning>,
    /// Frames per mode (for tests).
    mode_frames: [u64; fdk_aac_rs_fdk::par::tune::MAX_MODES],
}

impl EncThreads {
    /// Sets the thread count; a pool of another size is dropped.
    pub fn set(&mut self, threads: u32) {
        self.threads = threads;
        self.failed = 0;
    }

    /// The thread count (at least 1).
    pub fn get(&self) -> u32 {
        self.threads.max(1)
    }

    /// Sets how the encoder chooses its thread mode (tests; the default is
    /// [`fdk_aac_rs_fdk::par::tune::TUNE_ENV`] or adaptive).
    pub fn set_tuning(&mut self, tuning: Tuning) {
        self.tuning = Some(tuning);
        if let Some(t) = self.tuner.as_mut() {
            t.set_tuning(tuning);
        }
    }

    /// The frames encoded in each thread mode (tests).
    pub fn mode_frames(&self) -> [u64; fdk_aac_rs_fdk::par::tune::MAX_MODES] {
        self.mode_frames
    }

    /// The modes of a frame with `channels` channels; `reserve` is 1 with SBR pipelining.
    fn modes(&self, channels: i32, reserve: u32) -> &'static [EncMode] {
        let pool = self.threads.saturating_sub(reserve).min(channels.clamp(0, 64) as u32) >= 2;
        match (reserve > 0, pool) {
            (false, false) => &MODES_SEQ,
            (false, true) => &MODES_POOL,
            (true, false) => &MODES_SBR,
            (true, true) => &MODES_SBR_POOL,
        }
    }

    /// The mode of the next frame (with `channels` channels and `reserve`, see
    /// [`EncThreads::modes`]).
    pub(crate) fn mode(&mut self, channels: i32, reserve: u32) -> EncMode {
        if self.threads <= 1 {
            return SEQ;
        }
        let modes = self.modes(channels, reserve);
        let tuning = self.tuning;
        let t = self.tuner.get_or_insert_with(|| {
            Tuner::new(tuning.unwrap_or_else(|| Tuning::from_env(Tuning::Adaptive)))
        });
        t.set_modes(modes.len());
        modes[t.mode().min(modes.len() - 1)]
    }

    /// The pool and scratch memory for a frame with `channels` channels: `None` with one thread
    /// or one channel (nothing to distribute), otherwise a pool of `min(threads, channels)`
    /// threads, created here (or `None` if the threads cannot be spawned: the sequential code
    /// runs).
    pub fn par(&mut self, channels: i32) -> Option<EncPar<'_>> {
        self.par_with(channels, 0)
    }

    /// [`EncThreads::par`] with `reserve` of the threads kept for other work (the SBR thread of
    /// the frame pipelining): a pool of `min(threads - reserve, channels)` threads.
    ///
    /// Called once per frame, when its core starts: this is where the mode choice measures
    /// the frame period and the mode of the frame decides whether the core uses the pool.
    pub fn par_with(&mut self, channels: i32, reserve: u32) -> Option<EncPar<'_>> {
        if self.threads <= 1 {
            return None;
        }
        let start = Instant::now();
        let mode = self.mode(channels, reserve);
        if let Some(t) = self.tuner.as_mut() {
            self.mode_frames[t.mode().min(fdk_aac_rs_fdk::par::tune::MAX_MODES - 1)] += 1;
            t.tick(start);
        }
        if !mode.pool {
            return None;
        }
        let budget = self.threads.saturating_sub(reserve);
        let n = budget.min(channels.clamp(0, 64) as u32) as usize;
        if n <= 1 {
            return None;
        }
        if self.pool.as_ref().is_some_and(|p| p.threads() != n) {
            self.pool = None;
        }
        if self.pool.is_none() {
            if self.failed == self.threads {
                return None;
            }
            match ThreadPool::new(n) {
                Ok(p) => self.pool = Some(p),
                Err(_) => {
                    self.failed = self.threads;
                    return None;
                }
            }
        }
        Some(EncPar {
            pool: self.pool.as_ref()?,
            scratch: &mut self.scratch,
            psy_dynamic: &mut self.psy_dynamic,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(_: &mut [FixpDbl]) {}

    #[test]
    fn carry_moves_counters_and_hook() {
        let pool = ThreadPool::new(4).unwrap();
        let before = COUNTERS.map(|(get, _)| get());
        psy_main::psy_main_set_spectrum_hook(Some(hook));
        let mut seen = vec![false; 64];
        for _ in 0..20 {
            for_each_mut(Some(&pool), &mut seen, |_, s| {
                for (_, add) in COUNTERS {
                    add(1);
                }
                *s = psy_main::psy_main_spectrum_hook().is_some();
                std::thread::sleep(std::time::Duration::from_micros(5));
            });
            assert!(seen.iter().all(|&s| s));
        }
        psy_main::psy_main_set_spectrum_hook(None);
        for (k, (get, _)) in COUNTERS.iter().enumerate() {
            assert_eq!(get() - before[k], 20 * 64, "counter {k}");
        }
    }

    #[test]
    fn pool_size_follows_threads_and_channels() {
        let mut t = EncThreads::default();
        assert_eq!(t.get(), 1);
        assert!(t.par(2).is_none());
        t.set(4);
        assert!(t.par(1).is_none());
        assert_eq!(t.par(2).unwrap().pool.threads(), 2);
        assert_eq!(t.par(6).unwrap().pool.threads(), 4);
        t.set(1);
        assert!(t.par(6).is_none());
    }
}
