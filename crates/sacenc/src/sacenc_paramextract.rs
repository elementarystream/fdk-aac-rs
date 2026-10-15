// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_paramextract.h, libSACenc/src/sacenc_paramextract.cpp.

//! Port of `sacenc_paramextract.h` and `sacenc_paramextract.cpp`: the parameter extraction
//! (CLD and ICC) of a two-to-one (TTO) box.
//!
//! # Pointers
//!
//! `TTO_BOX` keeps pointers to its quantization tables (selected by the coarse quantization
//! flags, which the port keeps instead), to the table of the imaginary signs (one table) and to
//! the parameter band to hybrid band offsets of the encoder. The offsets are passed to
//! [`fdk_sacenc_apply_tto_box`] by the caller, which reads the encoder's array through the
//! pointer at the time of the call, as the C code does.
//!
//! # Variants
//!
//! The code is identical in the `aarch64` and `x86` builds; the results differ through the
//! primitives (`fMult`, `fMultDiv2`, `CalcLdData`, `fDivNormHighPrec`) and the vector
//! functions.

use fdk_aac_rs_fdk::common_fix::*;

use crate::sacenc_const::NUM_QMF_BANDS;
use crate::sacenc_lib::*;
use crate::sacenc_vectorfunctions::*;

mod tables;
pub use tables::*;

/// `MAX_CLD_QUANT_FINE`
pub const MAX_CLD_QUANT_FINE: u8 = 31;
/// `MAX_CLD_QUANT_COARSE`
pub const MAX_CLD_QUANT_COARSE: u8 = 15;
/// `OFFSET_CLD_QUANT_COARSE`
pub const OFFSET_CLD_QUANT_COARSE: u8 = 7;
/// `OFFSET_CLD_QUANT_FINE`
pub const OFFSET_CLD_QUANT_FINE: u8 = 15;

/// `MAX_ICC_QUANT_COARSE`
pub const MAX_ICC_QUANT_COARSE: u8 = 4;
/// `MAX_ICC_QUANT_FINE`
pub const MAX_ICC_QUANT_FINE: u8 = 8;
/// `OFFSET_ICC_QUANT_COARSE`
pub const OFFSET_ICC_QUANT_COARSE: u8 = 0;
/// `OFFSET_ICC_QUANT_FINE`
pub const OFFSET_ICC_QUANT_FINE: u8 = 0;

/// `MAX_NUM_PARAM_BANDS`
pub const MAX_NUM_PARAM_BANDS: usize = 28;

/// `NUM_MAPPED_HYBRID_BANDS`
pub const NUM_MAPPED_HYBRID_BANDS: usize = 16;

/// `LOG10_2_10`: `10.0f * log10(2.f)`
const LOG10_2_10: f32 = 3.01029995664f32;
/// `SCALE_CLDE_SF`: maxVal in Quant tab is +/- 50
const SCALE_CLDE_SF: i32 = 7;

/// `BOX_SUBBAND_CONFIG`
pub type BoxSubbandConfig = i32;
pub const BOX_SUBBANDS_INVALID: BoxSubbandConfig = 0;
pub const BOX_SUBBANDS_4: BoxSubbandConfig = 4;
pub const BOX_SUBBANDS_5: BoxSubbandConfig = 5;
pub const BOX_SUBBANDS_7: BoxSubbandConfig = 7;
pub const BOX_SUBBANDS_9: BoxSubbandConfig = 9;
pub const BOX_SUBBANDS_12: BoxSubbandConfig = 12;
pub const BOX_SUBBANDS_15: BoxSubbandConfig = 15;
pub const BOX_SUBBANDS_23: BoxSubbandConfig = 23;

/// `BOX_QUANTMODE`
pub type BoxQuantMode = i32;
pub const BOX_QUANTMODE_INVALID: BoxQuantMode = -1;
pub const BOX_QUANTMODE_FINE: BoxQuantMode = 0;
pub const BOX_QUANTMODE_EBQ1: BoxQuantMode = 1;
pub const BOX_QUANTMODE_EBQ2: BoxQuantMode = 2;
pub const BOX_QUANTMODE_RESERVED3: BoxQuantMode = 3;
pub const BOX_QUANTMODE_RESERVED4: BoxQuantMode = 4;
pub const BOX_QUANTMODE_RESERVED5: BoxQuantMode = 5;
pub const BOX_QUANTMODE_RESERVED6: BoxQuantMode = 6;
pub const BOX_QUANTMODE_RESERVED7: BoxQuantMode = 7;

