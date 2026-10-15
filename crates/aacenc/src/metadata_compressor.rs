// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/metadata_compressor.h, libAACenc/src/metadata_compressor.cpp.

//! Port of `metadata_compressor.h` and `metadata_compressor.cpp`: the DRC compressor (gain
//! generator for the MPEG DRC `dynrng` and the ETSI `compr` values).
//!
//! `DRC_COMP` is defined in the `.cpp` file; [`DrcComp`] has its C layout, so that the tests can
//! compare the complete state. `FDK_DRC_Generator_Open()` allocates it (`Box`).
//!
//! # Input samples
//!
//! `FDK_DRC_Generator_Calc()` reads its input in two ways: the loudness measurement reads channel
//! `c` at `inSamples[c * inSamplesBufSize + i]` (planar, as the encoder passes it), the peak
//! measurement (`findPeakLevels()`) at `inSamples[i * channels + channelIdx]` (interleaved). The
//! port reproduces both. Bounded: reads outside the slice return 0.
//!
//! # Compiled undefined behavior
//!
//! Some `fixMax()` calls of `findPeakLevels()` are compiled as unsigned comparisons (see
//! `fix_max_after_abs`); the port reproduces this.

use crate::aacenc::{ChannelOrder, AAC_ENC_OK};
use crate::channel_map::fdk_aac_enc_init_channel_mapping;
use crate::qc_data::ChannelMapping;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::clz::fixnormz_d;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fixpoint_math::{calc_ld_data, f2_pow, f_div_norm, f_mult_norm, LD_DATA_SHIFT};
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::scale::scale_value;

/// `LFE_LEV_SCALE`
pub const LFE_LEV_SCALE: i32 = 2;

/// `DRC_PROFILE`: DRC compression profiles.
pub type DrcProfile = i32;
/// `DRC_NONE`
pub const DRC_NONE: DrcProfile = 0;
/// `DRC_FILMSTANDARD`
pub const DRC_FILMSTANDARD: DrcProfile = 1;
/// `DRC_FILMLIGHT`
pub const DRC_FILMLIGHT: DrcProfile = 2;
/// `DRC_MUSICSTANDARD`
pub const DRC_MUSICSTANDARD: DrcProfile = 3;
/// `DRC_MUSICLIGHT`
pub const DRC_MUSICLIGHT: DrcProfile = 4;
/// `DRC_SPEECH`
pub const DRC_SPEECH: DrcProfile = 5;
/// `DRC_DELAY_TEST`
pub const DRC_DELAY_TEST: DrcProfile = 6;
/// `DRC_NOT_PRESENT`
pub const DRC_NOT_PRESENT: DrcProfile = -2;

/// `FIXP_ILOG2_DIV2`: `FL2FXCONST_DBL(ILOG2 / 2)` with `ILOG2 = 1.442695041f` (1/LOG2)
const FIXP_ILOG2_DIV2: FixpDbl = fl2fxconst_dbl((1.442695041f32 / 2.0f32) as f64);

/// Max number of audio input channels.
pub const MAX_DRC_CHANNELS: usize = 8;
/// Max 8 channel.
const DOWNMIX_SHIFT: i32 = 3;
/// Scaling used in weighting filter.
const WEIGHTING_FILTER_SHIFT: i32 = 2;

const METADATA_INT_BITS: i32 = 10;
const METADATA_LINT_BITS: i32 = 20;
/// `METADATA_INT_SCALE` (`INT64(1) << METADATA_INT_BITS`)
const METADATA_INT_SCALE: i64 = 1i64 << METADATA_INT_BITS;
const METADATA_FRACT_BITS: i32 = DFRACT_BITS - 1 - METADATA_INT_BITS;
/// `METADATA_FRACT_SCALE` (`INT64(1) << METADATA_FRACT_BITS`)
const METADATA_FRACT_SCALE: i64 = 1i64 << METADATA_FRACT_BITS;

// Enum for channel assignment.
const L: usize = 0;
const R: usize = 1;
const C: usize = 2;
const LFE: usize = 3;
const LS: usize = 4;
const RS: usize = 5;
const S: usize = 6;
const LS2: usize = 7;
const RS2: usize = 8;

/// `struct WEIGHTING_STATES`: weighting filter filter states.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeightingStates {
    pub x1: FixpDbl,
    pub x2: FixpDbl,
    pub y1: FixpDbl,
    pub y2: FixpDbl,
}

/// `struct DRC_COMP`: Dynamic Range Control compressor structure (layout identical to the C
/// struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrcComp {
    /// Max boost threshold.
    pub max_boost_thr: [FixpDbl; 2],
    /// Boost threshold.
    pub boost_thr: [FixpDbl; 2],
    /// Early cut threshold.
    pub early_cut_thr: [FixpDbl; 2],
    /// Cut threshold.
    pub cut_thr: [FixpDbl; 2],
    /// Max cut threshold.
    pub max_cut_thr: [FixpDbl; 2],

    /// Precalculated factor for boost compression.
    pub boost_fac: [FixpDbl; 2],
    /// Precalculated factor for early cut compression.
    pub early_cut_fac: [FixpDbl; 2],
    /// Precalculated factor for cut compression.
    pub cut_fac: [FixpDbl; 2],

    /// Maximum boost.
    pub max_boost: [FixpDbl; 2],
    /// Maximum cut.
    pub max_cut: [FixpDbl; 2],
    /// Maximum early cut.
    pub max_early_cut: [FixpDbl; 2],

    /// Fast attack coefficient.
    pub fast_attack: [FixpDbl; 2],
    /// Fast release coefficient.
    pub fast_decay: [FixpDbl; 2],
    /// Slow attack coefficient.
    pub slow_attack: [FixpDbl; 2],
    /// Slow release coefficient.
    pub slow_decay: [FixpDbl; 2],
    /// Hold time in blocks.
    pub hold_off: [u32; 2],

    /// Slow/fast attack threshold.
    pub attack_thr: [FixpDbl; 2],
    /// Slow/fast release threshold.
    pub decay_thr: [FixpDbl; 2],

    /// DRC profile.
    pub profile: [DrcProfile; 2],
    /// Block length in samples.
    pub block_length: i32,
    /// Sample rate.
    pub sample_rate: u32,
    /// Channel configuration.
    pub chan_config: ChannelMode,

    /// Use weighting filter.
    pub use_weighting: u8,
    pub _pad0: [u8; 3],

    /// Number of channels.
    pub channels: u32,
    /// Number of full range channels.
    pub full_channels: u32,
    /// Offsets of interleaved channel samples (L, R, C, LFE, Ls, Rs, S, Ls2, Rs2).
    pub channel_idx: [i32; 9],

    /// level smoothing states
    pub smooth_level: [FixpDbl; 2],
    /// gain smoothing states
    pub smooth_gain: [FixpDbl; 2],
    /// hold counter
    pub hold_cnt: [u32; 2],

    /// limiter gain
    pub lim_gain: [FixpDbl; 2],
    /// limiter decay (linear)
    pub lim_decay: FixpDbl,
    /// max peak of previous block (stereo/mono)
    pub prev_peak: [FixpDbl; 2],

    /// array holds weighting filter states
    pub filter: [WeightingStates; MAX_DRC_CHANNELS],
}

// Profile tables.

/// `tabMaxBoostThr` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabMaxBoostThr LONG`)
pub static TAB_MAX_BOOST_THR: [FixpDbl; 6] =
    [-90177536, -111149056, -115343360, -136314880, -104857600, -83886080];
/// `tabBoostThr` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabBoostThr LONG`)
pub static TAB_BOOST_THR: [FixpDbl; 6] =
    [-65011712, -85983232, -65011712, -85983232, -65011712, -65011712];
/// `tabEarlyCutThr` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabEarlyCutThr LONG`)
pub static TAB_EARLY_CUT_THR: [FixpDbl; 6] =
    [-54525952, -44040192, -54525952, -44040192, -54525952, -41943040];
