// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libPCMutils/include/limiter.h, libPCMutils/src/limiter.cpp.

//! Port of `limiter.h`: the hard (time domain) limiter for clipping prevention.
//!
//! `TDLimiterPtr` handles become `Box<TDLimiter>`; the functions take `Option<&mut TDLimiter>`
//! where the C++ code checks for `NULL`. `pcmLimiter_Destroy()` takes the box.
//!
//! Variants: both reference builds compile the same code apart from the primitives (`fMult`,
//! `invFixp`) and the two `SATURATE_LEFT_SHIFT` macros (`arm/scale_arm.h` and generic), which give
//! the same values (checked in the disassembly for all amounts, see [`pcm_limiter_apply`]).
//!
//! Buffers: the C++ code allocates `maxBuf` and `delayBuf` for the attack time and the number of
//! channels of `pcmLimiter_Create()`. Where the current parameters need more, the reference has
//! no defined result; the port extends the buffers with zeros (PORTING.md, "Deviations from the
//! reference").

use fdk_aac_rs_fdk::audio::{
    lib_version, lib_version_string, LibInfo, CAPF_LIMITER, FDK_MODULE_LAST, FDK_NONE, FDK_TDLIMIT,
};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::core::fdk_tools_get_lib_info;
use fdk_aac_rs_fdk::fixpoint_math::{f_div_norm_noexp, f_log2, f_pow, fixp_round_to_int, inv_fixp};
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::scale::{
    saturate_left_shift, scale_value, scale_value_saturate, scale_values_saturate,
};

use crate::pcm_utils::{
    PCMUTIL_LIB_BUILD_DATE, PCMUTIL_LIB_BUILD_TIME, PCMUTIL_LIB_VL0, PCMUTIL_LIB_VL1,
    PCMUTIL_LIB_VL2,
};

/// `TDLIMIT_LIB_TITLE`
pub const TDLIMIT_LIB_TITLE: &str = "TD Limiter Lib";

/// `TDL_ATTACK_DEFAULT_MS`: default attack time in ms
pub const TDL_ATTACK_DEFAULT_MS: u32 = 15;
/// `TDL_RELEASE_DEFAULT_MS`: default release time in ms
pub const TDL_RELEASE_DEFAULT_MS: u32 = 50;

/// `TDLIMITER_ERROR`
pub type TdLimiterError = i32;
/// `TDLIMIT_OK`
pub const TDLIMIT_OK: TdLimiterError = 0;
/// `TDLIMIT_UNKNOWN`
pub const TDLIMIT_UNKNOWN: TdLimiterError = -1;
/// `__error_codes_start`
pub const __error_codes_start: TdLimiterError = -100;
/// `TDLIMIT_INVALID_HANDLE`
pub const TDLIMIT_INVALID_HANDLE: TdLimiterError = -99;
/// `TDLIMIT_INVALID_PARAMETER`
pub const TDLIMIT_INVALID_PARAMETER: TdLimiterError = -98;
/// `__error_codes_end`
pub const __error_codes_end: TdLimiterError = -97;

/// `PCM_LIM`
pub type PcmLim = i32;
/// `PCM_LIM_BITS`
pub const PCM_LIM_BITS: i32 = 32;
/// `SAMPLE_BITS_LIM`
pub const SAMPLE_BITS_LIM: i32 = DFRACT_BITS;

/// `FL2FXCONST_DBL(1.0f / (1 << 1))`
const HALF: FixpDbl = fl2fxconst_dbl(1.0 / (1 << 1) as f64);
/// `FL2FXCONST_DBL(0.1f)`
const C_0_1: FixpDbl = fl2fxconst_dbl(0.1f32 as f64);
/// `FL2FXCONST_SGL(0.1f * (1 << 1))` (`float` arithmetic)
const C_0_1_X2: FixpSgl = fl2fxconst_sgl((0.1f32 * (1 << 1) as f32) as f64);
/// `FL2FXCONST_SGL(1.11111111f / (1 << 1))` (`float` arithmetic)
const C_1_11_DIV2: FixpSgl = fl2fxconst_sgl((1.11111111f32 / (1 << 1) as f32) as f64);
/// `FL2FXCONST_DBL(-6.0206f / (1 << 3))` (`float` arithmetic)
const C_M6_0206_DIV8: FixpDbl = fl2fxconst_dbl((-6.0206f32 / (1 << 3) as f32) as f64);

