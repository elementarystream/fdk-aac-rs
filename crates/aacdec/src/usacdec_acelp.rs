// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/usacdec_acelp.h, libAACdec/src/usacdec_acelp.cpp.

//! Port of `usacdec_acelp.h`: USAC ACELP frame decoder.
//!
//! The types of `usacdec_acelp.h` ([`CAcelpStaticMem`], [`CAcelpChannelData`]) are defined
//! first; the functions of `usacdec_acelp.h` and `usacdec_acelp.cpp` follow.

use fdk_aac_rs_fdk::bitstream::{fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::{
    f_add_saturate, f_div_norm_high_prec, f_max, f_min, f_mult, f_mult_add_div2, f_mult_div2,
    f_norm, f_pow2, f_pow2_div2, fl2fxconst_dbl, fl2fxconst_sgl, fx_dbl2fx_sgl, fx_sgl2fx_dbl,
    inv_sqrt_norm2, schur_div, FixpDbl, FixpSgl, DFRACT_BITS, MAXVAL_DBL,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::lpc::{fx_dbl2fx_lpc, FixpLpc};
use fdk_aac_rs_fdk::mdct::MDCT_OUT_HEADROOM;
use fdk_aac_rs_fdk::scale::{
    saturate_left_shift, scale_value, scale_value_saturate, scale_values, scale_values_saturate,
};

use crate::channelinfo::AAC_DEC_DECODE_FRAME_ERROR;
use crate::usacdec_ace_d4t64::{
    d_acelp_decode_4t64, fx_cod2fx_dbl, fx_dbl2fx_cod, fx_sgl2fx_cod, FixpCod,
};
use crate::usacdec_ace_ltp::{pred_lt4, pred_lt4_postfilter};
use crate::usacdec_const::{
    PIT_MAX_12k8, PIT_MIN_12k8, BPF_DELAY, FSCALE_DENOM, LFAC, LP_FILTER_SCALE, L_DIV, L_INTERPOL,
    L_SUBFR, M_LP_FILTER_ORDER, NB_DIV, NB_SUBFR, PIT_MAX_MAX, SF_CODE, SF_EXC, SF_GAIN_C,
    SF_GAIN_P, SF_PFAC, SF_STAB,
};
use crate::usacdec_lpc::{e_lpc_a_weight_in_place, e_lpc_f_lsp_a_conversion};
use crate::usacdec_rom::{FDK_T_QUA_GAIN7B, LSP_INTERPOL_FACTOR, SF_QUA_GAIN7B};

mod fast;

// ---------------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------------

/// `CAcelpStaticMem`: the ACELP internal persistent memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CAcelpStaticMem {
    /// `old_exc_mem`
    pub old_exc_mem: [FixpDbl; (PIT_MAX_MAX + L_INTERPOL) as usize],
    /// `old_syn_mem`: synthesis filter states
    pub old_syn_mem: [FixpDbl; M_LP_FILTER_ORDER as usize],
    /// `A`
    pub a: [FixpSgl; M_LP_FILTER_ORDER as usize],
    /// `A_exp`
    pub a_exp: i32,
    /// `gc_threshold`
    pub gc_threshold: FixpDbl,
    /// `de_emph_mem`
    pub de_emph_mem: FixpDbl,
    /// `past_gpit`
    pub past_gpit: FixpSgl,
    /// `past_gcode`
    pub past_gcode: FixpDbl,
    /// `old_T0`
    pub old_t0: u16,
    /// `old_T0_frac`
    pub old_t0_frac: u8,
    /// `deemph_mem_wsyn`
    pub deemph_mem_wsyn: FixpDbl,
    /// `wsyn_rms`
    pub wsyn_rms: FixpDbl,
    /// `seed_ace`
    pub seed_ace: i16,
}

impl Default for CAcelpStaticMem {
    fn default() -> Self {
        CAcelpStaticMem {
            old_exc_mem: [0; (PIT_MAX_MAX + L_INTERPOL) as usize],
            old_syn_mem: [0; M_LP_FILTER_ORDER as usize],
            a: [0; M_LP_FILTER_ORDER as usize],
            a_exp: 0,
            gc_threshold: 0,
            de_emph_mem: 0,
            past_gpit: 0,
            past_gcode: 0,
            old_t0: 0,
            old_t0_frac: 0,
            deemph_mem_wsyn: 0,
            wsyn_rms: 0,
            seed_ace: 0,
        }
    }
}

/// `CAcelpChannelData`: the parameter data needed to decode one ACELP frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CAcelpChannelData {
    /// `acelp_core_mode`: acelp core mode for whole ACELP frame
    pub acelp_core_mode: u8,
    /// `mean_energy`: mean excitation energy index for whole ACELP frame
    pub mean_energy: u8,
    /// `T0`
    pub t0: [u16; NB_SUBFR as usize],
    /// `T0_frac`
    pub t0_frac: [u8; NB_SUBFR as usize],
    /// `ltp_filtering_flag`: controls whether LTP postfilter is active for each ACELP subframe
    pub ltp_filtering_flag: [u8; NB_SUBFR as usize],
    /// `icb_index`: innovative codebook index for each ACELP subframe
    pub icb_index: [[i16; 8]; NB_SUBFR as usize],
    /// `gains`: gain index for each ACELP subframe
    pub gains: [u8; NB_SUBFR as usize],
}

// ---------------------------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------------------------

/// `E_UTIL_random(seed)`
#[inline]
pub fn e_util_random(seed: &mut i16) -> i16 {
    *seed = (((*seed as i32 * 31821) >> 1) + 13849) as i16;
    *seed
}

/// `PIT_FR2_12k8`: minimum pitch lag with resolution 1/2
const PIT_FR2_12k8: i32 = 128;
/// `PIT_FR1_12k8`: minimum pitch lag with resolution 1
const PIT_FR1_12k8: i32 = 160;

/// `TILT_CODE2`: ACELP code pre-emphasis factor (*2), `FL2FXCONST_SGL(0.3f * 2.0f)`
#[inline(always)]
fn tilt_code2() -> FixpSgl {
    fl2fxconst_sgl((0.3f32 * 2.0f32) as f64)
}
/// `PIT_SHARP`: pitch sharpening factor, `FL2FXCONST_SGL(0.85f)`
#[inline(always)]
fn pit_sharp() -> FixpSgl {
    fl2fxconst_sgl(0.85f32 as f64)
}
/// `PREEMPH_FAC`: ACELP synth pre-emphasis factor, `FL2FXCONST_SGL(0.68f)`
#[inline(always)]
fn preemph_fac() -> FixpSgl {
    fl2fxconst_sgl(0.68f32 as f64)
}

/// `ACELP_HEADROOM`
const ACELP_HEADROOM: i32 = 1;
/// `ACELP_OUTSCALE`: `MDCT_OUT_HEADROOM - ACELP_HEADROOM`
pub const ACELP_OUTSCALE: i32 = MDCT_OUT_HEADROOM - ACELP_HEADROOM;

const M: usize = M_LP_FILTER_ORDER as usize;
const L_SUBFR_U: usize = L_SUBFR as usize;
/// `PIT_MAX_MAX + L_INTERPOL`: length of the excitation memory
const EXC_MEM: usize = (PIT_MAX_MAX + L_INTERPOL) as usize;

/// `x[i]` of a C pointer into `buf`; 0 outside of `buf`.
#[inline(always)]
fn rd(buf: &[FixpDbl], idx: isize) -> FixpDbl {
    if idx >= 0 {
        buf.get(idx as usize).copied().unwrap_or(0)
    } else {
        0
    }
}

/// `E_UTIL_preemph(in, out, L)`: pre-emphasis (1 - mu z^-1) on the input signal. `in`
/// (`x[x_off..]`) is the input signal, `in[-1]` is also needed; `out`: output signal; `l`:
/// length of filtering.
pub fn e_util_preemph(x: &[FixpDbl], x_off: usize, out: &mut [FixpDbl], l: i32) {
    let fac = preemph_fac();
    for i in 0..l.max(0) as usize {
        let xi = x_off as isize + i as isize;
        out[i] = f_add_saturate(rd(x, xi), f_mult(fac, rd(x, xi - 1)).wrapping_neg());
    }
}

/// `Preemph_code(x)`: de-emphasis 1/(1 - TILT_CODE z^-1) on the innovative codebook vector.
pub fn preemph_code(x: &mut [FixpCod; L_SUBFR_U]) {
    let tilt = tilt_code2();
    for i in (1..L_SUBFR_U).rev() {
        let mut l_tmp = fx_cod2fx_dbl(x[i]);
        l_tmp = l_tmp.wrapping_sub(f_mult_div2(x[i - 1], tilt));
        x[i] = fx_dbl2fx_cod(l_tmp);
    }
}