/// `tabCutThr` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabCutThr LONG`)
pub static TAB_CUT_THR: [FixpDbl; 6] =
    [-33554432, -23068672, -33554432, -44040192, -33554432, -20971520];
/// `tabMaxCutThr` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabMaxCutThr LONG`)
pub static TAB_MAX_CUT_THR: [FixpDbl; 6] = [8388608, 18874368, 8388608, 18874368, 8388608, 8388608];
/// `tabBoostRatio` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabBoostRatio LONG`)
pub static TAB_BOOST_RATIO: [FixpDbl; 6] =
    [-1073741824, -1073741824, -1073741824, -1073741824, -1717986944, -1717986944];
/// `tabEarlyCutRatio` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabEarlyCutRatio LONG`)
pub static TAB_EARLY_CUT_RATIO: [FixpDbl; 6] =
    [-1073741824, -1073741824, -1073741824, 0, -1073741824, -1073741824];
/// `tabCutRatio` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabCutRatio LONG`)
pub static TAB_CUT_RATIO: [FixpDbl; 6] =
    [-2040109440, -2040109440, -2040109440, -1073741824, -2040109440, -2040109440];
/// `tabMaxBoost` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabMaxBoost LONG`)
pub static TAB_MAX_BOOST: [FixpDbl; 6] =
    [12582912, 12582912, 25165824, 25165824, 31457280, 31457280];
/// `tabMaxCut` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabMaxCut LONG`)
pub static TAB_MAX_CUT: [FixpDbl; 6] = [50331648, 50331648, 50331648, 31457280, 50331648, 50331648];
/// `tabFastAttack` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabFastAttack LONG`)
pub static TAB_FAST_ATTACK: [FixpDbl; 6] = [20972, 20972, 20972, 20972, 20972, 0];
/// `tabFastDecay` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabFastDecay LONG`)
pub static TAB_FAST_DECAY: [FixpDbl; 6] = [2097152, 2097152, 2097152, 2097152, 419430, 0];
/// `tabSlowAttack` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabSlowAttack LONG`)
pub static TAB_SLOW_ATTACK: [FixpDbl; 6] = [209715, 209715, 209715, 209715, 209715, 0];
/// `tabSlowDecay` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabSlowDecay LONG`)
pub static TAB_SLOW_DECAY: [FixpDbl; 6] = [6291456, 6291456, 20971520, 6291456, 2097152, 0];
/// `tabHoldOff` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabHoldOff INT`)
pub static TAB_HOLD_OFF: [i32; 6] = [10, 10, 10, 10, 10, 0];
/// `tabAttackThr` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabAttackThr LONG`)
pub static TAB_ATTACK_THR: [FixpDbl; 6] = [31457280, 31457280, 31457280, 31457280, 20971520, 0];
/// `tabDecayThr` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp tabDecayThr LONG`)
pub static TAB_DECAY_THR: [FixpDbl; 6] = [41943040, 41943040, 41943040, 41943040, 20971520, 0];

// Weighting filter coefficients (biquad bandpass).

/// `b0` (b1 = 0, b2 = -b0; dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp &b0 LONG --count 1`)
pub const B0: FixpDbl = 1139254272;
/// `a1` (dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp &a1 LONG --count 1`)
pub const A1: FixpDbl = -2045220096;
/// `a2` (a0 = 1; dumped with `scripts/dump_table.py libAACenc/src/metadata_compressor.cpp &a2 LONG --count 1`)
pub const A2: FixpDbl = -48293384;

/// `getShiftFactor(const UINT length)` (file-static): calculate scaling factor for denoted
/// processing block.
///
/// Deviation: the reference has no defined result for `length > 2^31`; the port stops at 32.
pub fn get_shift_factor(length: u32) -> u32 {
    let mut ld_n: u32 = 1;
    while ld_n < 32 && (1u32 << ld_n) < length {
        ld_n += 1;
    }
    ld_n
}

/// `fixpAdd(const FIXP_DBL value1, const int q1, FIXP_DBL* const pValue2, int* const pQ2)`
/// (file-static): sum up fixpoint values with best possible accuracy.
///
/// `scaleValue()` with amounts outside of -31..31 is compiled to register shifts (`lsl`/`asr`,
/// amount modulo 32), as [`scale_value`] computes it.
pub fn fixp_add(value1: FixpDbl, q1: i32, p_value2: &mut FixpDbl, p_q2: &mut i32) {
    let headroom1 = f_normz(fixp_abs(value1)) - 1;
    let headroom2 = f_normz(fixp_abs(*p_value2)) - 1;
    let mut result_scale = fix_max(q1.wrapping_sub(headroom1), (*p_q2).wrapping_sub(headroom2));

    if value1 != 0 && *p_value2 != 0 {
        result_scale += 1;
    }

    *p_value2 = scale_value(value1, q1.wrapping_sub(result_scale))
        .wrapping_add(scale_value(*p_value2, (*p_q2).wrapping_sub(result_scale)));
    *p_q2 = if *p_value2 != 0 { result_scale } else { DFRACT_BITS - 1 };
}

/// `fixpAdd(accu, granuleShift, &level_b, &level_e)` as both reference builds compiled it into
/// the unweighted loudness measurement of `FDK_DRC_Generator_Calc()`.
///
/// There, `accu` is a sum of `fPow2Div2()` values; since signed overflow is undefined, clang
/// assumes `accu >= 0`: it drops `fixp_abs(value1)` (`clz`/`bsr` of `value1` itself) and shifts
/// `value1` right logically (`lsr`/`shrl`) in `scaleValue(value1, q1 - resultScale)`. The sum
/// does overflow for 16 samples of -32768 (16 * 2^27 = 2^31), and then the result differs from
/// [`fixp_add`] (an arithmetic shift of `INT_MIN`). The weighted measurement keeps the
/// `fixp_abs()` and the arithmetic shift ([`fixp_add`]).
pub fn fixp_add_unweighted(value1: FixpDbl, q1: i32, p_value2: &mut FixpDbl, p_q2: &mut i32) {
    let headroom1 = fixnormz_d(value1) - 1;
    let headroom2 = f_normz(fixp_abs(*p_value2)) - 1;
    let mut result_scale = fix_max(q1.wrapping_sub(headroom1), (*p_q2).wrapping_sub(headroom2));

    if value1 != 0 && *p_value2 != 0 {
        result_scale += 1;
    }

    let s1 = q1.wrapping_sub(result_scale);
    let scaled1 = if s1 > 0 {
        value1.wrapping_shl(s1 as u32)
    } else {
        ((value1 as u32).wrapping_shr(s1.wrapping_neg() as u32)) as i32
    };
    *p_value2 = scaled1.wrapping_add(scale_value(*p_value2, (*p_q2).wrapping_sub(result_scale)));
    *p_q2 = if *p_value2 != 0 { result_scale } else { DFRACT_BITS - 1 };
}

/// `tc2Coeff(const FIXP_DBL t, const INT sampleRate, const INT blockLength)` (file-static):
/// function for converting time constant to filter coefficient;
/// `result = 1.0 - exp(-1.0/((t) * (f)))`.
pub fn tc2_coeff(t: FixpDbl, sample_rate: i32, block_length: i32) -> FixpDbl {
    let mut e_res: i32 = 0;

    // f = sampleRate/blockLength
    let sample_rate_fract: FixpDbl = sample_rate << (DFRACT_BITS - 1 - METADATA_LINT_BITS);
    let block_length_fract: FixpDbl = block_length << (DFRACT_BITS - 1 - METADATA_LINT_BITS);
    let mut f = f_div_norm(sample_rate_fract, block_length_fract, &mut e_res);
    f = scale_value(f, e_res.wrapping_sub(METADATA_INT_BITS)); // convert to METADATA_FRACT

    // product = t*f
    let mut product = f_mult_norm(t, f, &mut e_res);
    product = scale_value(product, e_res.wrapping_add(METADATA_INT_BITS)); // convert to METADATA_FRACT

    // exponent = (-1.0/((t) * (f)))
    let mut exponent = f_div_norm(METADATA_FRACT_SCALE as FixpDbl, product, &mut e_res);
    exponent = scale_value(exponent, e_res.wrapping_sub(METADATA_INT_BITS)); // convert to METADATA_FRACT

    // exponent * ld(e)
    exponent = f_mult(exponent, FIXP_ILOG2_DIV2) << 1; // e^(x) = 2^(x*ld(e))

    // exp(-1.0/((t) * (f)))
    let mut result =
        f2_pow(exponent.wrapping_neg(), DFRACT_BITS - 1 - METADATA_FRACT_BITS, &mut e_res);

    // result = 1.0 - exp(-1.0/((t) * (f)))
    result = MAXVAL_DBL.wrapping_sub(scale_value(result, e_res));

    result
}

