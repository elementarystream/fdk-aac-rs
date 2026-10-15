// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/src/drcDec_tools.h, libDRCdec/src/drcDec_tools.cpp.

//! Port of `drcDec_tools.h`: helper functions of the DRC decoder.
//!
//! The functions that return a pointer into the `UNI_DRC_CONFIG` (`selectDrcCoefficients`, ...)
//! return the index of the element instead (`None` for `NULL`).

use fdk_aac_rs_fdk::common_fix::{f_mult, f_mult_div2, fl2fxconst_dbl, FixpDbl, FixpSgl};
use fdk_aac_rs_fdk::common_fix::{DFRACT_BITS, MINVAL_DBL};
use fdk_aac_rs_fdk::fixpoint_math::{f2_pow, f_div_norm_noexp, f_log2, fixp_round};
use fdk_aac_rs_fdk::scale::scale_value;

use crate::drc_dec_types::*;
use crate::drc_decoder::*;

/// `getDeltaTmin(sampleRate)`
pub fn get_delta_tmin(sample_rate: i32) -> i32 {
    /* half_ms = round (0.0005 * sampleRate); */
    let half_ms = sample_rate.wrapping_add(1000) / 2000;
    let mut delta_tmin = 1i32;
    if sample_rate < 1000 {
        return DE_NOT_OK;
    }
    while delta_tmin <= half_ms {
        delta_tmin = delta_tmin.wrapping_shl(1);
    }
    delta_tmin
}

/// `selectDrcCoefficients(hUniDrcConfig, location)`: the index of the last
/// `drcCoefficientsUniDrc` with `drcLocation == location`, `None` if there is none (possible
/// during bitstream parsing).
pub fn select_drc_coefficients(h_uni_drc_config: &UniDrcConfig, location: i32) -> Option<usize> {
    let mut c: i32 = -1;
    for n in 0..h_uni_drc_config.drc_coefficients_uni_drc_count as usize {
        // The count is saturated to 2 by the reader; a larger value cannot be reached.
        let Some(coef) = h_uni_drc_config.drc_coefficients_uni_drc.get(n) else { break };
        if coef.drc_location as i32 == location {
            c = n as i32;
        }
    }
    if c >= 0 {
        return Some(c as usize);
    }
    None /* possible during bitstream parsing */
}

/// `selectDrcInstructions(hUniDrcConfig, drcSetId)`: the index of the first
/// `drcInstructionsUniDrc` (including the virtual ones) with the given `drcSetId`.
pub fn select_drc_instructions(h_uni_drc_config: &UniDrcConfig, drc_set_id: i32) -> Option<usize> {
    for i in 0..h_uni_drc_config.drc_instructions_count_incl_virtual as usize {
        // drcInstructionsCountInclVirtual is at most 12 + 1 + 6 (_generateVirtualDrcSets).
        let Some(inst) = h_uni_drc_config.drc_instructions_uni_drc.get(i) else { break };
        if inst.drc_set_id as i32 == drc_set_id {
            return Some(i);
        }
    }
    None
}

/// `selectDownmixInstructions(hUniDrcConfig, downmixId)`: the index of the first
/// `downmixInstructions` with the given `downmixId`.
pub fn select_downmix_instructions(
    h_uni_drc_config: &UniDrcConfig,
    downmix_id: i32,
) -> Option<usize> {
    for i in 0..h_uni_drc_config.downmix_instructions_count as usize {
        // The count is saturated to 6 by the reader.
        let Some(down) = h_uni_drc_config.downmix_instructions.get(i) else { break };
        if down.downmix_id as i32 == downmix_id {
            return Some(i);
        }
    }
    None
}