/// `Pit_shrp(x, pit_lag)`: apply the pitch sharpener to the innovative codebook vector.
pub fn pit_shrp(x: &mut [FixpCod; L_SUBFR_U], pit_lag: i32) {
    let sharp = pit_sharp();
    let mut i = pit_lag;
    while i < L_SUBFR {
        let mut l_tmp = fx_cod2fx_dbl(x[i as usize]);
        // x[i - pit_lag] is inside of x[] for pit_lag >= 0 (the pitch lag is never negative).
        let xp = x.get((i - pit_lag) as usize).copied().unwrap_or(0);
        l_tmp = l_tmp.wrapping_add(f_mult(xp, sharp));
        x[i as usize] = fx_dbl2fx_cod(l_tmp);
        i += 1;
    }
}

/// `SF_MEAN_ENER_LG10`
const SF_MEAN_ENER_LG10: i32 = 9;

/// `pow_10_mean_energy[4]`: `pow(10.0, {18, 30, 42, 54}/20.0) /(float)(1<<SF_MEAN_ENER_LG10)`
///
/// `scripts/dump_table.py libAACdec/src/usacdec_acelp.cpp pow_10_mean_energy LONG --hex`
pub static POW_10_MEAN_ENERGY: [FixpDbl; 4] = [0x01FC5EBD, 0x07E7DB92, 0x1F791F65, 0x7D4BFBA3];

/// `D_gain2_plus(index, code, gain_pit, gain_code, mean_ener_bits, bfi, past_gpit, past_gcode,
/// pEner_code, pEner_code_e)`: calculate the quantized codebook gain, the quantized pitch gain
/// and the unbiased innovative code vector energy.
#[allow(clippy::too_many_arguments)]
pub fn d_gain2_plus(
    index: i32,
    code: &[FixpCod; L_SUBFR_U],
    gain_pit: &mut FixpSgl,
    gain_code: &mut FixpDbl,
    mean_ener_bits: i32,
    bfi: i32,
    past_gpit: &mut FixpSgl,
    past_gcode: &mut FixpDbl,
    p_ener_code: &mut FixpDbl,
    p_ener_code_e: &mut i32,
) {
    let mut gcode0_e: i32 = 0;

    // ener_code = sum(code[]^2)
    let mut ener_code: FixpDbl = 0;
    for i in 0..L_SUBFR_U {
        ener_code = ener_code.wrapping_add(f_pow2_div2(code[i]));
    }

    // fNorm(ener_code) - 1: both reference builds assume that the sum of squares does not
    // overflow (ener_code >= 0) and compute it as clz(ener_code) - 2 (-1 for 0). The port does
    // the same, which differs from fNorm() only for an overflowed (negative) sum.
    let norm_m1 = if ener_code == 0 { -1 } else { (ener_code as u32).leading_zeros() as i32 - 2 };
    let mut ener_code_e = f_max(norm_m1, 0);
    ener_code <<= ener_code_e;
    ener_code_e = 2 * SF_CODE + 1 - ener_code_e;

    // export energy of code for calc_period_factor()
    *p_ener_code = ener_code;
    *p_ener_code_e = ener_code_e;

    ener_code = ener_code.wrapping_add(scale_value(fl2fxconst_dbl(0.01f32 as f64), -ener_code_e));

    // ener_code *= 1/L_SUBFR, and make exponent even (because of square root below).
    if ener_code_e & 1 != 0 {
        ener_code_e -= 5;
        ener_code >>= 1;
    } else {
        ener_code_e -= 6;
    }
    let gcode_inov = inv_sqrt_norm2(ener_code, &mut gcode0_e);
    let gcode_inov_e = gcode0_e - (ener_code_e >> 1);

    if bfi != 0 {
        let mut tgpit = *past_gpit;

        if tgpit > fl2fxconst_sgl((0.95f32 / (1 << SF_GAIN_P) as f32) as f64) {
            tgpit = fl2fxconst_sgl((0.95f32 / (1 << SF_GAIN_P) as f32) as f64);
        } else if tgpit < fl2fxconst_sgl((0.5f32 / (1 << SF_GAIN_P) as f32) as f64) {
            tgpit = fl2fxconst_sgl((0.5f32 / (1 << SF_GAIN_P) as f32) as f64);
        }
        *gain_pit = tgpit;
        tgpit = fx_dbl2fx_sgl(f_mult(tgpit, fl2fxconst_dbl(0.95f32 as f64)));
        *past_gpit = tgpit;

        tgpit = (fl2fxconst_sgl((1.4f32 / (1 << SF_GAIN_P) as f32) as f64) as i32 - tgpit as i32)
            as FixpSgl;
        let tgcode = f_mult(*past_gcode, tgpit) << SF_GAIN_P;
        *gain_code = scale_value(f_mult(tgcode, gcode_inov), gcode_inov_e);
        *past_gcode = tgcode;

        return;
    }

    // Decode gains
    //   gcode0 = pow(10.0, (float)mean_ener/20.0);
    //   gcode0 = gcode0 / sqrt(ener_code/L_SUBFR);
    // mean_ener_bits is a 2 bit value; other values read 0 (no defined reference result).
    let mut gcode0 = POW_10_MEAN_ENERGY.get(mean_ener_bits as usize).copied().unwrap_or(0);
    gcode0 = f_mult_div2(gcode0, gcode_inov);
    gcode0_e = gcode0_e + SF_MEAN_ENER_LG10 - (ener_code_e >> 1) + 1;

    // index is a 7 bit value; other values read 0 (no defined reference result).
    let i = (index << 1) as usize;
    *gain_pit = FDK_T_QUA_GAIN7B.get(i).copied().unwrap_or(0); // adaptive codebook gain
                                                               // t_qua_gain[ind2p1] : fixed codebook gain correction factor
    let ltmp = f_mult(FDK_T_QUA_GAIN7B.get(i + 1).copied().unwrap_or(0), gcode0);
    *gain_code = scale_value(ltmp, gcode0_e - SF_GAIN_C + SF_QUA_GAIN7B);

    // update bad frame handler
    *past_gpit = *gain_pit;

    // past_gcode  = gain_code/gcode_inov
    {
        let mut gcode_e: i32 = 0;

        let gcode_m = f_div_norm_high_prec(ltmp, gcode_inov, &mut gcode_e);
        gcode_e += (gcode0_e - SF_GAIN_C + SF_QUA_GAIN7B) - gcode_inov_e;
        *past_gcode = scale_value(gcode_m, gcode_e);
    }
}

/// `calc_period_factor(exc, gain_pit, gain_code, ener_code, ener_code_e)`: calculate the
/// period/voicing factor r_v (-1=unvoiced to 1=voiced), exponent `SF_PFAC`. `exc` is the pitch
/// excitation (`L_SUBFR` values).
///
/// The energy of the pitch excitation comes from `fast::pitch_energy`, which gives the same
/// values as the loop of the straight port [`calc_period_factor_ref`].
pub fn calc_period_factor(
    exc: &[FixpDbl],
    gain_pit: FixpSgl,
    gain_code: FixpDbl,
    ener_code: FixpDbl,
    ener_code_e: i32,
) -> FixpDbl {
    let (ener_exc, s) = fast::pitch_energy(exc);
    period_factor(ener_exc, s, gain_pit, gain_code, ener_code, ener_code_e)
}

/// The straight port of `calc_period_factor()` (see [`calc_period_factor`]).
pub fn calc_period_factor_ref(
    exc: &[FixpDbl],
    gain_pit: FixpSgl,
    gain_code: FixpDbl,
    ener_code: FixpDbl,
    ener_code_e: i32,
) -> FixpDbl {
    let (ener_exc, s) = pitch_energy_ref(exc);
    period_factor(ener_exc, s, gain_pit, gain_code, ener_code, ener_code_e)
}

/// The energy of the pitch excitation of `calc_period_factor()` and its scaling `s`.
fn pitch_energy_ref(exc: &[FixpDbl]) -> (FixpDbl, i32) {
    let mut s: i32 = 0;

    // energy of pitch excitation
    let mut ener_exc: FixpDbl = 0;
    for i in 0..L_SUBFR_U {
        // s stays far below 32 (at most one increment per sample with a halved sum).
        ener_exc = ener_exc.wrapping_add(f_pow2_div2(exc[i]).wrapping_shr(s as u32));
        if ener_exc >= fl2fxconst_dbl(0.5) {
            ener_exc >>= 1;
            s += 1;
        }
    }
    (ener_exc, s)
}