/// `pSamples[idx]` of `findPeakLevels()` with `pSamples = &inSamples[base]`; 0 outside the
/// slice (see the [module documentation](self)).
#[inline(always)]
fn sample(in_samples: &[IntPcm], base: usize, idx: i32) -> FixpSgl {
    let i = base as isize + idx as isize;
    if i < 0 {
        0
    } else {
        in_samples.get(i as usize).copied().unwrap_or(0)
    }
}

/// `fixMax(peak[0], fixp_abs(tmp))` at the second and third comparison of an iteration of the
/// 7.1/6.1-to-5.1 loop and at the Rt and Ro comparisons of `findPeakLevels()`, as the reference
/// builds compile it: an **unsigned** maximum.
///
/// `fAbs()` negates with signed overflow, so the compiler takes its result as non-negative, and
/// after the first `fixMax()` of an iteration (which it compiles as a signed maximum) it takes
/// `peak[0]` as non-negative too; it then compiles the following `fixMax()` calls of the same
/// iteration as unsigned comparisons (`csel ..., hi` on arm64, `ja`/`cmova` on x86-64 and
/// x86-32; the same in ref-arm64, ref-x86_64-mac, ref-x86_64 and ref-i686). The two differ when
/// `fixp_abs()` returns `MINVAL_DBL` (a downmix of exactly `MINVAL_DBL` after the wrapping
/// `<< 10` of the downmix gains, e.g. 16 dB gains on a sample of 1024 modulo 2048) or when
/// `peak[0]` is negative: the unsigned maximum then makes `peak[0]` `MINVAL_DBL`, and the next
/// signed maximum replaces it.
#[inline(always)]
fn fix_max_after_abs(peak: FixpDbl, abs: FixpDbl) -> FixpDbl {
    if (peak as u32) > (abs as u32) {
        peak
    } else {
        abs
    }
}