/// `struct TDLimiter`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TDLimiter {
    pub attack: u32,
    pub attack_const: FixpDbl,
    pub release_const: FixpDbl,
    pub attack_ms: u32,
    pub release_ms: u32,
    pub max_attack_ms: u32,
    pub threshold: FixpDbl,
    pub channels: u32,
    pub max_channels: u32,
    pub sample_rate: u32,
    pub max_sample_rate: u32,
    pub cor: FixpDbl,
    pub max: FixpDbl,
    /// `maxBuf`: `attack + 1` values of `pcmLimiter_Create()`
    pub max_buf: Vec<FixpDbl>,
    /// `delayBuf`: `attack * maxChannels` values of `pcmLimiter_Create()`
    pub delay_buf: Vec<FixpDbl>,
    pub max_buf_idx: u32,
    pub delay_buf_idx: u32,
    pub smooth_state0: FixpDbl,
    pub min_gain: FixpDbl,
    pub scaling: i32,
}

/// `TDLimiterPtr`
pub type TDLimiterPtr = Box<TDLimiter>;

/// `pow(0.1, 1.0 / (n + 1))` as computed by the limiter:
/// `scaleValue(fPow(FL2FXCONST_DBL(0.1f), 0, invFixp(n + 1), 0, &e), e)`.
fn pow_0_1(n: u32) -> FixpDbl {
    let mut e_ans = 0;
    let exponent = inv_fixp(n.wrapping_add(1) as FixpDbl);
    let c = f_pow(C_0_1, 0, exponent, 0, &mut e_ans);
    scale_value(c, e_ans)
}

impl TDLimiter {
    /// Extends the buffers with zeros where the current attack time and number of channels need
    /// more than `pcmLimiter_Create()` allocated (see the module documentation). `delayBuf` holds
    /// at least one sample per channel. The read and write positions are covered as well (they
    /// can stay beyond a reduced attack time until they wrap around).
    fn ensure_buffers(&mut self) {
        let need_max = (self.attack as usize + 1).max(self.max_buf_idx as usize + 1);
        if self.max_buf.len() < need_max {
            self.max_buf.resize(need_max, 0);
        }
        let slots = (self.attack.max(1) as usize).max(self.delay_buf_idx as usize + 1);
        let need_delay = slots.saturating_mul(self.channels as usize);
        if self.delay_buf.len() < need_delay {
            self.delay_buf.resize(need_delay, 0);
        }
    }
}

/// `pcmLimiter_Create(maxAttackMs, releaseMs, threshold, maxChannels, maxSampleRate)`: creates a
/// limiter.
///
/// * `max_attack_ms`: maximum and initial attack/lookahead time in milliseconds
/// * `release_ms`: release time in milliseconds (90% time constant)
/// * `threshold`: limiting threshold
/// * `max_channels`: maximum and initial number of channels
/// * `max_sample_rate`: maximum and initial sampling rate in Hz
///
/// Unlike the reference, which returns `NULL` when an allocation fails, the port aborts on
/// allocation failure (Rust allocation).
pub fn pcm_limiter_create(
    max_attack_ms: u32,
    release_ms: u32,
    threshold: FixpDbl,
    max_channels: u32,
    max_sample_rate: u32,
) -> Option<TDLimiterPtr> {
    // calc attack and release time in samples
    let attack = max_attack_ms.wrapping_mul(max_sample_rate) / 1000;
    let release = release_ms.wrapping_mul(max_sample_rate) / 1000;

    // attackConst = pow(0.1, 1.0 / (attack + 1))
    let attack_const = pow_0_1(attack);
    // releaseConst  = (float)pow(0.1, 1.0 / (release + 1))
    let release_const = pow_0_1(release);

    let mut limiter = Box::new(TDLimiter {
        attack,
        attack_const,
        release_const,
        attack_ms: max_attack_ms,
        release_ms,
        max_attack_ms,
        threshold,
        channels: max_channels,
        max_channels,
        sample_rate: max_sample_rate,
        max_sample_rate,
        cor: 0,
        max: 0,
        // FDKcalloc(attack + 1, sizeof(FIXP_DBL)), FDKcalloc(attack * maxChannels, ...): the
        // element counts are UINT products in C++.
        max_buf: vec![0; attack.wrapping_add(1) as usize],
        delay_buf: vec![0; attack.wrapping_mul(max_channels) as usize],
        max_buf_idx: 0,
        delay_buf_idx: 0,
        smooth_state0: 0,
        min_gain: 0,
        scaling: 0,
    });

    pcm_limiter_reset(Some(&mut limiter));

    Some(limiter)
}