/// The part of `calc_period_factor()` after the energy of the pitch excitation.
fn period_factor(
    ener_exc: FixpDbl,
    s: i32,
    gain_pit: FixpSgl,
    gain_code: FixpDbl,
    ener_code: FixpDbl,
    ener_code_e: i32,
) -> FixpDbl {
    let mut ener_exc = ener_exc;
    let mut ener_exc_e = f_norm(ener_exc);
    ener_exc = f_mult(ener_exc.wrapping_shl(ener_exc_e as u32), f_pow2(gain_pit));
    if ener_exc != 0 {
        ener_exc_e = 2 * SF_EXC + 1 + 2 * SF_GAIN_P - ener_exc_e + s;
    } else {
        ener_exc_e = 0;
    }

    // energy of innovative code excitation
    // L_tmp = ener_code * gain_code*gain_code;
    let mut l_tmp_e = f_norm(gain_code);
    let mut l_tmp = f_pow2(gain_code.wrapping_shl(l_tmp_e as u32));
    l_tmp = f_mult(ener_code, l_tmp);
    l_tmp_e = 2 * SF_GAIN_C + ener_code_e - 2 * l_tmp_e;

    // Find common exponent
    let period_fac;
    {
        let den: FixpDbl;

        let exp_diff = ener_exc_e - l_tmp_e;
        if exp_diff >= 0 {
            ener_exc >>= 1;
            if exp_diff <= DFRACT_BITS - 2 {
                l_tmp >>= exp_diff + 1;
            } else {
                l_tmp = 0;
            }
            let mut d = ener_exc.wrapping_add(l_tmp);
            if ener_exc_e < DFRACT_BITS - 1 {
                d = d.wrapping_add(scale_value(fl2fxconst_dbl(0.01f32 as f64), -ener_exc_e - 1));
            }
            den = d;
        } else {
            if exp_diff >= -(DFRACT_BITS - 2) {
                ener_exc >>= 1 - exp_diff;
            } else {
                ener_exc = 0;
            }
            l_tmp >>= 1;
            let mut d = ener_exc.wrapping_add(l_tmp);
            if l_tmp_e < DFRACT_BITS - 1 {
                // -L_tmp_e - 1 can exceed 31 for a vanishing codebook gain and energy
                // (undefined in C); see the disassembly notes in the module documentation.
                d = d.wrapping_add(scale_value(fl2fxconst_dbl(0.01f32 as f64), -l_tmp_e - 1));
            }
            den = d;
        }
        let mut num = ener_exc.wrapping_sub(l_tmp);
        num >>= SF_PFAC;

        if den > 0 {
            if ener_exc > l_tmp {
                period_fac = schur_div(num, den, 16);
            } else {
                period_fac = schur_div(num.wrapping_neg(), den, 16).wrapping_neg();
            }
        } else {
            period_fac = MAXVAL_DBL;
        }
    }

    // exponent = SF_PFAC
    period_fac
}

/// `noise_enhancer(gain_code, period_fac, stab_fac, p_gc_threshold)`: enhance the excitation
/// on noise (modify the gain of the code). If the signal is noisy and the LPC filter is
/// stable, move the gain of the code 1.5 dB toward the gain of code threshold. Returns the
/// smoothed gain of code g_sc, exponent `SF_GAIN_C`.
pub fn noise_enhancer(
    mut gain_code: FixpDbl,
    period_fac: FixpDbl,
    stab_fac: FixpSgl,
    p_gc_threshold: &mut FixpDbl,
) -> FixpDbl {
    let gc_thres = *p_gc_threshold;

    let mut l_tmp = gain_code;
    if l_tmp < gc_thres {
        // +1.5dB => *(1.0+0.19)
        l_tmp = l_tmp.wrapping_add(f_mult_div2(gain_code, fl2fxconst_sgl(2.0 * 0.19f32 as f64)));
        if l_tmp > gc_thres {
            l_tmp = gc_thres;
        }
    } else {
        // -1.5dB => *10^(-1.5/20)
        l_tmp = f_mult(gain_code, fl2fxconst_sgl((1.0f32 / 1.19f32) as f64));
        if l_tmp < gc_thres {
            l_tmp = gc_thres;
        }
    }
    *p_gc_threshold = l_tmp;

    // voicing factor     lambda = 0.5*(1-period_fac)
    // gain smoothing factor S_m = lambda*stab_fac  (=fac)
    //                           = 0.5(stab_fac - stab_fac * period_fac)
    let fac =
        (fx_sgl2fx_dbl(stab_fac) >> (SF_PFAC + 1)).wrapping_sub(f_mult_div2(stab_fac, period_fac));
    // fac_e = SF_PFAC + SF_STAB
    fdk_assert!(fac >= 0);

    // gain_code = (float)((fac*tmp) + ((1.0-fac)*gain_code));
    gain_code = f_mult(fac, l_tmp).wrapping_sub(f_mult(
        fl2fxconst_dbl((-1.0f32 / (1 << (SF_PFAC + SF_STAB)) as f32) as f64).wrapping_add(fac),
        gain_code,
    ));
    gain_code <<= SF_PFAC + SF_STAB;

    gain_code
}

/// `BuildAdaptiveExcitation(code, exc, gain_pit, gain_code, gain_code_smoothed, period_fac,
/// exc2)`: update the adaptive codebook u'(n) (`exc`, `L_SUBFR` values at `exc_buf[exc_off..]`)
/// and build the post-processed excitation u(n) (`exc2`).
///
/// In C, `code[]` and `exc2[]` share memory (`exc2[i]` overlaps `code[2i - 64..2i - 62]`); every
/// value of `code[]` is read before its memory is overwritten, so separate buffers give the
/// same result.
///
/// The optimized form `fast::build_adaptive_excitation` gives the same results as the straight
/// port [`build_adaptive_excitation_ref`].
#[allow(clippy::too_many_arguments)]
pub fn build_adaptive_excitation(
    code: &[FixpCod; L_SUBFR_U],
    exc_buf: &mut [FixpDbl],
    exc_off: usize,
    gain_pit: FixpSgl,
    gain_code: FixpDbl,
    gain_code_smoothed: FixpDbl,
    period_fac: FixpDbl,
    exc2: &mut [FixpDbl],
) {
    fast::build_adaptive_excitation(
        code,
        exc_buf,
        exc_off,
        gain_pit,
        gain_code,
        gain_code_smoothed,
        period_fac,
        exc2,
    )
}

/// The straight port of `BuildAdaptiveExcitation()` (see [`build_adaptive_excitation`]).
#[allow(clippy::too_many_arguments)]
pub fn build_adaptive_excitation_ref(
    code: &[FixpCod; L_SUBFR_U],
    exc_buf: &mut [FixpDbl],
    exc_off: usize,
    gain_pit: FixpSgl,
    gain_code: FixpDbl,
    gain_code_smoothed: FixpDbl,
    period_fac: FixpDbl,
    exc2: &mut [FixpDbl],
) {
    // SF_HEADROOM (1), SF (SF_CODE + SF_GAIN_C + 1 - SF_EXC - SF_HEADROOM),
    // SF_GAIN_P2 (SF_GAIN_P - SF_HEADROOM)
    const SF_HEADROOM: i32 = 1;
    const SF: i32 = SF_CODE + SF_GAIN_C + 1 - SF_EXC - SF_HEADROOM;
    const SF_GAIN_P2: i32 = SF_GAIN_P - SF_HEADROOM;

    let exc = &mut exc_buf[exc_off..exc_off + L_SUBFR_U];
    let mut e = 0usize;
    let mut c = 0usize;
    let mut x2 = 0usize;

    // cpe = (1+r_v)/8 * 2 ; ( SF = -1)
    let cpe = (period_fac >> (2 - SF_PFAC)).wrapping_add(fl2fxconst_dbl(0.25));

    // u'(n)
    let mut tmp = f_mult_div2(exc[e], gain_pit) << (SF_GAIN_P2 + 1); // v(0)*g_p
    exc[e] = tmp.wrapping_add(f_mult_div2(code[0], gain_code) << SF) << SF_HEADROOM;
    e += 1;

    // u(n)
    let code_smooth_prev = f_mult_div2(code[c], gain_code_smoothed) << SF; // c(0) * g_sc
    c += 1;
    let mut code_i = code[c];
    c += 1;
    let mut code_smooth = f_mult_div2(code_i, gain_code_smoothed) << SF; // c(1) * g_sc
    tmp = tmp.wrapping_add(code_smooth_prev); // tmp = v(0)*g_p + c(0)*g_sc
    let mut cpe_code_smooth = f_mult_div2(cpe, code_smooth);
    exc2[x2] = tmp.wrapping_sub(cpe_code_smooth) << SF_HEADROOM;
    x2 += 1;
    let mut cpe_code_smooth_prev = f_mult_div2(cpe, code_smooth_prev);

    for _ in 0..(L_SUBFR_U - 2) {
        // u'(n)
        tmp = f_mult_div2(exc[e], gain_pit) << (SF_GAIN_P2 + 1);
        exc[e] = tmp.wrapping_add(f_mult_div2(code_i, gain_code) << SF) << SF_HEADROOM;
        e += 1;
        // u(n)
        tmp = tmp.wrapping_add(code_smooth); // += g_sc * c(i)
        tmp = tmp.wrapping_sub(cpe_code_smooth_prev);
        cpe_code_smooth_prev = cpe_code_smooth;
        code_i = code[c];
        c += 1;
        code_smooth = f_mult_div2(code_i, gain_code_smoothed) << SF;
        cpe_code_smooth = f_mult_div2(cpe, code_smooth);
        // tmp - c_pe * g_sc * c(i+1)
        exc2[x2] = tmp.wrapping_sub(cpe_code_smooth) << SF_HEADROOM;
        x2 += 1;
    }

    // u'(n)
    tmp = f_mult_div2(exc[e], gain_pit) << (SF_GAIN_P2 + 1);
    exc[e] = tmp.wrapping_add(f_mult_div2(code_i, gain_code) << SF) << SF_HEADROOM;
    // u(n)
    tmp = tmp.wrapping_add(code_smooth);
    tmp = tmp.wrapping_sub(cpe_code_smooth_prev);
    exc2[x2] = tmp << SF_HEADROOM;
}