/// `TTO_BOX_CONFIG`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TtoBoxConfig {
    pub b_use_coarse_quant_cld: u8,
    pub b_use_coarse_quant_icc: u8,
    pub b_use_coherence_icc_only: u8,

    pub subband_config: BoxSubbandConfig,
    pub box_quant_mode: BoxQuantMode,

    pub n_hybrid_bands_max: u8,

    pub b_frame_keep: u8,
}

/// `TTO_BOX` (`struct T_TTO_BOX`). The table pointers are represented by the coarse
/// quantization flags (see the module documentation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TtoBox {
    pub p_cld_fdk: [FixpDbl; MAX_NUM_PARAM_BANDS],
    pub p_icc_fdk: [FixpDbl; MAX_NUM_PARAM_BANDS],
    pub p_cld_quant_fdk: [FixpDbl; MAX_NUM_PARAM_BANDS],

    /// `pIccQuantTable__FDK`, `pCldQuantTableDec__FDK`, `pCldQuantTableEnc__FDK` are set
    /// (non-`NULL`), selected by `b_use_coarse_quant_icc`/`b_use_coarse_quant_cld`.
    pub tables_set: bool,

    pub p_cld_eb_q_idx: [i8; MAX_NUM_PARAM_BANDS],
    pub p_icc_downmix_idx: [i8; MAX_NUM_PARAM_BANDS],

    /// `pParameterBand2HybridBandOffset` is set (non-`NULL`).
    pub p_parameter_band2_hybrid_band_offset_set: bool,
    /// `pSubbandImagSign` is set (non-`NULL`).
    pub p_subband_imag_sign_set: bool,
    pub n_hybrid_bands_max: u8,
    pub n_parameter_bands: u8,
    pub b_frame_keep: u8,

    pub icc_correlation_coherence_border: u8,
    pub box_quant_mode: BoxQuantMode,

    pub n_icc_quant_steps: u8,
    pub n_icc_quant_offset: u8,

    pub n_cld_quant_steps: u8,
    pub n_cld_quant_offset: u8,

    pub b_use_coarse_quant_cld: u8,
    pub b_use_coarse_quant_icc: u8,
}

/// `HANDLE_TTO_BOX`
pub type HandleTtoBox = Option<Box<TtoBox>>;

/// `BOX_SUBBAND_SETUP`
#[derive(Clone, Copy, Debug)]
pub struct BoxSubbandSetup {
    pub subband_config: BoxSubbandConfig,
    pub n_parameter_bands: u8,
    pub p_subband2_parameter_index_ld: &'static [u8; NUM_QMF_BANDS],
    pub icc_correlation_coherence_border: u8,
}

/// `boxSubbandSetup`, built from its dump [`BOX_SUBBAND_SETUP_RAW`].
pub static BOX_SUBBAND_SETUP: [BoxSubbandSetup; 7] = box_subband_setup();

/// The `subband2Parameter*_Ld` tables in the order of the table index of
/// [`BOX_SUBBAND_SETUP_RAW`]. Not a table of the C++ code: the order is the one in which the
/// prelude of the dump (`sacenc_paramextract/gen_tables.py`) maps the pointer
/// `pSubband2ParameterIndexLd` to an index; the cref test `tables` checks it.
pub static SUBBAND2_PARAMETER_TABLES: [&[u8; NUM_QMF_BANDS]; 7] = [
    &SUBBAND2_PARAMETER4_LD,
    &SUBBAND2_PARAMETER5_LD,
    &SUBBAND2_PARAMETER7_LD,
    &SUBBAND2_PARAMETER9_LD,
    &SUBBAND2_PARAMETER12_LD,
    &SUBBAND2_PARAMETER15_LD,
    &SUBBAND2_PARAMETER23_LD,
];

const fn box_subband_setup() -> [BoxSubbandSetup; 7] {
    let mut v = [BoxSubbandSetup {
        subband_config: 0,
        n_parameter_bands: 0,
        p_subband2_parameter_index_ld: SUBBAND2_PARAMETER_TABLES[0],
        icc_correlation_coherence_border: 0,
    }; 7];
    let mut i = 0;
    while i < 7 {
        let r = &BOX_SUBBAND_SETUP_RAW;
        v[i] = BoxSubbandSetup {
            subband_config: r[4 * i],
            n_parameter_bands: r[4 * i + 1] as u8,
            p_subband2_parameter_index_ld: SUBBAND2_PARAMETER_TABLES[r[4 * i + 2] as usize],
            icc_correlation_coherence_border: r[4 * i + 3] as u8,
        };
        i += 1;
    }
    v
}

