// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/usacdec_const.h.

//! Port of `usacdec_const.h`: USAC related constants.
//!
//! All constants are `i32` (the C `int` of the macros); array dimensions use `as usize`.
//! `LPD_MAX_CORE_SR` keeps its default (the reference build does not define it).

/* scale factors */
/// `SF_CODE`: exponent of code[], fixed codebook vector
pub const SF_CODE: i32 = 6;
/// `SF_GAIN_C`: exponent of gain code and smoothed gain code
pub const SF_GAIN_C: i32 = 16;
/// `SF_EXC`: exponent of exc[] and exc2[], excitation buffer
pub const SF_EXC: i32 = 16;
/// `SF_GAIN_P`: exponent of gain_pit
pub const SF_GAIN_P: i32 = 1;
/// `SF_PFAC`: exponent of period/voicing factor
pub const SF_PFAC: i32 = 0;
/// `SF_SYNTH`: exponent of synthesis buffer
pub const SF_SYNTH: i32 = SF_EXC;
/// `SF_A_COEFFS`: exponent of LP domain synthesis filter coefficient
pub const SF_A_COEFFS: i32 = 3;
/// `SF_STAB`: exponent of stability factor
pub const SF_STAB: i32 = 1;

/* definitions which are independent of coreCoderFrameLength */
/// `M_LP_FILTER_ORDER`: LP filter order
pub const M_LP_FILTER_ORDER: i32 = 16;
/// `LP_FILTER_SCALE`: LP filter scale
pub const LP_FILTER_SCALE: i32 = 4;

/// `PIT_MIN_12k8`: minimum pitch lag with resolution 1/4
pub const PIT_MIN_12k8: i32 = 34;
/// `PIT_MAX_12k8`: maximum pitch lag for fs=12.8kHz
pub const PIT_MAX_12k8: i32 = 231;
/// `FSCALE_DENOM`: frequency scale denominator
pub const FSCALE_DENOM: i32 = 12800;
/// `FAC_FSCALE_MIN`: minimum allowed frequency scale for acelp decoder
pub const FAC_FSCALE_MIN: i32 = 6000;

/// `LPD_MAX_CORE_SR`: default value from ref soft
pub const LPD_MAX_CORE_SR: i32 = 24000;
/// `FAC_FSCALE_MAX`: maximum allowed frequency scale for acelp decoder
pub const FAC_FSCALE_MAX: i32 = LPD_MAX_CORE_SR;

/// `PIT_MAX_TMP`: maximum pitch lag (= 411 for fs_max = 24000)
pub const PIT_MAX_TMP: i32 = PIT_MAX_12k8
    + (6 * ((((FAC_FSCALE_MAX * PIT_MIN_12k8) + (FSCALE_DENOM / 2)) / FSCALE_DENOM)
        - PIT_MIN_12k8));
/// `PIT_MAX_MAX`: `PIT_MAX_TMP`, at least 256 (because of tcx time domain concealment)
pub const PIT_MAX_MAX: i32 = if PIT_MAX_TMP < 256 { 256 } else { PIT_MAX_TMP };

/// `NB_DIV`: number of division (20ms) per 80ms frame
pub const NB_DIV: i32 = 4;
/// `L_SUBFR`: subframe size (5ms)
pub const L_SUBFR: i32 = 64;
/// `BPF_SFD`: bass postfilter delay (subframe)
pub const BPF_SFD: i32 = 1;
/// `BPF_DELAY`: bass postfilter delay (samples)
pub const BPF_DELAY: i32 = BPF_SFD * L_SUBFR;

/// `L_FILT`: delay of up-sampling filter (bass post-filter)
pub const L_FILT: i32 = 12;
/// `L_EXTRA`: for bass post-filter
pub const L_EXTRA: i32 = 96;
/// `L_INTERPOL`: length of filter for interpolation (acelp decoder)
pub const L_INTERPOL: i32 = 16 + 1;