/// `int_lpc_acelp(lsp_old, lsp_new, subfr_nr, nb_subfr, A, A_exp)`: interpolate the LPC vector
/// in the LSP domain for the current subframe and convert it to the LP domain (`A`, exponent
/// `A_exp`).
pub fn int_lpc_acelp(
    lsp_old: &[FixpLpc],
    lsp_new: &[FixpLpc],
    subfr_nr: i32,
    nb_subfr: i32,
    a: &mut [FixpLpc],
    a_exp: &mut i32,
) {
    let mut lsp_interpol = [0 as FixpLpc; M];

    fdk_assert!((nb_subfr == 3) || (nb_subfr == 4));

    // lsp_interpol_factor[2][4]: indices outside of it (nb_subfr not 3 or 4, never used) have
    // no defined reference result; the port reads 0.
    let tab = &LSP_INTERPOL_FACTOR[(nb_subfr & 0x1) as usize];
    let fac_old = tab.get(((nb_subfr - 1) - subfr_nr) as usize).copied().unwrap_or(0);
    let fac_new = tab.get(subfr_nr as usize).copied().unwrap_or(0);
    for i in 0..M {
        lsp_interpol[i] = fx_dbl2fx_lpc(
            f_mult_div2(lsp_old[i], fac_old).wrapping_add(f_mult_div2(lsp_new[i], fac_new)) << 1,
        );
    }

    e_lpc_f_lsp_a_conversion(&lsp_interpol, a, a_exp);
}

/// `Syn_filt(a, a_exp, length, x, y)`: LP synthesis by filtering the post-processed excitation
/// `x` through the LP synthesis filter 1/A(z). `y` (`y_buf[y_off..]`) is the synthesis signal;
/// the filter memory is `y[-M_LP_FILTER_ORDER..-1]`.
///
/// The in-place use of C (`x == y` with a zero input in `CLpd_Acelp_Zir()`) is equivalent to a
/// separate zero input: `x[i]` is read before `y[i]` is written.
///
/// The optimized form `fast::syn_filt` gives the same results as the straight port
/// [`syn_filt_ref`].
pub fn syn_filt(
    a: &[FixpLpc],
    a_exp: i32,
    length: i32,
    x: &[FixpDbl],
    y_buf: &mut [FixpDbl],
    y_off: usize,
) {
    fast::syn_filt(a, a_exp, length, x, y_buf, y_off)
}

/// The straight port of `Syn_filt()` (see [`syn_filt`]).
pub fn syn_filt_ref(
    a: &[FixpLpc],
    a_exp: i32,
    length: i32,
    x: &[FixpDbl],
    y_buf: &mut [FixpDbl],
    y_off: usize,
) {
    for i in 0..length.max(0) as usize {
        let mut l_tmp: FixpDbl = 0;
        let yi = y_off + i;

        for j in 0..M {
            l_tmp =
                l_tmp.wrapping_sub(f_mult_div2(a[j], y_buf[yi - (j + 1)]) >> (LP_FILTER_SCALE - 1));
        }

        l_tmp = scale_value(l_tmp, a_exp + LP_FILTER_SCALE);
        y_buf[yi] = f_add_saturate(l_tmp, x[i]);
    }
}

/// `Deemph(x, y, L, mem)`: de-emphasis 1/(1 - mu z^-1) on the input signal. `mem`: memory
/// (signal[-1]).
///
/// The optimized form `fast::deemph` gives the same results as the straight port
/// [`deemph_ref`].
pub fn deemph(x: &[FixpDbl], y: &mut [FixpDbl], l: i32, mem: &mut FixpDbl) {
    fast::deemph(x, y, l, mem)
}

/// The straight port of `Deemph()` (see [`deemph`]).
pub fn deemph_ref(x: &[FixpDbl], y: &mut [FixpDbl], l: i32, mem: &mut FixpDbl) {
    let fac = preemph_fac();
    let mut yi = *mem;

    for i in 0..l.max(0) as usize {
        let mut xi = x[i] >> 1;
        xi = f_mult_add_div2(xi, fac, yi);
        yi = saturate_left_shift(xi, 1, 32);
        y[i] = yi;
    }
    *mem = yi;
}

/// `E_UTIL_residu(a, a_exp, x, y, l)`: compute the LP residual `y` by filtering the input `x`
/// (`x_buf[x_off..]`; the values `x[-M..-1]` are needed) through the analysis filter A(z).
pub fn e_util_residu(
    a: &[FixpLpc],
    a_exp: i32,
    x_buf: &[FixpDbl],
    x_off: usize,
    y: &mut [FixpDbl],
    l: i32,
) {
    // (note that values x[-m..-1] are needed)
    for i in 0..l.max(0) as usize {
        let mut s: FixpDbl = 0;
        let xi = x_off + i;

        for j in 0..M {
            s = s.wrapping_add(f_mult_div2(a[j], x_buf[xi - j - 1]) >> (LP_FILTER_SCALE - 1));
        }

        s = scale_value(s, a_exp + LP_FILTER_SCALE);
        y[i] = f_add_saturate(s, x_buf[xi]);
    }
}

/// `num_acb_idx_bits_table[2][NB_SUBFR]`: maps the subframe number to the number of bits used
/// for `acb_index`.
static NUM_ACB_IDX_BITS_TABLE: [[u8; NB_SUBFR as usize]; 2] = [
    [9, 6, 9, 6], // coreCoderFrameLength == 1024
    [9, 6, 6, 0], // coreCoderFrameLength == 768
];