/// `fdk_sacenc_getCldQuantOffset(bUseCoarseQuant)`
#[inline]
pub fn fdk_sacenc_get_cld_quant_offset(b_use_coarse_quant: i32) -> u8 {
    if b_use_coarse_quant != 0 {
        OFFSET_CLD_QUANT_COARSE
    } else {
        OFFSET_CLD_QUANT_FINE
    }
}
/// `fdk_sacenc_getIccQuantOffset(bUseCoarseQuant)`
#[inline]
pub fn fdk_sacenc_get_icc_quant_offset(b_use_coarse_quant: i32) -> u8 {
    if b_use_coarse_quant != 0 {
        OFFSET_ICC_QUANT_COARSE
    } else {
        OFFSET_ICC_QUANT_FINE
    }
}
/// `fdk_sacenc_getNumberCldQuantLevels(bUseCoarseQuant)`
#[inline]
pub fn fdk_sacenc_get_number_cld_quant_levels(b_use_coarse_quant: i32) -> u8 {
    if b_use_coarse_quant != 0 {
        MAX_CLD_QUANT_COARSE
    } else {
        MAX_CLD_QUANT_FINE
    }
}
/// `fdk_sacenc_getNumberIccQuantLevels(bUseCoarseQuant)`
#[inline]
pub fn fdk_sacenc_get_number_icc_quant_levels(b_use_coarse_quant: i32) -> u8 {
    if b_use_coarse_quant != 0 {
        MAX_ICC_QUANT_COARSE
    } else {
        MAX_ICC_QUANT_FINE
    }
}

impl TtoBox {
    /// `pIccQuantTable__FDK`
    pub fn icc_quant_table(&self) -> &'static [FixpDbl] {
        if self.b_use_coarse_quant_icc != 0 {
            &ICC_QUANT_TABLE_COARSE_FDK
        } else {
            &ICC_QUANT_TABLE_FINE_FDK
        }
    }
    /// `pCldQuantTableDec__FDK`
    pub fn cld_quant_table_dec(&self) -> &'static [FixpDbl] {
        if self.b_use_coarse_quant_cld != 0 {
            &CLD_QUANT_TABLE_COARSE_DEC_FDK
        } else {
            &CLD_QUANT_TABLE_FINE_DEC_FDK
        }
    }
    /// `pCldQuantTableEnc__FDK`
    pub fn cld_quant_table_enc(&self) -> &'static [FixpDbl] {
        if self.b_use_coarse_quant_cld != 0 {
            &CLD_QUANT_TABLE_COARSE_ENC_FDK
        } else {
            &CLD_QUANT_TABLE_FINE_ENC_FDK
        }
    }
}

/// `getBoxSubbandSetup(subbandConfig)` (static in C)
pub fn get_box_subband_setup(subband_config: BoxSubbandConfig) -> Option<&'static BoxSubbandSetup> {
    BOX_SUBBAND_SETUP.iter().find(|s| s.subband_config == subband_config)
}

/// `ApplyBBCuesFDK(pData, nParamBands)` (static in C): replaces the data by its mean.
pub fn apply_bb_cues_fdk(p_data: &mut [FixpDbl], n_param_bands: i32) {
    let mut s = 0;
    let inv_param_bands = f_div_norm_high_prec(1, n_param_bands, &mut s);
    s = -s;

    let shr = |v: FixpDbl| v.wrapping_shr(s as u32);
    let mut tmp = shr(f_mult(p_data[0], inv_param_bands));
    for i in 1..n_param_bands.max(0) as usize {
        tmp = tmp.wrapping_add(shr(f_mult(p_data[i], inv_param_bands)));
    }

    for v in p_data.iter_mut().take(n_param_bands.max(0) as usize) {
        *v = tmp;
    }
}

/// `getNumberParameterBands(subbandConfig)` (static in C)
pub fn get_number_parameter_bands(subband_config: BoxSubbandConfig) -> i32 {
    get_box_subband_setup(subband_config).map_or(0, |s| s.n_parameter_bands as i32)
}

