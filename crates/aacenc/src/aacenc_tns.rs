// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/aacenc_tns.h, libAACenc/src/aacenc_tns.cpp.

//! Port of `aacenc_tns.h` and `aacenc_tns.cpp`: temporal noise shaping.
//!
//! The functions declared in `tns_func.h` are defined here (as in C) and re-exported by
//! [`crate::tns_func`]. The file-static tables of `aacenc_tns.cpp` are generated into [`tables`]
//! by `aacenc_tns/gen_tables.py`.
//!
//! The function bodies are the same in the `aarch64` and `x86` preprocessed sources; the
//! variants differ only in the primitives (`fMult`, `invSqrtNorm2`, `CountLeadingBits`, ...),
//! which `fdk_aac_rs_fdk` selects.

pub mod tables;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;

use crate::aac_enc_rom::{
    FDK_AAC_ENC_TNS_COEFF_3_BORDERS, FDK_AAC_ENC_TNS_COEFF_4_BORDERS, FDK_AAC_ENC_TNS_ENC_COEFF_3,
    FDK_AAC_ENC_TNS_ENC_COEFF_4,
};
use crate::aacenc::{AacEncoderError, AAC_ENC_OK};
use crate::psy_configuration::PsyConfiguration;
use crate::psy_const::{FALSE, MAX_SFB_LONG, MAX_SFB_SHORT, SHORT_WINDOW, TRANS_FAC, TRUE};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::lpc::{clpc_analysis, clpc_auto_to_parcor, clpc_parcor_to_lpc_sgl, FixpLpc};
use tables::*;

/// `TNS_ENABLE_MASK`
pub const TNS_ENABLE_MASK: i32 = 0xf;
/// `TNS_MAX_ORDER`: TNS max filter order for Low Complexity MPEG4 profile
pub const TNS_MAX_ORDER: usize = 12;
/// `MAX_NUM_OF_FILTERS`
pub const MAX_NUM_OF_FILTERS: usize = 2;
/// `HIFILT`: index of higher filter
pub const HIFILT: usize = 0;
/// `LOFILT`: index of lower filter
pub const LOFILT: usize = 1;

/// `TNS_PARAMETER_TABULATED`: stuff that is tabulated dependent on bitrate etc.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TnsParameterTabulated {
    pub filter_enabled: [i32; MAX_NUM_OF_FILTERS],
    /// min. prediction gain for using tns TABUL
    pub thresh_on: [i32; MAX_NUM_OF_FILTERS],
    /// lowest freq for lpc TABUL
    pub filter_start_freq: [i32; MAX_NUM_OF_FILTERS],
    /// Limit for TNS order TABUL
    pub tns_limit_order: [i32; MAX_NUM_OF_FILTERS],
    /// Filtering direction, 0=up, 1=down TABUL
    pub tns_filter_direction: [i32; MAX_NUM_OF_FILTERS],
    pub acf_split: [i32; MAX_NUM_OF_FILTERS],
    /// TNS max. time resolution TABUL
    pub tns_time_resolution: [FixpDbl; MAX_NUM_OF_FILTERS],
    pub seperate_filters_allowed: i32,
}

/// `TNS_CONFIG`: assigned at init time.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TnsConfig {
    pub conf_tab: TnsParameterTabulated,
    pub is_low_delay: i32,
    pub tns_active: i32,
    /// max. order of tns filter
    pub max_order: i32,
    pub coef_res: i32,
    pub acf_window: [[FixpDbl; TNS_MAX_ORDER + 3 + 1]; MAX_NUM_OF_FILTERS],
    pub lpc_start_band: [i32; MAX_NUM_OF_FILTERS],
    pub lpc_start_line: [i32; MAX_NUM_OF_FILTERS],
    pub lpc_stop_band: i32,
    pub lpc_stop_line: i32,
}

/// `TNS_SUBBLOCK_INFO`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TnsSubblockInfo {
    pub tns_active: [i32; MAX_NUM_OF_FILTERS],
    pub prediction_gain: [i32; MAX_NUM_OF_FILTERS],
}

/// Number of `INT`-sized words of a `TNS_SUBBLOCK_INFO`.
pub const TNS_SUBBLOCK_INFO_WORDS: usize = 2 * MAX_NUM_OF_FILTERS;

/// Size in words of `TNS_DATA_LONG` (`subBlockInfo`, `ratioMultTable[MAX_SFB_LONG]`).
pub const TNS_DATA_LONG_WORDS: usize = TNS_SUBBLOCK_INFO_WORDS + MAX_SFB_LONG;
/// Size in words of `TNS_DATA_SHORT` (`subBlockInfo[TRANS_FAC]`,
/// `ratioMultTable[TRANS_FAC][MAX_SFB_SHORT]`).
pub const TNS_DATA_SHORT_WORDS: usize =
    TRANS_FAC as usize * TNS_SUBBLOCK_INFO_WORDS + TRANS_FAC as usize * MAX_SFB_SHORT;
/// Size in words of the union `TNS_DATA_RAW`.
pub const TNS_DATA_RAW_WORDS: usize = if TNS_DATA_SHORT_WORDS > TNS_DATA_LONG_WORDS {
    TNS_DATA_SHORT_WORDS
} else {
    TNS_DATA_LONG_WORDS
};

/// `TNS_DATA_RAW`: the union of `TNS_DATA_LONG` (`.Long`) and `TNS_DATA_SHORT` (`.Short`).
///
/// The C code reads the union through the other member than it wrote (e.g. `FDKaacEnc_psyMain`
/// passes `dataRaw.Long.subBlockInfo` of a short block to the PNS detection), so the union is
/// kept as one array of `INT`/`FIXP_DBL` words with the memory layout of C:
///
/// | member | word offset |
/// |---|---|
/// | `Long.subBlockInfo` | 0 |
/// | `Long.ratioMultTable[sfb]` | 4 + sfb |
/// | `Short.subBlockInfo[w]` | 4 w |
/// | `Short.ratioMultTable[w][sfb]` | 32 + 15 w + sfb |
///
/// Within a `TNS_SUBBLOCK_INFO` at offset `o`: `tnsActive[f]` is word `o + f`,
/// `predictionGain[f]` is word `o + 2 + f`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TnsDataRaw(pub [i32; TNS_DATA_RAW_WORDS]);

impl Default for TnsDataRaw {
    fn default() -> Self {
        TnsDataRaw([0; TNS_DATA_RAW_WORDS])
    }
}

impl TnsDataRaw {
    /// Word offset of `Long.subBlockInfo`.
    pub const LONG_SUB_BLOCK_INFO: usize = 0;
    /// Word offset of `Long.ratioMultTable`.
    pub const LONG_RATIO_MULT_TABLE: usize = TNS_SUBBLOCK_INFO_WORDS;
    /// Word offset of `Short.ratioMultTable`.
    pub const SHORT_RATIO_MULT_TABLE: usize = TRANS_FAC as usize * TNS_SUBBLOCK_INFO_WORDS;

    /// Word offset of `Short.subBlockInfo[w]`.
    pub const fn short_sub_block_info(w: usize) -> usize {
        w * TNS_SUBBLOCK_INFO_WORDS
    }