/// `deriveDrcChannelGroups(...)`.
///
/// `duckingModificationForChannel` and `duckingModificationForChannelGroup` are only accessed
/// for the ducking effects (`EB_DUCK_OTHER`, `EB_DUCK_SELF`), as in C; a `None` there corresponds
/// to the `NULL` that the C callers pass for the other effects.
pub fn derive_drc_channel_groups(
    drc_set_effect: i32,                                                 /* in */
    channel_count: i32,                                                  /* in */
    gain_set_index: &[i8; 8],                                            /* in */
    ducking_modification_for_channel: Option<&[DuckingModification; 8]>, /* in */
    n_drc_channel_groups: &mut u8,                                       /* out */
    unique_index: &mut [i8; 8], /* out (gainSetIndexForChannelGroup) */
    group_for_channel: &mut [i8; 8], /* out */
    ducking_modification_for_channel_group: Option<&mut [DuckingModification; 8]>, /* out */
) -> DrcError {
    let mut ducking_sequence: i32 = -1;
    let mut unique_scaling = [0 as FixpSgl; 8];

    for g in 0..8 {
        unique_index[g] = -10;
        unique_scaling[g] = -32768; /* FIXP_SGL(-1.0f) */
    }

    let mut g: usize = 0;

    // The ducking scaling of channel c (the C code reads it only for the ducking effects, for
    // which the callers pass the array).
    let factor_of = |c: usize| -> FixpSgl {
        ducking_modification_for_channel.map_or(0, |d| d[c].ducking_scaling)
    };

    if drc_set_effect & EB_DUCK_OTHER != 0 {
        let mut c = 0;
        while c < channel_count {
            let mut is_match = 0;
            if c >= 8 {
                return DE_MEMORY_ERROR;
            }
            let idx = gain_set_index[c as usize] as i32;
            let factor = factor_of(c as usize);
            if idx < 0 {
                for n in 0..g {
                    if unique_scaling[n] == factor {
                        is_match = 1;
                        group_for_channel[c as usize] = n as i8;
                        break;
                    }
                }
                if is_match == 0 {
                    if g >= 8 {
                        return DE_MEMORY_ERROR;
                    }
                    unique_index[g] = idx as i8;
                    unique_scaling[g] = factor;
                    group_for_channel[c as usize] = g as i8;
                    g += 1;
                }
            } else {
                if (ducking_sequence > 0) && (ducking_sequence != idx) {
                    return DE_NOT_OK;
                }
                ducking_sequence = idx;
                group_for_channel[c as usize] = -1;
            }
            c += 1;
        }
        if ducking_sequence == -1 {
            return DE_NOT_OK;
        }
    } else if drc_set_effect & EB_DUCK_SELF != 0 {
        let mut c = 0;
        while c < channel_count {
            let mut is_match = 0;
            if c >= 8 {
                return DE_MEMORY_ERROR;
            }
            let idx = gain_set_index[c as usize] as i32;
            let factor = factor_of(c as usize);
            if idx >= 0 {
                for n in 0..g {
                    if (unique_index[n] as i32 == idx) && (unique_scaling[n] == factor) {
                        is_match = 1;
                        group_for_channel[c as usize] = n as i8;
                        break;
                    }
                }
                if is_match == 0 {
                    if g >= 8 {
                        return DE_MEMORY_ERROR;
                    }
                    unique_index[g] = idx as i8;
                    unique_scaling[g] = factor;
                    group_for_channel[c as usize] = g as i8;
                    g += 1;
                }
            } else {
                group_for_channel[c as usize] = -1;
            }
            c += 1;
        }
    } else {
        /* no ducking */
        let mut c = 0;
        while c < channel_count {
            if c >= 8 {
                return DE_MEMORY_ERROR;
            }
            let idx = gain_set_index[c as usize] as i32;
            let mut is_match = 0;
            if idx >= 0 {
                for n in 0..g {
                    if unique_index[n] as i32 == idx {
                        is_match = 1;
                        group_for_channel[c as usize] = n as i8;
                        break;
                    }
                }
                if is_match == 0 {
                    if g >= 8 {
                        return DE_MEMORY_ERROR;
                    }
                    unique_index[g] = idx as i8;
                    group_for_channel[c as usize] = g as i8;
                    g += 1;
                }
            } else {
                group_for_channel[c as usize] = -1;
            }
            c += 1;
        }
    }
    *n_drc_channel_groups = g as u8;

    if drc_set_effect & (EB_DUCK_OTHER | EB_DUCK_SELF) != 0 {
        if let Some(dm) = ducking_modification_for_channel_group {
            for g in 0..*n_drc_channel_groups as usize {
                if drc_set_effect & EB_DUCK_OTHER != 0 {
                    unique_index[g] = ducking_sequence as i8;
                }
                dm[g].ducking_scaling = unique_scaling[g];
                if unique_scaling[g] != 8192
                /* FL2FXCONST_SGL(1.0f / (float)(1 << 2)) */
                {
                    dm[g].ducking_scaling_present = 1;
                } else {
                    dm[g].ducking_scaling_present = 0;
                }
            }
        }
    }

    DE_OK
}

/// `FL2FXCONST_DBL(0.1660964f * (float)(1 << 2))`
const DB2LIN_FACTOR: FixpDbl = fl2fxconst_dbl((0.1660964f32 * 4.0f32) as f64);
/// `FL2FXCONST_DBL(6.02059991f / (float)(1 << 3))`
const LIN2DB_FACTOR: FixpDbl = fl2fxconst_dbl((6.02059991f32 / 8.0f32) as f64);
/// `FL2FXCONST_DBL(0.1666667f * (float)(1 << 2))`
const APPROX_DB2LIN_FACTOR: FixpDbl = fl2fxconst_dbl((0.1666667f32 * 4.0f32) as f64);

/// `dB2lin(dB_m, dB_e, pLin_e)`: get linear value from dB.
///
/// `lin_val = 10^(dB_val/20) = 2^(log2(10)/20*dB_val)` with `dB_val = dB_m *2^dB_e` and
/// `lin_val = lin_m * 2^lin_e`
pub fn db2lin(db_m: FixpDbl, db_e: i32, p_lin_e: &mut i32) -> FixpDbl {
    f2_pow(f_mult(db_m, DB2LIN_FACTOR), db_e - 2, p_lin_e)
}

