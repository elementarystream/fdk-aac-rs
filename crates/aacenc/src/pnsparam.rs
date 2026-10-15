// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/pnsparam.h, libAACenc/src/pnsparam.cpp.

//! Port of `pnsparam.h` and `pnsparam.cpp`: PNS parameters depending on bitrate and bandwidth.
//!
//! The tables of `pnsparam.cpp` are generated into [`tables`] by `pnsparam/gen_tables.py`.

pub mod tables;

use crate::aacenc::{AacEncoderError, AAC_ENC_OK, AAC_ENC_PNS_TABLE_ERROR};
use crate::aacenc_tns::fdk_aac_enc_freq_to_band_width_rounding;
use crate::psy_const::MAX_GROUPED_SFB;
use fdk_aac_rs_fdk::common_fix::*;
use tables::*;

/// `NUM_PNSINFOTAB`
pub const NUM_PNSINFOTAB: usize = 4;
/// `PNS_TABLE_ERROR`
pub const PNS_TABLE_ERROR: i32 = -1;

/// `USE_POWER_DISTRIBUTION`
pub const USE_POWER_DISTRIBUTION: u16 = 1 << 0;
/// `USE_PSYCH_TONALITY`
pub const USE_PSYCH_TONALITY: u16 = 1 << 1;
/// `USE_TNS_GAIN_THR`
pub const USE_TNS_GAIN_THR: u16 = 1 << 2;
/// `USE_TNS_PNS`
pub const USE_TNS_PNS: u16 = 1 << 3;
/// `JUST_LONG_WINDOW`
pub const JUST_LONG_WINDOW: u16 = 1 << 4;
/// `IS_LOW_COMPLEXITY`
pub const IS_LOW_COMPLEXITY: u16 = 1 << 5;

/// `NOISEPARAMS` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoiseParams {
    /// PNS start band
    pub start_sfb: i16,
    /// detection algorithm flags
    pub detection_algorithm_flags: u16,
    pub ref_power: FixpDbl,
    pub ref_tonality: FixpDbl,
    pub tns_gain_threshold: i32,
    pub tns_pns_gain_threshold: i32,
    pub min_sfb_width: i32,
    pub pow_dist_psd_curve: [FixpSgl; MAX_GROUPED_SFB],
    pub gap_fill_thr: FixpSgl,
}

impl Default for NoiseParams {
    fn default() -> Self {
        NoiseParams {
            start_sfb: 0,
            detection_algorithm_flags: 0,
            ref_power: 0,
            ref_tonality: 0,
            tns_gain_threshold: 0,
            tns_pns_gain_threshold: 0,
            min_sfb_width: 0,
            pow_dist_psd_curve: [0; MAX_GROUPED_SFB],
            gap_fill_thr: 0,
        }
    }
}

impl NoiseParams {
    /// `powDistPSDcurve[i]` read through the C layout: index -1 is the upper half of
    /// `minSfbWidth`, index `MAX_GROUPED_SFB` is `gapFillThr` (reproduced from the C layout).
    /// Other indices outside the array read 0.
    fn pow_dist_psd_curve_at(&self, i: i32) -> FixpSgl {
        if (0..MAX_GROUPED_SFB as i32).contains(&i) {
            self.pow_dist_psd_curve[i as usize]
        } else if i == -1 {
            (self.min_sfb_width >> 16) as FixpSgl
        } else if i == MAX_GROUPED_SFB as i32 {
            self.gap_fill_thr
        } else {
            0
        }
    }

    /// Writes `powDistPSDcurve[i]` through the C layout (see [`Self::pow_dist_psd_curve_at`]);
    /// writes outside the structure are dropped.
    fn set_pow_dist_psd_curve_at(&mut self, i: i32, v: FixpSgl) {
        if (0..MAX_GROUPED_SFB as i32).contains(&i) {
            self.pow_dist_psd_curve[i as usize] = v;
        } else if i == -1 {
            self.min_sfb_width = (self.min_sfb_width & 0xFFFF) | ((v as i32) << 16);
        } else if i == MAX_GROUPED_SFB as i32 {
            self.gap_fill_thr = v;
        }
    }
}

/// `PNS_INFO_TAB` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PnsInfoTab {
    pub start_freq: i16,
    // Parameters for detection
    pub ref_power: FixpSgl,
    pub ref_tonality: FixpSgl,
    /// scaled by TNS_PREDGAIN_SCALE (=1000)
    pub tns_gain_threshold: i16,
    /// scaled by TNS_PREDGAIN_SCALE (=1000)
    pub tns_pns_gain_threshold: i16,
    pub gap_fill_thr: FixpSgl,
    pub min_sfb_width: i16,
    pub detection_algorithm_flags: u16,
}

/// `AUTO_PNS_TAB` (layout identical to the C struct, with its padding as `_pad`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoPnsTab {
    pub br_from: u32,
    pub br_to: u32,
    pub s16000: u8,
    pub s22050: u8,
    pub s24000: u8,
    pub s32000: u8,
    pub s44100: u8,
    pub s48000: u8,
    pub _pad: [u8; 2],
}