/// `getSubband2ParameterIndex(subbandConfig)` (static in C)
pub fn get_subband2_parameter_index(
    subband_config: BoxSubbandConfig,
) -> Option<&'static [u8; NUM_QMF_BANDS]> {
    get_box_subband_setup(subband_config).map(|s| s.p_subband2_parameter_index_ld)
}

/// `getBoxSubbandSetup(subbandConfig)->...` as compiled in
/// `fdk_sacenc_calcParameterBand2HybridBandOffset`, which dereferences the result without a
/// check: both builds assume that the configuration is found.
///
/// - arm64 selects the entry with a chain of comparisons (`csel`): above 8, 15 gives the
///   15-band and any other value above 14 the 23-band entry, 9 the 9-band and 10..=14 the
///   12-band entry; up to 8, 4 and 5 give their entries and any other value the 7-band entry.
/// - x86 has no defined result for values outside of 4..=23.
///
/// The port uses the arm64 selection on both variants (a deviation of the x86 variant).
fn calc_box_subband_setup(subband_config: BoxSubbandConfig) -> &'static BoxSubbandSetup {
    let k = if subband_config > 8 {
        if subband_config > 14 {
            if subband_config == 15 {
                5
            } else {
                6
            }
        } else if subband_config == 9 {
            3
        } else {
            4
        }
    } else if subband_config == 4 {
        0
    } else if subband_config == 5 {
        1
    } else {
        2
    };
    &BOX_SUBBAND_SETUP[k]
}

/// `fdk_sacenc_calcParameterBand2HybridBandOffset(subbandConfig, nHybridBands,
/// pParameterBand2HybridBandOffset)`. For a configuration without a setup entry see
/// `calc_box_subband_setup()`.
pub fn fdk_sacenc_calc_parameter_band2_hybrid_band_offset(
    subband_config: BoxSubbandConfig,
    n_hybrid_bands: i32,
    p_parameter_band2_hybrid_band_offset: &mut [u8],
) {
    let setup = calc_box_subband_setup(subband_config);
    let p_subband2_parameter_index = setup.p_subband2_parameter_index_ld;

    let mut pb = 0usize;
    let mut i = 0i32;
    while i < n_hybrid_bands - 1 {
        if p_subband2_parameter_index[(i + 1) as usize] != p_subband2_parameter_index[i as usize] {
            p_parameter_band2_hybrid_band_offset[pb] = (i + 1) as u8;
            pb += 1;
        }
        i += 1;
    }
    p_parameter_band2_hybrid_band_offset[pb] = (i + 1) as u8;
}

/// `fdk_sacenc_getSubbandImagSign()`
pub fn fdk_sacenc_get_subband_imag_sign() -> &'static [i32; NUM_QMF_BANDS] {
    &SUBBAND_IMAG_SIGN_LD
}

/// `getIccCorrelationCoherenceBorder(subbandConfig, bUseCoherenceOnly)` (static in C)
pub fn get_icc_correlation_coherence_border(
    subband_config: BoxSubbandConfig,
    b_use_coherence_only: i32,
) -> i32 {
    match get_box_subband_setup(subband_config) {
        None => 0,
        Some(s) => {
            if b_use_coherence_only != 0 {
                0
            } else {
                s.icc_correlation_coherence_border as i32
            }
        }
    }
}