    /// Word offset of `Short.ratioMultTable[w]`.
    pub const fn short_ratio_mult_table(w: usize) -> usize {
        Self::SHORT_RATIO_MULT_TABLE + w * MAX_SFB_SHORT
    }

    /// The `TNS_SUBBLOCK_INFO` at word offset `o`.
    pub fn sub_block_info(&self, o: usize) -> TnsSubblockInfo {
        let s = &self.0[o..o + TNS_SUBBLOCK_INFO_WORDS];
        TnsSubblockInfo { tns_active: [s[0], s[1]], prediction_gain: [s[2], s[3]] }
    }

    /// Stores a `TNS_SUBBLOCK_INFO` at word offset `o`.
    pub fn set_sub_block_info(&mut self, o: usize, v: &TnsSubblockInfo) {
        let s = &mut self.0[o..o + TNS_SUBBLOCK_INFO_WORDS];
        s[0] = v.tns_active[0];
        s[1] = v.tns_active[1];
        s[2] = v.prediction_gain[0];
        s[3] = v.prediction_gain[1];
    }

    /// `tnsActive[f]` of the `TNS_SUBBLOCK_INFO` at word offset `o`.
    pub fn tns_active(&self, o: usize, f: usize) -> i32 {
        self.0[o + f]
    }

    /// `&tnsActive[f]` of the `TNS_SUBBLOCK_INFO` at word offset `o`.
    pub fn tns_active_mut(&mut self, o: usize, f: usize) -> &mut i32 {
        &mut self.0[o + f]
    }

    /// `predictionGain[f]` of the `TNS_SUBBLOCK_INFO` at word offset `o`.
    pub fn prediction_gain(&self, o: usize, f: usize) -> i32 {
        self.0[o + MAX_NUM_OF_FILTERS + f]
    }

    /// `&predictionGain[f]` of the `TNS_SUBBLOCK_INFO` at word offset `o`.
    pub fn prediction_gain_mut(&mut self, o: usize, f: usize) -> &mut i32 {
        &mut self.0[o + MAX_NUM_OF_FILTERS + f]
    }
}

/// `TNS_DATA`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TnsData {
    pub num_of_subblocks: i32,
    pub data_raw: TnsDataRaw,
    pub tns_max_scale_spec: i32,
    pub filters_merged: i32,
}

/// `TNS_INFO`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TnsInfo {
    pub num_of_filters: [i32; TRANS_FAC as usize],
    pub coef_res: [i32; TRANS_FAC as usize],
    pub length: [[i32; MAX_NUM_OF_FILTERS]; TRANS_FAC as usize],
    pub order: [[i32; MAX_NUM_OF_FILTERS]; TRANS_FAC as usize],
    pub direction: [[i32; MAX_NUM_OF_FILTERS]; TRANS_FAC as usize],
    pub coef_compress: [[i32; MAX_NUM_OF_FILTERS]; TRANS_FAC as usize],
    pub coef: [[[i32; TNS_MAX_ORDER]; MAX_NUM_OF_FILTERS]; TRANS_FAC as usize],
}

/// `FILTER_DIRECTION`: 0 = up, 1 = down
const FILTER_DIRECTION: i32 = 0;

/// `TNS_INFO_TAB` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TnsInfoTab {
    /// noneSbr=0, useSbr=1
    pub bit_rate_from: [i32; 2],
    /// noneSbr=0, useSbr=1
    pub bit_rate_to: [i32; 2],
    /// mono=0, stereo=1
    pub param_tab: [TnsParameterTabulated; 2],
}

/// `TNS_TIMERES_SCALE`
const TNS_TIMERES_SCALE: i32 = 1;

/// `TNS_MAX_TAB_ENTRY` (layout identical to the C struct, with its padding as `_pad`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TnsMaxTabEntry {
    pub sampling_rate: i32,
    /// long=0; short=1
    pub max_bands: [i8; 2],
    pub _pad: [i8; 2],
}

/// `a / b` as the arm64 reference computes it (`sdiv`): 0 for `b == 0`, `INT_MIN` for
/// `INT_MIN / -1`, on every variant.
#[inline]
fn div_i32(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

/// `FDKaacEnc_GetTnsParam(bitRate, channels, sbrLd)`
pub fn fdk_aac_enc_get_tns_param(
    bit_rate: i32,
    channels: i32,
    sbr_ld: i32,
) -> Option<&'static TnsParameterTabulated> {
    let mut tns_config_tab = None;
    let s = if sbr_ld != 0 { 1 } else { 0 };

    for e in TNS_INFO_TAB.iter() {
        if (bit_rate >= e.bit_rate_from[s]) && bit_rate <= e.bit_rate_to[s] {
            tns_config_tab = Some(&e.param_tab[if channels == 1 { 0 } else { 1 }]);
        }
    }

    tns_config_tab
}

/// `getTnsMaxBands(sampleRate, granuleLength, isShortBlock)`
pub fn get_tns_max_bands(sample_rate: i32, granule_length: i32, is_short_block: i32) -> i32 {
    let mut num_bands = -1;
    let p_max_bands_tab: Option<&[TnsMaxTabEntry]> = match granule_length {
        960 | 1024 => Some(&TNS_MAX_BANDS_TAB_1024),
        120 => Some(&TNS_MAX_BANDS_TAB_120),
        128 => Some(&TNS_MAX_BANDS_TAB_128),
        240 => Some(&TNS_MAX_BANDS_TAB_240),
        256 => Some(&TNS_MAX_BANDS_TAB_256),
        480 => Some(&TNS_MAX_BANDS_TAB_480),
        512 => Some(&TNS_MAX_BANDS_TAB_512),
        _ => None,
    };

    if let Some(tab) = p_max_bands_tab {
        for e in tab {
            num_bands = e.max_bands[if is_short_block == 0 { 0 } else { 1 }] as i32;
            if sample_rate >= e.sampling_rate {
                break;
            }
        }
    }

    num_bands
}

/// `FDKaacEnc_FreqToBandWidthRounding(freq, fs, numOfBands, bandStartOffset)`: returns index of
/// nearest band border.
///
/// `band_start_offset` holds `numOfBands + 1` band borders (increasing).
pub fn fdk_aac_enc_freq_to_band_width_rounding(
    freq: i32,
    fs: i32,
    num_of_bands: i32,
    band_start_offset: &[i32],
) -> i32 {
    let n = num_of_bands as usize;

    // assert(freq >= 0);
    let line_number = div_i32(
        div_i32(freq.wrapping_mul(band_start_offset[n]).wrapping_mul(4), fs).wrapping_add(1),
        2,
    );

    // freq > fs/2
    if line_number >= band_start_offset[n] {
        return num_of_bands;
    }

    // find band the line number lies in
    let mut band = 0;
    while band < n {
        if band_start_offset[band + 1] > line_number {
            break;
        }
        band += 1;
    }

    // round to nearest band border
    if line_number - band_start_offset[band] > band_start_offset[band + 1] - line_number {
        band += 1;
    }

    band as i32
}

