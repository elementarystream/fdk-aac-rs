// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/src/drcGainDec_preprocess.h, libDRCdec/src/drcGainDec_preprocess.cpp.

//! Port of `drcGainDec_preprocess.h`: conversion of the decoded gain nodes to linear gains,
//! including the gain modifications (characteristics, scaling, offsets, ducking, limiter peak
//! target).
//!
//! The pointers of `NODE_MODIFICATION` to the modifications and characteristics are replaced by
//! copies of the pointed-to values, taken where C sets the pointers; the data does not change
//! while `prepareDrcGain` runs. A characteristic of the CICP tables (`CUSTOM_DRC_CHAR_SIGMOID` or
//! `CUSTOM_DRC_CHAR_NODES`, which C casts to `CUSTOM_DRC_CHAR*`) is copied into a cleared
//! `CUSTOM_DRC_CHAR`; it is only read through the member that matches its format, as in C.
//!
//! The characteristic functions (`_getSlopeSign`, `_compressorIO_*`, `_mapGain`) are file-static
//! in C; they are public here for the differential tests.

use fdk_aac_rs_fdk::common_fix::{f_max, f_mult, f_mult_div2, fl2fxconst_dbl, FixpDbl, FixpSgl};
use fdk_aac_rs_fdk::common_fix::{DFRACT_BITS, MAXVAL_DBL, MAXVAL_SGL};
use fdk_aac_rs_fdk::fixpoint_math::{
    f_add_norm, f_div_norm, f_div_norm_noexp, f_div_norm_signed, f_pow,
};
use fdk_aac_rs_fdk::scale::{saturate_left_shift, scale_value_saturate};

use crate::drc_dec_gain_decoder::*;
use crate::drc_dec_rom::*;
use crate::drc_dec_tools::*;
use crate::drc_dec_types::*;
use crate::drc_decoder::*;

/// `SLOPE_FACTOR_DB_TO_LINEAR`: `FL2FXCONST_DBL(0.1151f * (float)(1 << 3))`, ln(10) / 20
const SLOPE_FACTOR_DB_TO_LINEAR: FixpDbl = fl2fxconst_dbl((0.1151f32 * 8.0f32) as f64);

/// `(FIXP_DBL)((LONG)(val) << (DFRACT_BITS - FRACT_BITS))` (`FX_SGL2FX_DBL`) of an `INT`
/// expression (the macro does not truncate its argument to `FIXP_SGL`); the shift can overflow
/// (undefined, a plain `lsl`/`shl` in both builds).
#[inline]
fn sgl2dbl(v: i32) -> FixpDbl {
    v.wrapping_shl(16)
}

/// `NODE_MODIFICATION`
#[derive(Clone, Copy)]
struct NodeModification {
    drc_set_effect: i32,
    /// `pDMod`
    p_dmod: Option<DuckingModification>,
    /// `pGMod` (the 4 gain modifications of a channel group)
    p_gmod: Option<[GainModification; 4]>,
    drc_characteristic_present: i32,
    characteristic_format_source: [CharacteristicFormat; 2],
    /// `pCCharSource` (`None` for `NULL`)
    p_cchar_source: [Option<CustomDrcChar>; 2],
    characteristic_format_target: [CharacteristicFormat; 2],
    /// `pCCharTarget` (`None` for `NULL`)
    p_cchar_target: [Option<CustomDrcChar>; 2],
    slope_is_negative: i32,
    limiter_peak_target_present: i32,
    limiter_peak_target: FixpSgl,
    loudness_normalization_gain_db: FixpDbl,
    compress: FixpSgl,
    boost: FixpSgl,
}

impl NodeModification {
    /// The cleared structure (`FDKmemclear`).
    const ZERO: Self = Self {
        drc_set_effect: 0,
        p_dmod: None,
        p_gmod: None,
        drc_characteristic_present: 0,
        characteristic_format_source: [0; 2],
        p_cchar_source: [None; 2],
        characteristic_format_target: [0; 2],
        p_cchar_target: [None; 2],
        slope_is_negative: 0,
        limiter_peak_target_present: 0,
        limiter_peak_target: 0,
        loudness_normalization_gain_db: 0,
        compress: 0,
        boost: 0,
    };
}

/// A `CUSTOM_DRC_CHAR` holding a sigmoid characteristic of the CICP table.
fn cchar_from_sigmoid(s: &CustomDrcCharSigmoid) -> CustomDrcChar {
    let mut c = CustomDrcChar::ZERO;
    *c.sigmoid_mut() = *s;
    c
}

/// A `CUSTOM_DRC_CHAR` holding a nodes characteristic of the CICP table.
fn cchar_from_nodes(n: &CustomDrcCharNodes) -> CustomDrcChar {
    let mut c = CustomDrcChar::ZERO;
    *c.nodes_mut() = *n;
    c
}