/// `pcmLimiter_Apply(limiter, samplesIn, samplesOut, pGainPerSample, scaling, nSamples)`:
/// limits `n_samples` interleaved samples of `channels` channels.
///
/// * `samples_in`: interleaved input samples (at least `n_samples * channels`)
/// * `samples_out`: interleaved output samples (at least `n_samples * channels`)
/// * `p_gain_per_sample`: an additional gain for each sample, or `None` (`NULL`)
/// * `scaling`: scaling of the output samples
///
/// Shift amounts: `threshold >> scaling`, `0x8000 >> (scaling + 1)` and the shifts of
/// `SATURATE_LEFT_SHIFT(x, scaling + 1, 32)` are out of range for some `scaling`; both reference
/// builds shift by a register (`asr`/`lsr`/`lsl` on arm64, `sar`/`shr`/`shl` on x86-64, also in
/// the comparisons of the saturation), which takes the amount modulo 32. The port does the same.
///
/// This is the optimized version of [`pcm_limiter_apply_straight`]: the samples are processed on
/// slices of one frame (no bounds checks in the loops), the search of the running maximum is a
/// fold over the window (the maximum does not depend on the order, so the compiler vectorizes
/// it), the loops are specialized for one and two channels, and the gain of an unchanged maximum
/// is not divided again. Every value, every state change
/// and the order of the saturations are the same. Buffers shorter than `n_samples` frames (on
/// which the straight version panics part way through) take the straight version.
pub fn pcm_limiter_apply(
    limiter: Option<&mut TDLimiter>,
    samples_in: &[PcmLim],
    samples_out: &mut [IntPcm],
    p_gain_per_sample: Option<&[FixpDbl]>,
    scaling: i32,
    n_samples: u32,
) -> TdLimiterError {
    let Some(limiter) = limiter else {
        return TDLIMIT_INVALID_HANDLE;
    };
    let channels = limiter.channels as usize;
    let n = n_samples as usize;
    let len = n.checked_mul(channels);
    let fits = matches!(len, Some(len) if samples_in.len() >= len && samples_out.len() >= len)
        && p_gain_per_sample.is_none_or(|g| g.len() >= n);
    match (fits, channels) {
        (true, 1) => {
            limiter_apply_fast::<1>(limiter, samples_in, samples_out, p_gain_per_sample, scaling, n)
        }
        (true, 2) => {
            limiter_apply_fast::<2>(limiter, samples_in, samples_out, p_gain_per_sample, scaling, n)
        }
        (true, c) if c > 0 => {
            limiter_apply_fast::<0>(limiter, samples_in, samples_out, p_gain_per_sample, scaling, n)
        }
        _ => pcm_limiter_apply_straight(
            Some(limiter),
            samples_in,
            samples_out,
            p_gain_per_sample,
            scaling,
            n_samples,
        ),
    }
}