/// `findPeakLevels(HDRC_COMP drcComp, const INT_PCM* const inSamples, const FIXP_DBL clev,
/// const FIXP_DBL slev, const FIXP_DBL ext_leva, const FIXP_DBL ext_levb, const FIXP_DBL lfe_lev,
/// const FIXP_DBL dmxGain5, const FIXP_DBL dmxGain2, FIXP_DBL peak[2])` (file-static).
#[allow(clippy::too_many_arguments)]
pub fn find_peak_levels(
    drc_comp: &DrcComp,
    in_samples: &[IntPcm],
    clev: FixpDbl,
    slev: FixpDbl,
    ext_leva: FixpDbl,
    ext_levb: FixpDbl,
    lfe_lev: FixpDbl,
    dmx_gain5: FixpDbl,
    dmx_gain2: FixpDbl,
    peak: &mut [FixpDbl; 2],
) {
    let ci = &drc_comp.channel_idx;
    let mut max_sample: IntPcm = 0;

    // pSamples = &inSamples[i * drcComp->channels] (UINT arithmetic)
    let base = |i: i32| (i as u32).wrapping_mul(drc_comp.channels) as usize;
    // pSamples[channelIdx[k]]
    let smp = |b: usize, k: usize| sample(in_samples, b, ci[k]);
    // fMultDiv2(g, (FIXP_PCM)pSamples[channelIdx[k]]) >> (DOWNMIX_SHIFT - 1)
    let md2 = |g: FixpDbl, b: usize, k: usize| f_mult_div2(g, smp(b, k)) >> (DOWNMIX_SHIFT - 1);
    // fMultDiv2(lfe_lev, (FIXP_PCM)pSamples[channelIdx[k]]) >> (DOWNMIX_SHIFT - 1 - LFE_LEV_SCALE)
    let lfe_d2 =
        |b: usize, k: usize| f_mult_div2(lfe_lev, smp(b, k)) >> (DOWNMIX_SHIFT - 1 - LFE_LEV_SCALE);
    // fMult(lfe_lev, (FIXP_PCM)pSamples[channelIdx[k]]) >> (DOWNMIX_SHIFT - 1 - LFE_LEV_SCALE)
    let lfe_m =
        |b: usize, k: usize| f_mult(lfe_lev, smp(b, k)) >> (DOWNMIX_SHIFT - 1 - LFE_LEV_SCALE);
    // fMult(g, (FIXP_PCM)pSamples[channelIdx[k]]) >> (DOWNMIX_SHIFT - 1)
    let mm = |g: FixpDbl, b: usize, k: usize| f_mult(g, smp(b, k)) >> (DOWNMIX_SHIFT - 1);
    // FX_PCM2FX_DBL((FIXP_PCM)pSamples[channelIdx[k]]) >> DOWNMIX_SHIFT
    let full = |b: usize, k: usize| fx_pcm2fx_dbl(smp(b, k)) >> DOWNMIX_SHIFT;
    // apply scaling of downmix gains
    // only for positive values only, as legacy decoders might not know this parameter
    let dmx = |mut tmp: FixpDbl| {
        if dmx_gain2 > 0 {
            if drc_comp.full_channels > 5 {
                tmp = f_mult(dmx_gain5, tmp) << (DFRACT_BITS - 1 - METADATA_FRACT_BITS);
            }
            tmp = f_mult(dmx_gain2, tmp) << (DFRACT_BITS - 1 - METADATA_FRACT_BITS);
        }
        tmp
    };
    let c707 = fl2fxconst_dbl(0.707f32 as f64);
    let c7 = fl2fxconst_dbl(0.7f32 as f64);

    // find peak level
    peak[0] = 0;
    peak[1] = 0;
    for i in 0..drc_comp.block_length {
        let b = base(i);
        // single channels
        for c in 0..drc_comp.channels as i32 {
            max_sample = f_max(max_sample, f_abs(sample(in_samples, b, c)));
        }
    }
    peak[0] = fix_max(peak[0], fx_pcm2fx_dbl(max_sample) >> DOWNMIX_SHIFT);

    // 7.1/6.1 to 5.1 downmixes
    if drc_comp.full_channels > 5 {
        for i in 0..drc_comp.block_length {
            let b = base(i);

            // channel 1 (L, Ls,...)
            let mut tmp: FixpDbl = 0;
            match drc_comp.chan_config {
                MODE_6_1 => {
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 4)); // Ls
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 6)); // Cs
                }
                MODE_7_1_BACK | MODE_7_1_REAR_SURROUND => {
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 4)); // Ls
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 7)); // Lrs / Lss
                }
                MODE_1_2_2_2_1 | MODE_7_1_FRONT_CENTER => {
                    tmp = tmp.wrapping_add(full(b, 0)); // L
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 7)); // Lc
                }
                MODE_7_1_TOP_FRONT => {
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 0)); // L
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 7)); // Lvh
                }
                _ => {}
            }
            peak[0] = fix_max(peak[0], fixp_abs(tmp));

            // channel 2 (R, Rs,...)
            tmp = 0;
            match drc_comp.chan_config {
                MODE_6_1 => {
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 5)); // Rs
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 6)); // Cs
                }
                MODE_7_1_BACK | MODE_7_1_REAR_SURROUND => {
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 5)); // Rs
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 8)); // Rrs / Rss
                }
                MODE_1_2_2_2_1 | MODE_7_1_FRONT_CENTER => {
                    tmp = tmp.wrapping_add(full(b, 1)); // R
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 8)); // Rc
                }
                MODE_7_1_TOP_FRONT => {
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 1)); // R
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 8)); // Rvh
                }
                _ => {}
            }
            peak[0] = fix_max_after_abs(peak[0], fixp_abs(tmp));

            // channel 3 (C)
            tmp = 0;
            match drc_comp.chan_config {
                MODE_1_2_2_2_1 | MODE_7_1_FRONT_CENTER => {
                    tmp = tmp.wrapping_add(full(b, 2)); // C
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 7)); // Lc
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 8)); // Rc
                }
                _ => {}
            }
            peak[0] = fix_max_after_abs(peak[0], fixp_abs(tmp));
        } // for (blocklength)

        // take downmix gain into accout
        peak[0] = f_mult(dmx_gain5, peak[0]) << (DFRACT_BITS - 1 - METADATA_FRACT_BITS);
    }

    // 7.1 / 5.1 to stereo downmixes
    if drc_comp.full_channels > 2 {
        // Lt/Rt downmix
        for i in 0..drc_comp.block_length {
            let b = base(i);

            // Lt
            let mut tmp: FixpDbl = 0;
            if ci[LS] >= 0 {
                tmp = tmp.wrapping_sub(md2(c707, b, LS)); // Ls
            }
            if ci[LS2] >= 0 {
                tmp = tmp.wrapping_sub(md2(c707, b, LS2)); // Ls2
            }
            if ci[RS] >= 0 {
                tmp = tmp.wrapping_sub(md2(c707, b, RS)); // Rs
            }
            if ci[RS2] >= 0 {
                tmp = tmp.wrapping_sub(md2(c707, b, RS2)); // Rs2
            }
            if ci[LS] >= 0 && ci[LS2] >= 0 {
                tmp = f_mult(c707, tmp); // 7.1ch
            }
            if ci[S] >= 0 {
                tmp = tmp.wrapping_sub(md2(c707, b, S)); // S
            }
            if ci[C] >= 0 {
                tmp = tmp.wrapping_add(md2(c707, b, C)); // C
            }
            tmp = tmp.wrapping_add(full(b, L)); // L

            tmp = dmx(tmp);
            peak[0] = fix_max(peak[0], fixp_abs(tmp));

            // Rt
            tmp = 0;
            if ci[LS] >= 0 {
                tmp = tmp.wrapping_add(md2(c707, b, LS)); // Ls
            }
            if ci[LS2] >= 0 {
                tmp = tmp.wrapping_add(md2(c707, b, LS2)); // Ls2
            }
            if ci[RS] >= 0 {
                tmp = tmp.wrapping_add(md2(c707, b, RS)); // Rs
            }
            if ci[RS2] >= 0 {
                tmp = tmp.wrapping_add(md2(c707, b, RS2)); // Rs2
            }
            if ci[RS] >= 0 && ci[RS2] >= 0 {
                tmp = f_mult(c707, tmp); // 7.1ch
            }
            if ci[S] >= 0 {
                tmp = tmp.wrapping_add(md2(c707, b, S)); // S
            }
            if ci[C] >= 0 {
                tmp = tmp.wrapping_add(md2(c707, b, C)); // C
            }
            tmp = tmp.wrapping_add(full(b, R)); // R

            tmp = dmx(tmp);
            peak[0] = fix_max_after_abs(peak[0], fixp_abs(tmp));
        }

        // Lo/Ro downmix
        for i in 0..drc_comp.block_length {
            let b = base(i);

            // Lo
            let mut tmp: FixpDbl = 0;
            match drc_comp.chan_config {
                MODE_6_1 => {
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_leva), b, 4)); // Ls
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_levb), b, 6)); // Cs
                    tmp = tmp.wrapping_add(md2(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(full(b, 0)); // L
                    tmp = tmp.wrapping_add(lfe_d2(b, 3)); // LFE
                }
                MODE_7_1_BACK | MODE_7_1_REAR_SURROUND => {
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_leva), b, 4)); // Ls
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_levb), b, 7)); // Lrs / Lss
                    tmp = tmp.wrapping_add(md2(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(full(b, 0)); // L
                    tmp = tmp.wrapping_add(lfe_d2(b, 3)); // LFE
                }
                MODE_1_2_2_2_1 | MODE_7_1_FRONT_CENTER => {
                    tmp = tmp.wrapping_add(full(b, 0)); // L
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 7)); // Lc
                    tmp = tmp.wrapping_add(md2(f_mult(ext_leva, clev), b, 7)); // Lc - second path
                    tmp = tmp.wrapping_add(md2(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(md2(slev, b, 4)); // Ls
                    tmp = tmp.wrapping_add(lfe_d2(b, 3)); // LFE
                }
                MODE_7_1_TOP_FRONT => {
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 0)); // L
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 7)); // Lvh
                    tmp = tmp.wrapping_add(md2(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(md2(slev, b, 4)); // Ls
                    tmp = tmp.wrapping_add(lfe_d2(b, 3)); // LFE
                }
                _ => {
                    if ci[LS] >= 0 {
                        tmp = tmp.wrapping_add(md2(slev, b, LS)); // Ls
                    }
                    if ci[LS2] >= 0 {
                        tmp = tmp.wrapping_add(md2(slev, b, LS2)); // Ls2
                    }
                    if ci[LS] >= 0 && ci[LS2] >= 0 {
                        tmp = f_mult(c707, tmp); // 7.1ch
                    }
                    if ci[S] >= 0 {
                        // S
                        tmp = tmp.wrapping_add(
                            f_mult_div2(slev, f_mult(c7, smp(b, S))) >> (DOWNMIX_SHIFT - 1),
                        );
                    }
                    if ci[C] >= 0 {
                        tmp = tmp.wrapping_add(md2(clev, b, C)); // C
                    }
                    if ci[3] >= 0 {
                        tmp = tmp.wrapping_add(lfe_d2(b, 3)); // LFE
                    }
                    tmp = tmp.wrapping_add(full(b, L)); // L
                }
            }

            tmp = dmx(tmp);
            peak[0] = fix_max(peak[0], fixp_abs(tmp));

            // Ro
            tmp = 0;
            match drc_comp.chan_config {
                MODE_6_1 => {
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_leva), b, 5)); // Rs
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_levb), b, 6)); // Cs
                    tmp = tmp.wrapping_add(md2(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(full(b, 1)); // R
                    tmp = tmp.wrapping_add(lfe_d2(b, 3)); // LFE
                }
                MODE_7_1_BACK | MODE_7_1_REAR_SURROUND => {
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_leva), b, 5)); // Rs
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_levb), b, 8)); // Rrs / Rss
                    tmp = tmp.wrapping_add(md2(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(full(b, 1)); // R
                    tmp = tmp.wrapping_add(lfe_d2(b, 3)); // LFE
                }
                MODE_1_2_2_2_1 | MODE_7_1_FRONT_CENTER => {
                    tmp = tmp.wrapping_add(full(b, 1)); // R
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 8)); // Rc
                    tmp = tmp.wrapping_add(md2(f_mult(ext_leva, clev), b, 8)); // Rc - second path
                    tmp = tmp.wrapping_add(md2(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(md2(slev, b, 5)); // Rs
                    tmp = tmp.wrapping_add(lfe_d2(b, 3)); // LFE
                }
                MODE_7_1_TOP_FRONT => {
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 1)); // R
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 8)); // Rvh
                    tmp = tmp.wrapping_add(md2(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(md2(slev, b, 5)); // Rs
                    tmp = tmp.wrapping_add(lfe_d2(b, 3)); // LFE
                }
                _ => {
                    if ci[RS] >= 0 {
                        tmp = tmp.wrapping_add(md2(slev, b, RS)); // Rs
                    }
                    if ci[RS2] >= 0 {
                        tmp = tmp.wrapping_add(md2(slev, b, RS2)); // Rs2
                    }
                    if ci[RS] >= 0 && ci[RS2] >= 0 {
                        tmp = f_mult(c707, tmp); // 7.1ch
                    }
                    if ci[S] >= 0 {
                        // S
                        tmp = tmp.wrapping_add(
                            f_mult_div2(slev, f_mult(c7, smp(b, S))) >> (DOWNMIX_SHIFT - 1),
                        );
                    }
                    if ci[C] >= 0 {
                        tmp = tmp.wrapping_add(md2(clev, b, C)); // C
                    }
                    if ci[3] >= 0 {
                        tmp = tmp.wrapping_add(lfe_d2(b, 3)); // LFE
                    }
                    tmp = tmp.wrapping_add(full(b, R)); // R
                }
            }

            tmp = dmx(tmp);
            peak[0] = fix_max_after_abs(peak[0], fixp_abs(tmp));
        }
    }

    peak[1] = fix_max(peak[0], peak[1]);

    // Mono Downmix - for comp_val only
    if drc_comp.full_channels > 1 {
        for i in 0..drc_comp.block_length {
            let b = base(i);

            let mut tmp: FixpDbl = 0;
            match drc_comp.chan_config {
                MODE_6_1 => {
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_leva), b, 4)); // Ls
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_leva), b, 5)); // Rs
                    tmp = tmp.wrapping_add(mm(f_mult(slev, ext_levb), b, 6)); // Cs
                    tmp = tmp.wrapping_add(mm(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(full(b, 0)); // L
                    tmp = tmp.wrapping_add(full(b, 1)); // R
                    tmp = tmp.wrapping_add(lfe_m(b, 3)); // LFE
                }
                MODE_7_1_BACK | MODE_7_1_REAR_SURROUND => {
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_leva), b, 4)); // Ls
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_leva), b, 5)); // Rs
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_levb), b, 7)); // Lrs / Lss
                    tmp = tmp.wrapping_add(md2(f_mult(slev, ext_levb), b, 8)); // Rrs / Rss
                    tmp = tmp.wrapping_add(mm(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(full(b, 0)); // L
                    tmp = tmp.wrapping_add(full(b, 1)); // R
                    tmp = tmp.wrapping_add(lfe_m(b, 3)); // LFE
                }
                MODE_1_2_2_2_1 | MODE_7_1_FRONT_CENTER => {
                    tmp = tmp.wrapping_add(full(b, 0)); // L
                    tmp = tmp.wrapping_add(full(b, 1)); // R
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 7)); // Lc
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 8)); // Rc
                    tmp = tmp.wrapping_add(md2(f_mult(ext_leva, clev), b, 7)); // Lc - second path
                    tmp = tmp.wrapping_add(md2(f_mult(ext_leva, clev), b, 8)); // Rc - second path
                    tmp = tmp.wrapping_add(mm(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(md2(slev, b, 4)); // Ls
                    tmp = tmp.wrapping_add(md2(slev, b, 5)); // Rs
                    tmp = tmp.wrapping_add(lfe_m(b, 3)); // LFE
                }
                MODE_7_1_TOP_FRONT => {
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 0)); // L
                    tmp = tmp.wrapping_add(md2(ext_leva, b, 1)); // R
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 7)); // Lvh
                    tmp = tmp.wrapping_add(md2(ext_levb, b, 8)); // Rvh
                    tmp = tmp.wrapping_add(mm(clev, b, 2)); // C
                    tmp = tmp.wrapping_add(md2(slev, b, 4)); // Ls
                    tmp = tmp.wrapping_add(md2(slev, b, 5)); // Rs
                    tmp = tmp.wrapping_add(lfe_m(b, 3)); // LFE
                }
                _ => {
                    if ci[LS] >= 0 {
                        tmp = tmp.wrapping_add(md2(slev, b, LS)); // Ls
                    }
                    if ci[LS2] >= 0 {
                        tmp = tmp.wrapping_add(md2(slev, b, LS2)); // Ls2
                    }
                    if ci[RS] >= 0 {
                        tmp = tmp.wrapping_add(md2(slev, b, RS)); // Rs
                    }
                    if ci[RS2] >= 0 {
                        tmp = tmp.wrapping_add(md2(slev, b, RS2)); // Rs2
                    }
                    if ci[LS] >= 0 && ci[LS2] >= 0 {
                        tmp = f_mult(c707, tmp); // 7.1ch
                    }
                    // if ((drcComp->channelIdx[RS] >= 0) && (drcComp->channelIdx[RS2] >= 0)) tmp *=0.707f; /* 7.1ch */
                    if ci[S] >= 0 {
                        // S
                        tmp = tmp.wrapping_add(
                            f_mult_div2(slev, f_mult(c7, smp(b, S))) >> (DOWNMIX_SHIFT - 1),
                        );
                    }
                    if ci[C] >= 0 {
                        tmp = tmp.wrapping_add(mm(clev, b, C)); // C (2*clev)
                    }
                    if ci[3] >= 0 {
                        tmp = tmp.wrapping_add(lfe_m(b, 3)); // LFE
                    }
                    tmp = tmp.wrapping_add(full(b, L)); // L
                    tmp = tmp.wrapping_add(full(b, R)); // R
                }
            }

            tmp = dmx(tmp);
            peak[1] = fix_max(peak[1], fixp_abs(tmp));
        }
    }
}

