// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/psy_configuration.h, libAACenc/src/psy_configuration.cpp.

//! Port of `psy_configuration.h` and `psy_configuration.cpp`: configuration of the
//! psychoacoustic model.
//!
//! The file-static tables of `psy_configuration.cpp` are generated into [`tables`] by
//! `psy_configuration/gen_tables.py`.

pub mod tables;

use crate::aacenc::{
    AacEncoderError, AAC_ENC_INVALID_FRAME_LENGTH, AAC_ENC_OK, AAC_ENC_UNSUPPORTED_SAMPLINGRATE,
};
use crate::aacenc_pns::PnsConfig;
use crate::aacenc_tns::TnsConfig;
use crate::psy_const::*;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::trig_fcts::fixp_atan;
use tables::*;

/// `THR_SHIFTBITS`
pub const THR_SHIFTBITS: i32 = 4;
/// `PCM_QUANT_THR_SCALE`
pub const PCM_QUANT_THR_SCALE: i32 = 16;
/// `BITS_PER_LINE_SHIFT`
pub const BITS_PER_LINE_SHIFT: i32 = 3;

/// `C_RATIO`: `FL2FXCONST_DBL(0.001258925f) << THR_SHIFTBITS` (`pow(10.0f, -(29.0f/10.0f))`)
pub const C_RATIO: FixpDbl = 0x02940a10;

/// `PSY_CONFIGURATION` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsyConfiguration {
    /// number of existing sf bands
    pub sfb_cnt: i32,
    /// number of sf bands containing energy after lowpass
    pub sfb_active: i32,
    pub sfb_active_lfe: i32,
    pub sfb_offset: [i32; MAX_SFB + 1],
    /// LC, LD or ELD
    pub filterbank: i32,
    pub sfb_pcm_quant_threshold: [FixpDbl; MAX_SFB],
    /// preecho control
    pub max_allowed_increase_factor: i32,
    pub min_remaining_threshold_factor: FixpSgl,
    pub lowpass_line: i32,
    pub lowpass_line_lfe: i32,
    /// for level dependend tmn
    pub clip_energy: FixpDbl,
    pub sfb_mask_low_factor: [FixpDbl; MAX_SFB],
    pub sfb_mask_high_factor: [FixpDbl; MAX_SFB],
    pub sfb_mask_low_factor_spr_en: [FixpDbl; MAX_SFB],
    pub sfb_mask_high_factor_spr_en: [FixpDbl; MAX_SFB],
    /// minimum snr (formerly known as bmax)
    pub sfb_min_snr_ld_data: [FixpDbl; MAX_SFB],
    pub tns_conf: TnsConfig,
    pub pns_conf: PnsConfig,
    pub granule_length: i32,
    pub allow_is: i32,
    pub allow_ms: i32,
}

impl Default for PsyConfiguration {
    fn default() -> Self {
        PsyConfiguration {
            sfb_cnt: 0,
            sfb_active: 0,
            sfb_active_lfe: 0,
            sfb_offset: [0; MAX_SFB + 1],
            filterbank: 0,
            sfb_pcm_quant_threshold: [0; MAX_SFB],
            max_allowed_increase_factor: 0,
            min_remaining_threshold_factor: 0,
            lowpass_line: 0,
            lowpass_line_lfe: 0,
            clip_energy: 0,
            sfb_mask_low_factor: [0; MAX_SFB],
            sfb_mask_high_factor: [0; MAX_SFB],
            sfb_mask_low_factor_spr_en: [0; MAX_SFB],
            sfb_mask_high_factor_spr_en: [0; MAX_SFB],
            sfb_min_snr_ld_data: [0; MAX_SFB],
            tns_conf: TnsConfig::default(),
            pns_conf: PnsConfig::default(),
            granule_length: 0,
            allow_is: 0,
            allow_ms: 0,
        }
    }
}

/// `SFB_PARAM_LONG` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SfbParamLong {
    /// Number of scalefactor bands
    pub sfb_cnt: u8,
    /// Width of scalefactor bands for long blocks
    pub sfb_width: [u8; MAX_SFB_LONG],
}

/// `SFB_PARAM_SHORT` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SfbParamShort {
    /// Number of scalefactor bands
    pub sfb_cnt: u8,
    /// Width of scalefactor bands for short blocks
    pub sfb_width: [u8; MAX_SFB_SHORT],
}