/// [`pcm_limiter_apply`] for `CH` channels (0: `limiter.channels`, at least 1), on buffers of at
/// least `n_samples` frames.
#[inline(always)]
fn limiter_apply_fast<const CH: usize>(
    limiter: &mut TDLimiter,
    samples_in: &[PcmLim],
    samples_out: &mut [IntPcm],
    p_gain_per_sample: Option<&[FixpDbl]>,
    scaling: i32,
    n_samples: usize,
) -> TdLimiterError {
    let mut additional_gain: FixpDbl = 0;
    let mut min_gain: FixpDbl = HALF;

    limiter.ensure_buffers();

    let channels = if CH == 0 { limiter.channels as usize } else { CH };
    let attack = limiter.attack;
    let attack_const = limiter.attack_const;
    let release_const = limiter.release_const;
    let threshold = limiter.threshold.wrapping_shr(scaling as u32);

    let mut max = limiter.max;
    let mut max_buf_idx = limiter.max_buf_idx;
    let mut cor = limiter.cor;
    let mut delay_buf_idx = limiter.delay_buf_idx;

    let mut smooth_state0 = limiter.smooth_state0;

    if limiter.scaling != scaling {
        let sf = limiter.scaling.wrapping_sub(scaling);
        // INT len = attack * channels (UINT product)
        let len = attack.wrapping_mul(channels as u32) as usize;
        scale_values_saturate(&mut limiter.delay_buf[..len], sf);
        scale_values_saturate(&mut limiter.max_buf[..attack as usize + 1], sf);
        max = scale_value_saturate(max, sf);
        limiter.scaling = scaling;
    }

    // ensure_buffers(): max_buf holds attack + 1 values and max_buf_idx, delay_buf the slots of
    // attack (at least 1) and delay_buf_idx.
    let max_buf = &mut limiter.max_buf[..];
    let delay_buf = &mut limiter.delay_buf[..];
    let sh = scaling.wrapping_add(1);
    // (FIXP_DBL)0x8000 >> (scaling + 1)
    let rnd = 0x8000i32.wrapping_shr(sh as u32);

    let samples_in = &samples_in[..n_samples * channels];
    let samples_out = &mut samples_out[..n_samples * channels];
    // The last maximum above the threshold and its gain (none yet: max > threshold differs).
    let mut div_max: FixpDbl = threshold;
    let mut div_gain: FixpDbl = 0;

    // The maximum absolute sample value of all channels of frame `i`, including the additional
    // gain.
    let frame_max = |i: usize| -> FixpDbl {
        let mut tmp: FixpDbl = 0;
        for &tmp2 in &samples_in[i * channels..(i + 1) * channels] {
            let tmp2 = if tmp2 == MINVAL_DBL { MAXVAL_DBL } else { f_abs(tmp2) };
            tmp = f_max(tmp, tmp2);
        }
        if let Some(g) = p_gain_per_sample {
            tmp = f_mult(tmp, g[i]);
        }
        tmp
    };

    let mut i = 0;
    while i < n_samples {
        // The steady state of a limiter that does not limit: with smoothState0 == cor == 1.0
        // (HALF) and max <= threshold, every frame whose maximum is at most the threshold stores
        // the threshold into maxBuf and as the new maximum (tmp = threshold >= max), gets the
        // gain HALF (the release step subtracts fMult(releaseConst, 0) == 0, so smoothState0 and
        // cor stay HALF, and minGain, which starts at HALF, does not change) and passes the delay
        // line with gain 1.0. Such runs of frames are processed in bulk, which gives the same
        // values and state as the frame loop below.
        if smooth_state0 == HALF && cor == HALF && max <= threshold {
            let mut r = 0;
            if p_gain_per_sample.is_none() {
                // Without gains, the frames of a block are all at most the threshold exactly
                // when the saturated absolute values of all their samples are (fAbs with
                // MINVAL_DBL -> MAXVAL_DBL is saturating_abs()); checked 32 frames at a time.
                const BLOCK: usize = 32;
                while i + r + BLOCK <= n_samples {
                    let block = &samples_in[(i + r) * channels..(i + r + BLOCK) * channels];
                    if block.iter().fold(0, |m: FixpDbl, &x| m.max(x.saturating_abs())) > threshold
                    {
                        break;
                    }
                    r += BLOCK;
                }
            }
            while i + r < n_samples && frame_max(i + r) <= threshold {
                r += 1;
            }
            if r > 0 {
                // maxBuf[maxBufIdx++] = threshold, r times (an index beyond the window, left by
                // a shorter attack time, is written alone and wraps to 0 as in the frame loop)
                let mut left = r;
                while left > 0 {
                    let idx = max_buf_idx as usize;
                    let len = if max_buf_idx <= attack {
                        (attack as usize + 1 - idx).min(left)
                    } else {
                        1
                    };
                    max_buf[idx..idx + len].fill(threshold);
                    max_buf_idx += len as u32;
                    if max_buf_idx >= attack.wrapping_add(1) {
                        max_buf_idx = 0;
                    }
                    left -= len;
                }
                max = threshold;
                let mut f = i;
                while f < i + r {
                    // a segment of frames whose delay slots do not wrap around (one frame for a
                    // slot index at or beyond the attack time)
                    let len = if delay_buf_idx < attack {
                        (attack - delay_buf_idx).min((i + r - f) as u32) as usize
                    } else {
                        1
                    };
                    let d0 = delay_buf_idx as usize * channels;
                    let delay = &mut delay_buf[d0..d0 + len * channels];
                    let inp = &samples_in[f * channels..(f + len) * channels];
                    let out = &mut samples_out[f * channels..(f + len) * channels];
                    for ((d, &x), o) in delay.iter_mut().zip(inp).zip(out.iter_mut()) {
                        *o = fx_dbl2fx_pcm(saturate_left_shift(
                            (*d >> 1).wrapping_add(rnd),
                            sh,
                            DFRACT_BITS,
                        ));
                        *d = x;
                    }
                    if let Some(g) = p_gain_per_sample {
                        for (k, d) in delay.chunks_exact_mut(channels).enumerate() {
                            let gk = g[f + k];
                            for d in d.iter_mut() {
                                *d = f_mult(*d, gk);
                            }
                        }
                    }
                    delay_buf_idx += len as u32;
                    if delay_buf_idx >= attack {
                        delay_buf_idx = 0;
                    }
                    f += len;
                }
                i += r;
                continue;
            }
        }

        let inp = &samples_in[i * channels..(i + 1) * channels];
        let out = &mut samples_out[i * channels..(i + 1) * channels];
        let mut tmp = frame_max(i);
        if let Some(g) = p_gain_per_sample {
            additional_gain = g[i];
        }

        // set threshold as lower border to save calculations in running maximum algorithm
        tmp = f_max(tmp, threshold);

        // running maximum
        let old = max_buf[max_buf_idx as usize];
        max_buf[max_buf_idx as usize] = tmp;

        if tmp >= max {
            // new sample is greater than old maximum, so it is the new maximum
            max = tmp;
        } else if old < max {
            // maximum does not change, as the sample, which has left the window was not the
            // maximum
        } else {
            // the old maximum has left the window, we have to search the complete buffer for the
            // new max (fMax is associative and commutative)
            let window = &max_buf[..=attack as usize];
            max = window.iter().fold(window[0], |m, &v| f_max(m, v));
        }
        max_buf_idx += 1;
        if max_buf_idx >= attack.wrapping_add(1) {
            max_buf_idx = 0;
        }

        // calc gain
        // gain is downscaled by one, so that gain = 1.0 can be represented
        let mut gain: FixpDbl = if max > threshold {
            if threshold >= 0 {
                // The quotient of the last maximum is reused: fDivNorm() is a pure function, and
                // for 0 <= threshold < max none of its assertions can fail (so the checked
                // profile counts nothing either way).
                if max != div_max {
                    div_max = max;
                    div_gain = f_div_norm_noexp(threshold, max) >> 1;
                }
                div_gain
            } else {
                f_div_norm_noexp(threshold, max) >> 1
            }
        } else {
            HALF
        };

        // gain smoothing, method: TDL_EXPONENTIAL
        if gain < smooth_state0 {
            cor = f_min(
                cor,
                f_mult_div2(gain.wrapping_sub(f_mult_div2(C_0_1_X2, smooth_state0)), C_1_11_DIV2)
                    << 2,
            );
        } else {
            cor = gain;
        }

        // smoothing filter
        if cor < smooth_state0 {
            smooth_state0 = f_mult(attack_const, smooth_state0.wrapping_sub(cor)).wrapping_add(cor); // attack
            smooth_state0 = f_max(smooth_state0, gain); // avoid overshooting target
        } else {
            smooth_state0 =
                cor.wrapping_sub(f_mult(release_const, cor.wrapping_sub(smooth_state0)));
            // release
        }

        gain = smooth_state0;

        let p_delay_buf = delay_buf_idx as usize * channels;
        let delay = &mut delay_buf[p_delay_buf..p_delay_buf + channels];
        if gain < HALF {
            gain = gain.wrapping_shl(1);
            // lookahead delay, apply gain
            for ((d, &x), o) in delay.iter_mut().zip(inp).zip(out.iter_mut()) {
                let mut tmp = *d;
                *d = if p_gain_per_sample.is_some() { f_mult(x, additional_gain) } else { x };
                // Apply gain to delayed signal
                tmp = f_mult_div2(tmp, gain);
                *o = fx_dbl2fx_pcm(saturate_left_shift(tmp.wrapping_add(rnd), sh, DFRACT_BITS));
            }
            gain >>= 1;
        } else {
            // lookahead delay, apply gain=1.0f
            for ((d, &x), o) in delay.iter_mut().zip(inp).zip(out.iter_mut()) {
                let tmp = *d;
                *d = if p_gain_per_sample.is_some() { f_mult(x, additional_gain) } else { x };
                *o = fx_dbl2fx_pcm(saturate_left_shift(
                    (tmp >> 1).wrapping_add(rnd),
                    sh,
                    DFRACT_BITS,
                ));
            }
        }

        delay_buf_idx += 1;
        if delay_buf_idx >= attack {
            delay_buf_idx = 0;
        }

        // save minimum gain factor
        if gain < min_gain {
            min_gain = gain;
        }
        i += 1;
    }

    limiter.max = max;
    limiter.max_buf_idx = max_buf_idx;
    limiter.cor = cor;
    limiter.delay_buf_idx = delay_buf_idx;

    limiter.smooth_state0 = smooth_state0;

    limiter.min_gain = min_gain;

    TDLIMIT_OK
}