/// `_getCicpCharacteristic(cicpCharacteristic, pCharacteristicFormat, pCCharSource)`
fn get_cicp_characteristic(
    cicp_characteristic: i32,
    p_characteristic_format: &mut [CharacteristicFormat; 2],
    p_cchar_source: &mut [Option<CustomDrcChar>; 2],
) -> DrcError {
    if (cicp_characteristic < 1) || (cicp_characteristic > 11) {
        return DE_NOT_OK;
    }

    if cicp_characteristic < 7 {
        /* sigmoid characteristic */
        let i = (cicp_characteristic - 1) as usize;
        p_characteristic_format[CS_LEFT as usize] = CF_SIGMOID;
        p_cchar_source[CS_LEFT as usize] = Some(cchar_from_sigmoid(&CICP_DRC_CHAR_SIGMOID_LEFT[i]));
        p_characteristic_format[CS_RIGHT as usize] = CF_SIGMOID;
        p_cchar_source[CS_RIGHT as usize] =
            Some(cchar_from_sigmoid(&CICP_DRC_CHAR_SIGMOID_RIGHT[i]));
    } else {
        /* nodes characteristic */
        let i = (cicp_characteristic - 7) as usize;
        p_characteristic_format[CS_LEFT as usize] = CF_NODES;
        p_cchar_source[CS_LEFT as usize] = Some(cchar_from_nodes(&CICP_DRC_CHAR_NODES_LEFT[i]));
        p_characteristic_format[CS_RIGHT as usize] = CF_NODES;
        p_cchar_source[CS_RIGHT as usize] = Some(cchar_from_nodes(&CICP_DRC_CHAR_NODES_RIGHT[i]));
    }
    DE_OK
}

/// `_getSign(in)`
fn get_sign(in_: i32) -> i32 {
    if in_ > 0 {
        return 1;
    }
    if in_ < 0 {
        return -1;
    }
    0
}

/// `_getSlopeSign(drcCharFormat, pCChar, pSlopeSign)`
pub fn get_slope_sign(
    drc_char_format: CharacteristicFormat,
    p_cchar: &CustomDrcChar,
    p_slope_sign: &mut i32,
) -> DrcError {
    if drc_char_format == CF_SIGMOID {
        *p_slope_sign = if p_cchar.sigmoid().flip_sign != 0 { 1 } else { -1 };
    } else {
        let nodes = p_cchar.nodes();
        let mut slope_sign = 0;
        // characteristicNodeCount is at most 4 (reader, CICP tables).
        for k in 0..(nodes.characteristic_node_count as usize).min(4) {
            let tmp_slope_sign = if nodes.node_level[k + 1] > nodes.node_level[k] {
                get_sign(nodes.node_gain[k + 1] as i32 - nodes.node_gain[k] as i32)
            } else {
                -get_sign(nodes.node_gain[k + 1] as i32 - nodes.node_gain[k] as i32)
            };
            if (slope_sign != 0 || tmp_slope_sign != 0) && (slope_sign == -tmp_slope_sign) {
                return DE_NOT_OK; /* DRC characteristic is not invertible */
            } else {
                slope_sign = tmp_slope_sign;
            }
        }
        *p_slope_sign = slope_sign;
    }
    DE_OK
}

/// `_isSlopeNegative(drcCharFormat, pCChar, pSlopeIsNegative)`
fn is_slope_negative(
    drc_char_format: &[CharacteristicFormat; 2],
    p_cchar: &[Option<CustomDrcChar>; 2],
    p_slope_is_negative: &mut i32,
) -> DrcError {
    let mut slope_sign = [0i32; 2];

    // The sources are set together with their formats (never NULL here).
    let l = p_cchar[CS_LEFT as usize].unwrap_or(CustomDrcChar::ZERO);
    let mut err = get_slope_sign(drc_char_format[CS_LEFT as usize], &l, &mut slope_sign[0]);
    if err != 0 {
        return err;
    }

    let r = p_cchar[CS_RIGHT as usize].unwrap_or(CustomDrcChar::ZERO);
    err = get_slope_sign(drc_char_format[CS_RIGHT as usize], &r, &mut slope_sign[1]);
    if err != 0 {
        return err;
    }

    if (slope_sign[0] != 0 || slope_sign[1] != 0) && (slope_sign[0] == -slope_sign[1]) {
        return DE_NOT_OK; /* DRC characteristic is not invertible */
    }

    *p_slope_is_negative = (slope_sign[0] < 0) as i32;
    DE_OK
}

