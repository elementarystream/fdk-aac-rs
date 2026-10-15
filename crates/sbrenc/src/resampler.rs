// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/resampler.h, libSBRenc/src/resampler.cpp.

//! Port of `resampler.h` and `resampler.cpp`: the IIR downsampler of the core coder path of
//! dual-rate SBR (and the LFE channel).
//!
//! The `.ii` files of the two reference builds differ only in the form of the
//! `SATURATE_RIGHT_SHIFT` macro (`scale_arm.h` vs `scale.h`), which compute the same values
//! ([`saturate_right_shift`]).
//!
//! # Undefined behavior
//!
//! The biquad sections add and shift without overflow checks; both builds use 32 bit wrapping
//! instructions, so the port uses `wrapping_*`.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::machine_type::{IntPcm, SAMPLE_BITS};
use fdk_aac_rs_fdk::scale::saturate_right_shift;

#[cfg(target_arch = "aarch64")]
mod neon;
mod tables;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;
pub use tables::*;

/// `MAXNR_SECTIONS`
pub const MAXNR_SECTIONS: usize = 15;

/// `FIXP_BQS`
pub type FixpBqs = FixpDbl;

/// `B1`
const B1: usize = 0;
/// `B2`
const B2: usize = 1;
/// `A1`
const A1: usize = 2;
/// `A2`
const A2: usize = 3;

/// `BIQUAD_COEFSTEP`
const BIQUAD_COEFSTEP: usize = 4;

/// `BIQUAD_SCALE`
const BIQUAD_SCALE: i32 = 12;

/// `struct FILTER_PARAM`
#[derive(Debug, PartialEq, Eq)]
pub struct FilterParam {
    /// `coeffa`: SOS matrix, one row/section (B1, B2, A1, A2), scaled with `BQC()`
    pub coeffa: &'static [FixpSgl],
    /// `g`: overall gain
    pub g: FixpDbl,
    /// `Wc`: normalized passband bandwidth at input samplerate * 1000
    pub wc: i32,
    /// `noCoeffs`: number of filter coeffs
    pub no_coeffs: i32,
    /// `delay`: delay in samples at input samplerate
    pub delay: i32,
}

/// `LP_FILTER`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LpFilter {
    /// `states`: state buffer
    pub states: [[FixpBqs; 2]; MAXNR_SECTIONS + 1],
    /// `coeffa`: pointer to filter coeffs (`None` for `NULL`)
    pub coeffa: Option<&'static [FixpSgl]>,
    /// `gain`: overall gain factor
    pub gain: FixpDbl,
    /// `Wc`: normalized cutoff freq * 1000
    pub wc: i32,
    /// `noCoeffs`: number of filter coeffs sets
    pub no_coeffs: i32,
    /// `ptr`: index to rinbuffers
    pub ptr: i32,
}

impl Default for LpFilter {
    fn default() -> Self {
        LpFilter {
            states: [[0; 2]; MAXNR_SECTIONS + 1],
            coeffa: None,
            gain: 0,
            wc: 0,
            no_coeffs: 0,
            ptr: 0,
        }
    }
}

/// `DOWNSAMPLER`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Downsampler {
    /// `downFilter`: filter instance
    pub down_filter: LpFilter,
    /// `ratio`: downsampling ration
    pub ratio: i32,
    /// `delay`: downsampling delay (source fs)
    pub delay: i32,
    /// `pending`: number of pending output samples
    pub pending: i32,
}

/// `FDKaacEnc_InitDownsampler(DownSampler, Wc, ratio)`: resets the downsampler, clears the
/// delay lines and selects the filter for the cutoff frequency `wc`. Returns 1.
pub fn fdk_aac_enc_init_downsampler(down_sampler: &mut Downsampler, wc: i32, ratio: i32) -> i32 {
    down_sampler.down_filter.states = [[0; 2]; MAXNR_SECTIONS + 1];
    down_sampler.down_filter.ptr = 0;

    // find applicable parameter set
    let mut current_set = FILTER_PARAM_SET[0];
    for set in FILTER_PARAM_SET.iter().skip(1) {
        if set.wc <= wc {
            break;
        }
        current_set = set;
    }

    down_sampler.down_filter.coeffa = Some(current_set.coeffa);

    down_sampler.down_filter.gain = current_set.g;
    fdk_assert!(current_set.no_coeffs <= MAXNR_SECTIONS as i32 * 2);

    down_sampler.down_filter.no_coeffs = current_set.no_coeffs;
    down_sampler.delay = current_set.delay;
    down_sampler.down_filter.wc = current_set.wc;

    down_sampler.ratio = ratio;
    down_sampler.pending = ratio - 1;
    1
}