/// `FDKaacEnc_lookUpPnsUse(bitRate, sampleRate, numChan, isLC)`: function to look up used pns
/// level.
///
/// Deviation: where the reference has no defined result, the port returns 0 (no PNS).
pub fn fdk_aac_enc_look_up_pns_use(
    bit_rate: i32,
    sample_rate: i32,
    num_chan: i32,
    is_lc: i32,
) -> i32 {
    let level_table: &[AutoPnsTab] = if is_lc != 0 {
        &LEVEL_TABLE_LOW_COMPLEXITY
    } else if num_chan > 1 {
        // (E)LD
        &LEVEL_TABLE_STEREO
    } else {
        &LEVEL_TABLE_MONO
    };

    let mut i = 0;
    while i < level_table.len() {
        if (bit_rate as u32 >= level_table[i].br_from) && (bit_rate as u32 <= level_table[i].br_to)
        {
            break;
        }
        i += 1;
    }

    // sanity check
    if PNS_INFO_TAB.len() < i {
        return PNS_TABLE_ERROR;
    }

    let Some(entry) = level_table.get(i) else {
        // (no defined reference result, see above)
        return 0;
    };

    match sample_rate {
        16000 => entry.s16000 as i32,
        22050 => entry.s22050 as i32,
        24000 => entry.s24000 as i32,
        32000 => entry.s32000 as i32,
        44100 => entry.s44100 as i32,
        48000 => entry.s48000 as i32,
        _ => {
            if is_lc != 0 {
                entry.s48000 as i32
            } else {
                0
            }
        }
    }
}

/// `FDKaacEnc_GetPnsParam(np, bitRate, sampleRate, sfbCnt, sfbOffset, usePns, numChan, isLC)`:
/// gets PNS parameters depending on bitrate and bandwidth or bitsPerLine.
///
/// `sfb_offset` holds `sfbCnt + 1` offsets. `powDistPSDcurve[sfbCnt]` is set from
/// `powDistPSDcurve[sfbCnt - 1]`, which the loop does not write: the value of a previous call
/// (or of the caller's initialization) is copied, as in C.
pub fn fdk_aac_enc_get_pns_param(
    np: &mut NoiseParams,
    bit_rate: i32,
    sample_rate: i32,
    sfb_cnt: i32,
    sfb_offset: &[i32],
    use_pns: &mut i32,
    num_chan: i32,
    is_lc: i32,
) -> AacEncoderError {
    if *use_pns <= 0 {
        return AAC_ENC_OK;
    }

    let pns_info_tab: &[PnsInfoTab];
    let h_use_pns;
    if is_lc != 0 {
        np.detection_algorithm_flags = IS_LOW_COMPLEXITY;

        pns_info_tab = &PNS_INFO_TAB_LOW_COMPLEXITY;

        // new pns params
        h_use_pns = fdk_aac_enc_look_up_pns_use(bit_rate, sample_rate, num_chan, is_lc);
        if h_use_pns == 0 {
            *use_pns = 0;
            return AAC_ENC_OK;
        }

        if h_use_pns == PNS_TABLE_ERROR {
            return AAC_ENC_PNS_TABLE_ERROR;
        }
    } else {
        np.detection_algorithm_flags = 0;
        pns_info_tab = &PNS_INFO_TAB;

        // new pns params
        h_use_pns = fdk_aac_enc_look_up_pns_use(bit_rate, sample_rate, num_chan, is_lc);
        if h_use_pns == 0 {
            *use_pns = 0;
            return AAC_ENC_OK;
        }
        if h_use_pns == PNS_TABLE_ERROR {
            return AAC_ENC_PNS_TABLE_ERROR;
        }
    }

    // select correct row of tuning table (the level tables hold only rows 1..=len)
    let pns_info = &pns_info_tab[(h_use_pns - 1) as usize];

    np.start_sfb = fdk_aac_enc_freq_to_band_width_rounding(
        pns_info.start_freq as i32,
        sample_rate,
        sfb_cnt,
        sfb_offset,
    ) as i16;

    np.detection_algorithm_flags |= pns_info.detection_algorithm_flags;

    np.ref_power = fx_sgl2fx_dbl(pns_info.ref_power);
    np.ref_tonality = fx_sgl2fx_dbl(pns_info.ref_tonality);
    np.tns_gain_threshold = pns_info.tns_gain_threshold as i32;
    np.tns_pns_gain_threshold = pns_info.tns_pns_gain_threshold as i32;
    np.min_sfb_width = pns_info.min_sfb_width as i32;

    np.gap_fill_thr = pns_info.gap_fill_thr; // for LC it is always FL2FXCONST_SGL(0.5)

    // assuming a constant dB/Hz slope in the signal's PSD curve,
    // the detection threshold needs to be corrected for the width of the band

    for i in 0..(sfb_cnt - 1).max(0) as usize {
        let mut qtmp = 0;

        let sfb_width = sfb_offset[i + 1].wrapping_sub(sfb_offset[i]);

        let tmp = f_pow(np.ref_power, 0, sfb_width, DFRACT_BITS - 1 - 5, &mut qtmp);
        np.set_pow_dist_psd_curve_at(i as i32, (scale_value(tmp, qtmp) >> 16) as FixpSgl);
    }
    let v = np.pow_dist_psd_curve_at(sfb_cnt - 1);
    np.set_pow_dist_psd_curve_at(sfb_cnt, v);

    AAC_ENC_OK
}