/// `_prepareDrcCharacteristic(pDChar, pCoef, b, pNodeMod)`
fn prepare_drc_characteristic(
    p_dchar: &DrcCharacteristic,
    p_coef: &DrcCoefficientsUniDrc,
    b: usize,
    p_node_mod: &mut NodeModification,
) -> DrcError {
    p_node_mod.drc_characteristic_present = p_dchar.present as i32;
    if p_node_mod.drc_characteristic_present != 0 {
        if p_dchar.is_cicp == 1 {
            let err = get_cicp_characteristic(
                p_dchar.cicp_index as i32,
                &mut p_node_mod.characteristic_format_source,
                &mut p_node_mod.p_cchar_source,
            );
            if err != 0 {
                return err;
            }
        } else {
            // custom.left / custom.right are 4 bit values (reader), the arrays have 16 entries.
            let l = (p_dchar.custom_left() & 15) as usize;
            let r = (p_dchar.custom_right & 15) as usize;
            p_node_mod.characteristic_format_source[CS_LEFT as usize] =
                p_coef.characteristic_left_format[l] as CharacteristicFormat;
            p_node_mod.p_cchar_source[CS_LEFT as usize] =
                Some(p_coef.custom_characteristic_left[l]);
            p_node_mod.characteristic_format_source[CS_RIGHT as usize] =
                p_coef.characteristic_right_format[r] as CharacteristicFormat;
            p_node_mod.p_cchar_source[CS_RIGHT as usize] =
                Some(p_coef.custom_characteristic_right[r]);
        }
        let mut slope_is_negative = p_node_mod.slope_is_negative;
        let err = is_slope_negative(
            &p_node_mod.characteristic_format_source,
            &p_node_mod.p_cchar_source,
            &mut slope_is_negative,
        );
        p_node_mod.slope_is_negative = slope_is_negative;
        if err != 0 {
            return err;
        }

        if let Some(gmod) = p_node_mod.p_gmod {
            // b < 4: bandCountForChannelGroup is at most 4.
            let gm = gmod[b.min(3)];
            if gm.target_characteristic_left_present != 0 {
                let i = (gm.target_characteristic_left_index & 15) as usize;
                p_node_mod.characteristic_format_target[CS_LEFT as usize] =
                    p_coef.characteristic_left_format[i] as CharacteristicFormat;
                p_node_mod.p_cchar_target[CS_LEFT as usize] =
                    Some(p_coef.custom_characteristic_left[i]);
            }
            if gm.target_characteristic_right_present != 0 {
                let i = (gm.target_characteristic_right_index & 15) as usize;
                p_node_mod.characteristic_format_target[CS_RIGHT as usize] =
                    p_coef.characteristic_right_format[i] as CharacteristicFormat;
                p_node_mod.p_cchar_target[CS_RIGHT as usize] =
                    Some(p_coef.custom_characteristic_right[i]);
            }
        }
    }
    DE_OK
}

/// `_compressorIO_sigmoid_common(tmp, gainDbLimit, exp, inverse, out)`
fn compressor_io_sigmoid_common(
    tmp: FixpDbl,           /* e = 7 */
    gain_db_limit: FixpDbl, /* e = 6 */
    exp: FixpDbl,           /* e = 5 */
    inverse: i32,
    out: &mut FixpDbl, /* e = 7 */
) -> DrcError {
    let mut e_x = 0;
    let mut e_tmp1 = 0;
    let mut e_tmp2 = 0;
    let mut e_inv_exp = 0;
    let mut e_denom = 0;
    let mut e_out = 0;

    if exp < fl2fxconst_dbl(1.0 / 32.0) {
        return DE_NOT_OK;
    }

    /* x = tmp / gainDbLimit; */
    let x = f_div_norm_signed(tmp, gain_db_limit, &mut e_x);
    e_x += 7 - 6;
    if x < 0 {
        return DE_NOT_OK;
    }

    /* out = tmp / pow(1.0f +/- pow(x, exp), 1.0f/exp); */
    let mut tmp1 = f_pow(x, e_x, exp, 5, &mut e_tmp1);
    if inverse != 0 {
        tmp1 = tmp1.wrapping_neg();
    }
    let tmp2 = f_add_norm(fl2fxconst_dbl(1.0 / 2.0), 1, tmp1, e_tmp1, &mut e_tmp2);
    let inv_exp = f_div_norm(fl2fxconst_dbl(1.0 / 2.0), exp, &mut e_inv_exp);
    e_inv_exp += 1 - 5;
    if tmp2 < 0 {
        return DE_NOT_OK;
    }
    let denom = f_pow(tmp2, e_tmp2, inv_exp, e_inv_exp, &mut e_denom);
    *out = f_div_norm_signed(tmp, denom, &mut e_out);
    e_out += 7 - e_denom;
    *out = scale_value_saturate(*out, e_out - 7);
    DE_OK
}