/// `AdvanceFilter(downFilter, pInput, downRatio)`: filters `down_ratio` input samples and
/// returns the last output sample. `p_input` is the buffer and `pos` the index of the first
/// sample (the downsampler filters in place).
fn advance_filter(
    down_filter: &mut LpFilter,
    buf: &[IntPcm],
    pos: usize,
    down_ratio: i32,
) -> IntPcm {
    let mut y: FixpDbl = 0;
    let coeffa = down_filter.coeffa.unwrap_or(&[]);

    for n in 0..down_ratio.max(0) as usize {
        let states = &mut down_filter.states;
        let mut coeff = 0usize;

        let s1 = down_filter.ptr as usize;
        let s2 = s1 ^ 1;

        // SAMPLE_BITS == 16
        let mut input: FixpDbl = (buf.get(pos + n).copied().unwrap_or(0) as FixpDbl)
            << (DFRACT_BITS - SAMPLE_BITS - BIQUAD_SCALE);

        let mut state1 = states[0][s1];
        let mut state2 = states[0][s2];

        // Loop over sections
        for i in 0..(down_filter.no_coeffs.max(0) as usize).min(MAXNR_SECTIONS) {
            // Load merged states (from next section)
            let state1b = states[i + 1][s1];
            let state2b = states[i + 1][s2];

            let c = &coeffa[coeff..coeff + BIQUAD_COEFSTEP];
            let state0 =
                input.wrapping_add(f_mult(state1, c[B1])).wrapping_add(f_mult(state2, c[B2]));
            y = state0.wrapping_sub(f_mult(state1b, c[A1])).wrapping_sub(f_mult(state2b, c[A2]));

            // Store new feed forward merge state
            states[i + 1][s2] = y << 1;
            // Store new feed backward state
            states[i][s2] = input << 1;

            // Feedback output to next section.
            input = y;

            // Transfer merged states
            state1 = state1b;
            state2 = state2b;

            // Step to next coef set
            coeff += BIQUAD_COEFSTEP;
        }
        down_filter.ptr ^= 1;
    }
    // Apply global gain
    y = f_mult(y, down_filter.gain);

    // Apply final gain/scaling to output (SAMPLE_BITS == 16)
    saturate_right_shift(
        y.wrapping_add(1 << (DFRACT_BITS - SAMPLE_BITS - BIQUAD_SCALE - 1)),
        DFRACT_BITS - SAMPLE_BITS - BIQUAD_SCALE,
        SAMPLE_BITS,
    ) as IntPcm
}

/// `FDKaacEnc_Downsample(DownSampler, inSamples, numInSamples, outSamples, numOutSamples)`:
/// downsamples `num_in_samples` samples starting at `buf[in_pos]` into `buf[out_pos..]`. The C
/// function takes two pointers that the SBR encoder points into the same buffer (in place, with
/// the output behind the input); the port takes the buffer and both positions. Returns 0.
///
/// Deviation: where the reference has no defined result, input samples outside of `buf` read
/// as 0 and output samples outside of `buf` are dropped.
pub fn fdk_aac_enc_downsample(
    down_sampler: &mut Downsampler,
    buf: &mut [IntPcm],
    in_pos: usize,
    num_in_samples: i32,
    out_pos: usize,
    num_out_samples: &mut i32,
) -> i32 {
    #[cfg(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64"))]
    if let Some(r) =
        downsample_vec(down_sampler, buf, in_pos, num_in_samples, out_pos, num_out_samples)
    {
        return r;
    }
    fdk_aac_enc_downsample_straight(
        down_sampler,
        buf,
        in_pos,
        num_in_samples,
        out_pos,
        num_out_samples,
    )
}