/// `DecodePitchLag(hBs, num_acb_idx_bits, PIT_MIN, PIT_FR2, PIT_FR1, PIT_MAX, pT0, pT0_frac,
/// pT0_min, pT0_max)`
#[allow(clippy::too_many_arguments)]
pub fn decode_pitch_lag(
    h_bs: &mut FdkBitStream,
    num_acb_idx_bits: u8,
    pit_min: i32,
    pit_fr2: i32,
    pit_fr1: i32,
    pit_max: i32,
    p_t0: &mut i32,
    p_t0_frac: &mut i32,
    p_t0_min: &mut i32,
    p_t0_max: &mut i32,
) -> i32 {
    let error = 0;
    let t0;
    let t0_frac;

    fdk_assert!((num_acb_idx_bits == 9) || (num_acb_idx_bits == 6));

    let mut acb_idx = fdk_read_bits(h_bs, num_acb_idx_bits as u32) as i32;

    if num_acb_idx_bits == 6 {
        // When the pitch value is encoded on 6 bits, a pitch resolution of 1/4 is always used in
        // the range [T1-8, T1+7.75], where T1 is nearest integer to the fractional pitch lag of
        // the previous subframe.
        t0 = *p_t0_min + acb_idx / 4;
        t0_frac = acb_idx & 0x3;
    } else {
        // num_acb_idx_bits == 9
        // When the pitch value is encoded on 9 bits, a fractional pitch delay is used with
        // resolutions 0.25 in the range [TMIN, TFR2-0.25], resolutions 0.5 in the range [TFR2,
        // TFR1-0.5], and integers only in the range [TFR1, TMAX]. NOTE: for small sampling
        // rates TMAX can get smaller than TFR1.
        if acb_idx < (pit_fr2 - pit_min) * 4 {
            // first interval with 0.25 pitch resolution
            t0 = pit_min + (acb_idx / 4);
            t0_frac = acb_idx & 0x3;
        } else if acb_idx < ((pit_fr2 - pit_min) * 4 + (pit_fr1 - pit_fr2) * 2) {
            // second interval with 0.5 pitch resolution
            acb_idx -= (pit_fr2 - pit_min) * 4;
            t0 = pit_fr2 + (acb_idx / 2);
            t0_frac = (acb_idx & 0x1) * 2;
        } else {
            // third interval with 1.0 pitch resolution
            t0 = acb_idx + pit_fr1 - ((pit_fr2 - pit_min) * 4) - ((pit_fr1 - pit_fr2) * 2);
            t0_frac = 0;
        }
        // find T0_min and T0_max for subframe 1 or 3
        let mut t0_min = t0 - 8;
        if t0_min < pit_min {
            t0_min = pit_min;
        }
        let mut t0_max = t0_min + 15;
        if t0_max > pit_max {
            t0_max = pit_max;
            t0_min = t0_max - 15;
        }
        *p_t0_min = t0_min;
        *p_t0_max = t0_max;
    }
    *p_t0 = t0;
    *p_t0_frac = t0_frac;

    error
}

/// `ConcealPitchLag(acelp_mem, PIT_MAX, pT0, pT0_frac)`
pub fn conceal_pitch_lag(
    acelp_mem: &mut CAcelpStaticMem,
    pit_max: i32,
    p_t0: &mut i32,
    p_t0_frac: &mut i32,
) {
    if acelp_mem.old_t0 as i32 >= pit_max {
        acelp_mem.old_t0 = (pit_max - 5) as u16;
    }
    *p_t0 = acelp_mem.old_t0 as i32;
    *p_t0_frac = acelp_mem.old_t0_frac as i32;
}

/// `tab_coremode2nbits[8]`
///
/// `scripts/dump_table.py libAACdec/src/usacdec_acelp.cpp tab_coremode2nbits UCHAR`
pub static TAB_COREMODE2NBITS: [u8; 8] = [20, 28, 36, 44, 52, 64, 12, 16];

/// `MapCoreMode2NBits(core_mode)`: `core_mode` is a 3 bit value; other values have no defined
/// reference result (0 in the port).
fn map_core_mode2_nbits(core_mode: i32) -> i32 {
    TAB_COREMODE2NBITS.get(core_mode as usize).copied().unwrap_or(0) as i32
}

/// `CLpd_AcelpDecode(acelp_mem, i_offset, lsp_old, lsp_new, stab_fac, pAcelpData,
/// numLostSubframes, lastLpcLost, frameCnt, synth, pT, pit_gain, coreCoderFrameLength)`: decode
/// one ACELP frame (three or four ACELP subframes with 64 samples each).
///
/// `synth`: ACELP output signal (`coreCoderFrameLength / NB_DIV` values); `p_t`: decoded pitch
/// lag values; `pit_gain`: decoded pitch gains (one per subframe).
///
/// `exc[]` is read at `exc[i_subfr - T0 - 16]`; the pitch lags of valid configurations stay
/// inside the excitation memory (`PIT_MAX <= PIT_MAX_MAX`). Positions outside of it have no
/// defined reference result; the port reads 0.
#[allow(clippy::too_many_arguments)]
pub fn clpd_acelp_decode(
    acelp_mem: &mut CAcelpStaticMem,
    i_offset: i32,
    lsp_old: &[FixpLpc],
    lsp_new: &[FixpLpc],
    stab_fac: FixpSgl,
    p_acelp_data: &CAcelpChannelData,
    num_lost_subframes: i32,
    last_lpc_lost: i32,
    frame_cnt: i32,
    synth: &mut [FixpDbl],
    p_t: &mut [i32],
    pit_gain: &mut [FixpDbl],
    core_coder_frame_length: i32,
) {
    let mut t0: i32 = -1;
    let mut t0_frac: i32 = -1; // mark invalid

    let mut pit_gain_index = 0usize;

    let pit_max = PIT_MAX_12k8 + (6 * i_offset); // maximum pitch lag

    let mut a = [0 as FixpLpc; M];
    let mut a_exp: i32 = 0;

    let mut gain_pit: FixpSgl = 0;
    let mut gain_code: FixpDbl = 0;
    let mut ener_code: FixpDbl = 0;
    let mut ener_code_e: i32 = 0;
    let bfi = (num_lost_subframes > 0) as i32;

    // 411 + 17 + 256 + 1 = 685
    let mut exc_buf = [0 as FixpDbl; EXC_MEM + L_DIV as usize + 1];
    // 16 + 256 = 272
    let mut syn_buf = [0 as FixpDbl; M + L_DIV as usize];
    // code[L_SUBFR] and exc2[L_SUBFR] share memory in C (see build_adaptive_excitation()).
    let mut code = [0 as FixpCod; L_SUBFR_U];
    let mut exc2 = [0 as FixpDbl; L_SUBFR_U];

    let syn = M; // syn = syn_buf + M_LP_FILTER_ORDER
    let exc = EXC_MEM; // exc = exc_buf + PIT_MAX_MAX + L_INTERPOL

    syn_buf[..M].copy_from_slice(&acelp_mem.old_syn_mem);
    exc_buf[..EXC_MEM].copy_from_slice(&acelp_mem.old_exc_mem);

    // FDKmemclear(exc_buf + (PIT_MAX_MAX + L_INTERPOL), (L_DIV + 1) * sizeof(FIXP_DBL)): the
    // array is zero-initialized.

    // l_div is 192 or 256 for the supported core coder frame lengths (768, 1024); the buffers
    // hold 256 (other lengths have no defined reference result; the port clamps).
    let l_div = (core_coder_frame_length / NB_DIV).clamp(0, L_DIV);

    let mut i_subfr = 0;
    let mut subfr_nr = 0usize;
    while i_subfr < l_div {
        // Decode pitch lag (T0 and T0_frac)
        if bfi != 0 {
            conceal_pitch_lag(acelp_mem, pit_max, &mut t0, &mut t0_frac);
        } else {
            t0 = p_acelp_data.t0[subfr_nr] as i32;
            t0_frac = p_acelp_data.t0_frac[subfr_nr] as i32;
        }

        // Find the pitch gain, the interpolation filter and the adaptive codebook vector.
        pred_lt4(&mut exc_buf, exc + i_subfr as usize, t0, t0_frac);

        if (bfi == 0 && p_acelp_data.ltp_filtering_flag[subfr_nr] == 0)
            || (bfi != 0 && num_lost_subframes == 1 && stab_fac < fl2fxconst_sgl(0.25f32 as f64))
        {
            // find pitch excitation with lp filter: v'(n) => v(n)
            pred_lt4_postfilter(&mut exc_buf, exc + i_subfr as usize);
        }

        // Decode innovative codebook.
        // Add the fixed-gain pitch contribution to code[].
        if bfi != 0 {
            for n in 0..L_SUBFR_U {
                code[n] = fx_sgl2fx_cod(e_util_random(&mut acelp_mem.seed_ace)) >> 4;
            }
        } else {
            let nbits = map_core_mode2_nbits(p_acelp_data.acelp_core_mode as i32);
            d_acelp_decode_4t64(&p_acelp_data.icb_index[subfr_nr], nbits, &mut code);
        }

        let mut t = t0;
        if t0_frac > 2 {
            t += 1;
        }

        preemph_code(&mut code);
        pit_shrp(&mut code, t);

        // Output pitch lag for bass post-filter
        if t > pit_max {
            p_t[subfr_nr] = pit_max;
        } else {
            p_t[subfr_nr] = t;
        }
        d_gain2_plus(
            p_acelp_data.gains[subfr_nr] as i32,
            &code,          // (i)  : Innovative code vector, exponent = SF_CODE
            &mut gain_pit,  // (o)  : Quantized pitch gain, exponent = SF_GAIN_P
            &mut gain_code, // (o)  : Quantized codebook gain
            p_acelp_data.mean_energy as i32, // (i)  : mean_ener defined in open-loop (2 bits)
            bfi,
            &mut acelp_mem.past_gpit,
            &mut acelp_mem.past_gcode,
            &mut ener_code,   // (o)  : Innovative code vector energy
            &mut ener_code_e, // (o)  : Innovative code vector energy exponent
        );

        pit_gain[pit_gain_index] = fx_sgl2fx_dbl(gain_pit);
        pit_gain_index += 1;

        // calc periodicity factor r_v
        let period_fac = calc_period_factor(
            &exc_buf[exc + i_subfr as usize..], // (i) : pitch excitation, exponent = SF_EXC
            gain_pit,                           // (i) : gain of pitch, exponent = SF_GAIN_P
            gain_code,                          // (i) : gain of code
            ener_code,                          // (i) : Energy of code[]
            ener_code_e,                        // (i) : Exponent of energy of code[]
        );

        if last_lpc_lost != 0 && frame_cnt == 0 {
            if gain_pit > fl2fxconst_sgl((1.0f32 / (1 << SF_GAIN_P) as f32) as f64) {
                gain_pit = fl2fxconst_sgl((1.0f32 / (1 << SF_GAIN_P) as f32) as f64);
            }
        }

        let gain_code_smooth = noise_enhancer(
            gain_code,  // (i) : Quantized codebook gain
            period_fac, // (i) : periodicity factor (-1=unvoiced to 1=voiced)
            stab_fac,   // (i) : stability factor (0 <= ... < 1), exponent = 1
            &mut acelp_mem.gc_threshold,
        );

        // Compute adaptive codebook update u'(n), pitch enhancement c'(n) and post-processed
        // excitation u(n).
        build_adaptive_excitation(
            &code,
            &mut exc_buf,
            exc + i_subfr as usize,
            gain_pit,
            gain_code,
            gain_code_smooth,
            period_fac,
            &mut exc2,
        );

        // Interpolate filter coeffs for current subframe in lsp domain and convert to LP domain
        int_lpc_acelp(
            lsp_old,         // input : LSPs from past frame
            lsp_new,         // input : LSPs from present frame
            subfr_nr as i32, // input : ACELP subframe index
            core_coder_frame_length / L_DIV,
            &mut a, // output: LP coefficients of this subframe
            &mut a_exp,
        );

        syn_filt(
            &a, // (i) : a[m] prediction coefficients
            a_exp,
            L_SUBFR, // (i) : length
            &exc2,   // (i) : input signal
            &mut syn_buf,
            syn + i_subfr as usize, // (i/o) : filter states / output signal
        );

        i_subfr += L_SUBFR;
        subfr_nr += 1;
    } // end of subframe loop

    // update pitch value for bfi procedure
    acelp_mem.old_t0_frac = t0_frac as u8;
    acelp_mem.old_t0 = t0 as u16;

    // save old excitation and old synthesis memory for next ACELP frame
    let l = l_div as usize;
    acelp_mem.old_exc_mem.copy_from_slice(&exc_buf[exc + l - EXC_MEM..exc + l]);
    acelp_mem.old_syn_mem.copy_from_slice(&syn_buf[l..l + M]);

    deemph(&syn_buf[syn..], synth, l_div, &mut acelp_mem.de_emph_mem); // ref soft: mem = synth[-1]

    scale_values(&mut synth[..l], -ACELP_OUTSCALE);
    acelp_mem.deemph_mem_wsyn = acelp_mem.de_emph_mem;
}