/// `pcmLimiter_Apply()` as ported (see [`pcm_limiter_apply`]).
pub fn pcm_limiter_apply_straight(
    limiter: Option<&mut TDLimiter>,
    samples_in: &[PcmLim],
    samples_out: &mut [IntPcm],
    p_gain_per_sample: Option<&[FixpDbl]>,
    scaling: i32,
    n_samples: u32,
) -> TdLimiterError {
    let mut additional_gain: FixpDbl = 0;
    let mut min_gain: FixpDbl = HALF;

    let Some(limiter) = limiter else {
        return TDLIMIT_INVALID_HANDLE;
    };
    limiter.ensure_buffers();

    let channels = limiter.channels as usize;
    let attack = limiter.attack;
    let attack_const = limiter.attack_const;
    let release_const = limiter.release_const;
    let threshold = limiter.threshold.wrapping_shr(scaling as u32);

    let mut max = limiter.max;
    let mut max_buf_idx = limiter.max_buf_idx;
    let mut cor = limiter.cor;
    let mut delay_buf_idx = limiter.delay_buf_idx;

    let mut smooth_state0 = limiter.smooth_state0;

    if limiter.scaling != scaling {
        let sf = limiter.scaling.wrapping_sub(scaling);
        // INT len = attack * channels (UINT product)
        let len = attack.wrapping_mul(channels as u32) as usize;
        scale_values_saturate(&mut limiter.delay_buf[..len], sf);
        scale_values_saturate(&mut limiter.max_buf[..attack as usize + 1], sf);
        max = scale_value_saturate(max, sf);
        limiter.scaling = scaling;
    }

    let max_buf = &mut limiter.max_buf;
    let delay_buf = &mut limiter.delay_buf;
    let sh = scaling.wrapping_add(1);
    // (FIXP_DBL)0x8000 >> (scaling + 1)
    let rnd = 0x8000i32.wrapping_shr(sh as u32);

    let mut p_in = 0usize;
    for i in 0..n_samples as usize {
        // get maximum absolute sample value of all channels, including the additional gain.
        let mut tmp: FixpDbl = 0;
        for j in 0..channels {
            let tmp2 = samples_in[p_in + j];
            let tmp2 = if tmp2 == MINVAL_DBL { MAXVAL_DBL } else { f_abs(tmp2) };
            tmp = f_max(tmp, tmp2);
        }

        if let Some(g) = p_gain_per_sample {
            additional_gain = g[i];
            tmp = f_mult(tmp, additional_gain);
        }

        // set threshold as lower border to save calculations in running maximum algorithm
        tmp = f_max(tmp, threshold);

        // running maximum
        let old = max_buf[max_buf_idx as usize];
        max_buf[max_buf_idx as usize] = tmp;

        if tmp >= max {
            // new sample is greater than old maximum, so it is the new maximum
            max = tmp;
        } else if old < max {
            // maximum does not change, as the sample, which has left the window was not the
            // maximum
        } else {
            // the old maximum has left the window, we have to search the complete buffer for the
            // new max
            max = max_buf[0];
            for j in 1..=attack as usize {
                max = f_max(max, max_buf[j]);
            }
        }
        max_buf_idx += 1;
        if max_buf_idx >= attack.wrapping_add(1) {
            max_buf_idx = 0;
        }

        // calc gain
        // gain is downscaled by one, so that gain = 1.0 can be represented
        let mut gain: FixpDbl =
            if max > threshold { f_div_norm_noexp(threshold, max) >> 1 } else { HALF };

        // gain smoothing, method: TDL_EXPONENTIAL
        // first order IIR filter with attack correction to avoid overshoots

        // correct the 'aiming' value of the exponential attack to avoid the remaining overshoot
        if gain < smooth_state0 {
            cor = f_min(
                cor,
                f_mult_div2(gain.wrapping_sub(f_mult_div2(C_0_1_X2, smooth_state0)), C_1_11_DIV2)
                    << 2,
            );
        } else {
            cor = gain;
        }

        // smoothing filter
        if cor < smooth_state0 {
            smooth_state0 = f_mult(attack_const, smooth_state0.wrapping_sub(cor)).wrapping_add(cor); // attack
            smooth_state0 = f_max(smooth_state0, gain); // avoid overshooting target
        } else {
            // sign inversion twice to round towards +infinity, so that gain can converge to 1.0
            // again, for bit-identical output when limiter is not active.
            // -fMult(releaseConst, -(smoothState0 - cor)) + cor, computed as in the reference
            // builds: cor - fMult(releaseConst, cor - smoothState0) (wrapping).
            smooth_state0 =
                cor.wrapping_sub(f_mult(release_const, cor.wrapping_sub(smooth_state0)));
            // release
        }

        gain = smooth_state0;

        let p_delay_buf = delay_buf_idx as usize * channels;
        if gain < HALF {
            gain = gain.wrapping_shl(1);
            // lookahead delay, apply gain
            for j in 0..channels {
                let mut tmp = delay_buf[p_delay_buf + j];

                delay_buf[p_delay_buf + j] = if p_gain_per_sample.is_some() {
                    f_mult(samples_in[p_in + j], additional_gain)
                } else {
                    samples_in[p_in + j]
                };

                // Apply gain to delayed signal
                tmp = f_mult_div2(tmp, gain);
                // SAMPLE_BITS != DFRACT_BITS
                samples_out[p_in + j] =
                    fx_dbl2fx_pcm(saturate_left_shift(tmp.wrapping_add(rnd), sh, DFRACT_BITS));
            }
            gain >>= 1;
        } else {
            // lookahead delay, apply gain=1.0f
            for j in 0..channels {
                let tmp = delay_buf[p_delay_buf + j];
                delay_buf[p_delay_buf + j] = if p_gain_per_sample.is_some() {
                    f_mult(samples_in[p_in + j], additional_gain)
                } else {
                    samples_in[p_in + j]
                };

                // SAMPLE_BITS != DFRACT_BITS
                samples_out[p_in + j] = fx_dbl2fx_pcm(saturate_left_shift(
                    (tmp >> 1).wrapping_add(rnd),
                    sh,
                    DFRACT_BITS,
                ));
            }
        }

        delay_buf_idx += 1;
        if delay_buf_idx >= attack {
            delay_buf_idx = 0;
        }

        // save minimum gain factor
        if gain < min_gain {
            min_gain = gain;
        }

        // advance sample pointer by <channel> samples
        p_in += channels;
    }

    limiter.max = max;
    limiter.max_buf_idx = max_buf_idx;
    limiter.cor = cor;
    limiter.delay_buf_idx = delay_buf_idx;

    limiter.smooth_state0 = smooth_state0;

    limiter.min_gain = min_gain;

    TDLIMIT_OK
}