/// `_compressorIO_sigmoid(pCChar, inLevelDb, outGainDb)`
pub fn compressor_io_sigmoid(
    p_cchar: &CustomDrcCharSigmoid,
    in_level_db: FixpDbl,      /* e = 7 */
    out_gain_db: &mut FixpDbl, /* e = 7 */
) -> DrcError {
    let exp = p_cchar.exp;
    let mut err = DE_OK;

    let mut tmp = f_mult_div2(
        (DRC_INPUT_LOUDNESS_TARGET >> 1).wrapping_sub(in_level_db >> 1),
        p_cchar.io_ratio,
    );
    tmp = saturate_left_shift(tmp, 2 + 1 + 1, DFRACT_BITS);
    if exp < MAXVAL_SGL as FixpSgl {
        /* x = tmp / gainDbLimit; */
        /* *outGainDb = tmp / pow(1.0f + pow(x, exp), 1.0f/exp); */
        err = compressor_io_sigmoid_common(
            tmp,
            sgl2dbl(p_cchar.gain as i32),
            sgl2dbl(exp as i32),
            0,
            out_gain_db,
        );
        if err != 0 {
            return err;
        }
    } else {
        *out_gain_db = tmp; /* scaling of outGainDb (7) is equal to scaling of tmp (7) */
    }
    if p_cchar.flip_sign == 1 {
        *out_gain_db = out_gain_db.wrapping_neg();
    }
    err
}

/// `_compressorIO_sigmoid_inverse(pCChar, gainDb, inLev)`
pub fn compressor_io_sigmoid_inverse(
    p_cchar: &CustomDrcCharSigmoid,
    gain_db: FixpSgl,
    in_lev: &mut FixpDbl,
) -> DrcError {
    let io_ratio = p_cchar.io_ratio;
    let exp = p_cchar.exp;
    let mut tmp = sgl2dbl(gain_db as i32);
    let mut e_out = 0;

    if p_cchar.flip_sign == 1 {
        tmp = tmp.wrapping_neg();
    }
    if exp < MAXVAL_SGL as FixpSgl {
        /* x = tmp / gainDbLimit; */
        /* tmp = tmp / pow(1.0f - pow(x, exp), 1.0f / exp); */
        let t = tmp;
        let err = compressor_io_sigmoid_common(
            t,
            sgl2dbl(p_cchar.gain as i32),
            sgl2dbl(exp as i32),
            1,
            &mut tmp,
        );
        if err != 0 {
            return err;
        }
    }
    if io_ratio == 0 {
        return DE_NOT_OK;
    }
    let mut tmp_out = f_div_norm_signed(tmp, sgl2dbl(io_ratio as i32), &mut e_out);
    e_out += 7 - 2;
    let e_in = e_out;
    tmp_out = f_add_norm(DRC_INPUT_LOUDNESS_TARGET, 7, tmp_out.wrapping_neg(), e_in, &mut e_out);
    *in_lev = scale_value_saturate(tmp_out, e_out - 7);

    DE_OK
}

/// `_compressorIO_nodes(pCChar, inLevelDb, outGainDb)`
pub fn compressor_io_nodes(
    p_cchar: &CustomDrcCharNodes,
    in_level_db: FixpDbl,      /* e = 7 */
    out_gain_db: &mut FixpDbl, /* e = 7 */
) -> DrcError {
    let node_level = &p_cchar.node_level;
    let node_gain = &p_cchar.node_gain;
    // characteristicNodeCount is at most 4 (reader, CICP tables).
    let count = (p_cchar.characteristic_node_count as usize).min(4);

    if in_level_db < DRC_INPUT_LOUDNESS_TARGET {
        for n in 0..count {
            if (in_level_db <= sgl2dbl(node_level[n] as i32))
                && (in_level_db > sgl2dbl(node_level[n + 1] as i32))
            {
                let w = f_div_norm_noexp(
                    in_level_db.wrapping_sub(sgl2dbl(node_level[n + 1] as i32)),
                    sgl2dbl(node_level[n] as i32 - node_level[n + 1] as i32),
                );
                *out_gain_db = f_mult(w, node_gain[n])
                    .wrapping_add(f_mult(MAXVAL_DBL.wrapping_sub(w), node_gain[n + 1]));
                /* *outGainDb = (w * nodeGain[n] + (1.0-w) * nodeGain[n+1]); */
                return DE_OK;
            }
        }
    } else {
        for n in 0..count {
            if (in_level_db >= sgl2dbl(node_level[n] as i32))
                && (in_level_db < sgl2dbl(node_level[n + 1] as i32))
            {
                let w = f_div_norm_noexp(
                    sgl2dbl(node_level[n + 1] as i32).wrapping_sub(in_level_db),
                    sgl2dbl(node_level[n + 1] as i32 - node_level[n] as i32),
                );
                *out_gain_db = f_mult(w, node_gain[n])
                    .wrapping_add(f_mult(MAXVAL_DBL.wrapping_sub(w), node_gain[n + 1]));
                /* *outGainDb = (w * nodeGain[n] + (1.0-w) * nodeGain[n+1]); */
                return DE_OK;
            }
        }
    }
    *out_gain_db = sgl2dbl(node_gain[p_cchar.characteristic_node_count.min(4) as usize] as i32);
    DE_OK
}