/// `FDKaacEnc_InitTnsConfiguration(bitRate, sampleRate, channels, blockType, granuleLength,
/// isLowDelay, ldSbrPresent, tC, pC, active, useTnsPeak)`: fill `TNS_CONFIG` structure with
/// sensible content.
///
/// The encoder passes `tC = &pC->tnsConf`; this function does not read `pC->tnsConf`, so a
/// caller can pass a copy of it and store it back.
pub fn fdk_aac_enc_init_tns_configuration(
    bit_rate: i32,
    sample_rate: i32,
    channels: i32,
    block_type: i32,
    granule_length: i32,
    is_low_delay: i32,
    ld_sbr_present: i32,
    t_c: &mut TnsConfig,
    p_c: &PsyConfiguration,
    active: i32,
    _use_tns_peak: i32,
) -> AacEncoderError {
    // float acfTimeRes   = (blockType == SHORT_WINDOW) ? 0.125f : 0.046875f;

    if channels <= 0 {
        return 1;
    }

    t_c.is_low_delay = is_low_delay;

    // initialize TNS filter flag, order, and coefficient resolution (in bits per coeff)
    t_c.tns_active = if active != 0 { TRUE } else { FALSE };
    t_c.max_order = if block_type == SHORT_WINDOW { 5 } else { 12 }; // maximum: 7, 20
    if bit_rate < 16000 {
        t_c.max_order -= 2;
    }
    t_c.coef_res = if block_type == SHORT_WINDOW { 3 } else { 4 };

    // LPC stop line: highest MDCT line to be coded, but do not go beyond TNS_MAX_BANDS!
    t_c.lpc_stop_band = get_tns_max_bands(
        sample_rate,
        granule_length,
        if block_type == SHORT_WINDOW { 1 } else { 0 },
    );

    if t_c.lpc_stop_band < 0 {
        return 1;
    }

    t_c.lpc_stop_band = f_min(t_c.lpc_stop_band, p_c.sfb_active);
    t_c.lpc_stop_line = p_c.sfb_offset[t_c.lpc_stop_band as usize];

    match granule_length {
        960 | 1024 => {
            // TNS start line: skip lower MDCT lines to prevent artifacts due to filter mismatch
            if block_type == SHORT_WINDOW {
                t_c.lpc_start_band[LOFILT] = 0;
            } else {
                t_c.lpc_start_band[LOFILT] = if sample_rate < 9391 {
                    2
                } else if sample_rate < 18783 {
                    4
                } else {
                    8
                };
            }
            t_c.lpc_start_line[LOFILT] = p_c.sfb_offset[t_c.lpc_start_band[LOFILT] as usize];

            // The offsets are not negative and sfbOffset[0] is 0, so i stays >= 0.
            let mut i = t_c.lpc_stop_band;
            while p_c.sfb_offset[i as usize]
                > (t_c.lpc_start_line[LOFILT]
                    + (t_c.lpc_stop_line - t_c.lpc_start_line[LOFILT]) / 4)
            {
                i -= 1;
            }
            t_c.lpc_start_band[HIFILT] = i;
            t_c.lpc_start_line[HIFILT] = p_c.sfb_offset[i as usize];

            t_c.conf_tab.thresh_on[HIFILT] = 1437;
            t_c.conf_tab.thresh_on[LOFILT] = 1500;

            t_c.conf_tab.tns_limit_order[HIFILT] = t_c.max_order;
            t_c.conf_tab.tns_limit_order[LOFILT] = f_max(0, t_c.max_order - 7);

            t_c.conf_tab.tns_filter_direction[HIFILT] = FILTER_DIRECTION;
            t_c.conf_tab.tns_filter_direction[LOFILT] = FILTER_DIRECTION;

            // signal Merged4to2QuartersAutoCorrelation in FDKaacEnc_MergedAutoCorrelation
            t_c.conf_tab.acf_split[HIFILT] = -1;
            t_c.conf_tab.acf_split[LOFILT] = -1;

            t_c.conf_tab.filter_enabled[HIFILT] = 1;
            t_c.conf_tab.filter_enabled[LOFILT] = 1;
            t_c.conf_tab.seperate_filters_allowed = 1;

            // compute autocorrelation window based on maximum filter order for given block type
            // for (i = 0; i <= tC->maxOrder + 3; i++) {
            //   float acfWinTemp = acfTimeRes * i;
            //   acfWindow[i] = FL2FXCONST_DBL(1.0f - acfWinTemp * acfWinTemp);
            // }
            // The short window has 8 of the 16 values; the others are left unchanged.
            let win: &[FixpDbl] =
                if block_type == SHORT_WINDOW { &ACF_WINDOW_SHORT } else { &ACF_WINDOW_LONG };
            let n = win.len().min(TNS_MAX_ORDER + 3 + 1);
            t_c.acf_window[HIFILT][..n].copy_from_slice(&win[..n]);
            t_c.acf_window[LOFILT][..n].copy_from_slice(&win[..n]);
        }
        480 | 512 => {
            if let Some(p_cfg) = fdk_aac_enc_get_tns_param(bit_rate, channels, ld_sbr_present) {
                t_c.conf_tab = *p_cfg;

                t_c.lpc_start_band[HIFILT] = fdk_aac_enc_freq_to_band_width_rounding(
                    p_cfg.filter_start_freq[HIFILT],
                    sample_rate,
                    p_c.sfb_cnt,
                    &p_c.sfb_offset,
                );
                t_c.lpc_start_line[HIFILT] = p_c.sfb_offset[t_c.lpc_start_band[HIFILT] as usize];
                t_c.lpc_start_band[LOFILT] = fdk_aac_enc_freq_to_band_width_rounding(
                    p_cfg.filter_start_freq[LOFILT],
                    sample_rate,
                    p_c.sfb_cnt,
                    &p_c.sfb_offset,
                );
                t_c.lpc_start_line[LOFILT] = p_c.sfb_offset[t_c.lpc_start_band[LOFILT] as usize];

                fdk_aac_enc_calc_gauss_window(
                    &mut t_c.acf_window[HIFILT],
                    t_c.max_order + 1,
                    sample_rate,
                    granule_length,
                    p_cfg.tns_time_resolution[HIFILT],
                    TNS_TIMERES_SCALE,
                );
                fdk_aac_enc_calc_gauss_window(
                    &mut t_c.acf_window[LOFILT],
                    t_c.max_order + 1,
                    sample_rate,
                    granule_length,
                    p_cfg.tns_time_resolution[LOFILT],
                    TNS_TIMERES_SCALE,
                );
            } else {
                t_c.tns_active = FALSE; // no configuration available, disable tns tool
            }
        }
        _ => {
            t_c.tns_active = FALSE; // no configuration available, disable tns tool
        }
    }

    AAC_ENC_OK
}