/// `pcmLimiter_SetThreshold(limiter, threshold)`: sets the limiter threshold.
pub fn pcm_limiter_set_threshold(
    limiter: Option<&mut TDLimiter>,
    threshold: FixpDbl,
) -> TdLimiterError {
    let Some(limiter) = limiter else {
        return TDLIMIT_INVALID_HANDLE;
    };

    limiter.threshold = threshold;

    TDLIMIT_OK
}

/// `pcmLimiter_Reset(limiter)`: resets the limiter state.
pub fn pcm_limiter_reset(limiter: Option<&mut TDLimiter>) -> TdLimiterError {
    let Some(limiter) = limiter else {
        return TDLIMIT_INVALID_HANDLE;
    };
    limiter.ensure_buffers();

    limiter.max_buf_idx = 0;
    limiter.delay_buf_idx = 0;
    limiter.max = 0;
    limiter.cor = HALF;
    limiter.smooth_state0 = HALF;
    limiter.min_gain = HALF;
    limiter.scaling = 0;

    let n = limiter.attack as usize + 1;
    limiter.max_buf[..n].fill(0);
    let n = limiter.attack.wrapping_mul(limiter.channels) as usize;
    limiter.delay_buf[..n].fill(0);

    TDLIMIT_OK
}

/// `pcmLimiter_Destroy(limiter)`: frees the limiter.
pub fn pcm_limiter_destroy(limiter: Option<TDLimiterPtr>) -> TdLimiterError {
    match limiter {
        Some(l) => {
            drop(l);
            TDLIMIT_OK
        }
        None => TDLIMIT_INVALID_HANDLE,
    }
}