/// `SFB_INFO_TAB`: the scalefactor band tables of a sample rate (`NULL` → `None`).
#[derive(Clone, Copy, Debug)]
pub struct SfbInfoTab {
    pub sample_rate: i32,
    pub param_long: &'static SfbParamLong,
    pub param_short: Option<&'static SfbParamShort>,
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

/// `FDKaacEnc_initSfbTable(sampleRate, blockType, granuleLength, sfbOffset, sfbCnt)`: the
/// scalefactor band offsets (`sfbCnt + 1` values) of a sample rate and block type.
///
/// Deviations, where the reference has no defined result:
///
/// - For the LD tables (`granuleLength` 512 and 480) and a sample rate that is not in the
///   table, the port searches the 18 entries and returns `AAC_ENC_UNSUPPORTED_SAMPLINGRATE`.
/// - `SHORT_WINDOW` with the LD tables (no `paramShort`): the port returns
///   `AAC_ENC_UNSUPPORTED_SAMPLINGRATE`.
/// - A block type other than 0..=3: the port reads no widths, as if `*sfbCnt` were 0, and drops
///   a write outside `sfbOffset`.
pub fn fdk_aac_enc_init_sfb_table(
    sample_rate: i32,
    block_type: i32,
    granule_length: i32,
    sfb_offset: &mut [i32],
    sfb_cnt: &mut i32,
) -> AacEncoderError {
    let mut spec_start_offset: i32 = 0;
    let mut granule_length_window = granule_length;
    let mut sfb_width: Option<&[u8]> = None;

    // select table
    let sfb_info: &[SfbInfoTab] = match granule_length {
        1024 | 960 => &SFB_INFO_TAB,
        512 => &SFB_INFO_TAB_LD_512,
        480 => &SFB_INFO_TAB_LD_480,
        _ => return AAC_ENC_INVALID_FRAME_LENGTH,
    };

    let Some(info) = sfb_info.iter().find(|e| e.sample_rate == sample_rate) else {
        return AAC_ENC_UNSUPPORTED_SAMPLINGRATE;
    };
    match block_type {
        LONG_WINDOW | START_WINDOW | STOP_WINDOW => {
            sfb_width = Some(&info.param_long.sfb_width);
            *sfb_cnt = info.param_long.sfb_cnt as i32;
        }
        SHORT_WINDOW => {
            // (no paramShort in the LD tables)
            let Some(p) = info.param_short else {
                return AAC_ENC_UNSUPPORTED_SAMPLINGRATE;
            };
            sfb_width = Some(&p.sfb_width);
            *sfb_cnt = p.sfb_cnt as i32;
            granule_length_window /= TRANS_FAC;
        }
        _ => {}
    }

    // calc sfb offsets
    let mut i = 0;
    if let Some(w) = sfb_width {
        while i < *sfb_cnt {
            sfb_offset[i as usize] = spec_start_offset;
            spec_start_offset += w[i as usize] as i32;
            if spec_start_offset >= granule_length_window {
                i += 1;
                break;
            }
            i += 1;
        }
    }
    *sfb_cnt = fix_min(i, *sfb_cnt);
    // Bounded: a write outside sfbOffset is dropped.
    if let Some(o) = usize::try_from(*sfb_cnt).ok().and_then(|n| sfb_offset.get_mut(n)) {
        *o = fix_min(spec_start_offset, granule_length_window);
    }
    AAC_ENC_OK
}

/// `FDKaacEnc_BarcLineValue(noOfLines, fftLine, samplingFreq)`: calculates barc value for one
/// frequency line (q25).
pub fn fdk_aac_enc_barc_line_value(no_of_lines: i32, fft_line: i32, sampling_freq: i32) -> FixpDbl {
    let fourby3em4: FixpDbl = 0x45e7b273; // 4.0/3 * 0.0001 in q43
    let pzzz76: FixpDbl = 0x639d5e4a; // 0.00076 in q41
    let one3p3: FixpDbl = 0x35333333; // 13.3 in q26
    let threep5: FixpDbl = 0x1c000000; // 3.5 in q27
    let inv480: FixpDbl = 0x44444444; // 1/480 in q39

    // Theoritical maximum of center_freq (samp_freq*0.5) is 96khz * 0.5 = 48000
    // Theoritical maximum of x1 is 1.3333333e-4f * center_freq = 6.4, can keep in q28
    // Theoritical maximum of x2 is 0.00076f * center_freq = 36.48, can keep in q25

    let mut center_freq = fft_line.wrapping_mul(sampling_freq); // q11 or q8

    center_freq = match no_of_lines {
        1024 => center_freq.wrapping_shl(2), // q13
        128 => center_freq.wrapping_shl(5),  // q13
        512 => fft_line.wrapping_mul(sampling_freq).wrapping_shl(3), // q13
        480 => f_mult(center_freq, inv480).wrapping_shl(4), // q13
        _ => 0,
    };

    let x1 = f_mult(center_freq, fourby3em4); // q13 * q43 - (DFRACT_BITS-1) = q25
    let x2 = f_mult(center_freq, pzzz76).wrapping_shl(2); // q13 * q41 - (DFRACT_BITS-1) + 2 = q25

    let atan1 = fixp_atan(x1);
    let atan2 = fixp_atan(x2);

    // q25 (q26 * q30 - (DFRACT_BITS-1)) + q25 (q27 * q30 * q30)
    f_mult(one3p3, atan2).wrapping_add(f_mult(threep5, f_mult(atan1, atan1)))
}

/// `PCM_QUANT_NOISE`: `FDKpow(10.0f, - 20.f / 10.0f) * ABS_LOW * NORM_PCM_ENERGY *
/// FDKpow(2,PCM_QUANT_THR_SCALE)`
const PCM_QUANT_NOISE: FixpDbl = 0x00547062;

/// `FDKaacEnc_InitMinPCMResolution(numPb, pbOffset, sfbPCMquantThreshold)`: do not consider
/// energies below a certain input signal level, i.e. of -96dB or 1 bit at 16 bit PCM
/// resolution, might need to be configurable to e.g. 24 bit PCM Input or a lower resolution for
/// low bit rates.
pub fn fdk_aac_enc_init_min_pcm_resolution(
    num_pb: i32,
    pb_offset: &[i32],
    sfb_pcm_quant_threshold: &mut [FixpDbl],
) {
    for i in 0..num_pb.max(0) as usize {
        sfb_pcm_quant_threshold[i] =
            pb_offset[i + 1].wrapping_sub(pb_offset[i]).wrapping_mul(PCM_QUANT_NOISE);
    }
}

/// `getMaskFactor(dbVal_fix, dbVal_e, ten_fix, ten_e)`: `10^(-dbVal)` saturated to
/// `MAXVAL_DBL`.
pub fn get_mask_factor(
    db_val_fix: FixpDbl,
    db_val_e: i32,
    ten_fix: FixpDbl,
    ten_e: i32,
) -> FixpDbl {
    let mut q_msk = 0;

    let mut mask_factor = f_pow(
        ten_fix,
        DFRACT_BITS - 1 - ten_e,
        db_val_fix.wrapping_neg(),
        DFRACT_BITS - 1 - db_val_e,
        &mut q_msk,
    );
    q_msk = fix_min(DFRACT_BITS - 1, fix_max(-(DFRACT_BITS - 1), q_msk));

    if (q_msk > 0) && (mask_factor > MAXVAL_DBL >> q_msk) {
        mask_factor = MAXVAL_DBL;
    } else {
        mask_factor = scale_value(mask_factor, q_msk);
    }

    mask_factor
}

/// `FDKaacEnc_initSpreading(numPb, pbBarcValue, pbMaskLoFactor, pbMaskHiFactor,
/// pbMaskLoFactorSprEn, pbMaskHiFactorSprEn, bitrate, blockType)`
pub fn fdk_aac_enc_init_spreading(
    num_pb: i32,
    pb_barc_value: &[FixpDbl],
    pb_mask_lo_factor: &mut [FixpDbl],
    pb_mask_hi_factor: &mut [FixpDbl],
    pb_mask_lo_factor_spr_en: &mut [FixpDbl],
    pb_mask_hi_factor_spr_en: &mut [FixpDbl],
    bitrate: i32,
    block_type: i32,
) {
    let mask_high: FixpDbl = 0x30000000; // 1.5 in q29
    let mask_low: FixpDbl = 0x60000000; // 3.0 in q29
    let mask_low_spr_en_long: FixpDbl = 0x60000000; // 3.0 in q29
    let mask_high_spr_en_long: FixpDbl = 0x40000000; // 2.0 in q29
    let mask_high_spr_en_long_low_br: FixpDbl = 0x30000000; // 1.5 in q29
    let mask_low_spr_en_short: FixpDbl = 0x40000000; // 2.0 in q29
    let mask_high_spr_en_short: FixpDbl = 0x30000000; // 1.5 in q29
    let ten: FixpDbl = 0x50000000; // 10.0 in q27

    let mask_low_spr_en;
    let mask_high_spr_en;
    if block_type != SHORT_WINDOW {
        mask_low_spr_en = mask_low_spr_en_long;
        mask_high_spr_en =
            if bitrate > 20000 { mask_high_spr_en_long } else { mask_high_spr_en_long_low_br };
    } else {
        mask_low_spr_en = mask_low_spr_en_short;
        mask_high_spr_en = mask_high_spr_en_short;
    }

    for i in 0..num_pb.max(0) as usize {
        if i > 0 {
            let d = pb_barc_value[i].wrapping_sub(pb_barc_value[i - 1]);
            pb_mask_hi_factor[i] = get_mask_factor(f_mult(mask_high, d), 23, ten, 27);

            pb_mask_lo_factor[i - 1] = get_mask_factor(f_mult(mask_low, d), 23, ten, 27);

            pb_mask_hi_factor_spr_en[i] = get_mask_factor(f_mult(mask_high_spr_en, d), 23, ten, 27);

            pb_mask_lo_factor_spr_en[i - 1] =
                get_mask_factor(f_mult(mask_low_spr_en, d), 23, ten, 27);
        } else {
            let last = (num_pb - 1) as usize;
            pb_mask_hi_factor[i] = 0;
            pb_mask_lo_factor[last] = 0;
            pb_mask_hi_factor_spr_en[i] = 0;
            pb_mask_lo_factor_spr_en[last] = 0;
        }
    }
}

/// `FDKaacEnc_initBarcValues(numPb, pbOffset, numLines, samplingFrequency, pbBval)`
pub fn fdk_aac_enc_init_barc_values(
    num_pb: i32,
    pb_offset: &[i32],
    num_lines: i32,
    sampling_frequency: i32,
    pb_bval: &mut [FixpDbl],
) {
    let max_barc: FixpDbl = 0x30000000; // 24.0 in q25

    for i in 0..num_pb.max(0) as usize {
        let v1 = fdk_aac_enc_barc_line_value(num_lines, pb_offset[i], sampling_frequency);
        let v2 = fdk_aac_enc_barc_line_value(num_lines, pb_offset[i + 1], sampling_frequency);
        let cur_bark = (v1 >> 1) + (v2 >> 1);
        pb_bval[i] = fix_min(cur_bark, max_barc);
    }
}

/// `FDKaacEnc_initMinSnr(bitrate, samplerate, numLines, sfbOffset, sfbActive, blockType,
/// sfbMinSnrLdData)`
pub fn fdk_aac_enc_init_min_snr(
    bitrate: i32,
    samplerate: i32,
    num_lines: i32,
    sfb_offset: &[i32],
    sfb_active: i32,
    block_type: i32,
    sfb_min_snr_ld_data: &mut [FixpDbl],
) {
    // Fix conversion variables
    let mut qbfac = 0;
    let mut qperwin = 0;
    let mut qdiv = 0;
    let mut qtmp = 0;

    let max_barc: FixpDbl = 0x30000000; // 24.0 in q25
    let max_barcp1: FixpDbl = 0x32000000; // 25.0 in q25
    let bits2pefac: FixpDbl = 0x4b851eb8; // 1.18 in q30
    let pers2p4: FixpDbl = 0x624dd2f2; // 0.024 in q36
    let onep5: FixpDbl = 0x60000000; // 1.5 in q30
    let max_snr: FixpDbl = 0x33333333; // 0.8 in q30
    let min_snr: FixpDbl = 0x003126e9; // 0.003 in q30

    // relative number of active barks
    let barc_factor = f_div_norm(
        fix_min(
            fdk_aac_enc_barc_line_value(num_lines, sfb_offset[sfb_active as usize], samplerate),
            max_barc,
        ),
        max_barcp1,
        &mut qbfac,
    );

    qbfac = DFRACT_BITS - 1 - qbfac;

    let mut pe_per_window = f_div_norm(bitrate, samplerate, &mut qperwin);
    qperwin = DFRACT_BITS - 1 - qperwin;
    pe_per_window = f_mult(pe_per_window, bits2pefac);
    qperwin = qperwin + 30 - (DFRACT_BITS - 1);
    pe_per_window = f_mult(pe_per_window, pers2p4);
    qperwin = qperwin + 36 - (DFRACT_BITS - 1);

    match num_lines {
        1024 => qperwin -= 10,
        128 => qperwin -= 7,
        512 => qperwin -= 9,
        480 => {
            qperwin -= 9;
            pe_per_window = f_mult(pe_per_window, fl2fxconst_dbl((480.0f32 / 512.0f32) as f64));
        }
        _ => {}
    }

    // for short blocks it is assumed that more bits are available
    if block_type == SHORT_WINDOW {
        pe_per_window = f_mult(pe_per_window, onep5);
        qperwin = qperwin + 30 - (DFRACT_BITS - 1);
    }
    let pe_part_const = f_div_norm(pe_per_window, barc_factor, &mut qdiv);
    let qpeprt_const = qperwin - qbfac + DFRACT_BITS - 1 - qdiv;

    for sfb in 0..sfb_active.max(0) as usize {
        let barc_width = fdk_aac_enc_barc_line_value(num_lines, sfb_offset[sfb + 1], samplerate)
            .wrapping_sub(fdk_aac_enc_barc_line_value(num_lines, sfb_offset[sfb], samplerate));

        // adapt to sfb bands
        let mut pe_part = f_mult(pe_part_const, barc_width);
        let mut qpeprt = qpeprt_const + 25 - (DFRACT_BITS - 1);

        // pe -> snr calculation
        let sfb_width = sfb_offset[sfb + 1] - sfb_offset[sfb];
        pe_part = f_div_norm(pe_part, sfb_width, &mut qdiv);
        qpeprt += DFRACT_BITS - 1 - qdiv;

        let mut tmp = f2_pow(pe_part, DFRACT_BITS - 1 - qpeprt, &mut qtmp);
        qtmp = DFRACT_BITS - 1 - qtmp;

        // Subtract 1.5
        let mut qsnr = fix_min(qtmp, 30);
        tmp = tmp.wrapping_shr((qtmp - qsnr) as u32);

        let one_point5 = if (30 + 1 - qsnr) > (DFRACT_BITS - 1) {
            0
        } else {
            onep5.wrapping_shr((30 + 1 - qsnr) as u32)
        };

        let mut snr = (tmp >> 1).wrapping_sub(one_point5);
        qsnr -= 1;

        // max(snr, 1.0)
        let one_qsnr: FixpDbl = if qsnr > 0 { 1i32.wrapping_shl(qsnr as u32) } else { 0 };

        snr = fix_max(one_qsnr, snr);

        // 1/snr
        snr = f_div_norm(one_qsnr, snr, &mut qsnr);
        qsnr = DFRACT_BITS - 1 - qsnr;
        snr = if qsnr > 30 { snr.wrapping_shr((qsnr - 30) as u32) } else { snr };

        // upper limit is -1 dB
        snr = if snr > max_snr { max_snr } else { snr };

        // lower limit is -25 dB
        snr = if snr < min_snr { min_snr } else { snr };
        snr <<= 1;

        sfb_min_snr_ld_data[sfb] = calc_ld_data(snr);
    }
}

/// `FDKaacEnc_InitPsyConfiguration(bitrate, samplerate, bandwidth, blocktype, granuleLength,
/// useIS, useMS, psyConf, filterbank)`
///
/// `bitrate / bandwidth`: x86 division by zero gives the arm64 result (0) on every variant.
pub fn fdk_aac_enc_init_psy_configuration(
    bitrate: i32,
    samplerate: i32,
    bandwidth: i32,
    blocktype: i32,
    granule_length: i32,
    use_is: i32,
    use_ms: i32,
    psy_conf: &mut PsyConfiguration,
    filterbank: FbType,
) -> AacEncoderError {
    let mut sfb_barc_val = [0 as FixpDbl; MAX_SFB];
    let frame_length_long = granule_length;
    let frame_length_short = granule_length / TRANS_FAC;

    let downscale_factor = match granule_length {
        256 | 240 => 2,
        128 | 120 => 4,
        _ => 1,
    };

    *psy_conf = PsyConfiguration::default();
    psy_conf.granule_length = granule_length;
    psy_conf.filterbank = filterbank;

    psy_conf.allow_is = ((use_is != 0) && (div_i32(bitrate, bandwidth) < 5)) as i32;
    psy_conf.allow_ms = use_ms;

    // init sfb table
    let error_status = fdk_aac_enc_init_sfb_table(
        samplerate.wrapping_mul(downscale_factor),
        blocktype,
        granule_length.wrapping_mul(downscale_factor),
        &mut psy_conf.sfb_offset,
        &mut psy_conf.sfb_cnt,
    );

    if error_status != AAC_ENC_OK {
        return error_status;
    }

    let sfb_cnt = psy_conf.sfb_cnt;

    // calculate barc values for each pb
    fdk_aac_enc_init_barc_values(
        sfb_cnt,
        &psy_conf.sfb_offset,
        psy_conf.sfb_offset[sfb_cnt as usize],
        samplerate,
        &mut sfb_barc_val,
    );

    fdk_aac_enc_init_min_pcm_resolution(
        sfb_cnt,
        &psy_conf.sfb_offset,
        &mut psy_conf.sfb_pcm_quant_threshold,
    );

    // calculate spreading function
    fdk_aac_enc_init_spreading(
        sfb_cnt,
        &sfb_barc_val,
        &mut psy_conf.sfb_mask_low_factor,
        &mut psy_conf.sfb_mask_high_factor,
        &mut psy_conf.sfb_mask_low_factor_spr_en,
        &mut psy_conf.sfb_mask_high_factor_spr_en,
        bitrate,
        blocktype,
    );

    // init ratio

    psy_conf.max_allowed_increase_factor = 2; // integer
    psy_conf.min_remaining_threshold_factor = 0x0148; // FL2FXCONST_SGL(0.01f); fract

    psy_conf.clip_energy = 0x773593ff; // FL2FXCONST_DBL(1.0e9*NORM_PCM_ENERGY);

    if blocktype != SHORT_WINDOW {
        psy_conf.lowpass_line =
            div_i32(2i32.wrapping_mul(bandwidth).wrapping_mul(frame_length_long), samplerate);
        psy_conf.lowpass_line_lfe = LFE_LOWPASS_LINE;
    } else {
        psy_conf.lowpass_line =
            div_i32(2i32.wrapping_mul(bandwidth).wrapping_mul(frame_length_short), samplerate);
        psy_conf.lowpass_line_lfe = 0; // LFE only in lonf blocks
                                       // psyConf->clipEnergy /= (TRANS_FAC * TRANS_FAC);
        psy_conf.clip_energy >>= 6;
    }

    let mut sfb = 0;
    while sfb < sfb_cnt {
        if psy_conf.sfb_offset[sfb as usize] >= psy_conf.lowpass_line {
            break;
        }
        sfb += 1;
    }
    psy_conf.sfb_active = f_max(sfb, 1);

    sfb = 0;
    while sfb < sfb_cnt {
        if psy_conf.sfb_offset[sfb as usize] >= psy_conf.lowpass_line_lfe {
            break;
        }
        sfb += 1;
    }
    psy_conf.sfb_active_lfe = sfb;
    psy_conf.sfb_active = f_max(psy_conf.sfb_active, psy_conf.sfb_active_lfe);

    // calculate minSnr
    fdk_aac_enc_init_min_snr(
        bitrate,
        samplerate.wrapping_mul(downscale_factor),
        psy_conf.sfb_offset[sfb_cnt as usize],
        &psy_conf.sfb_offset,
        psy_conf.sfb_active,
        blocktype,
        &mut psy_conf.sfb_min_snr_ld_data,
    );

    AAC_ENC_OK
}