/// `FDKaacEnc_ScaleUpSpectrum(dest, src, startLine, stopLine)`: scales up spectrum lines in a
/// given frequency section. Returns the scale factor.
#[inline]
pub fn fdk_aac_enc_scale_up_spectrum(
    dest: &mut [FixpDbl],
    src: &[FixpDbl],
    start_line: i32,
    stop_line: i32,
) -> i32 {
    let mut max_val = fl2fxconst_dbl(0.0);

    // Get highest value in given spectrum
    for i in start_line.max(0)..stop_line {
        max_val = fix_max(max_val, fixp_abs(src[i as usize]));
    }
    let scale = count_leading_bits(max_val);

    // Scale spectrum according to highest value
    for i in start_line.max(0)..stop_line {
        dest[i as usize] = src[i as usize].wrapping_shl(scale as u32);
    }

    scale
}

/// `FDKaacEnc_CalcAutoCorrValue(spectrum, startLine, stopLine, lag, scale)`: calculate
/// autocorellation value for one lag.
///
/// On x86 with AVX2 the sum runs in the vector kernel of `x86.rs` (a wrapping sum of the same
/// per-line values), elsewhere it is [`fdk_aac_enc_calc_auto_corr_value_straight`] (which the
/// compiler vectorizes on AArch64).
#[inline]
pub fn fdk_aac_enc_calc_auto_corr_value(
    spectrum: &[FixpDbl],
    start_line: i32,
    stop_line: i32,
    lag: i32,
    scale: i32,
) -> FixpDbl {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if crate::simd::avx2() && (0..32).contains(&scale) && lag >= 0 {
        let s = start_line.max(0);
        let e = stop_line - lag;
        if s >= e {
            return fl2fxconst_dbl(0.0);
        }
        let (s, e, l) = (s as usize, e as usize, lag as usize);
        let a = &spectrum[s..e];
        let b = &spectrum[s + l..e + l];
        // SAFETY: the CPU supports AVX2.
        return unsafe { x86::auto_corr_sum(a, b, scale, lag == 0) };
    }
    fdk_aac_enc_calc_auto_corr_value_straight(spectrum, start_line, stop_line, lag, scale)
}

/// `FDKaacEnc_CalcAutoCorrValue()`: straight port, kept as the reference of
/// [`fdk_aac_enc_calc_auto_corr_value`].
#[doc(hidden)]
#[inline]
pub fn fdk_aac_enc_calc_auto_corr_value_straight(
    spectrum: &[FixpDbl],
    start_line: i32,
    stop_line: i32,
    lag: i32,
    scale: i32,
) -> FixpDbl {
    let mut result = fl2fxconst_dbl(0.0);

    // This versions allows to save memory accesses, when computing pow2
    // It is of interest for ARM, XTENSA without parallel memory access
    if lag == 0 {
        for i in start_line.max(0)..stop_line {
            result = result.wrapping_add(f_pow2(spectrum[i as usize]) >> scale);
        }
    } else {
        for i in start_line.max(0)..(stop_line - lag) {
            result = result
                .wrapping_add(f_mult(spectrum[i as usize], spectrum[(i + lag) as usize]) >> scale);
        }
    }

    result
}

/// `HLM_MIN_NRG`: 2^-28
const HLM_MIN_NRG: f32 = 0.0000000037252902984619140625;

/// `FDKaacEnc_AutoCorrNormFac(value, scale, sc)`: energy normalization factor `1/value` (as the
/// mantissa of `invSqrtNorm2` squared); adds the exponent to `*sc`.
#[inline]
pub fn fdk_aac_enc_auto_corr_norm_fac(value: FixpDbl, scale: i32, sc: &mut i32) -> FixpDbl {
    let a;
    let b;

    if scale >= 0 {
        a = value;
        b = fl2fxconst_dbl(HLM_MIN_NRG as f64) >> fix_min(DFRACT_BITS - 1, scale);
    } else {
        // fixMin(DFRACT_BITS - 1, -scale): clang assumes that -scale does not overflow, so
        // INT_MIN gives 31 (as in scaleValue); unreachable, scale is at least -62.
        a = value >> fix_min((DFRACT_BITS - 1) as i64, -(scale as i64)) as i32;
        b = fl2fxconst_dbl(HLM_MIN_NRG as f64);
    }

    if a > b {
        let mut shift = 0;
        let tmp = inv_sqrt_norm2(value, &mut shift);

        *sc = sc.wrapping_add(2 * shift);
        f_mult(tmp, tmp)
    } else {
        // MAX_INV_NRGFAC*FDKpow(2,-28) = 1/2^-28 * 2^-28 = 1.0
        *sc = sc.wrapping_add(scale.wrapping_add(28));
        MAXVAL_DBL
    }
}