/// `CLpd_AcelpReset(acelp_mem)`: reset the ACELP internal memory.
pub fn clpd_acelp_reset(acelp: &mut CAcelpStaticMem) {
    acelp.gc_threshold = 0;

    acelp.past_gpit = 0;
    acelp.past_gcode = 0;
    acelp.old_t0 = 64;
    acelp.old_t0_frac = 0;
    acelp.deemph_mem_wsyn = 0;
    acelp.wsyn_rms = 0;
    acelp.seed_ace = 0;
}

/// `CLpd_TcxTDConceal(acelp_mem, pitch, lsp_old, lsp_new, stab_fac, nLostSf, synth,
/// coreCoderFrameLength, last_tcx_noise_factor)`: TCX time domain concealment (compare to
/// figure 13a on page 54 in 3GPP TS 26.290). `synth` receives `coreCoderFrameLength / NB_DIV`
/// values.
///
/// For a pitch of 0 (`last_tcx_pitch`) the reference has no defined result; the port reads the
/// zero-initialized excitation buffer.
#[allow(clippy::too_many_arguments)]
pub fn clpd_tcx_td_conceal(
    acelp_mem: &mut CAcelpStaticMem,
    pitch: &i16,
    lsp_old: &[FixpLpc],
    lsp_new: &[FixpLpc],
    _stab_fac: FixpSgl,
    n_lost_sf: i32,
    synth: &mut [FixpDbl],
    core_coder_frame_length: i32,
    _last_tcx_noise_factor: u8,
) {
    // repeat past excitation with pitch from previous decoded TCX frame
    let mut exc_buf = [0 as FixpDbl; EXC_MEM + L_DIV as usize]; // 411 +  17 + 256 + 1 =
    let mut syn_buf = [0 as FixpDbl; M + L_DIV as usize]; // 256 +  16           =
    let mut ns_buf = [0 as FixpDbl; L_DIV as usize + 1];
    let syn = M;
    let exc = EXC_MEM;
    let ns = 1usize;
    let t: i32 = f_min(*pitch, PIT_MAX_MAX as i16) as i32;
    let l_div = (core_coder_frame_length / NB_DIV).clamp(0, L_DIV);

    syn_buf[..M].copy_from_slice(&acelp_mem.old_syn_mem);
    exc_buf[..EXC_MEM].copy_from_slice(&acelp_mem.old_exc_mem);

    // if we lost all packets (i.e. 1 packet of TCX-20 ms, 2 packets of the TCX-40 ms or 4
    // packets of the TCX-80ms), we lost the whole coded frame extrapolation strategy: repeat
    // lost excitation and use extrapolated LSFs

    // AMR-WB+ like TCX TD concealment

    // number of lost frame cmpt
    let fact_exc: FixpDbl =
        if n_lost_sf < 2 { fl2fxconst_dbl(0.8f32 as f64) } else { fl2fxconst_dbl(0.4f32 as f64) };

    // repeat past excitation
    for i in 0..l_div as isize {
        // exc[i - T]: T is at most PIT_MAX_MAX; positions outside of the buffer read 0 (no
        // defined reference result).
        let v = rd(&exc_buf, exc as isize + i - t as isize);
        exc_buf[exc + i as usize] = f_mult(fact_exc, v);
    }

    let tmp = f_mult(fact_exc, acelp_mem.wsyn_rms);
    acelp_mem.wsyn_rms = tmp;

    // init deemph_mem_wsyn
    acelp_mem.deemph_mem_wsyn = exc_buf[exc - 1];

    ns_buf[ns - 1] = acelp_mem.deemph_mem_wsyn;

    let mut i_subfr = 0;
    let mut subfr_nr = 0;
    while i_subfr < l_div {
        let mut t_res = [0 as FixpDbl; L_SUBFR_U];
        let mut a = [0 as FixpLpc; M];
        let mut a_exp: i32 = 0;
        let isf = i_subfr as usize;

        // interpolate LPC coefficients
        int_lpc_acelp(lsp_old, lsp_new, subfr_nr, l_div / L_SUBFR, &mut a, &mut a_exp);

        syn_filt(
            &a,    // (i) : a[m] prediction coefficients
            a_exp, // (i) : length
            L_SUBFR,
            &exc_buf[exc + isf..exc + isf + L_SUBFR_U], // (i) : input signal
            &mut syn_buf,
            syn + isf, // (i/o) : filter states / output signal
        );

        // overwrite A as it is not needed any longer
        e_lpc_a_weight_in_place(&mut a, M_LP_FILTER_ORDER);

        e_util_residu(&a, a_exp, &syn_buf, syn + isf, &mut t_res, L_SUBFR);

        deemph(&t_res, &mut ns_buf[ns + isf..], L_SUBFR, &mut acelp_mem.deemph_mem_wsyn);

        // Amplitude limiter (saturate at wsyn_rms)
        for i in (ns + isf)..(ns + isf + L_SUBFR_U) {
            if ns_buf[i] > tmp {
                ns_buf[i] = tmp;
            } else {
                if ns_buf[i] < tmp.wrapping_neg() {
                    ns_buf[i] = tmp.wrapping_neg();
                }
            }
        }

        e_util_preemph(&ns_buf, ns + isf, &mut t_res, L_SUBFR);

        syn_filt(
            &a,    // (i) : a[m] prediction coefficients
            a_exp, // (i) : length
            L_SUBFR,
            &t_res, // (i) : input signal
            &mut syn_buf,
            syn + isf, // (i/o) : filter states / output signal
        );

        synth[isf..isf + L_SUBFR_U].copy_from_slice(&syn_buf[syn + isf..syn + isf + L_SUBFR_U]);

        i_subfr += L_SUBFR;
        subfr_nr += 1;
    }

    // save old excitation and old synthesis memory for next ACELP frame
    let l = l_div as usize;
    acelp_mem.old_exc_mem.copy_from_slice(&exc_buf[exc + l - EXC_MEM..exc + l]);
    acelp_mem.old_syn_mem.copy_from_slice(&syn_buf[l..l + M]);
    acelp_mem.de_emph_mem = acelp_mem.deemph_mem_wsyn;
}