/* definitions for coreCoderFrameLength = 1024 */
/// `L_FRAME_PLUS_1024`: length of one 80ms superframe
pub const L_FRAME_PLUS_1024: i32 = 1024;
/// `L_DIV_1024`: length of one acelp or tcx20 frame
pub const L_DIV_1024: i32 = L_FRAME_PLUS_1024 / NB_DIV;
/// `NB_SUBFR_1024`: number of 5ms subframe per division
pub const NB_SUBFR_1024: i32 = L_DIV_1024 / L_SUBFR;
/// `NB_SUBFR_SUPERFR_1024`: number of 5ms subframe per 80ms frame
pub const NB_SUBFR_SUPERFR_1024: i32 = L_FRAME_PLUS_1024 / L_SUBFR;
/// `AAC_SFD_1024`: AAC delay (subframe)
pub const AAC_SFD_1024: i32 = NB_SUBFR_SUPERFR_1024 / 2;
/// `AAC_DELAY_1024`: AAC delay (samples)
pub const AAC_DELAY_1024: i32 = AAC_SFD_1024 * L_SUBFR;
/// `SYN_SFD_1024`: synthesis delay (subframe)
pub const SYN_SFD_1024: i32 = AAC_SFD_1024 - BPF_SFD;
/// `SYN_DELAY_1024`: synthesis delay (samples)
pub const SYN_DELAY_1024: i32 = SYN_SFD_1024 * L_SUBFR;
/// `LFAC_1024`: FAC frame length
pub const LFAC_1024: i32 = L_DIV_1024 / 2;
/// `LFAC_SHORT_1024`: for transitions EIGHT_SHORT FD->LPD and vv.
pub const LFAC_SHORT_1024: i32 = L_DIV_1024 / 4;
/// `FDNS_NPTS_1024`: FD noise shaping resolution (64=100Hz/point)
pub const FDNS_NPTS_1024: i32 = 64;

/* definitions for coreCoderFrameLength = 768 */
/// `L_FRAME_PLUS_768`
pub const L_FRAME_PLUS_768: i32 = 768;
/// `L_DIV_768`: length of one acelp or tcx20 frame
pub const L_DIV_768: i32 = L_FRAME_PLUS_768 / NB_DIV;
/// `NB_SUBFR_768`: number of 5ms subframe per division
pub const NB_SUBFR_768: i32 = L_DIV_768 / L_SUBFR;
/// `NB_SUBFR_SUPERFR_768`: number of 5ms subframe per 80ms frame
pub const NB_SUBFR_SUPERFR_768: i32 = L_FRAME_PLUS_768 / L_SUBFR;
/// `AAC_SFD_768`: AAC delay (subframe)
pub const AAC_SFD_768: i32 = NB_SUBFR_SUPERFR_768 / 2;
/// `AAC_DELAY_768`: AAC delay (samples)
pub const AAC_DELAY_768: i32 = AAC_SFD_768 * L_SUBFR;
/// `SYN_SFD_768`: synthesis delay (subframe)
pub const SYN_SFD_768: i32 = AAC_SFD_768 - BPF_SFD;
/// `SYN_DELAY_768`: synthesis delay (samples)
pub const SYN_DELAY_768: i32 = SYN_SFD_768 * L_SUBFR;
/// `LFAC_768`: FAC frame length
pub const LFAC_768: i32 = L_DIV_768 / 2;
/// `LFAC_SHORT_768`: for transitions EIGHT_SHORT FD->LPD and vv.
pub const LFAC_SHORT_768: i32 = L_DIV_768 / 4;

/* maximum (used for memory allocation) */
/// `L_FRAME_PLUS`
pub const L_FRAME_PLUS: i32 = L_FRAME_PLUS_1024;
/// `L_DIV`
pub const L_DIV: i32 = L_DIV_1024;
/// `NB_SUBFR`
pub const NB_SUBFR: i32 = NB_SUBFR_1024;
/// `NB_SUBFR_SUPERFR`
pub const NB_SUBFR_SUPERFR: i32 = NB_SUBFR_SUPERFR_1024;
/// `AAC_SFD`
pub const AAC_SFD: i32 = AAC_SFD_1024;
/// `AAC_DELAY`
pub const AAC_DELAY: i32 = AAC_DELAY_1024;
/// `SYN_SFD`
pub const SYN_SFD: i32 = SYN_SFD_1024;
/// `SYN_DELAY`
pub const SYN_DELAY: i32 = SYN_DELAY_1024;
/// `LFAC`
pub const LFAC: i32 = LFAC_1024;
/// `LFAC_SHORT`
pub const LFAC_SHORT: i32 = LFAC_SHORT_1024;
/// `FDNS_NPTS`
pub const FDNS_NPTS: i32 = FDNS_NPTS_1024;