/// `FDKaacEnc_MergedAutoCorrelation(spectrum, isLowDelay, acfWindow, lpcStartLine,
/// lpcStopLine, maxOrder, acfSplit, _rxx1, _rxx2)`: autocorrelation function for the 1st and
/// for the merged 2nd, 3rd and 4th quarter of the spectrum, normalized and windowed.
///
/// `_rxx1` and `_rxx2` hold `maxOrder + 1` values; the values the function does not compute
/// are left unchanged.
pub fn fdk_aac_enc_merged_auto_correlation(
    spectrum: &[FixpDbl],
    is_low_delay: i32,
    acf_window: &[[FixpDbl; TNS_MAX_ORDER + 3 + 1]; MAX_NUM_OF_FILTERS],
    lpc_start_line: &[i32; MAX_NUM_OF_FILTERS],
    lpc_stop_line: i32,
    max_order: i32,
    acf_split: &[i32; MAX_NUM_OF_FILTERS],
    rxx1: &mut [FixpDbl],
    rxx2: &mut [FixpDbl],
) {
    let i;
    let idx0;
    let idx1;
    let idx2;
    let idx3;
    let idx4;

    // buffer for temporal spectrum (C_ALLOC_SCRATCH; only the lines written below are read)
    let mut p_spectrum = [0 as FixpDbl; 1024];

    // MDCT line indices separating the 1st, 2nd, 3rd, and 4th analysis quarters
    if (acf_split[LOFILT] == -1) || (acf_split[HIFILT] == -1) {
        // autocorrelation function for 1st, 2nd, 3rd, and 4th quarter of the spectrum
        idx0 = lpc_start_line[LOFILT];
        i = lpc_stop_line - lpc_start_line[LOFILT];
        idx1 = idx0 + i / 4;
        idx2 = idx0 + i / 2;
        idx3 = idx0 + i * 3 / 4;
        idx4 = lpc_stop_line;
    } else {
        fdk_assert!(acf_split[LOFILT] == 1);
        fdk_assert!(acf_split[HIFILT] == 3);
        i = (lpc_stop_line - lpc_start_line[HIFILT]) / 3;
        idx0 = lpc_start_line[LOFILT];
        idx1 = lpc_start_line[HIFILT];
        idx2 = idx1 + i;
        idx3 = idx2 + i;
        idx4 = lpc_stop_line;
    }

    // copy spectrum to temporal buffer and scale up as much as possible
    let sc1 = fdk_aac_enc_scale_up_spectrum(&mut p_spectrum, spectrum, idx0, idx1);
    let sc2 = fdk_aac_enc_scale_up_spectrum(&mut p_spectrum, spectrum, idx1, idx2);
    let sc3 = fdk_aac_enc_scale_up_spectrum(&mut p_spectrum, spectrum, idx2, idx3);
    let sc4 = fdk_aac_enc_scale_up_spectrum(&mut p_spectrum, spectrum, idx3, idx4);

    // get scaling values for summation
    let nsc = |len: i32| {
        let mut n = 1;
        while (1 << n) < len {
            n += 1;
        }
        n
    };
    let nsc1 = nsc(idx1 - idx0);
    let nsc2 = nsc(idx2 - idx1);
    let nsc3 = nsc(idx3 - idx2);
    let nsc4 = nsc(idx4 - idx3);

    // compute autocorrelation value at lag zero, i. e. energy, for each quarter
    let rxx1_0 = fdk_aac_enc_calc_auto_corr_value(&p_spectrum, idx0, idx1, 0, nsc1);
    let rxx2_0 = fdk_aac_enc_calc_auto_corr_value(&p_spectrum, idx1, idx2, 0, nsc2);
    let rxx3_0 = fdk_aac_enc_calc_auto_corr_value(&p_spectrum, idx2, idx3, 0, nsc3);
    let rxx4_0 = fdk_aac_enc_calc_auto_corr_value(&p_spectrum, idx3, idx4, 0, nsc4);

    // compute energy normalization factors, i. e. 1/energy (saves some divisions)
    if rxx1_0 != fl2fxconst_dbl(0.0) {
        let mut sc_fac1 = -1;
        let fac1 = fdk_aac_enc_auto_corr_norm_fac(rxx1_0, (-2 * sc1) + nsc1, &mut sc_fac1);
        rxx1[0] = scale_value(f_mult(rxx1_0, fac1), sc_fac1);

        if is_low_delay != 0 {
            for lag in 1..=max_order {
                // compute energy-normalized and windowed autocorrelation values at this lag
                let x1 = fdk_aac_enc_calc_auto_corr_value(&p_spectrum, idx0, idx1, lag, nsc1);
                rxx1[lag as usize] = f_mult(
                    scale_value(f_mult(x1, fac1), sc_fac1),
                    acf_window[LOFILT][lag as usize],
                );
            }
        } else {
            for lag in 1..=max_order {
                if (3 * lag) <= max_order + 3 {
                    let x1 = fdk_aac_enc_calc_auto_corr_value(&p_spectrum, idx0, idx1, lag, nsc1);
                    rxx1[lag as usize] = f_mult(
                        scale_value(f_mult(x1, fac1), sc_fac1),
                        acf_window[LOFILT][(3 * lag) as usize],
                    );
                }
            }
        }
    }

    // auto corr over upper 3/4 of spectrum
    if !((rxx2_0 == fl2fxconst_dbl(0.0))
        && (rxx3_0 == fl2fxconst_dbl(0.0))
        && (rxx4_0 == fl2fxconst_dbl(0.0)))
    {
        let mut fac2 = fl2fxconst_dbl(0.0);
        let mut fac3 = fl2fxconst_dbl(0.0);
        let mut fac4 = fl2fxconst_dbl(0.0);
        let mut sc_fac2 = 0;
        let mut sc_fac3 = 0;
        let mut sc_fac4 = 0;

        if rxx2_0 != fl2fxconst_dbl(0.0) {
            fac2 = fdk_aac_enc_auto_corr_norm_fac(rxx2_0, (-2 * sc2) + nsc2, &mut sc_fac2);
            sc_fac2 -= 2;
        }
        if rxx3_0 != fl2fxconst_dbl(0.0) {
            fac3 = fdk_aac_enc_auto_corr_norm_fac(rxx3_0, (-2 * sc3) + nsc3, &mut sc_fac3);
            sc_fac3 -= 2;
        }
        if rxx4_0 != fl2fxconst_dbl(0.0) {
            fac4 = fdk_aac_enc_auto_corr_norm_fac(rxx4_0, (-2 * sc4) + nsc4, &mut sc_fac4);
            sc_fac4 -= 2;
        }

        rxx2[0] = scale_value(f_mult(rxx2_0, fac2), sc_fac2)
            .wrapping_add(scale_value(f_mult(rxx3_0, fac3), sc_fac3))
            .wrapping_add(scale_value(f_mult(rxx4_0, fac4), sc_fac4));

        for lag in 1..=max_order {
            // merge quarters 2, 3, 4 into one autocorrelation; quarter 1 stays separate
            let x2 = scale_value(
                f_mult(fdk_aac_enc_calc_auto_corr_value(&p_spectrum, idx1, idx2, lag, nsc2), fac2),
                sc_fac2,
            )
            .wrapping_add(scale_value(
                f_mult(fdk_aac_enc_calc_auto_corr_value(&p_spectrum, idx2, idx3, lag, nsc3), fac3),
                sc_fac3,
            ))
            .wrapping_add(scale_value(
                f_mult(fdk_aac_enc_calc_auto_corr_value(&p_spectrum, idx3, idx4, lag, nsc4), fac4),
                sc_fac4,
            ));

            rxx2[lag as usize] = f_mult(x2, acf_window[HIFILT][lag as usize]);
        }
    }
}