/// `Acelp_PreProcessing(synth_buf, old_synth, pitch, old_T_pf, pit_gain, old_gain_pf,
/// samplingRate, i_offset, coreCoderFrameLength, synSfd, nbSubfrSuperfr)`: initialization of
/// the memory before one LPD frame is decoded.
#[allow(clippy::too_many_arguments)]
pub fn acelp_pre_processing(
    synth_buf: &mut [FixpDbl],
    old_synth: &[FixpDbl],
    pitch: &mut [i32],
    old_t_pf: &[i32],
    pit_gain: &mut [FixpDbl],
    old_gain_pf: &[FixpDbl],
    sampling_rate: i32,
    i_offset: &mut i32,
    _core_coder_frame_length: i32,
    syn_sfd: i32,
    nb_subfr_superfr: i32,
) {
    // init beginning of synth_buf with old synthesis from previous frame
    let n = (PIT_MAX_MAX - BPF_DELAY) as usize;
    synth_buf[..n].copy_from_slice(&old_synth[..n]);

    // calculate pitch lag offset for ACELP decoder
    *i_offset = (sampling_rate.wrapping_mul(PIT_MIN_12k8).wrapping_add(FSCALE_DENOM / 2))
        / FSCALE_DENOM
        - PIT_MIN_12k8;

    // for bass postfilter
    pitch[..(syn_sfd.max(0) as usize)].copy_from_slice(&old_t_pf[..(syn_sfd.max(0) as usize)]);
    pit_gain[..(syn_sfd.max(0) as usize)]
        .copy_from_slice(&old_gain_pf[..(syn_sfd.max(0) as usize)]);
    for n in 0..nb_subfr_superfr.max(0) as usize {
        pitch[n + syn_sfd as usize] = L_SUBFR;
        pit_gain[n + syn_sfd as usize] = 0;
    }
}

/// `Acelp_PostProcessing(synth_buf, old_synth, pitch, old_T_pf, coreCoderFrameLength, synSfd,
/// nbSubfrSuperfr)`: save the tail of the buffers for the initialization of the next LPD frame.
pub fn acelp_post_processing(
    synth_buf: &[FixpDbl],
    old_synth: &mut [FixpDbl],
    pitch: &[i32],
    old_t_pf: &mut [i32],
    core_coder_frame_length: i32,
    syn_sfd: i32,
    nb_subfr_superfr: i32,
) {
    // store last part of synth_buf (which is not handled by the IMDCT overlap) for next frame
    let n = (PIT_MAX_MAX - BPF_DELAY) as usize;
    let o = core_coder_frame_length as usize;
    old_synth[..n].copy_from_slice(&synth_buf[o..o + n]);

    // for bass postfilter
    for n in 0..syn_sfd.max(0) as usize {
        old_t_pf[n] = pitch[nb_subfr_superfr as usize + n];
    }
}

/// `L_FAC_ZIR`: `LFAC`
const L_FAC_ZIR: usize = LFAC as usize;

/// `CLpd_Acelp_Zir(A, A_exp, acelp_mem, length, zir, doDeemph)`: calculate the zero input
/// response (zir) of the ACELP synthesis filter (`length` values, at most `LFAC`).
///
/// Quirk: without de-emphasis, C copies the start of its work buffer (the synthesis filter
/// memory followed by the response), not the response itself; the port does the same.
pub fn clpd_acelp_zir(
    a: &[FixpLpc],
    a_exp: i32,
    acelp_mem: &mut CAcelpStaticMem,
    length: i32,
    zir: &mut [FixpDbl],
    do_deemph: i32,
) {
    let mut tmp_buf = [0 as FixpDbl; L_FAC_ZIR + M];
    fdk_assert!(length as usize <= L_FAC_ZIR);
    // A larger length has no defined reference result; the port clamps (length is fac_length).
    let length = length.clamp(0, L_FAC_ZIR as i32);

    tmp_buf[..M].copy_from_slice(&acelp_mem.old_syn_mem);
    // FDKmemset(tmp_buf + M_LP_FILTER_ORDER, 0, L_FAC_ZIR * sizeof(FIXP_DBL)): already zero.

    let zero = [0 as FixpDbl; L_FAC_ZIR];
    syn_filt(a, a_exp, length, &zero, &mut tmp_buf, M);
    let n = length as usize;
    if do_deemph == 0 {
        // if last lpd mode was TD concealment, then bypass deemph
        zir[..n].copy_from_slice(&tmp_buf[..n]);
    } else {
        deemph(&tmp_buf[M..], zir, length, &mut acelp_mem.de_emph_mem);
        scale_values(&mut zir[..n], -ACELP_OUTSCALE);
    }
}

/// `CLpd_AcelpPrepareInternalMem(synth, last_lpd_mode, last_last_lpd_mode, A_new, A_new_exp,
/// A_old, A_old_exp, acelp_mem, coreCoderFrameLength, clearOldExc, lpd_mode)`: initialize the
/// ACELP internal memory in case of FAC before the ACELP decoder is called.
///
/// `synth` (`synth_buf[synth_off..]`) points to end+1 of the past valid synthesis signal;
/// `synth[-(PIT_MAX_MAX + L_INTERPOL + M_LP_FILTER_ORDER + 1)..-1]` are read (the decoder's
/// synthesis buffer always holds them; outside of `synth_buf` the port reads 0).
#[allow(clippy::too_many_arguments)]
pub fn clpd_acelp_prepare_internal_mem(
    synth_buf_in: &[FixpDbl],
    synth_off: usize,
    last_lpd_mode: u8,
    last_last_lpd_mode: u8,
    a_new: &[FixpLpc],
    a_new_exp: i32,
    a_old: &[FixpLpc],
    a_old_exp: i32,
    acelp_mem: &mut CAcelpStaticMem,
    core_coder_frame_length: i32,
    clear_old_exc: i32,
    lpd_mode: u8,
) {
    // length of one ACELP/TCX20 frame (192 or 256)
    let l_div = (core_coder_frame_length / NB_DIV).clamp(0, EXC_MEM as i32);

    let mut synth_buf = [0 as FixpDbl; EXC_MEM + M];
    let syn = M; // syn = &synth_buf[M_LP_FILTER_ORDER]

    let l_div_partial = EXC_MEM as i32 - l_div;
    // &synth[-(PIT_MAX_MAX + L_INTERPOL + M_LP_FILTER_ORDER)]
    let s0 = synth_off as isize - (EXC_MEM + M) as isize;

    if lpd_mode == 4 {
        // Bypass Domain conversion. TCXTD Concealment does no deemphasis in the end.
        for i in 0..EXC_MEM + M {
            synth_buf[i] = rd(synth_buf_in, s0 + i as isize);
        }
        // Set deemphasis memory state for TD concealment
        acelp_mem.deemph_mem_wsyn =
            scale_value_saturate(rd(synth_buf_in, synth_off as isize - 1), ACELP_OUTSCALE);
    } else {
        // convert past [PIT_MAX_MAX+L_INTERPOL+M_LP_FILTER_ORDER] synthesis to preemph domain
        let mut padded = [0 as FixpDbl; EXC_MEM + M + 1];
        for i in 0..EXC_MEM + M + 1 {
            padded[i] = rd(synth_buf_in, s0 - 1 + i as isize);
        }
        e_util_preemph(&padded, 1, &mut synth_buf, (EXC_MEM + M) as i32);
        scale_values_saturate(&mut synth_buf, ACELP_OUTSCALE);
    }

    // Set deemphasis memory state
    acelp_mem.de_emph_mem =
        scale_value_saturate(rd(synth_buf_in, synth_off as isize - 1), ACELP_OUTSCALE);

    // update acelp synth filter memory
    acelp_mem.old_syn_mem.copy_from_slice(&synth_buf[syn + EXC_MEM - M..syn + EXC_MEM]);

    if clear_old_exc != 0 {
        acelp_mem.old_exc_mem.fill(0);
        return;
    }

    let old_exc_mem = &mut acelp_mem.old_exc_mem;
    let ldp = l_div_partial as usize;
    let ld = l_div as usize;

    // update past [PIT_MAX_MAX+L_INTERPOL] samples of exc memory
    if last_lpd_mode == 1 {
        // last frame was TCX20
        if last_last_lpd_mode == 0 {
            // ACELP -> TCX20 -> ACELP transition
            // Delay valid part of excitation buffer (from previous ACELP frame) by l_div
            // samples
            old_exc_mem.copy_within(ld..ld + ldp, 0);
        } else if last_last_lpd_mode > 0 {
            // TCX -> TCX20 -> ACELP transition
            e_util_residu(a_old, a_old_exp, &synth_buf, syn, &mut old_exc_mem[..], l_div_partial);
        }
        e_util_residu(a_new, a_new_exp, &synth_buf, syn + ldp, &mut old_exc_mem[ldp..], l_div);
    } else {
        // prev frame was FD, TCX40 or TCX80
        let exc_a_new_length: i32 = if core_coder_frame_length / 2 > EXC_MEM as i32 {
            EXC_MEM as i32
        } else {
            (core_coder_frame_length / 2).max(0)
        };
        let exc_a_old_length = EXC_MEM as i32 - exc_a_new_length;
        e_util_residu(a_old, a_old_exp, &synth_buf, syn, &mut old_exc_mem[..], exc_a_old_length);
        e_util_residu(
            a_new,
            a_new_exp,
            &synth_buf,
            syn + exc_a_old_length as usize,
            &mut old_exc_mem[exc_a_old_length as usize..],
            exc_a_new_length,
        );
    }
}