/// `FDK_DRC_Generator_Open(HDRC_COMP *phDrcComp)`: open a DRC Compressor instance (allocate
/// memory for a compressor instance). Returns 0 on success.
///
/// The `NULL` handle pointer of C has no counterpart; an allocation failure aborts.
pub fn fdk_drc_generator_open(ph_drc_comp: &mut Option<Box<DrcComp>>) -> i32 {
    // allocate memory; FDKmemclear(hDcComp, sizeof(DRC_COMP))
    // Return drc compressor instance
    *ph_drc_comp = Some(Box::default());
    0
}

/// `FDK_DRC_Generator_Close(HDRC_COMP *phDrcComp)`: deallocate instance and free whole memory.
/// Returns 0 on success.
pub fn fdk_drc_generator_close(ph_drc_comp: &mut Option<Box<DrcComp>>) -> i32 {
    *ph_drc_comp = None;
    0
}

/// `FL2FXCONST_DBL(((0.006f / 256) * blockLength) / METADATA_INT_SCALE)` of
/// `FDK_DRC_Generator_Initialize`, evaluated at run time.
///
/// The 64-bit builds round the `float` expression to `float` and evaluate the macro in `double`.
/// The `x86_32` build keeps everything on the x87 unit in extended precision (`fildl`, `fmuls`
/// with `0.006f / 256` and `1.0f / 1024`, `fmuls 2^31`, `fadds 0.5f` resp. `-0.5f`, the comparison
/// with `2147483647.0` resp. `-2147483648.0f`, `fistpl`; [`fdk_aac_rs_fdk::x87`]).
fn lim_decay(block_length: i32) -> FixpDbl {
    if fdk_aac_rs_fdk::ARCH == fdk_aac_rs_fdk::Arch::X86_32 {
        fdk_aac_rs_fdk::x87::fl2fxconst_dbl_mul2(
            block_length,
            0.006f32 / 256.0f32,
            1.0f32 / METADATA_INT_SCALE as f32,
        )
    } else {
        fl2fxconst_dbl(
            (((0.006f32 / 256.0f32) * block_length as f32) / METADATA_INT_SCALE as f32) as f64,
        )
    }
}