/// `FDKaacEnc_TnsDetect(tnsData, tC, tnsInfo, sfbCnt, spectrum, subBlockNumber, blockType)`: do
/// decision, if TNS shall be used or not.
///
/// `spectrum` is the spectrum of the (sub)block (`mdctSpectrum + wOffset`). Writes the
/// `TNS_SUBBLOCK_INFO` of the block (`dataRaw.Short.subBlockInfo[subBlockNumber]` for short
/// blocks, `dataRaw.Long.subBlockInfo` otherwise), `filtersMerged` and the entries
/// `[subBlockNumber]` of `tnsInfo`.
pub fn fdk_aac_enc_tns_detect(
    tns_data: &mut TnsData,
    t_c: &TnsConfig,
    tns_info: &mut TnsInfo,
    sfb_cnt: i32,
    spectrum: &[FixpDbl],
    sub_block_number: i32,
    block_type: i32,
) -> i32 {
    // autocorrelation function for 1st, 2nd, 3rd, and 4th quarter of the spectrum.
    let mut rxx1 = [0 as FixpDbl; TNS_MAX_ORDER + 1]; // higher part
    let mut rxx2 = [0 as FixpDbl; TNS_MAX_ORDER + 1]; // lower part
    let mut parcor_tmp = [0 as FixpLpc; TNS_MAX_ORDER];

    let sbn = sub_block_number as usize;

    let tsbi = if block_type == SHORT_WINDOW {
        TnsDataRaw::short_sub_block_info(sbn)
    } else {
        TnsDataRaw::LONG_SUB_BLOCK_INFO
    };
    let raw = &mut tns_data.data_raw;

    tns_data.filters_merged = FALSE;

    *raw.tns_active_mut(tsbi, HIFILT) = FALSE;
    *raw.prediction_gain_mut(tsbi, HIFILT) = 1000;
    *raw.tns_active_mut(tsbi, LOFILT) = FALSE;
    *raw.prediction_gain_mut(tsbi, LOFILT) = 1000;

    tns_info.num_of_filters[sbn] = 0;
    tns_info.coef_res[sbn] = t_c.coef_res;
    for i in 0..t_c.max_order.max(0) as usize {
        tns_info.coef[sbn][HIFILT][i] = 0;
        tns_info.coef[sbn][LOFILT][i] = 0;
    }

    tns_info.length[sbn][HIFILT] = 0;
    tns_info.length[sbn][LOFILT] = 0;
    tns_info.order[sbn][HIFILT] = 0;
    tns_info.order[sbn][LOFILT] = 0;

    if (t_c.tns_active != 0) && (t_c.max_order > 0) {
        fdk_aac_enc_merged_auto_correlation(
            spectrum,
            t_c.is_low_delay,
            &t_c.acf_window,
            &t_c.lpc_start_line,
            t_c.lpc_stop_line,
            t_c.max_order,
            &t_c.conf_tab.acf_split,
            &mut rxx1,
            &mut rxx2,
        );

        // compute higher TNS filter coefficients in lattice form (ParCor) with
        // LeRoux-Gueguen/Schur algorithm
        {
            let mut prediction_gain_m = 0;
            let mut prediction_gain_e = 0;

            clpc_auto_to_parcor(
                &mut rxx2,
                0,
                &mut parcor_tmp,
                t_c.conf_tab.tns_limit_order[HIFILT],
                Some((&mut prediction_gain_m, &mut prediction_gain_e)),
            );
            *raw.prediction_gain_mut(tsbi, HIFILT) =
                f_mult_norm_to(prediction_gain_m, prediction_gain_e, 1000, 31, 31);
        }

        // non-linear quantization of TNS lattice coefficients with given resolution
        fdk_aac_enc_parcor2_index(
            &parcor_tmp,
            &mut tns_info.coef[sbn][HIFILT],
            t_c.conf_tab.tns_limit_order[HIFILT],
            t_c.coef_res,
        );

        // reduce filter order by truncating trailing zeros, compute sum(abs(coefs))
        let mut i = t_c.conf_tab.tns_limit_order[HIFILT] - 1;
        while i >= 0 {
            if tns_info.coef[sbn][HIFILT][i as usize] != 0 {
                break;
            }
            i -= 1;
        }

        tns_info.order[sbn][HIFILT] = i + 1;

        let mut sum_sqr_coef = 0;
        while i >= 0 {
            sum_sqr_coef +=
                tns_info.coef[sbn][HIFILT][i as usize] * tns_info.coef[sbn][HIFILT][i as usize];
            i -= 1;
        }

        tns_info.direction[sbn][HIFILT] = t_c.conf_tab.tns_filter_direction[HIFILT];
        tns_info.length[sbn][HIFILT] = sfb_cnt - t_c.lpc_start_band[HIFILT];

        // disable TNS if predictionGain is less than 3dB or sumSqrCoef is too small
        if (raw.prediction_gain(tsbi, HIFILT) > t_c.conf_tab.thresh_on[HIFILT])
            || (sum_sqr_coef > (t_c.conf_tab.tns_limit_order[HIFILT] / 2 + 2))
        {
            *raw.tns_active_mut(tsbi, HIFILT) = TRUE;
            tns_info.num_of_filters[sbn] += 1;

            // compute second filter for lower quarter; only allowed for long windows!
            if (block_type != SHORT_WINDOW)
                && (t_c.conf_tab.filter_enabled[LOFILT] != 0)
                && (t_c.conf_tab.seperate_filters_allowed != 0)
            {
                // compute second filter for lower frequencies

                // compute TNS filter in lattice (ParCor) form with LeRoux-Gueguen algorithm
                let pred_gain;
                {
                    let mut prediction_gain_m = 0;
                    let mut prediction_gain_e = 0;

                    clpc_auto_to_parcor(
                        &mut rxx1,
                        0,
                        &mut parcor_tmp,
                        t_c.conf_tab.tns_limit_order[LOFILT],
                        Some((&mut prediction_gain_m, &mut prediction_gain_e)),
                    );
                    pred_gain = f_mult_norm_to(prediction_gain_m, prediction_gain_e, 1000, 31, 31);
                }

                // non-linear quantization of TNS lattice coefficients with given resolution
                fdk_aac_enc_parcor2_index(
                    &parcor_tmp,
                    &mut tns_info.coef[sbn][LOFILT],
                    t_c.conf_tab.tns_limit_order[LOFILT],
                    t_c.coef_res,
                );

                // reduce filter order by truncating trailing zeros, compute sum(abs(coefs))
                i = t_c.conf_tab.tns_limit_order[LOFILT] - 1;
                while i >= 0 {
                    if tns_info.coef[sbn][LOFILT][i as usize] != 0 {
                        break;
                    }
                    i -= 1;
                }
                tns_info.order[sbn][LOFILT] = i + 1;

                sum_sqr_coef = 0;
                while i >= 0 {
                    sum_sqr_coef += tns_info.coef[sbn][LOFILT][i as usize]
                        * tns_info.coef[sbn][LOFILT][i as usize];
                    i -= 1;
                }

                tns_info.direction[sbn][LOFILT] = t_c.conf_tab.tns_filter_direction[LOFILT];
                tns_info.length[sbn][LOFILT] =
                    t_c.lpc_start_band[HIFILT] - t_c.lpc_start_band[LOFILT];

                // filter lower quarter if gain is high enough, but not if it's too high
                if ((pred_gain > t_c.conf_tab.thresh_on[LOFILT])
                    && (pred_gain < (16000 * t_c.conf_tab.tns_limit_order[LOFILT])))
                    || ((sum_sqr_coef > 9)
                        && (sum_sqr_coef < 22 * t_c.conf_tab.tns_limit_order[LOFILT]))
                {
                    // compare lower to upper filter; if they are very similar, merge them
                    *raw.tns_active_mut(tsbi, LOFILT) = TRUE;
                    sum_sqr_coef = 0;
                    i = 0;
                    while i < t_c.conf_tab.tns_limit_order[LOFILT] {
                        sum_sqr_coef += f_abs(
                            tns_info.coef[sbn][HIFILT][i as usize]
                                - tns_info.coef[sbn][LOFILT][i as usize],
                        );
                        i += 1;
                    }
                    if (sum_sqr_coef < 2)
                        && (tns_info.direction[sbn][LOFILT] == tns_info.direction[sbn][HIFILT])
                    {
                        tns_data.filters_merged = TRUE;
                        tns_info.length[sbn][HIFILT] = sfb_cnt - t_c.lpc_start_band[LOFILT];
                        while i < tns_info.order[sbn][HIFILT] {
                            if f_abs(tns_info.coef[sbn][HIFILT][i as usize]) > 1 {
                                break;
                            }
                            i += 1;
                        }
                        i -= 1;
                        while i >= 0 {
                            if tns_info.coef[sbn][HIFILT][i as usize] != 0 {
                                break;
                            }
                            i -= 1;
                        }
                        if i < tns_info.order[sbn][HIFILT] {
                            tns_info.order[sbn][HIFILT] = i + 1;
                        }
                    } else {
                        tns_info.num_of_filters[sbn] += 1;
                    }
                } // filter lower part
                *tns_data.data_raw.prediction_gain_mut(tsbi, LOFILT) = pred_gain;
            } // second filter allowed
        } // if predictionGain > 1437 ...
    } // maxOrder > 0 && tnsActive

    0
}