/// `lin2dB(lin_m, lin_e, pDb_e)`: get dB value from linear value.
///
/// `dB_val = 20*log10(lin_val)` with `dB_val = dB_m *2^dB_e` and `lin_val = lin_m * 2^lin_e`
pub fn lin2db(lin_m: FixpDbl, lin_e: i32, p_db_e: &mut i32) -> FixpDbl {
    let db_m;

    if lin_m == 0 {
        /* return very small value representing -inf */
        db_m = MINVAL_DBL;
        *p_db_e = DFRACT_BITS - 1;
    } else {
        /* 20*log10(lin_val) = 20/log2(10)*log2(lin_val) */
        db_m = f_mult_div2(LIN2DB_FACTOR, f_log2(lin_m, lin_e, p_db_e));
        *p_db_e += 3 + 1;
    }

    db_m
}

/// `approxDb2lin(dB_m, dB_e, pLin_e)`: get linear value from approximate dB.
///
/// `lin_val = 2^(dB_val/6)` with `dB_val = dB_m *2^dB_e` and `lin_val = lin_m * 2^lin_e`
pub fn approx_db2lin(db_m: FixpDbl, db_e: i32, p_lin_e: &mut i32) -> FixpDbl {
    f2_pow(f_mult(db_m, APPROX_DB2LIN_FACTOR), db_e - 2, p_lin_e)
}

/// `pCoef->gainSet[seq].bandCount` for any `seq`, read from the memory image of the
/// `UNI_DRC_CONFIG`: an index past `gainSet` reaches the following fields of the C structure;
/// reproduced from the C layout. 0 outside of the structure.
pub(crate) fn gain_set_band_count_oob(
    h_uni_drc_config: &UniDrcConfig,
    coef: usize,
    seq: i32,
) -> u8 {
    if (0..12).contains(&seq) && coef < 2 {
        return h_uni_drc_config.drc_coefficients_uni_drc[coef].gain_set[seq as usize].band_count;
    }
    let off = offsets::drc_coefficients_uni_drc(coef) as isize
        + offsets::gain_set_band_count(0) as isize
        + seq as isize * core::mem::size_of::<GainSet>() as isize;
    if off < 0 {
        return 0;
    }
    pod_read_u8(h_uni_drc_config, off as usize).unwrap_or(0)
}

/// `bitstreamContainsMultibandDrc(hUniDrcConfig, downmixId)`
///
/// `bandCount` of a `seq` (`gainSetIndexForChannelGroup`) past `gainSet` is read from the
/// following fields of the C structure; reproduced from the C layout.
pub fn bitstream_contains_multiband_drc(h_uni_drc_config: &UniDrcConfig, downmix_id: i32) -> i32 {
    let mut is_multiband = 0;

    let Some(p_coef) = select_drc_coefficients(h_uni_drc_config, LOCATION_SELECTED) else {
        return 0;
    };

    for i in 0..h_uni_drc_config.drc_instructions_uni_drc_count as usize {
        let Some(p_inst) = h_uni_drc_config.drc_instructions_uni_drc.get(i) else { break };
        for d in 0..p_inst.downmix_id_count as usize {
            // downmixIdCount is at most 8 (the reader rejects more).
            let Some(&id) = p_inst.downmix_id.get(d) else { break };
            if downmix_id == id as i32 {
                for g in 0..p_inst.n_drc_channel_groups as usize {
                    let Some(&seq) = p_inst.gain_set_index_for_channel_group.get(g) else {
                        break;
                    };
                    if gain_set_band_count_oob(h_uni_drc_config, p_coef, seq as i32) > 1 {
                        is_multiband = 1;
                    }
                }
            }
        }
    }

    is_multiband
}

/// `getDownmixOffset(pDown, baseChannelCount)`
pub fn get_downmix_offset(p_down: &DownmixInstructions, base_channel_count: i32) -> FixpDbl {
    let mut downmix_offset: FixpDbl = fl2fxconst_dbl(1.0 / 2.0); /* e = 1 */
    if (p_down.bs_downmix_offset == 1) || (p_down.bs_downmix_offset == 2) {
        let mut e_a = 0;
        let mut e_downmix_offset = 0;
        if base_channel_count <= p_down.target_channel_count as i32 {
            return downmix_offset;
        }

        let q = f_div_norm_noexp(p_down.target_channel_count as FixpDbl, base_channel_count); /* e = 0 */
        let mut a = lin2db(q, 0, &mut e_a);
        if p_down.bs_downmix_offset == 2 {
            e_a += 1; /* a *= 2 */
        }
        /* a = 0.5 * round (a) */
        a = fixp_round(a, e_a) >> 1;
        downmix_offset = db2lin(a, e_a, &mut e_downmix_offset);
        downmix_offset = scale_value(downmix_offset, e_downmix_offset - 1);
    }
    downmix_offset
}