/// `_compressorIO_nodes_inverse(pCChar, gainDb, inLev)`
pub fn compressor_io_nodes_inverse(
    p_cchar: &CustomDrcCharNodes,
    gain_db: FixpSgl,     /* e = 7 */
    in_lev: &mut FixpDbl, /* e = 7 */
) -> DrcError {
    let mut gain_is_negative = 0;
    let node_level = &p_cchar.node_level;
    let node_gain = &p_cchar.node_gain;
    // characteristicNodeCount is at most 4 (reader, CICP tables).
    let node_count = (p_cchar.characteristic_node_count as usize).min(4);
    for k in 0..node_count {
        if p_cchar.node_gain[k + 1] < 0 {
            gain_is_negative = 1;
        }
    }
    if gain_is_negative == 1 {
        if gain_db <= node_gain[node_count] {
            *in_lev = sgl2dbl(node_level[node_count] as i32);
        } else {
            if gain_db >= 0 {
                *in_lev = DRC_INPUT_LOUDNESS_TARGET;
            } else {
                for n in 0..node_count {
                    if (gain_db <= node_gain[n]) && (gain_db > node_gain[n + 1]) {
                        let gain_delta = node_gain[n].wrapping_sub(node_gain[n + 1]);
                        if gain_delta == 0 {
                            *in_lev = sgl2dbl(node_level[n] as i32);
                            return DE_OK;
                        }
                        let w = f_div_norm_noexp(
                            sgl2dbl(gain_db.wrapping_sub(node_gain[n + 1]) as i32),
                            sgl2dbl(gain_delta as i32),
                        );
                        *in_lev = f_mult(w, node_level[n])
                            .wrapping_add(f_mult(MAXVAL_DBL.wrapping_sub(w), node_level[n + 1]));
                        /* *inLev = (w * nodeLevel[n] + (1.0-w) * nodeLevel[n+1]); */
                        return DE_OK;
                    }
                }
                *in_lev = sgl2dbl(node_level[node_count] as i32);
            }
        }
    } else {
        if gain_db >= node_gain[node_count] {
            *in_lev = sgl2dbl(node_level[node_count] as i32);
        } else {
            if gain_db <= 0 {
                *in_lev = DRC_INPUT_LOUDNESS_TARGET;
            } else {
                for n in 0..node_count {
                    if (gain_db >= node_gain[n]) && (gain_db < node_gain[n + 1]) {
                        let gain_delta = node_gain[n + 1].wrapping_sub(node_gain[n]);
                        if gain_delta == 0 {
                            *in_lev = sgl2dbl(node_level[n] as i32);
                            return DE_OK;
                        }
                        let w = f_div_norm_noexp(
                            sgl2dbl(node_gain[n + 1].wrapping_sub(gain_db) as i32),
                            sgl2dbl(gain_delta as i32),
                        );
                        *in_lev = f_mult(w, node_level[n])
                            .wrapping_add(f_mult(MAXVAL_DBL.wrapping_sub(w), node_level[n + 1]));
                        /* *inLev = (w * nodeLevel[n] + (1.0-w) * nodeLevel[n+1]); */
                        return DE_OK;
                    }
                }
                *in_lev = sgl2dbl(node_level[node_count] as i32);
            }
        }
    }
    DE_OK
}

/// `_mapGain(pCCharFormatSource, pCCharSource, pCCharFormatTarget, pCCharTarget, gainInDb,
/// gainOutDb)`
pub fn map_gain(
    p_cchar_format_source: CharacteristicFormat,
    p_cchar_source: &CustomDrcChar,
    p_cchar_format_target: CharacteristicFormat,
    p_cchar_target: &CustomDrcChar,
    gain_in_db: FixpSgl,       /* e = 7 */
    gain_out_db: &mut FixpDbl, /* e = 7 */
) -> DrcError {
    let mut in_level: FixpDbl = 0;
    let mut err;

    match p_cchar_format_source {
        CF_SIGMOID => {
            err =
                compressor_io_sigmoid_inverse(p_cchar_source.sigmoid(), gain_in_db, &mut in_level);
            if err != 0 {
                return err;
            }
        }
        CF_NODES => {
            err = compressor_io_nodes_inverse(p_cchar_source.nodes(), gain_in_db, &mut in_level);
            if err != 0 {
                return err;
            }
        }
        _ => return DE_NOT_OK,
    }
    match p_cchar_format_target {
        CF_SIGMOID => {
            err = compressor_io_sigmoid(p_cchar_target.sigmoid(), in_level, gain_out_db);
            if err != 0 {
                return err;
            }
        }
        CF_NODES => {
            err = compressor_io_nodes(p_cchar_target.nodes(), in_level, gain_out_db);
            if err != 0 {
                return err;
            }
        }
        _ => {}
    }
    DE_OK
}