/// `pcmLimiter_GetDelay(limiter)`: the delay caused by the limiter in samples per channel.
pub fn pcm_limiter_get_delay(limiter: &TDLimiter) -> u32 {
    limiter.attack
}

/// `pcmLimiter_GetMaxGainReduction(limiter)`: the maximum gain reduction of the last processed
/// block in dB.
pub fn pcm_limiter_get_max_gain_reduction(limiter: &TDLimiter) -> i32 {
    // maximum gain reduction in dB = -20 * log10(limiter->minGain)
    // = -20 * log2(limiter->minGain)/log2(10) = -6.0206*log2(limiter->minGain)
    let mut e_ans = 0;

    let loggain = f_log2(limiter.min_gain, 1, &mut e_ans);

    let max_gain_reduction = f_mult(loggain, C_M6_0206_DIV8);

    fixp_round_to_int(max_gain_reduction, e_ans + 3)
}

/// `pcmLimiter_SetNChannels(limiter, nChannels)`: sets the number of channels (at most
/// `maxChannels`).
pub fn pcm_limiter_set_n_channels(
    limiter: Option<&mut TDLimiter>,
    n_channels: u32,
) -> TdLimiterError {
    let Some(limiter) = limiter else {
        return TDLIMIT_INVALID_HANDLE;
    };

    if n_channels > limiter.max_channels {
        return TDLIMIT_INVALID_PARAMETER;
    }

    limiter.channels = n_channels;

    TDLIMIT_OK
}