/// `CLpd_ACELP_GetFreeExcMem(acelp_mem, length)`: borrow the static excitation memory from the
/// ACELP decoder.
///
/// The caller has to take care not to overwrite valid memory areas. During TCX/FAC calculations
/// and before `CLpd_AcelpPrepareInternalMem()` is called, the following memory size is
/// available:
/// - 256 samples in case of ACELP -> TCX20 -> ACELP transition
/// - PIT_MAX_MAX+L_INTERPOL samples in all other cases
pub fn clpd_acelp_get_free_exc_mem(acelp_mem: &mut CAcelpStaticMem, length: i32) -> &mut [FixpDbl] {
    fdk_assert!(length <= PIT_MAX_MAX + L_INTERPOL);
    &mut acelp_mem.old_exc_mem[..]
}

/// `CLpd_AcelpRead(hBs, acelp, acelp_core_mode, coreCoderFrameLength, i_offset)`: read the
/// `acelp_coding()` bitstream part. Returns 0 or `AAC_DEC_DECODE_FRAME_ERROR`.
///
/// Note: `usacdec_acelp.h` declares the last two parameters in the order `i_offset,
/// coreCoderFrameLength`; the definition (ported here) and the caller use
/// `coreCoderFrameLength, i_offset`.
pub fn clpd_acelp_read(
    h_bs: &mut FdkBitStream,
    acelp: &mut CAcelpChannelData,
    acelp_core_mode: i32,
    core_coder_frame_length: i32,
    i_offset: i32,
) -> i32 {
    let nb_subfr = core_coder_frame_length / L_DIV;
    let num_acb_index_bits =
        if nb_subfr == 4 { &NUM_ACB_IDX_BITS_TABLE[0] } else { &NUM_ACB_IDX_BITS_TABLE[1] };
    let mut error = 0;

    let pit_min = PIT_MIN_12k8 + i_offset;
    let pit_fr2 = PIT_FR2_12k8 - i_offset;
    let pit_fr1 = PIT_FR1_12k8;
    let pit_max = PIT_MAX_12k8 + (6 * i_offset);
    let mut t0 = 0;
    let mut t0_frac = 0;
    let mut t0_min = 0;
    let mut t0_max = 0;

    if pit_max > PIT_MAX_MAX {
        return AAC_DEC_DECODE_FRAME_ERROR;
    }

    acelp.acelp_core_mode = acelp_core_mode as u8;

    let nbits = map_core_mode2_nbits(acelp_core_mode);

    // decode mean energy with 2 bits : 18, 30, 42 or 54 dB
    acelp.mean_energy = fdk_read_bits(h_bs, 2) as u8;

    // nb_subfr is 3 or 4 for the supported core coder frame lengths; the arrays hold NB_SUBFR
    // subframes (more subframes have no defined reference result; the port clamps).
    for sfr in 0..(nb_subfr.clamp(0, NB_SUBFR) as usize) {
        // read ACB index and store T0 and T0_frac for each ACELP subframe.
        error = decode_pitch_lag(
            h_bs,
            num_acb_index_bits[sfr],
            pit_min,
            pit_fr2,
            pit_fr1,
            pit_max,
            &mut t0,
            &mut t0_frac,
            &mut t0_min,
            &mut t0_max,
        );
        if error != 0 {
            return error;
        }
        acelp.t0[sfr] = t0 as u16;
        acelp.t0_frac[sfr] = t0_frac as u8;
        acelp.ltp_filtering_flag[sfr] = fdk_read_bits(h_bs, 1) as u8;
        let icb = &mut acelp.icb_index[sfr];
        match nbits {
            12 => {
                // 12 bits AMR-WB codebook is used
                icb[0] = fdk_read_bits(h_bs, 1) as i16;
                icb[1] = fdk_read_bits(h_bs, 5) as i16;
                icb[2] = fdk_read_bits(h_bs, 1) as i16;
                icb[3] = fdk_read_bits(h_bs, 5) as i16;
            }
            16 => {
                // 16 bits AMR-WB codebook is used
                icb[0] = fdk_read_bits(h_bs, 1) as i16;
                icb[1] = fdk_read_bits(h_bs, 5) as i16;
                icb[2] = fdk_read_bits(h_bs, 5) as i16;
                icb[3] = fdk_read_bits(h_bs, 5) as i16;
            }
            20 => {
                // 20 bits AMR-WB codebook is used
                icb[0] = fdk_read_bits(h_bs, 5) as i16;
                icb[1] = fdk_read_bits(h_bs, 5) as i16;
                icb[2] = fdk_read_bits(h_bs, 5) as i16;
                icb[3] = fdk_read_bits(h_bs, 5) as i16;
            }
            28 => {
                // 28 bits AMR-WB codebook is used
                icb[0] = fdk_read_bits(h_bs, 9) as i16;
                icb[1] = fdk_read_bits(h_bs, 9) as i16;
                icb[2] = fdk_read_bits(h_bs, 5) as i16;
                icb[3] = fdk_read_bits(h_bs, 5) as i16;
            }
            36 => {
                // 36 bits AMR-WB codebook is used
                icb[0] = fdk_read_bits(h_bs, 9) as i16;
                icb[1] = fdk_read_bits(h_bs, 9) as i16;
                icb[2] = fdk_read_bits(h_bs, 9) as i16;
                icb[3] = fdk_read_bits(h_bs, 9) as i16;
            }
            44 => {
                // 44 bits AMR-WB codebook is used
                icb[0] = fdk_read_bits(h_bs, 13) as i16;
                icb[1] = fdk_read_bits(h_bs, 13) as i16;
                icb[2] = fdk_read_bits(h_bs, 9) as i16;
                icb[3] = fdk_read_bits(h_bs, 9) as i16;
            }
            52 => {
                // 52 bits AMR-WB codebook is used
                icb[0] = fdk_read_bits(h_bs, 13) as i16;
                icb[1] = fdk_read_bits(h_bs, 13) as i16;
                icb[2] = fdk_read_bits(h_bs, 13) as i16;
                icb[3] = fdk_read_bits(h_bs, 13) as i16;
            }
            64 => {
                // 64 bits AMR-WB codebook is used
                icb[0] = fdk_read_bits(h_bs, 2) as i16;
                icb[1] = fdk_read_bits(h_bs, 2) as i16;
                icb[2] = fdk_read_bits(h_bs, 2) as i16;
                icb[3] = fdk_read_bits(h_bs, 2) as i16;
                icb[4] = fdk_read_bits(h_bs, 14) as i16;
                icb[5] = fdk_read_bits(h_bs, 14) as i16;
                icb[6] = fdk_read_bits(h_bs, 14) as i16;
                icb[7] = fdk_read_bits(h_bs, 14) as i16;
            }
            _ => {
                fdk_assert!(false);
            }
        }
        acelp.gains[sfr] = fdk_read_bits(h_bs, 7) as u8;
    }

    error
}