/// `FDKaacEnc_TnsSync(tnsDataDest, tnsDataSrc, tnsInfoDest, tnsInfoSrc, blockTypeDest,
/// blockTypeSrc, tC)`: synchronize TNS parameters when TNS gain difference small (relative).
pub fn fdk_aac_enc_tns_sync(
    tns_data_dest: &mut TnsData,
    tns_data_src: &TnsData,
    tns_info_dest: &mut TnsInfo,
    tns_info_src: &TnsInfo,
    block_type_dest: i32,
    block_type_src: i32,
    t_c: &TnsConfig,
) {
    // if one channel contains short blocks and the other not, do not synchronize
    if (block_type_src == SHORT_WINDOW && block_type_dest != SHORT_WINDOW)
        || (block_type_dest == SHORT_WINDOW && block_type_src != SHORT_WINDOW)
    {
        return;
    }

    let n_windows = if block_type_dest != SHORT_WINDOW { 1 } else { 8 };
    // sbInfoDest = &dataRaw.Long.subBlockInfo or &dataRaw.Short.subBlockInfo[0], both at word
    // offset 0; window w is the TNS_SUBBLOCK_INFO at offset w * TNS_SUBBLOCK_INFO_WORDS.

    for w in 0..n_windows {
        let o = TnsDataRaw::short_sub_block_info(w);
        let mut do_sync = 1;
        let mut abs_diff_sum = 0;

        // if TNS is active in at least one channel, check if ParCor coefficients of higher
        // filter are similar
        if tns_data_dest.data_raw.tns_active(o, HIFILT) != 0
            || tns_data_src.data_raw.tns_active(o, HIFILT) != 0
        {
            for i in 0..t_c.max_order.max(0) as usize {
                let abs_diff = f_abs(
                    tns_info_dest.coef[w][HIFILT][i].wrapping_sub(tns_info_src.coef[w][HIFILT][i]),
                );
                abs_diff_sum += abs_diff;
                // if coefficients diverge too much between channels, do not synchronize
                if (abs_diff > 1) || (abs_diff_sum > 2) {
                    do_sync = 0;
                    break;
                }
            }

            if do_sync != 0 {
                // if no significant difference was detected, synchronize coefficient sets
                if tns_data_src.data_raw.tns_active(o, HIFILT) != 0 {
                    // no dest filter, or more dest than source filters: use one dest filter
                    if (tns_data_dest.data_raw.tns_active(o, HIFILT) == 0)
                        || ((tns_data_dest.data_raw.tns_active(o, HIFILT) != 0)
                            && (tns_info_dest.num_of_filters[w] > tns_info_src.num_of_filters[w]))
                    {
                        tns_info_dest.num_of_filters[w] = 1;
                        *tns_data_dest.data_raw.tns_active_mut(o, HIFILT) = 1;
                    }
                    tns_data_dest.filters_merged = tns_data_src.filters_merged;
                    tns_info_dest.order[w][HIFILT] = tns_info_src.order[w][HIFILT];
                    tns_info_dest.length[w][HIFILT] = tns_info_src.length[w][HIFILT];
                    tns_info_dest.direction[w][HIFILT] = tns_info_src.direction[w][HIFILT];
                    tns_info_dest.coef_compress[w][HIFILT] = tns_info_src.coef_compress[w][HIFILT];

                    for i in 0..t_c.max_order.max(0) as usize {
                        tns_info_dest.coef[w][HIFILT][i] = tns_info_src.coef[w][HIFILT][i];
                    }
                } else {
                    tns_info_dest.num_of_filters[w] = 0;
                    *tns_data_dest.data_raw.tns_active_mut(o, HIFILT) = 0;
                }
            }
        }
    }
}

/// `FDKaacEnc_TnsEncode(tnsInfo, tnsData, numOfSfb, tC, lowPassLine, spectrum, subBlockNumber,
/// blockType)`: perform TNS encoding (filters `spectrum` in place).
///
/// `spectrum` is the spectrum of the (sub)block (`mdctSpectrum + wOffset`). Returns 1 if TNS is
/// not active in the block, 0 otherwise.
pub fn fdk_aac_enc_tns_encode(
    tns_info: &TnsInfo,
    tns_data: &TnsData,
    _num_of_sfb: i32,
    t_c: &TnsConfig,
    _low_pass_line: i32,
    spectrum: &mut [FixpDbl],
    sub_block_number: i32,
    block_type: i32,
) -> i32 {
    let sbn = sub_block_number as usize;

    if ((block_type == SHORT_WINDOW)
        && (tns_data.data_raw.tns_active(TnsDataRaw::short_sub_block_info(sbn), HIFILT) == 0))
        || ((block_type != SHORT_WINDOW)
            && (tns_data.data_raw.tns_active(TnsDataRaw::LONG_SUB_BLOCK_INFO, HIFILT) == 0))
    {
        return 1;
    }

    let mut start_line = if tns_data.filters_merged != 0 {
        t_c.lpc_start_line[LOFILT]
    } else {
        t_c.lpc_start_line[HIFILT]
    };
    let mut stop_line = t_c.lpc_stop_line;

    for i in 0..tns_info.num_of_filters[sbn].max(0) as usize {
        let mut lpc_coeff = [0 as FixpLpc; TNS_MAX_ORDER];
        let mut work_buffer = [0 as FixpDbl; TNS_MAX_ORDER];
        let mut parcor_tmp = [0 as FixpLpc; TNS_MAX_ORDER];

        fdk_aac_enc_index2_parcor(
            &tns_info.coef[sbn][i],
            &mut parcor_tmp,
            tns_info.order[sbn][i],
            t_c.coef_res,
        );

        let lpc_gain_factor = clpc_parcor_to_lpc_sgl(
            &parcor_tmp,
            &mut lpc_coeff,
            tns_info.order[sbn][i],
            &mut work_buffer,
        );

        work_buffer = [0; TNS_MAX_ORDER];
        clpc_analysis(
            &mut spectrum[start_line as usize..],
            stop_line - start_line,
            &lpc_coeff,
            lpc_gain_factor,
            tns_info.order[sbn][i],
            &mut work_buffer,
            None,
        );

        // update for second filter
        start_line = t_c.lpc_start_line[LOFILT];
        stop_line = t_c.lpc_start_line[HIFILT];
    }

    0
}

/// `PI_E`
const PI_E: i32 = 2;
/// `PI_M`: `FL2FXCONST_DBL(3.1416f / (float)(1 << PI_E))`
const PI_M: FixpDbl = fl2fxconst_dbl((3.1416f32 / (1 << PI_E) as f32) as f64);
/// `EULER_E`
const EULER_E: i32 = 2;
/// `EULER_M`: `FL2FXCONST_DBL(2.7183 / (float)(1 << EULER_E))`
const EULER_M: FixpDbl = fl2fxconst_dbl(2.7183 / (1 << EULER_E) as f64);
/// `COEFF_LOOP_SCALE`
const COEFF_LOOP_SCALE: i32 = 4;