/// `FDK_DRC_Generator_Initialize(HDRC_COMP drcComp, const DRC_PROFILE profileLine,
/// const DRC_PROFILE profileRF, const INT blockLength, const UINT sampleRate,
/// const CHANNEL_MODE channelMode, const CHANNEL_ORDER channelOrder, const UCHAR useWeighting)`:
/// configure DRC Compressor. Returns 0 on success, -1 or -2 on failure.
#[allow(clippy::too_many_arguments)]
pub fn fdk_drc_generator_initialize(
    drc_comp: &mut DrcComp,
    profile_line: DrcProfile,
    profile_rf: DrcProfile,
    block_length: i32,
    sample_rate: u32,
    channel_mode: ChannelMode,
    channel_order: ChannelOrder,
    use_weighting: u8,
) -> i32 {
    let mut channel_mapping = ChannelMapping::default();

    // FL2FXCONST_DBL(((0.006f / 256) * blockLength) / METADATA_INT_SCALE), computed at run time
    // in float (the INT64 divisor converted to float) and converted in double.
    drc_comp.lim_decay = lim_decay(block_length);

    // Save parameters.
    drc_comp.block_length = block_length;
    drc_comp.sample_rate = sample_rate;
    drc_comp.chan_config = channel_mode;
    drc_comp.use_weighting = use_weighting;

    if fdk_drc_generator_set_drc_profile(drc_comp, profile_line, profile_rf) != 0 {
        // expects initialized blockLength and sampleRate
        return -1;
    }

    // Set number of channels and channel offsets.
    if fdk_aac_enc_init_channel_mapping(channel_mode, channel_order, &mut channel_mapping)
        != AAC_ENC_OK
    {
        return -2;
    }

    drc_comp.channel_idx = [-1; 9];

    let el = &channel_mapping.el_info;
    let ci = &mut drc_comp.channel_idx;
    match channel_mode {
        MODE_1 => {
            // mono
            ci[C] = el[0].channel_index[0];
        }
        MODE_2 => {
            // stereo
            ci[L] = el[0].channel_index[0];
            ci[R] = el[0].channel_index[1];
        }
        MODE_1_2 => {
            // 3ch
            ci[L] = el[1].channel_index[0];
            ci[R] = el[1].channel_index[1];
            ci[C] = el[0].channel_index[0];
        }
        MODE_1_2_1 => {
            // 4ch
            ci[L] = el[1].channel_index[0];
            ci[R] = el[1].channel_index[1];
            ci[C] = el[0].channel_index[0];
            ci[S] = el[2].channel_index[0];
        }
        MODE_1_2_2 => {
            // 5ch
            ci[L] = el[1].channel_index[0];
            ci[R] = el[1].channel_index[1];
            ci[C] = el[0].channel_index[0];
            ci[LS] = el[2].channel_index[0];
            ci[RS] = el[2].channel_index[1];
        }
        MODE_1_2_2_1 => {
            // 5.1 ch
            ci[L] = el[1].channel_index[0];
            ci[R] = el[1].channel_index[1];
            ci[C] = el[0].channel_index[0];
            ci[LFE] = el[3].channel_index[0];
            ci[LS] = el[2].channel_index[0];
            ci[RS] = el[2].channel_index[1];
        }
        MODE_1_2_2_2_1 | MODE_7_1_FRONT_CENTER => {
            // 7.1 ch
            ci[L] = el[2].channel_index[0]; // l
            ci[R] = el[2].channel_index[1]; // r
            ci[C] = el[0].channel_index[0]; // c
            ci[LFE] = el[4].channel_index[0]; // lfe
            ci[LS] = el[3].channel_index[0]; // ls
            ci[RS] = el[3].channel_index[1]; // rs
            ci[LS2] = el[1].channel_index[0]; // lc
            ci[RS2] = el[1].channel_index[1]; // rc
        }
        MODE_7_1_BACK | MODE_7_1_REAR_SURROUND => {
            ci[L] = el[1].channel_index[0]; // l
            ci[R] = el[1].channel_index[1]; // r
            ci[C] = el[0].channel_index[0]; // c
            ci[LFE] = el[4].channel_index[0]; // lfe
            ci[LS] = el[3].channel_index[0]; // lrear
            ci[RS] = el[3].channel_index[1]; // rrear
            ci[LS2] = el[2].channel_index[0]; // ls
            ci[RS2] = el[2].channel_index[1]; // rs
        }
        MODE_6_1 => {
            ci[L] = el[1].channel_index[0]; // l
            ci[R] = el[1].channel_index[1]; // r
            ci[C] = el[0].channel_index[0]; // c
            ci[LFE] = el[4].channel_index[0]; // lfe
            ci[LS] = el[2].channel_index[0]; // ls
            ci[RS] = el[2].channel_index[1]; // rs
            ci[S] = el[3].channel_index[0]; // s
        }
        MODE_7_1_TOP_FRONT => {
            ci[L] = el[1].channel_index[0]; // l
            ci[R] = el[1].channel_index[1]; // r
            ci[C] = el[0].channel_index[0]; // c
            ci[LFE] = el[3].channel_index[0]; // lfe
            ci[LS] = el[2].channel_index[0]; // ls
            ci[RS] = el[2].channel_index[1]; // rs
            ci[LS2] = el[4].channel_index[0]; // lvh2
            ci[RS2] = el[4].channel_index[1]; // rvh2
        }
        _ => return -1,
    }

    drc_comp.full_channels = channel_mapping.n_channels_eff as u32;
    drc_comp.channels = channel_mapping.n_channels as u32;

    // Init states.
    drc_comp.smooth_level = [-(135 << METADATA_FRACT_BITS); 2];

    drc_comp.smooth_gain = [0; 2];
    drc_comp.hold_cnt = [0; 2];
    drc_comp.lim_gain = [0; 2];
    drc_comp.prev_peak = [0; 2];
    drc_comp.filter = [WeightingStates::default(); MAX_DRC_CHANNELS];

    0
}

/// `FDK_DRC_Generator_setDrcProfile(HDRC_COMP drcComp, const DRC_PROFILE profileLine,
/// const DRC_PROFILE profileRF)`: configure DRC Compressor Profile. Returns 0 on success, -1 for
/// an unknown profile (the profiles and the parameters of the line mode are set already if only
/// the RF profile is unknown).
///
/// `tabHoldOff[profileIdx] * 256 / drcComp->blockLength` is computed as the arm64 build does
/// (`sdiv`: 0 for a block length of 0) on every variant.
pub fn fdk_drc_generator_set_drc_profile(
    drc_comp: &mut DrcComp,
    profile_line: DrcProfile,
    profile_rf: DrcProfile,
) -> i32 {
    drc_comp.profile[0] = profile_line;
    drc_comp.profile[1] = profile_rf;

    for i in 0..2 {
        // get profile index
        let profile_idx: usize = match drc_comp.profile[i] {
            DRC_NONE | DRC_NOT_PRESENT | DRC_FILMSTANDARD => 0,
            DRC_FILMLIGHT => 1,
            DRC_MUSICSTANDARD => 2,
            DRC_MUSICLIGHT => 3,
            DRC_SPEECH => 4,
            DRC_DELAY_TEST => 5,
            _ => return -1,
        };

        // get parameters for selected profile
        drc_comp.max_boost_thr[i] = TAB_MAX_BOOST_THR[profile_idx];
        drc_comp.boost_thr[i] = TAB_BOOST_THR[profile_idx];
        drc_comp.early_cut_thr[i] = TAB_EARLY_CUT_THR[profile_idx];
        drc_comp.cut_thr[i] = TAB_CUT_THR[profile_idx];
        drc_comp.max_cut_thr[i] = TAB_MAX_CUT_THR[profile_idx];

        drc_comp.boost_fac[i] = TAB_BOOST_RATIO[profile_idx];
        drc_comp.early_cut_fac[i] = TAB_EARLY_CUT_RATIO[profile_idx];
        drc_comp.cut_fac[i] = TAB_CUT_RATIO[profile_idx];

        drc_comp.max_boost[i] = TAB_MAX_BOOST[profile_idx];
        drc_comp.max_cut[i] = TAB_MAX_CUT[profile_idx];
        // no scaling after mult needed, earlyCutFac is in FIXP_DBL
        drc_comp.max_early_cut[i] =
            -f_mult(drc_comp.cut_thr[i] - drc_comp.early_cut_thr[i], drc_comp.early_cut_fac[i]);

        let (sr, bl) = (drc_comp.sample_rate as i32, drc_comp.block_length);
        drc_comp.fast_attack[i] = tc2_coeff(TAB_FAST_ATTACK[profile_idx], sr, bl);
        drc_comp.fast_decay[i] = tc2_coeff(TAB_FAST_DECAY[profile_idx], sr, bl);
        drc_comp.slow_attack[i] = tc2_coeff(TAB_SLOW_ATTACK[profile_idx], sr, bl);
        drc_comp.slow_decay[i] = tc2_coeff(TAB_SLOW_DECAY[profile_idx], sr, bl);
        // sdiv: x / 0 == 0 (the dividend is at most 2560, so it cannot overflow)
        drc_comp.hold_off[i] =
            (TAB_HOLD_OFF[profile_idx] * 256).checked_div(bl).unwrap_or(0) as u32;

        drc_comp.attack_thr[i] = TAB_ATTACK_THR[profile_idx];
        drc_comp.decay_thr[i] = TAB_DECAY_THR[profile_idx];

        drc_comp.smooth_gain[i] = 0;
    }
    0
}