/// `pcmLimiter_SetSampleRate(limiter, sampleRate)`: sets the sampling rate (at most
/// `maxSampleRate`).
pub fn pcm_limiter_set_sample_rate(
    limiter: Option<&mut TDLimiter>,
    sample_rate: u32,
) -> TdLimiterError {
    let Some(limiter) = limiter else {
        return TDLIMIT_INVALID_HANDLE;
    };

    if sample_rate > limiter.max_sample_rate {
        return TDLIMIT_INVALID_PARAMETER;
    }

    // update attack and release time in samples
    let attack = limiter.attack_ms.wrapping_mul(sample_rate) / 1000;
    let release = limiter.release_ms.wrapping_mul(sample_rate) / 1000;

    // attackConst = pow(0.1, 1.0 / (attack + 1))
    let attack_const = pow_0_1(attack);
    // releaseConst  = (float)pow(0.1, 1.0 / (release + 1))
    let release_const = pow_0_1(release);

    limiter.attack = attack;
    limiter.attack_const = attack_const;
    limiter.release_const = release_const;
    limiter.sample_rate = sample_rate;

    TDLIMIT_OK
}

/// `pcmLimiter_SetAttack(limiter, attackMs)`: sets the attack time (at most `maxAttackMs`).
pub fn pcm_limiter_set_attack(limiter: Option<&mut TDLimiter>, attack_ms: u32) -> TdLimiterError {
    let Some(limiter) = limiter else {
        return TDLIMIT_INVALID_HANDLE;
    };

    if attack_ms > limiter.max_attack_ms {
        return TDLIMIT_INVALID_PARAMETER;
    }

    // calculate attack time in samples
    let attack = attack_ms.wrapping_mul(limiter.sample_rate) / 1000;

    // attackConst = pow(0.1, 1.0 / (attack + 1))
    let attack_const = pow_0_1(attack);

    limiter.attack = attack;
    limiter.attack_const = attack_const;
    limiter.attack_ms = attack_ms;

    TDLIMIT_OK
}

/// `pcmLimiter_SetRelease(limiter, releaseMs)`: sets the release time.
pub fn pcm_limiter_set_release(limiter: Option<&mut TDLimiter>, release_ms: u32) -> TdLimiterError {
    let Some(limiter) = limiter else {
        return TDLIMIT_INVALID_HANDLE;
    };

    // calculate  release time in samples
    let release = release_ms.wrapping_mul(limiter.sample_rate) / 1000;

    // releaseConst  = (float)pow(0.1, 1.0 / (release + 1))
    let release_const = pow_0_1(release);

    limiter.release_const = release_const;
    limiter.release_ms = release_ms;

    TDLIMIT_OK
}

/// `pcmLimiter_GetLibInfo(info)`: enters the library information of the limiter into the first
/// free entry of `info` (an array of `FDK_MODULE_LAST` entries), then the FDK tools'.
///
/// A slice shorter than `FDK_MODULE_LAST` entries without a free entry is treated as full.
pub fn pcm_limiter_get_lib_info(info: Option<&mut [LibInfo]>) -> TdLimiterError {
    let Some(info) = info else {
        return TDLIMIT_INVALID_PARAMETER;
    };

    // Search for next free tab
    let mut i = 0usize;
    while i < FDK_MODULE_LAST as usize {
        match info.get(i) {
            Some(e) if e.module_id == FDK_NONE => break,
            Some(_) => i += 1,
            None => return TDLIMIT_UNKNOWN,
        }
    }
    if i == FDK_MODULE_LAST as usize {
        return TDLIMIT_UNKNOWN;
    }

    // Add the library info
    let e = &mut info[i];
    e.module_id = FDK_TDLIMIT;
    e.version = lib_version(PCMUTIL_LIB_VL0, PCMUTIL_LIB_VL1, PCMUTIL_LIB_VL2);
    lib_version_string(e);
    e.build_date = PCMUTIL_LIB_BUILD_DATE;
    e.build_time = PCMUTIL_LIB_BUILD_TIME;
    e.title = TDLIMIT_LIB_TITLE;

    // Set flags
    e.flags = CAPF_LIMITER;

    // Add lib info for FDK tools (if not yet done).
    fdk_tools_get_lib_info(Some(info));

    TDLIMIT_OK
}