/// `(INT)(transformResolution * 1000.f)` on the x87 unit (`fildl`, `fmuls`, `fistpl`): the
/// product is exact (at most 41 significant bits), so it is the integer product (`INT_MIN` out
/// of range). Public for the differential tests against [`fdk_aac_rs_fdk::x87`].
pub fn transform_resolution_ms_x87(transform_resolution: i32) -> i32 {
    i32::try_from(transform_resolution as i64 * 1000).unwrap_or(i32::MIN)
}

/// `(INT)(transformResolution * 1000.f)` of `FDKaacEnc_CalcGaussWindow`: in `float` on the
/// 64-bit builds (`scvtf`/`fmul`/`fcvtzs` resp. `cvtsi2ss`/`mulss`/`cvttss2si`), exact on the
/// x87 unit of the `x86_32` build (`fildl`, `fmuls`, `fistpl`; `INT_MIN` out of range).
fn transform_resolution_ms(transform_resolution: i32) -> i32 {
    if fdk_aac_rs_fdk::ARCH == fdk_aac_rs_fdk::Arch::X86_32 {
        transform_resolution_ms_x87(transform_resolution)
    } else {
        (transform_resolution as f32 * 1000.0f32) as i32
    }
}

/// `FDKaacEnc_CalcGaussWindow(win, winSize, samplingRate, transformResolution, timeResolution,
/// timeResolution_e)`: autocorrelation window
/// `win[i] = exp(gaussExp * (i+0.5) * (i+0.5))` for `i < winSize`.
pub fn fdk_aac_enc_calc_gauss_window(
    win: &mut [FixpDbl],
    win_size: i32,
    sampling_rate: i32,
    transform_resolution: i32,
    time_resolution: FixpDbl,
    time_resolution_e: i32,
) {
    let mut e1 = 0;
    let mut e2 = 0;

    // calc. window exponent from time resolution:
    //
    //   gaussExp = PI * samplingRate * 0.001f * timeResolution / transformResolution;
    //   gaussExp = -0.5f * gaussExp * gaussExp;
    let mut gauss_exp_m = f_mult_norm(
        time_resolution,
        f_mult(
            PI_M,
            f_div_norm(sampling_rate, transform_resolution_ms(transform_resolution), &mut e1),
        ),
        &mut e2,
    );
    gauss_exp_m = f_pow2_div2(gauss_exp_m).wrapping_neg();
    let gauss_exp_e = 2 * (e1 + e2 + time_resolution_e + PI_E);

    fdk_assert!(win_size < (1 << COEFF_LOOP_SCALE));

    // calc. window coefficients
    //   win[i] = (float)exp( gaussExp * (i+0.5) * (i+0.5) );
    for i in 0..win_size.max(0) {
        win[i as usize] = f_pow(
            EULER_M,
            EULER_E,
            f_mult(
                gauss_exp_m,
                f_pow2(
                    i.wrapping_mul(fl2fxconst_dbl(
                        (1.0f32 / (1 << COEFF_LOOP_SCALE) as f32) as f64,
                    ))
                    .wrapping_add(fl2fxconst_dbl((0.5f32 / (1 << COEFF_LOOP_SCALE) as f32) as f64)),
                ),
            ),
            gauss_exp_e + 2 * COEFF_LOOP_SCALE,
            &mut e1,
        );

        win[i as usize] = scale_value_saturate(win[i as usize], e1);
    }
}

/// `FDKaacEnc_Search3(parcor)`: 3 bit quantization index of a ParCor coefficient.
pub fn fdk_aac_enc_search3(parcor: FixpLpc) -> i32 {
    let mut index = 0;

    for i in 0..8 {
        if parcor > FDK_AAC_ENC_TNS_COEFF_3_BORDERS[i] {
            index = i as i32;
        }
    }
    index - 4
}

/// `FDKaacEnc_Search4(parcor)`: 4 bit quantization index of a ParCor coefficient.
pub fn fdk_aac_enc_search4(parcor: FixpLpc) -> i32 {
    let mut index = 0;

    for i in 0..16 {
        if parcor > FDK_AAC_ENC_TNS_COEFF_4_BORDERS[i] {
            index = i as i32;
        }
    }
    index - 8
}

/// `FDKaacEnc_Parcor2Index(parcor, index, order, bitsPerCoeff)`: non-linear quantization of the
/// ParCor coefficients.
pub fn fdk_aac_enc_parcor2_index(
    parcor: &[FixpLpc],
    index: &mut [i32],
    order: i32,
    bits_per_coeff: i32,
) {
    for i in 0..order.max(0) as usize {
        if bits_per_coeff == 3 {
            index[i] = fdk_aac_enc_search3(parcor[i]);
        } else {
            index[i] = fdk_aac_enc_search4(parcor[i]);
        }
    }
}

/// `FDKaacEnc_Index2Parcor(index, parcor, order, bitsPerCoeff)`: inverse quantization for
/// reflection coefficients.
pub fn fdk_aac_enc_index2_parcor(
    index: &[i32],
    parcor: &mut [FixpLpc],
    order: i32,
    bits_per_coeff: i32,
) {
    for i in 0..order.max(0) as usize {
        parcor[i] = if bits_per_coeff == 4 {
            FDK_AAC_ENC_TNS_ENC_COEFF_4[(index[i] + 8) as usize]
        } else {
            FDK_AAC_ENC_TNS_ENC_COEFF_3[(index[i] + 4) as usize]
        };
    }
}

// The optimized autocorrelation sum against the straight port, without the C++ reference (also
// with `FDK_AAC_RS_ARCH=aarch64|x86_32|generic` on an x86 host, and with `FDK_AAC_RS_NO_SIMD`).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_auto_corr_value_equals_straight_port() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 32) as u32
        };
        for k in 0..200_000 {
            let n = 1 + (next() % 1024) as usize;
            let mode = next() % 4;
            let spec: Vec<FixpDbl> = (0..n)
                .map(|_| match mode {
                    0 => [i32::MIN, i32::MAX, -1, 0, 1, i32::MIN + 1][(next() % 6) as usize],
                    1 => next() as i32,
                    _ => (next() as i32) >> (next() % 32),
                })
                .collect();
            let start = (next() % (n as u32 + 8)) as i32 - 4;
            let stop = (next() % (n as u32 + 1)) as i32;
            let lag = if k % 4 == 0 { 0 } else { (next() % 13) as i32 };
            let scale = (next() % 32) as i32;
            if stop - lag > start.max(0) && stop > n as i32 {
                continue;
            }
            assert_eq!(
                fdk_aac_enc_calc_auto_corr_value(&spec, start, stop, lag, scale),
                fdk_aac_enc_calc_auto_corr_value_straight(&spec, start, stop, lag, scale),
                "CalcAutoCorrValue n {n} {start}..{stop} lag {lag} scale {scale}"
            );
        }
    }
}