/// `fdk_sacenc_createTtoBox(hTtoBox)`
pub fn fdk_sacenc_create_tto_box(h_tto_box: Option<&mut HandleTtoBox>) -> FdkSacencError {
    match h_tto_box {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            *h = Some(Box::default());
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_initTtoBox(hTtoBox, ttoBoxConfig, pParameterBand2HybridBandOffset)`:
/// `has_offsets` tells whether the offset pointer is non-`NULL`.
pub fn fdk_sacenc_init_tto_box(
    h_tto_box: &mut TtoBox,
    tto_box_config: &TtoBoxConfig,
    has_offsets: bool,
) -> FdkSacencError {
    if !has_offsets {
        return SACENC_INVALID_HANDLE;
    }
    *h_tto_box = TtoBox::default();

    h_tto_box.b_use_coarse_quant_cld = tto_box_config.b_use_coarse_quant_cld;
    h_tto_box.b_use_coarse_quant_icc = tto_box_config.b_use_coarse_quant_icc;
    h_tto_box.box_quant_mode = tto_box_config.box_quant_mode;
    h_tto_box.icc_correlation_coherence_border = get_icc_correlation_coherence_border(
        tto_box_config.subband_config,
        tto_box_config.b_use_coherence_icc_only as i32,
    ) as u8;
    h_tto_box.n_hybrid_bands_max = tto_box_config.n_hybrid_bands_max;
    h_tto_box.n_parameter_bands = get_number_parameter_bands(tto_box_config.subband_config) as u8;
    h_tto_box.b_frame_keep = tto_box_config.b_frame_keep;

    h_tto_box.n_icc_quant_steps =
        fdk_sacenc_get_number_icc_quant_levels(h_tto_box.b_use_coarse_quant_icc as i32);
    h_tto_box.n_icc_quant_offset =
        fdk_sacenc_get_icc_quant_offset(h_tto_box.b_use_coarse_quant_icc as i32);

    h_tto_box.tables_set = true;

    h_tto_box.n_cld_quant_steps =
        fdk_sacenc_get_number_cld_quant_levels(h_tto_box.b_use_coarse_quant_cld as i32);
    h_tto_box.n_cld_quant_offset =
        fdk_sacenc_get_cld_quant_offset(h_tto_box.b_use_coarse_quant_cld as i32);

    // sanity
    h_tto_box.p_parameter_band2_hybrid_band_offset_set = true;
    h_tto_box.p_subband_imag_sign_set = true;

    if (h_tto_box.box_quant_mode != BOX_QUANTMODE_FINE)
        && (h_tto_box.box_quant_mode != BOX_QUANTMODE_EBQ1)
        && (h_tto_box.box_quant_mode != BOX_QUANTMODE_EBQ2)
    {
        return SACENC_INIT_ERROR;
    }
    SACENC_OK
}

/// `fdk_sacenc_destroyTtoBox(hTtoBox)`
pub fn fdk_sacenc_destroy_tto_box(h_tto_box: &mut HandleTtoBox) -> FdkSacencError {
    *h_tto_box = None;
    SACENC_OK
}

/// `calculateIccFDK(nParamBand, correlationCoherenceBorder, pPwr1, pPwr2, pProdReal,
/// pProdImag, pIcc)` (static in C): correlation below the border, coherence above it.
pub fn calculate_icc_fdk(
    n_param_band: i32,
    correlation_coherence_border: i32,
    p_pwr1: &[FixpDbl],
    p_pwr2: &[FixpDbl],
    p_prod_real: &[FixpDbl],
    p_prod_imag: &[FixpDbl],
    p_icc: &mut [FixpDbl],
) -> FdkSacencError {
    // sanity check border
    if correlation_coherence_border > n_param_band {
        return SACENC_INVALID_CONFIG;
    }
    // correlation
    fdk_calc_correlation_vec(p_icc, p_prod_real, p_pwr1, p_pwr2, correlation_coherence_border);

    // coherence
    let b = correlation_coherence_border.max(0) as usize;
    calc_coherence_vec(
        &mut p_icc[b..],
        &p_prod_real[b..],
        &p_prod_imag[b..],
        &p_pwr1[b..],
        &p_pwr2[b..],
        0,
        0,
        n_param_band - correlation_coherence_border,
    );
    SACENC_OK
}

/// `QuantizeCoefFDK(input, nBands, quantTable, idxOffset, nQuantSteps, quantOut)` (static in
/// C): the nearest table entry by binary search.
pub fn quantize_coef_fdk(
    input: &[FixpDbl],
    n_bands: i32,
    quant_table: &[FixpDbl],
    idx_offset: i32,
    n_quant_steps: i32,
    quant_out: &mut [i8],
) {
    let reverse = quant_table[0] > quant_table[1];

    for band in 0..n_bands.max(0) as usize {
        let cur_val = input[band];

        let mut lower: i32 = 0;
        let mut upper: i32 = n_quant_steps - 1;

        if reverse {
            while upper - lower > 1 {
                let idx = (lower + upper) >> 1;
                let q_val = quant_table[idx as usize];
                if cur_val >= q_val {
                    upper = idx;
                } else {
                    lower = idx;
                }
            }

            if cur_val.wrapping_sub(quant_table[lower as usize])
                >= quant_table[upper as usize].wrapping_sub(cur_val)
            {
                quant_out[band] = (lower - idx_offset) as i8;
            } else {
                quant_out[band] = (upper - idx_offset) as i8;
            }
        } else {
            while upper - lower > 1 {
                let idx = (lower + upper) >> 1;
                let q_val = quant_table[idx as usize];
                if cur_val <= q_val {
                    upper = idx;
                } else {
                    lower = idx;
                }
            }

            if cur_val.wrapping_sub(quant_table[lower as usize])
                <= quant_table[upper as usize].wrapping_sub(cur_val)
            {
                quant_out[band] = (lower - idx_offset) as i8;
            } else {
                quant_out[band] = (upper - idx_offset) as i8;
            }
        }
    }
}

/// `deQuantizeCoefFDK(input, nBands, quantTable, idxOffset, dequantOut)` (static in C)
pub fn de_quantize_coef_fdk(
    input: &[i8],
    n_bands: i32,
    quant_table: &[FixpDbl],
    idx_offset: i32,
    dequant_out: &mut [FixpDbl],
) {
    for band in 0..n_bands.max(0) as usize {
        dequant_out[band] = quant_table[(input[band] as i32 + idx_offset) as usize];
    }
}

/// `CalculateCldFDK(pCld, pPwr1, pPwr2, scaleCh1, pbScaleCh1, scaleCh2, pbScaleCh2,
/// nParamBand)` (static in C)
pub fn calculate_cld_fdk(
    p_cld: &mut [FixpDbl],
    p_pwr1: &[FixpDbl],
    p_pwr2: &[FixpDbl],
    scale_ch1: i32,
    pb_scale_ch1: &[i32],
    scale_ch2: i32,
    pb_scale_ch2: &[i32],
    n_param_band: i32,
) {
    // consider SACENC_FLOAT_EPSILON in power calculation
    let max_pwr = fl2fxconst_dbl((30.0f32 / (1 << (LD_DATA_SHIFT + 1)) as f32) as f64);
    let k = fl2fxconst_dbl((LOG10_2_10 / (1 << SCALE_CLDE_SF) as f32) as f64);

    for i in 0..n_param_band.max(0) as usize {
        // `(FIXP_DBL)(scaleCh1 + pbScaleCh1[i]) << (DFRACT_BITS - 1 - LD_DATA_SHIFT)`
        let mut ld_pwr1 = (calc_ld_data(p_pwr1[i]) >> 1).wrapping_add(
            scale_ch1
                .wrapping_add(pb_scale_ch1[i])
                .wrapping_shl((DFRACT_BITS - 1 - LD_DATA_SHIFT) as u32),
        );
        let mut ld_pwr2 = (calc_ld_data(p_pwr2[i]) >> 1).wrapping_add(
            scale_ch2
                .wrapping_add(pb_scale_ch2[i])
                .wrapping_shl((DFRACT_BITS - 1 - LD_DATA_SHIFT) as u32),
        );

        ld_pwr1 = fix_max(fix_min(ld_pwr1, max_pwr), -max_pwr);
        ld_pwr2 = fix_max(fix_min(ld_pwr2, max_pwr), -max_pwr);

        // ldPwr1 and ldPwr2 are scaled by LD_DATA_SHIFT and additional 1 bit; 1 bit scale by
        // fMultDiv2()
        let mut cld = f_mult_div2(k, ld_pwr1 - ld_pwr2);

        cld = fix_min(cld, MAXVAL_DBL >> (LD_DATA_SHIFT + 2));
        cld = fix_max(cld, MINVAL_DBL >> (LD_DATA_SHIFT + 2));
        p_cld[i] = cld << (LD_DATA_SHIFT + 2);
    }
}

/// `fdk_sacenc_applyTtoBox(hTtoBox, nTimeSlots, startTimeSlot, nHybridBands,
/// ppHybridData1__FDK, ppHybridData2__FDK, pIccIdx, pbIccQuantCoarse, pCldIdx,
/// pbCldQuantCoarse, bUseBBCues, scaleCh1, scaleCh2)` with the parameter band offsets that
/// the box points to (see the module documentation). The hybrid data are indexed
/// `[time slot][band]`.
pub fn fdk_sacenc_apply_tto_box<R: AsRef<[FixpDpk]>, S: AsRef<[FixpDpk]>>(
    h_tto_box: &mut TtoBox,
    p_parameter_band2_hybrid_band_offset: &[u8],
    n_time_slots: i32,
    start_time_slot: i32,
    n_hybrid_bands: i32,
    pp_hybrid_data1_fdk: &[R],
    pp_hybrid_data2_fdk: &[S],
    p_icc_idx: &mut [i8],
    pb_icc_quant_coarse: &mut u8,
    p_cld_idx: &mut [i8],
    pb_cld_quant_coarse: &mut u8,
    b_use_bb_cues: i32,
    scale_ch1: i32,
    scale_ch2: i32,
) -> FdkSacencError {
    let mut power_hybrid_data1_fdk = [0 as FixpDbl; MAX_NUM_PARAM_BANDS];
    let mut power_hybrid_data2_fdk = [0 as FixpDbl; MAX_NUM_PARAM_BANDS];
    let mut prod_hybrid_data_real_fdk = [0 as FixpDbl; MAX_NUM_PARAM_BANDS];
    let mut prod_hybrid_data_imag_fdk = [0 as FixpDbl; MAX_NUM_PARAM_BANDS];

    let mut icc_downmix_fdk = [0 as FixpDbl; MAX_NUM_PARAM_BANDS];
    let mut icc_downmix_quant_fdk = [0 as FixpDbl; MAX_NUM_PARAM_BANDS];
    let mut pb_scale_ch1 = [0i32; MAX_NUM_PARAM_BANDS];
    let mut pb_scale_ch2 = [0i32; MAX_NUM_PARAM_BANDS];

    let n_param_bands = h_tto_box.n_parameter_bands as i32;
    let b_use_eb_q = (h_tto_box.box_quant_mode == BOX_QUANTMODE_EBQ1)
        || (h_tto_box.box_quant_mode == BOX_QUANTMODE_EBQ2);

    // sanity check
    if (n_hybrid_bands < 0) || (n_hybrid_bands > h_tto_box.n_hybrid_bands_max as i32) {
        return SACENC_INVALID_CONFIG;
    }

    let mut out_scale = 0; // scalefactor will not be evaluated
    let in_scale = 5; // scale factor determined empirically

    // calculate the headroom of the hybrid data for each parameter band
    let offsets = p_parameter_band2_hybrid_band_offset;
    fdk_calc_pb_scale_factor(
        pp_hybrid_data1_fdk,
        offsets,
        &mut pb_scale_ch1,
        start_time_slot,
        n_time_slots,
        n_param_bands,
    );
    fdk_calc_pb_scale_factor(
        pp_hybrid_data2_fdk,
        offsets,
        &mut pb_scale_ch2,
        start_time_slot,
        n_time_slots,
        n_param_bands,
    );

    let mut j = 0i32;
    for pb in 0..n_param_bands.max(0) as usize {
        let (mut data1, mut data2): (FixpDbl, FixpDbl) = (0, 0);
        while j < offsets[pb] as i32 {
            data1 = data1.wrapping_add(sum_up_cplx_pow2_dim2(
                pp_hybrid_data1_fdk,
                SUM_UP_STATIC_SCALE,
                in_scale + pb_scale_ch1[pb],
                &mut out_scale,
                start_time_slot,
                n_time_slots,
                j,
                j + 1,
            ));
            data2 = data2.wrapping_add(sum_up_cplx_pow2_dim2(
                pp_hybrid_data2_fdk,
                SUM_UP_STATIC_SCALE,
                in_scale + pb_scale_ch2[pb],
                &mut out_scale,
                start_time_slot,
                n_time_slots,
                j,
                j + 1,
            ));
            j += 1;
        }
        power_hybrid_data1_fdk[pb] = data1;
        power_hybrid_data2_fdk[pb] = data2;
    }

    let imag_sign = fdk_sacenc_get_subband_imag_sign();
    let mut j = 0i32;
    for pb in 0..n_param_bands.max(0) as usize {
        let (mut data_real, mut data_imag): (FixpDbl, FixpDbl) = (0, 0);
        while j < offsets[pb] as i32 {
            let mut scalar_prod = FixpDpk::default();
            cplx_cplx_scalar_product(
                &mut scalar_prod,
                pp_hybrid_data1_fdk,
                pp_hybrid_data2_fdk,
                in_scale + pb_scale_ch1[pb],
                in_scale + pb_scale_ch2[pb],
                &mut out_scale,
                start_time_slot,
                n_time_slots,
                j,
                j + 1,
            );
            data_real = data_real.wrapping_add(scalar_prod.re);
            if imag_sign[j as usize] < 0 {
                data_imag = data_imag.wrapping_sub(scalar_prod.im);
            } else {
                data_imag = data_imag.wrapping_add(scalar_prod.im);
            }
            j += 1;
        }
        prod_hybrid_data_real_fdk[pb] = data_real;
        prod_hybrid_data_imag_fdk[pb] = data_imag;
    }

    let error = calculate_icc_fdk(
        n_param_bands,
        h_tto_box.icc_correlation_coherence_border as i32,
        &power_hybrid_data1_fdk,
        &power_hybrid_data2_fdk,
        &prod_hybrid_data_real_fdk,
        &prod_hybrid_data_imag_fdk,
        &mut h_tto_box.p_icc_fdk,
    );
    if error != SACENC_OK {
        return error;
    }

    // calculate correlation based Icc for downmix
    let error = calculate_icc_fdk(
        n_param_bands,
        n_param_bands,
        &power_hybrid_data1_fdk,
        &power_hybrid_data2_fdk,
        &prod_hybrid_data_real_fdk,
        &prod_hybrid_data_imag_fdk,
        &mut icc_downmix_fdk,
    );
    if error != SACENC_OK {
        return error;
    }

    if !b_use_eb_q {
        calculate_cld_fdk(
            &mut h_tto_box.p_cld_fdk,
            &power_hybrid_data1_fdk,
            &power_hybrid_data2_fdk,
            scale_ch1.wrapping_add(in_scale + 1),
            &pb_scale_ch1,
            scale_ch2.wrapping_add(in_scale + 1),
            &pb_scale_ch2,
            n_param_bands,
        );
    }

    if b_use_bb_cues != 0 {
        apply_bb_cues_fdk(&mut h_tto_box.p_cld_fdk, n_param_bands);
        apply_bb_cues_fdk(&mut h_tto_box.p_icc_fdk, n_param_bands);
    }

    // quantize/de-quantize icc
    let icc_table = h_tto_box.icc_quant_table();
    quantize_coef_fdk(
        &h_tto_box.p_icc_fdk,
        n_param_bands,
        icc_table,
        h_tto_box.n_icc_quant_offset as i32,
        h_tto_box.n_icc_quant_steps as i32,
        p_icc_idx,
    );
    quantize_coef_fdk(
        &icc_downmix_fdk,
        n_param_bands,
        icc_table,
        h_tto_box.n_icc_quant_offset as i32,
        h_tto_box.n_icc_quant_steps as i32,
        &mut h_tto_box.p_icc_downmix_idx,
    );
    de_quantize_coef_fdk(
        &h_tto_box.p_icc_downmix_idx,
        n_param_bands,
        icc_table,
        h_tto_box.n_icc_quant_offset as i32,
        &mut icc_downmix_quant_fdk,
    );

    *pb_icc_quant_coarse = h_tto_box.b_use_coarse_quant_icc;

    // quantize/de-quantize cld
    if !b_use_eb_q {
        quantize_coef_fdk(
            &h_tto_box.p_cld_fdk,
            n_param_bands,
            h_tto_box.cld_quant_table_enc(),
            h_tto_box.n_cld_quant_offset as i32,
            h_tto_box.n_cld_quant_steps as i32,
            p_cld_idx,
        );
        let dec = h_tto_box.cld_quant_table_dec();
        de_quantize_coef_fdk(
            p_cld_idx,
            n_param_bands,
            dec,
            h_tto_box.n_cld_quant_offset as i32,
            &mut h_tto_box.p_cld_quant_fdk,
        );
    } else {
        let n = n_param_bands.max(0) as usize;
        p_cld_idx[..n].copy_from_slice(&h_tto_box.p_cld_eb_q_idx[..n]);
    }
    *pb_cld_quant_coarse = h_tto_box.b_use_coarse_quant_cld;

    SACENC_OK
}

/// `fdk_sacenc_subband2ParamBand(boxSubbandConfig, nSubband)`
pub fn fdk_sacenc_subband2_param_band(box_subband_config: BoxSubbandConfig, n_subband: i32) -> i32 {
    match get_subband2_parameter_index(box_subband_config) {
        Some(t) if (n_subband > -1) && (n_subband < 64) => t[n_subband as usize] as i32,
        _ => -1,
    }
}