/// `_toLinear(nodeMod, drcBand, gainDb, slopeDb, gainLin, slopeLin)`
fn to_linear(
    node_mod: &NodeModification,
    drc_band: usize,
    gain_db: FixpSgl,        /* in: gain value in dB, e = 7 */
    slope_db: FixpSgl,       /* in: slope value in dB/deltaTmin, e = 2 */
    gain_lin: &mut FixpDbl,  /* out: linear gain value, e = 7 */
    slope_lin: &mut FixpDbl, /* out: linear slope value, e = 7 */
) -> DrcError {
    let mut gain_ratio_m: FixpDbl = fl2fxconst_dbl(1.0 / 2.0);
    // drcBand < 4: bandCountForChannelGroup is at most 4.
    let p_gmod: Option<GainModification> = node_mod.p_gmod.map(|g| g[drc_band.min(3)]);
    let p_dmod = node_mod.p_dmod;
    let mut gain_lin_e = 0;
    let mut gain_ratio_e = 1;
    let mut gain_db_out_e = 0;

    if ((node_mod.drc_set_effect & (EB_DUCK_OTHER | EB_DUCK_SELF)) == 0)
        && (node_mod.drc_set_effect != EB_FADE)
        && (node_mod.drc_set_effect != EB_CLIPPING)
    {
        let mut gain_db_mapped: FixpDbl = 0;

        if let Some(gmod) = p_gmod {
            if node_mod.drc_characteristic_present != 0 {
                if ((gain_db > 0) && node_mod.slope_is_negative != 0)
                    || ((gain_db < 0) && node_mod.slope_is_negative == 0)
                {
                    /* left side */
                    if gmod.target_characteristic_left_present == 1 {
                        let l = CS_LEFT as usize;
                        let err = map_gain(
                            node_mod.characteristic_format_source[l],
                            &node_mod.p_cchar_source[l].unwrap_or(CustomDrcChar::ZERO),
                            node_mod.characteristic_format_target[l],
                            &node_mod.p_cchar_target[l].unwrap_or(CustomDrcChar::ZERO),
                            gain_db,
                            &mut gain_db_mapped,
                        );
                        if err != 0 {
                            return err;
                        }
                        /* target characteristic in payload */
                        gain_ratio_m = f_div_norm_signed(
                            gain_db_mapped,
                            sgl2dbl(gain_db as i32),
                            &mut gain_ratio_e,
                        );
                    }
                } else {
                    /* if (((gainDb < (FIXP_SGL)0) && nodeMod->slopeIsNegative) ||
                    ((gainDb > (FIXP_SGL)0) && !nodeMod->slopeIsNegative)) */

                    /* right side */
                    if gmod.target_characteristic_right_present == 1 {
                        let r = CS_RIGHT as usize;
                        let err = map_gain(
                            node_mod.characteristic_format_source[r],
                            &node_mod.p_cchar_source[r].unwrap_or(CustomDrcChar::ZERO),
                            node_mod.characteristic_format_target[r],
                            &node_mod.p_cchar_target[r].unwrap_or(CustomDrcChar::ZERO),
                            gain_db,
                            &mut gain_db_mapped,
                        );
                        if err != 0 {
                            return err;
                        }
                        /* target characteristic in payload */
                        gain_ratio_m = f_div_norm_signed(
                            gain_db_mapped,
                            sgl2dbl(gain_db as i32),
                            &mut gain_ratio_e,
                        );
                    }
                }
            }
        }
        if gain_db < 0 {
            gain_ratio_m = f_mult_div2(gain_ratio_m, node_mod.compress);
        } else {
            gain_ratio_m = f_mult_div2(gain_ratio_m, node_mod.boost);
        }
        gain_ratio_e += 2;
    }
    if let Some(gmod) = p_gmod {
        if gmod.gain_scaling_present == 1 {
            if gain_db < 0 {
                gain_ratio_m = f_mult_div2(gain_ratio_m, gmod.attenuation_scaling);
            } else {
                gain_ratio_m = f_mult_div2(gain_ratio_m, gmod.amplification_scaling);
            }
            gain_ratio_e += 3;
        }
    }
    if let Some(dmod) = p_dmod {
        if (node_mod.drc_set_effect & (EB_DUCK_OTHER | EB_DUCK_SELF) != 0)
            && (dmod.ducking_scaling_present == 1)
        {
            gain_ratio_m = f_mult_div2(gain_ratio_m, dmod.ducking_scaling);
            gain_ratio_e += 3;
        }
    }

    let gain_db_modified = f_mult_div2(gain_db, gain_ratio_m); /* resulting e: 7 + gainRatio_e + 1*/
    let mut gain_db_offset: FixpDbl = 0;

    if let Some(gmod) = p_gmod {
        if gmod.gain_offset_present == 1 {
            /* *gainLin *= (float)pow(2.0, (double)(pGMod->gainOffset/6.0f)); */
            gain_db_offset = gain_db_offset.wrapping_add(sgl2dbl(gmod.gain_offset as i32) >> 4);
            /* resulting e: 8 */
        }
    }
    if (node_mod.limiter_peak_target_present == 1) && (node_mod.drc_set_effect == EB_CLIPPING) {
        /* The only drcSetEffect is "clipping prevention" */
        /* loudnessNormalizationGainModificationDb is included in loudnessNormalizationGainDb */
        /* *gainLin *= (float)pow(2.0, max(0.0, -nodeModification->limiterPeakTarget -
         * nodeModification->loudnessNormalizationGainDb)/6.0); */
        gain_db_offset = gain_db_offset.wrapping_add(f_max(
            0,
            (sgl2dbl(-(node_mod.limiter_peak_target as i32)) >> 3)
                .wrapping_sub(node_mod.loudness_normalization_gain_db >> 1),
        )); /* resulting e: 8 */
    }
    let gain_db_out = if gain_db_offset != 0 {
        f_add_norm(gain_db_modified, 7 + gain_ratio_e + 1, gain_db_offset, 8, &mut gain_db_out_e)
    } else {
        gain_db_out_e = 7 + gain_ratio_e + 1;
        gain_db_modified
    };

    /* *gainLin = (float)pow(2.0, (double)(gainDb_modified[1] / 6.0f)); */
    let mut gain_lin_m = approx_db2lin(gain_db_out, gain_db_out_e, &mut gain_lin_e);
    *gain_lin = scale_value_saturate(gain_lin_m, gain_lin_e - 7);

    /* *slopeLin = SLOPE_FACTOR_DB_TO_LINEAR * gainRatio * *gainLin * slopeDb; */
    if slope_db == 0 {
        *slope_lin = 0;
    } else {
        let mut tmp_dbl = f_mult(slope_db, SLOPE_FACTOR_DB_TO_LINEAR); /* resulting e: 2 - 3 = -1 */
        tmp_dbl = f_mult(tmp_dbl, gain_ratio_m); /* resulting e: -1 + gainRatio_e */
        if gain_db_offset != 0 {
            /* recalculate gainLin from gainDb that wasn't modified by gainOffset and
             * limiterPeakTarget */
            gain_lin_m = approx_db2lin(gain_db_modified, 7 + gain_ratio_e, &mut gain_lin_e);
        }
        let slope_lin_m = f_mult(tmp_dbl, gain_lin_m);
        *slope_lin = scale_value_saturate(slope_lin_m, -1 + gain_ratio_e + gain_lin_e - 7);
    }

    if (node_mod.limiter_peak_target_present == 1) && (node_mod.drc_set_effect == EB_CLIPPING) {
        if *gain_lin >= fl2fxconst_dbl(1.0 / 128.0) {
            *gain_lin = fl2fxconst_dbl(1.0 / 128.0);
            *slope_lin = 0;
        }
    }

    DE_OK
}