/// `FDK_DRC_Generator_Calc(HDRC_COMP drcComp, const INT_PCM *const inSamples,
/// const UINT inSamplesBufSize, const INT dialnorm, const INT drc_TargetRefLevel,
/// const INT comp_TargetRefLevel, const FIXP_DBL clev, const FIXP_DBL slev,
/// const FIXP_DBL ext_leva, const FIXP_DBL ext_levb, const FIXP_DBL lfe_lev,
/// const INT dmxGain5, const INT dmxGain2, INT *const dynrng, INT *const compr)`: calculate DRC
/// Compressor Gain. Returns 0.
///
/// `in_samples`: see the [module documentation](self).
#[allow(clippy::too_many_arguments)]
pub fn fdk_drc_generator_calc(
    drc_comp: &mut DrcComp,
    in_samples: &[IntPcm],
    in_samples_buf_size: u32,
    dialnorm: i32,
    drc_target_ref_level: i32,
    comp_target_ref_level: i32,
    clev: FixpDbl,
    slev: FixpDbl,
    ext_leva: FixpDbl,
    ext_levb: FixpDbl,
    lfe_lev: FixpDbl,
    dmx_gain5: i32,
    dmx_gain2: i32,
    p_dynrng: &mut i32,
    p_compr: &mut i32,
) -> i32 {
    let mut peak: [FixpDbl; 2] = [0; 2];

    // pSamples[i] with pSamples = inSamples + c * inSamplesBufSize (UINT arithmetic)
    let planar = |c: i32, i: i32| -> FixpSgl {
        let base = (c as u32).wrapping_mul(in_samples_buf_size) as usize;
        if base as isize + i as isize >= in_samples.len() as isize {
            crate::metadata_main::input_overrun();
        }
        sample(in_samples, base, i)
    };

    // ************************************************************************
    // compressor
    // ************************************************************************
    if drc_comp.profile[0] != DRC_NONE || drc_comp.profile[1] != DRC_NONE {
        // Calc loudness level
        let mut level_b: FixpDbl = 0;
        let mut level_e: i32 = DFRACT_BITS - 1;

        // Increase energy time resolution with shorter processing blocks. 16 is an empiric value.
        let granule_length = fix_min(16, drc_comp.block_length);

        if drc_comp.use_weighting != 0 {
            // sum of filter coefficients about 2.5 -> squared value is 6.25
            // WEIGHTING_FILTER_SHIFT is 2 -> scaling about 16, therefore reduce granuleShift by 1.
            let granule_shift = get_shift_factor(granule_length as u32) as i32 - 1;

            for c in 0..drc_comp.channels as i32 {
                if c == drc_comp.channel_idx[LFE] {
                    continue; // skip LFE
                }

                // get filter states
                let f = drc_comp.filter[c as usize];
                let (mut x1, mut x2, mut y1, mut y2) = (f.x1, f.x2, f.y1, f.y2);

                let mut i: i32 = 0;

                loop {
                    let offset = i;
                    let mut accu: FixpDbl = 0;

                    i = offset;
                    while i < fix_min(offset.wrapping_add(granule_length), drc_comp.block_length) {
                        // apply weighting filter
                        let x = fx_pcm2fx_dbl(planar(c, i)) >> WEIGHTING_FILTER_SHIFT;

                        // y = b0 * (x - x2) - a1 * y1 - a2 * y2;
                        let y = f_mult(B0, x.wrapping_sub(x2))
                            .wrapping_sub(f_mult(A1, y1))
                            .wrapping_sub(f_mult(A2, y2));

                        x2 = x1;
                        x1 = x;
                        y2 = y1;
                        y1 = y;

                        // partial energy; granuleShift - 1 is -1 for block lengths below 3: a
                        // register shift (asr, sar) by 31.
                        accu = accu
                            .wrapping_add(f_pow2_div2(y).wrapping_shr((granule_shift - 1) as u32));
                        i += 1;
                    } // i

                    // sup up partial energies
                    fixp_add(
                        accu,
                        granule_shift + 2 * WEIGHTING_FILTER_SHIFT,
                        &mut level_b,
                        &mut level_e,
                    );

                    if i >= drc_comp.block_length {
                        break;
                    }
                }

                // save filter states
                drc_comp.filter[c as usize] = WeightingStates { x1, x2, y1, y2 };
            } // c
        } else {
            // weighting
            let granule_shift = get_shift_factor(granule_length as u32) as i32;

            for c in 0..drc_comp.channels as i32 {
                if c == drc_comp.channel_idx[LFE] {
                    continue; // skip LFE
                }

                let mut i: i32 = 0;

                loop {
                    let offset = i;
                    let mut accu: FixpDbl = 0;

                    i = offset;
                    while i < fix_min(offset.wrapping_add(granule_length), drc_comp.block_length) {
                        // partial energy
                        accu = accu.wrapping_add(f_pow2_div2(planar(c, i)) >> (granule_shift - 1));
                        i += 1;
                    } // i

                    // sup up partial energies (accu >= 0 assumed by the compiler, see
                    // fixp_add_unweighted)
                    fixp_add_unweighted(accu, granule_shift, &mut level_b, &mut level_e);

                    if i >= drc_comp.block_length {
                        break;
                    }
                }
            }
        } // weighting

        // Convert to dBFS, apply dialnorm
        // level scaling

        // descaled level in ld64 representation
        let mut ld_level = calc_ld_data(level_b)
            .wrapping_add((level_e - 12) << (DFRACT_BITS - 1 - LD_DATA_SHIFT))
            .wrapping_sub(calc_ld_data(drc_comp.block_length << (DFRACT_BITS - 1 - 12)));

        // if (level < 1e-10) level = 1e-10f;
        ld_level = f_max(ld_level, fl2fxconst_dbl(-0.51905126482615036685473741085772f32 as f64));

        // level = 10 * log(level)/log(10) + 3;
        //       = 10*log(2)/log(10) * ld(level) + 3;
        //       = 10 * 0.30102999566398119521373889472449 * ld(level) + 3
        //       = 10 * (0.30102999566398119521373889472449 * ld(level) + 0.3)
        //       = 10 * (0.30102999566398119521373889472449 * ld64(level) + 0.3/64) * 64
        //
        //    additional scaling with METADATA_FRACT_BITS:
        //       = 10 * (0.30102999566398119521373889472449 * ld64(level) + 0.3/64) * 64 *
        //         2^(METADATA_FRACT_BITS)
        //       = 10 * (0.30102999566398119521373889472449 * ld64(level) + 0.3/64) *
        //         2^(METADATA_FRACT_BITS+LD_DATA_SHIFT)
        //       = 10*2^(METADATA_FRACT_BITS+LD_DATA_SHIFT) * (
        //         0.30102999566398119521373889472449 * ld64(level) + 0.3/64 )
        let mut level = f_mult(
            10 << (METADATA_FRACT_BITS + LD_DATA_SHIFT),
            f_mult(fl2fxconst_dbl(0.30102999566398119521373889472449f32 as f64), ld_level)
                .wrapping_add(fl2fxconst_dbl(0.3f32 as f64) >> LD_DATA_SHIFT),
        );

        // level -= dialnorm + 31
        // this is fixed to Dolby-ReferenceLevel as compressor profiles are defined relative to this
        level = level.wrapping_sub(
            (dialnorm << (METADATA_FRACT_BITS - 16)).wrapping_add(31 << METADATA_FRACT_BITS),
        );

        for i in 0..2 {
            if drc_comp.profile[i] == DRC_NONE {
                // no compression
                drc_comp.smooth_gain[i] = 0;
            } else {
                // calc static gain
                let gain: FixpDbl = if level <= drc_comp.max_boost_thr[i] {
                    // max boost
                    drc_comp.max_boost[i]
                } else if level < drc_comp.boost_thr[i] {
                    // boost range
                    f_mult(level.wrapping_sub(drc_comp.boost_thr[i]), drc_comp.boost_fac[i])
                } else if level <= drc_comp.early_cut_thr[i] {
                    // null band
                    0
                } else if level <= drc_comp.cut_thr[i] {
                    // early cut range
                    f_mult(level.wrapping_sub(drc_comp.early_cut_thr[i]), drc_comp.early_cut_fac[i])
                } else if level < drc_comp.max_cut_thr[i] {
                    // cut range
                    f_mult(level.wrapping_sub(drc_comp.cut_thr[i]), drc_comp.cut_fac[i])
                        .wrapping_sub(drc_comp.max_early_cut[i])
                } else {
                    // max cut
                    drc_comp.max_cut[i].wrapping_neg()
                };

                // choose time constant
                let lvl2smthlvl = level.wrapping_sub(drc_comp.smooth_level[i]);
                let alpha = if gain < drc_comp.smooth_gain[i] {
                    // attack
                    if lvl2smthlvl > drc_comp.attack_thr[i] {
                        drc_comp.fast_attack[i] // fast attack
                    } else {
                        drc_comp.slow_attack[i] // slow attack
                    }
                } else {
                    // release
                    if lvl2smthlvl < drc_comp.decay_thr[i].wrapping_neg() {
                        drc_comp.fast_decay[i] // fast release
                    } else {
                        drc_comp.slow_decay[i] // slow release
                    }
                };

                // smooth gain & level
                if gain < drc_comp.smooth_gain[i] || drc_comp.hold_cnt[i] == 0 {
                    // hold gain unless we have an attack or hold period is over

                    // drcComp->smoothLevel[i] = (1-alpha) * drcComp->smoothLevel[i] + alpha * level;
                    let mut accu = f_mult(MAXVAL_DBL.wrapping_sub(alpha), drc_comp.smooth_level[i]);
                    accu = accu.wrapping_add(f_mult(alpha, level));
                    drc_comp.smooth_level[i] = accu;

                    // drcComp->smoothGain[i]  = (1-alpha) * drcComp->smoothGain[i] + alpha * gain;
                    accu = f_mult(MAXVAL_DBL.wrapping_sub(alpha), drc_comp.smooth_gain[i]);
                    accu = accu.wrapping_add(f_mult(alpha, gain));
                    drc_comp.smooth_gain[i] = accu;
                }

                // hold counter
                if drc_comp.hold_cnt[i] != 0 {
                    drc_comp.hold_cnt[i] -= 1;
                }
                if gain < drc_comp.smooth_gain[i] {
                    drc_comp.hold_cnt[i] = drc_comp.hold_off[i];
                }
            } // profile != DRC_NONE
        } // for i=1..2
    } else {
        // no compression
        drc_comp.smooth_gain[0] = 0;
        drc_comp.smooth_gain[1] = 0;
    }

    // ************************************************************************
    // limiter
    // ************************************************************************

    find_peak_levels(
        drc_comp,
        in_samples,
        clev,
        slev,
        ext_leva,
        ext_levb,
        lfe_lev,
        dmx_gain5 << (METADATA_FRACT_BITS - 16),
        dmx_gain2 << (METADATA_FRACT_BITS - 16),
        &mut peak,
    );

    for i in 0..2 {
        let tmp = drc_comp.prev_peak[i];
        drc_comp.prev_peak[i] = peak[i];
        peak[i] = fix_max(peak[i], tmp);

        // Convert to dBFS, apply dialnorm
        // descaled peak in ld64 representation
        let mut ld_peak =
            calc_ld_data(peak[i]).wrapping_add(DOWNMIX_SHIFT << (DFRACT_BITS - 1 - LD_DATA_SHIFT));

        // if (peak < 1e-6) level = 1e-6f;
        ld_peak = f_max(ld_peak, fl2fxconst_dbl(-0.31143075889569022011284244651463f32 as f64));

        // peak[i] = 20 * log(peak[i])/log(10) + 0.2f +
        //           (drcComp->smoothGain[i]*2^METADATA_FRACT_BITS)
        // peak[i] = 10*2^(METADATA_FRACT_BITS+LD_DATA_SHIFT) *
        //           2*0.30102999566398119521373889472449 * ld64(peak[i])
        //           + 0.2f*2^(-METADATA_FRACT_BITS) + drcComp->smoothGain[i]
        peak[i] = f_mult(
            10 << (METADATA_FRACT_BITS + LD_DATA_SHIFT),
            f_mult(fl2fx_dbl(2.0f32 * 0.30102999566398119521373889472449f32), ld_peak),
        );
        peak[i] = peak[i].wrapping_add(fl2fx_dbl(0.5f32) >> METADATA_INT_BITS); // add a little bit headroom
        peak[i] = peak[i].wrapping_add(drc_comp.smooth_gain[i]);
    }

    // peak -= dialnorm + 31; this is Dolby style only
    // peak[0] -= dialnorm - drc_TargetRefLevel
    peak[0] = peak[0]
        .wrapping_sub(dialnorm.wrapping_sub(drc_target_ref_level) << (METADATA_FRACT_BITS - 16));

    // peak += 11; this is Dolby style only; RF mode output is 11dB higher
    // peak += comp_TargetRefLevel - drc_TargetRefLevel;
    // peak[1] -= dialnorm - comp_TargetRefLevel
    peak[1] = peak[1]
        .wrapping_sub(dialnorm.wrapping_sub(comp_target_ref_level) << (METADATA_FRACT_BITS - 16));

    // limiter gain
    drc_comp.lim_gain[0] = drc_comp.lim_gain[0].wrapping_add(drc_comp.lim_decay); // linear limiter release
    drc_comp.lim_gain[0] = fix_min(drc_comp.lim_gain[0], peak[0].wrapping_neg());

    // linear limiter release
    drc_comp.lim_gain[1] = drc_comp.lim_gain[1].wrapping_add(drc_comp.lim_decay.wrapping_mul(2));
    drc_comp.lim_gain[1] = fix_min(drc_comp.lim_gain[1], peak[1].wrapping_neg());

    // ***********************************************************************

    // apply limiting, return DRC gains
    let mut tmp = drc_comp.smooth_gain[0];
    if drc_comp.lim_gain[0] < 0 {
        tmp = tmp.wrapping_add(drc_comp.lim_gain[0]);
    }
    *p_dynrng = scale_value(tmp, -(METADATA_FRACT_BITS - 16));

    tmp = drc_comp.smooth_gain[1];
    if drc_comp.lim_gain[1] < 0 {
        tmp = tmp.wrapping_add(drc_comp.lim_gain[1]);
    }
    *p_compr = scale_value(tmp, -(METADATA_FRACT_BITS - 16));

    0
}

/// `FDK_DRC_Generator_getDrcProfile(const HDRC_COMP drcComp)`: DRC profile for line mode.
pub fn fdk_drc_generator_get_drc_profile(drc_comp: &DrcComp) -> DrcProfile {
    drc_comp.profile[0]
}

/// `FDK_DRC_Generator_getCompProfile(const HDRC_COMP drcComp)`: DRC profile for RF mode.
pub fn fdk_drc_generator_get_comp_profile(drc_comp: &DrcComp) -> DrcProfile {
    drc_comp.profile[1]
}