/// [`fdk_aac_enc_downsample`] with the NEON cascade (`neon.rs`) or, on x86 with AVX2, the AVX2
/// cascade (`x86.rs`), for the inputs where it computes the same: a ratio of 1 or more, at least one section, all samples within `buf`, and the
/// output not after the input
/// (`out_pos <= in_pos`, as the SBR encoder calls it). Then output `k` (written after the input
/// samples up to `ratio * (k + 1) - 1` are filtered) lands at `out_pos + k`, before every input
/// sample read later, so reading the input first and writing the output afterwards gives the same
/// buffer. Returns `None` for the other inputs.
#[cfg(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64"))]
fn downsample_vec(
    down_sampler: &mut Downsampler,
    buf: &mut [IntPcm],
    in_pos: usize,
    num_in_samples: i32,
    out_pos: usize,
    num_out_samples: &mut i32,
) -> Option<i32> {
    const CHUNK: usize = 1024;
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !crate::simd::avx2() {
        return None;
    }
    let ratio = down_sampler.ratio;
    let f = &down_sampler.down_filter;
    let coeffa = f.coeffa?;
    let sections = (f.no_coeffs.max(0) as usize).min(MAXNR_SECTIONS);
    if ratio < 1
        || num_in_samples <= 0
        || out_pos > in_pos
        || sections == 0
        || coeffa.len() < sections * BIQUAD_COEFSTEP
    {
        return None;
    }
    let ratio = ratio as usize;
    let groups = (num_in_samples as usize).div_ceil(ratio);
    let total = groups * ratio;
    if in_pos + total > buf.len() || out_pos + groups > buf.len() {
        return None;
    }
    let gain = f.gain;
    let mut input = [0 as IntPcm; CHUNK];
    let mut ys = [0 as FixpDbl; CHUNK];
    let mut start = 0;
    while start < total {
        let len = (total - start).min(CHUNK);
        input[..len].copy_from_slice(&buf[in_pos + start..in_pos + start + len]);
        #[cfg(target_arch = "aarch64")]
        neon::cascade(&mut down_sampler.down_filter, sections, &input[..len], &mut ys[..len]);
        // SAFETY: the CPU supports AVX2 (checked above).
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        unsafe {
            x86::cascade(&mut down_sampler.down_filter, sections, &input[..len], &mut ys[..len])
        };
        // the outputs of the groups that end in this chunk (sample ratio * (k + 1) - 1)
        let mut j = (ratio - 1 - start % ratio) % ratio;
        while j < len {
            buf[out_pos + (start + j) / ratio] = filter_output(ys[j], gain);
            j += ratio;
        }
        start += len;
    }
    *num_out_samples = num_in_samples / ratio as i32;
    Some(0)
}

/// The end of `AdvanceFilter()`: the global gain and the scaling of the last section's output.
#[inline]
fn filter_output(y: FixpDbl, gain: FixpDbl) -> IntPcm {
    // Apply global gain
    let y = f_mult(y, gain);
    // Apply final gain/scaling to output (SAMPLE_BITS == 16)
    saturate_right_shift(
        y.wrapping_add(1 << (DFRACT_BITS - SAMPLE_BITS - BIQUAD_SCALE - 1)),
        DFRACT_BITS - SAMPLE_BITS - BIQUAD_SCALE,
        SAMPLE_BITS,
    ) as IntPcm
}

/// [`fdk_aac_enc_downsample`] (straight port, kept as its reference and for the inputs the
/// vector version does not take).
#[doc(hidden)]
pub fn fdk_aac_enc_downsample_straight(
    down_sampler: &mut Downsampler,
    buf: &mut [IntPcm],
    in_pos: usize,
    num_in_samples: i32,
    out_pos: usize,
    num_out_samples: &mut i32,
) -> i32 {
    *num_out_samples = 0;
    let ratio = down_sampler.ratio;
    let mut out = out_pos;
    let mut i = 0i32;
    while i < num_in_samples {
        let v = advance_filter(&mut down_sampler.down_filter, buf, in_pos + i as usize, ratio);
        if let Some(o) = buf.get_mut(out) {
            *o = v;
        }
        out += 1;
        if ratio <= 0 {
            break;
        }
        i += ratio;
    }
    if ratio != 0 {
        *num_out_samples = num_in_samples / ratio;
    }
    0
}