/// `hUniDrcGain->nNodes[seq]` for any `seq`: an index past `nNodes` (12 entries) reaches the next
/// field of the C structure (`gainNode`); reproduced from the C layout.
fn uni_drc_gain_n_nodes(g: &UniDrcGain, seq: usize) -> u8 {
    if seq < 12 {
        return g.n_nodes[seq];
    }
    pod_read_u8(g, offsets::uni_drc_gain_n_nodes(seq)).unwrap_or(0)
}

/// `hUniDrcGain->gainNode[seq][i]` for any `seq`: past `gainNode` (12 sequences) the port reads
/// the following bytes of `UNI_DRC_GAIN`, as the reference does, and 0 beyond the structure
/// (PORTING.md, "Deviations from the reference").
fn uni_drc_gain_gain_node(g: &UniDrcGain, seq: usize, i: usize) -> GainNode {
    if seq < 12 && i < 16 {
        return g.gain_node[seq][i];
    }
    let off = offsets::uni_drc_gain_gain_node(seq, i);
    GainNode {
        gain_db: pod_read_i16(g, off).unwrap_or(0),
        time: pod_read_i16(g, off + 2).unwrap_or(0),
    }
}

/// `prepareDrcGain(hGainDec, hUniDrcGain, compress, boost, loudnessNormalizationGainDb,
/// activeDrcIndex)`: prepare buffers containing linear nodes for each gain sequence
pub fn prepare_drc_gain(
    h_gain_dec: &mut DrcGainDecoder,
    h_uni_drc_config: &UniDrcConfig,
    h_uni_drc_gain: &UniDrcGain,
    compress: FixpSgl,
    boost: FixpSgl,
    loudness_normalization_gain_db: FixpDbl,
    active_drc_index: usize,
) -> DrcError {
    let mut node_mod = NodeModification::ZERO;
    let p_active_drc = h_gain_dec.active_drc[active_drc_index];
    let Some(inst_idx) = p_active_drc.inst() else {
        return DE_NOT_OK;
    };
    let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[inst_idx];

    node_mod.drc_set_effect = p_inst.drc_set_effect as i32;

    node_mod.compress = compress;
    node_mod.boost = boost;
    node_mod.loudness_normalization_gain_db = loudness_normalization_gain_db;
    node_mod.limiter_peak_target_present = p_inst.limiter_peak_target_present as i32;
    node_mod.limiter_peak_target = p_inst.limiter_peak_target;

    let drc_gain_buffers = &mut h_gain_dec.drc_gain_buffers;
    let mut gain_element_index = 0i32;
    for g in 0..(p_inst.n_drc_channel_groups as usize).min(8) {
        let Some(coef_idx) = p_active_drc.coef() else {
            return DE_NOT_OK;
        };
        let p_coef = &h_uni_drc_config.drc_coefficients_uni_drc[coef_idx];

        if p_active_drc.channel_group_is_parametric_drc[g] == 0 {
            let gain_set_index = p_inst.gain_set_index_for_channel_group[g] as i32;

            if node_mod.drc_set_effect & (EB_DUCK_OTHER | EB_DUCK_SELF) != 0 {
                node_mod.p_dmod = Some(p_active_drc.ducking_modification_for_channel_group[g]);
                node_mod.p_gmod = None;
            } else {
                node_mod.p_gmod = Some(p_inst.gain_modification_for_channel_group()[g]);
                node_mod.p_dmod = None;
            }

            let n_drc_bands = p_active_drc.band_count_for_channel_group[g] as usize;
            for b in 0..n_drc_bands {
                // gainSetIndex is in 0..gainSetCount and b < 4 for a non-parametric channel
                // group (_generateDrcInstructionsDerivedData).
                let p_gain_set = usize::try_from(gain_set_index)
                    .ok()
                    .and_then(|i| p_coef.gain_set.get(i))
                    .copied()
                    .unwrap_or(GainSet::ZERO);
                let seq = p_gain_set.gain_sequence_index[b.min(3)] as usize;
                let p_dchar = p_gain_set.drc_characteristic[b.min(3)];

                /* linearNodeBuffer contains a copy of the gain sequences (consisting of nodes)
                that are relevant for decoding. It also contains gain sequences of previous
                frames. */
                let lnbp = drc_gain_buffers.lnb_pointer as usize;
                let p_lnb = drc_gain_buffers
                    .lnb_mut(p_active_drc.active_drc_offset.wrapping_add(gain_element_index));
                p_lnb.gain_interpolation_type =
                    p_gain_set.gain_interpolation_type as GainInterpolationType;

                let err = prepare_drc_characteristic(&p_dchar, p_coef, b, &mut node_mod);
                if err != 0 {
                    return err;
                }

                /* copy a node buffer and convert from dB to linear */
                p_lnb.n_nodes[lnbp] = (uni_drc_gain_n_nodes(h_uni_drc_gain, seq) as i32).min(16);
                for i in 0..p_lnb.n_nodes[lnbp] as usize {
                    let mut gain_lin = 0;
                    let mut slope_lin = 0;
                    let node = uni_drc_gain_gain_node(h_uni_drc_gain, seq, i);
                    let err =
                        to_linear(&node_mod, b, node.gain_db, 0, &mut gain_lin, &mut slope_lin);
                    if err != 0 {
                        return err;
                    }
                    p_lnb.linear_node[lnbp][i].gain_lin = gain_lin;
                    p_lnb.linear_node[lnbp][i].time = node.time;
                }
                gain_element_index += 1;
            }
        } else {
            /* parametric DRC not supported */
            gain_element_index += 1;
        }
    }
    DE_OK
}
