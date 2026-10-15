// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_dmx_tdom_enh.h, libSACenc/src/sacenc_dmx_tdom_enh.cpp.

//! Port of `sacenc_dmx_tdom_enh.h` and `sacenc_dmx_tdom_enh.cpp`: the enhanced time domain
//! downmix (stereo to mono).
//!
//! # Variants
//!
//! The code is identical in the `aarch64` and `x86` builds apart from the expansion of
//! `SATURATE_SHIFT` (same values); the results differ through `fMult`, `fDivNorm`,
//! `invSqrtNorm2`, `sqrtFixp` and `fixp_sin`.
//!
//! # Undefined behavior
//!
//! - `fixpAddNorm()` shifts `value1` by `q1 - resultScale`, which can be -32 or less when the
//!   accumulated energy is much larger than the energy of a granule. Both builds compile the
//!   shift with a register amount (arm64 `lsl`/`lsr`, x86 `shll`/`shrl %cl`), taking the amount
//!   modulo 32; [`scale_value`] does the same. For the left and right energies both builds
//!   assume `value1 >= 0` (a sum of squares) and use `clz(value1)` without `fixp_abs` and a
//!   logical right shift; the energies are never negative (at most 2^30 per granule), so the
//!   port's `fixp_abs` and arithmetic shift give the same values.
//! - The energy loop is vectorized in both builds (NEON `ushl`/`sshl`, SSE `pslld`/`psrad`);
//!   its shift amounts (`in_margin - 1` in -1..=14, `granuleShift - 3` in 0..=5) are in range,
//!   where the vector shifts equal the scalar ones.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::machine_type::{IntPcm, SAMPLE_BITS};
use fdk_aac_rs_fdk::trig_fcts::fixp_sin;

use crate::sacenc_lib::*;

/// `PI_FLT`
const PI_FLT: f32 = 3.1415926535897931f32;
/// `ALPHA_FLT`
const ALPHA_FLT: f32 = 0.0001f32;

/// `PI_E`
const PI_E: i32 = 2;
/// `PI_M`: `FL2FXCONST_DBL(PI_FLT / (1 << PI_E))`
const PI_M: FixpDbl = fl2fxconst_dbl((PI_FLT / (1 << PI_E) as f32) as f64);

/// `ALPHA_E`
const ALPHA_E: i32 = 13;
/// `ALPHA_M`: `FL2FXCONST_DBL(ALPHA_FLT * (1 << ALPHA_E))`
const ALPHA_M: FixpDbl = fl2fxconst_dbl((ALPHA_FLT * (1 << ALPHA_E) as f32) as f64);

/// `FL2FXCONST_DBL(1.f - ALPHA_FLT)`
const ONE_MINUS_ALPHA: FixpDbl = fl2fxconst_dbl((1.0f32 - ALPHA_FLT) as f64);

const L: usize = 0;
const R: usize = 1;

/// `ENHANCED_TIME_DOMAIN_DMX` (`struct T_ENHANCED_TIME_DOMAIN_DMX`)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnhancedTimeDomainDmx {
    pub max_framelength: i32,

    pub framelength: i32,

    pub prev_gain_m: [FixpDbl; 2],
    pub prev_gain_e: i32,
    pub prev_h1_m: [FixpDbl; 2],
    pub prev_h1_e: i32,

    /// `sinusWindow_m`: `1 + maxFramelength` values
    pub sinus_window_m: Vec<FixpDbl>,
    pub sinus_window_e: i8,

    pub prev_left_m: FixpDbl,
    pub prev_left_e: i32,
    pub prev_right_m: FixpDbl,
    pub prev_right_e: i32,
    pub prev_xnrg_m: FixpDbl,
    pub prev_xnrg_e: i32,

    pub lin_bb_cld_weight_m: FixpDbl,
    pub lin_bb_cld_weight_e: i32,
    pub gain_weight_m: [FixpDbl; 2],
    pub gain_weight_e: i32,
}

/// `HANDLE_ENHANCED_TIME_DOMAIN_DMX`
pub type HandleEnhancedTimeDomainDmx = Option<Box<EnhancedTimeDomainDmx>>;

/// `invSqrtNorm2(op_m, op_e, result_e)` (static in C): `1 / sqrt(op)` of a value with an
/// exponent.
pub fn inv_sqrt_norm2_e(op_m: FixpDbl, op_e: i32, result_e: &mut i32) -> FixpDbl {
    let mut src_m = op_m;
    let mut src_e = op_e;

    if src_e & 1 != 0 {
        src_m >>= 1;
        src_e = src_e.wrapping_add(1);
    }

    src_m = inv_sqrt_norm2(src_m, result_e);
    *result_e = result_e.wrapping_sub(src_e >> 1);

    src_m
}

/// `sqrtFixp(op_m, op_e, result_e)` (static in C): `sqrt(op)` of a value with an exponent.
pub fn sqrt_fixp_e(op_m: FixpDbl, op_e: i32, result_e: &mut i32) -> FixpDbl {
    let mut src_m = op_m;
    let mut src_e = op_e;

    if src_e & 1 != 0 {
        src_m >>= 1;
        src_e = src_e.wrapping_add(1);
    }

    *result_e = src_e >> 1;
    sqrt_fixp(src_m)
}

/// `fixpAdd(src1_m, src1_e, src2_m, src2_e, dst_e)` (static in C)
pub fn fixp_add(
    src1_m: FixpDbl,
    src1_e: i32,
    src2_m: FixpDbl,
    src2_e: i32,
    dst_e: &mut i32,
) -> FixpDbl {
    if src1_m == 0 {
        *dst_e = src2_e;
        src2_m
    } else if src2_m == 0 {
        *dst_e = src1_e;
        src1_m
    } else {
        *dst_e = fix_max(src1_e, src2_e).wrapping_add(1);
        scale_value(src1_m, fix_max(src1_e.wrapping_sub(*dst_e), -(DFRACT_BITS - 1))).wrapping_add(
            scale_value(src2_m, fix_max(src2_e.wrapping_sub(*dst_e), -(DFRACT_BITS - 1))),
        )
    }
}

/// `fixpAddNorm(value1, q1, pValue2, pQ2)` (static in C): sums up fixed point values with the
/// best possible accuracy.
pub fn fixp_add_norm(value1: FixpDbl, q1: i32, p_value2: &mut FixpDbl, p_q2: &mut i32) {
    let headroom1 = f_normz(fixp_abs(value1)) - 1;
    let headroom2 = f_normz(fixp_abs(*p_value2)) - 1;
    let mut result_scale = fix_max(q1.wrapping_sub(headroom1), p_q2.wrapping_sub(headroom2));

    if (value1 != 0) && (*p_value2 != 0) {
        result_scale = result_scale.wrapping_add(1);
    }

    // `scaleValue(value1, q1 - resultScale)`: the amount can be -32 or less, modulo 32 (see
    // the module documentation).
    *p_value2 = scale_value(value1, q1.wrapping_sub(result_scale)).wrapping_add(scale_value(
        *p_value2,
        fix_max(-(DFRACT_BITS - 1), p_q2.wrapping_sub(result_scale)),
    ));
    *p_q2 = if *p_value2 != 0 { result_scale } else { DFRACT_BITS - 1 };
}

/// `fdk_sacenc_open_enhancedTimeDomainDmx(phEnhancedTimeDmx, framelength)`
pub fn fdk_sacenc_open_enhanced_time_domain_dmx(
    ph_enhanced_time_dmx: Option<&mut HandleEnhancedTimeDomainDmx>,
    framelength: i32,
) -> FdkSacencError {
    match ph_enhanced_time_dmx {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            *h = Some(Box::new(EnhancedTimeDomainDmx {
                sinus_window_m: vec![0; (1 + framelength).max(0) as usize],
                max_framelength: framelength,
                ..Default::default()
            }));
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_init_enhancedTimeDomainDmx(hEnhancedTimeDmx, pInputGain_m, inputGain_e,
/// outputGain_m, outputGain_e, framelength)`
pub fn fdk_sacenc_init_enhanced_time_domain_dmx(
    h: &mut EnhancedTimeDomainDmx,
    p_input_gain_m: &[FixpDbl; 2],
    input_gain_e: i32,
    output_gain_m: FixpDbl,
    output_gain_e: i32,
    framelength: i32,
) -> FdkSacencError {
    if framelength > h.max_framelength {
        return SACENC_INIT_ERROR;
    }

    h.framelength = framelength;

    let mut deltax_e = 0;
    let mut deltax_m = f_div_norm_high_prec(PI_M, 2i32.wrapping_mul(h.framelength), &mut deltax_e);
    deltax_m = scale_value(deltax_m, PI_E + deltax_e - (DFRACT_BITS - 1) - 1);
    deltax_e = 1;

    for smp in 0..(h.framelength + 1).max(0) {
        h.sinus_window_m[smp as usize] =
            f_mult(ALPHA_M, f_pow2(fixp_sin(smp.wrapping_mul(deltax_m), deltax_e)));
    }
    h.sinus_window_e = -ALPHA_E as i8;

    h.prev_left_m = 0;
    h.prev_right_m = 0;
    h.prev_xnrg_m = 0;
    h.prev_left_e = DFRACT_BITS - 1;
    h.prev_right_e = DFRACT_BITS - 1;
    h.prev_xnrg_e = DFRACT_BITS - 1;

    h.lin_bb_cld_weight_m = f_div_norm_high_prec(
        f_pow2(p_input_gain_m[L]),
        f_pow2(p_input_gain_m[R]),
        &mut h.lin_bb_cld_weight_e,
    );

    h.gain_weight_m[L] = f_mult(p_input_gain_m[L], output_gain_m);
    h.gain_weight_m[R] = f_mult(p_input_gain_m[R], output_gain_m);
    h.gain_weight_e = -f_norm(fix_max(h.gain_weight_m[L], h.gain_weight_m[R]));

    h.gain_weight_m[L] = scale_value(h.gain_weight_m[L], -h.gain_weight_e);
    h.gain_weight_m[R] = scale_value(h.gain_weight_m[R], -h.gain_weight_e);
    h.gain_weight_e = h.gain_weight_e.wrapping_add(input_gain_e).wrapping_add(output_gain_e);

    h.prev_gain_m[L] = h.gain_weight_m[L] >> 1;
    h.prev_gain_m[R] = h.gain_weight_m[R] >> 1;
    h.prev_gain_e = h.gain_weight_e.wrapping_add(1);

    h.prev_h1_m[L] = scale_value(h.gain_weight_m[L], -4);
    h.prev_h1_m[R] = scale_value(h.gain_weight_m[R], -4);
    h.prev_h1_e = (2 + 2i32).wrapping_add(h.gain_weight_e);

    SACENC_OK
}

/// `fdk_sacenc_apply_enhancedTimeDomainDmx(hEnhancedTimeDmx, inputTime, outputTimeDmx,
/// InputDelay)`: downmixes `framelength` samples of `inputTime[L/R][InputDelay..]`; the
/// energies are measured on `inputTime[L/R][0..framelength]`.
pub fn fdk_sacenc_apply_enhanced_time_domain_dmx<I: AsRef<[IntPcm]>>(
    h: &mut EnhancedTimeDomainDmx,
    input_time: &[I; 2],
    output_time_dmx: &mut [IntPcm],
    input_delay: i32,
) -> FdkSacencError {
    let in_l = input_time[L].as_ref();
    let in_r = input_time[R].as_ref();
    let fl = h.framelength;

    // Increase energy time resolution with shorter processing blocks. 128 is an empiric value.
    let granule_length = fix_min(128, fl);
    let mut granule_shift =
        if granule_length > 1 { (DFRACT_BITS - 1) - f_norm(granule_length - 1) } else { 0 };
    // one bit more headroom for worst case accumulation
    granule_shift = fix_max(3, granule_shift + 1);

    let mut smp = 0i32;

    // Prevent division by zero.
    let (mut bb_nrg_left_m, mut bb_nrg_right_m, mut bb_xnrg_m): (FixpDbl, FixpDbl, FixpDbl) =
        (1, 1, 1);
    let (mut bb_nrg_left_e, mut bb_nrg_right_e, mut bb_xnrg_e) = (0i32, 0i32, 0i32);

    loop {
        let offset = smp;
        let (mut partial_l, mut partial_r, mut partial_x): (FixpDbl, FixpDbl, FixpDbl) = (0, 0, 0);

        let end = fix_min(offset + granule_length, fl);
        let len = (end - offset).max(0) as usize;
        let o = offset as usize;
        let in_margin = fix_min(
            get_scalefactor_pcm(&in_l[o..], len, 1),
            get_scalefactor_pcm(&in_r[o..], len, 1),
        );

        // partial energy
        smp = offset;
        while smp < end {
            let input_l = scale_value_sgl(in_l[smp as usize], in_margin - 1);
            let input_r = scale_value_sgl(in_r[smp as usize], in_margin - 1);

            partial_l = partial_l.wrapping_add(f_pow2_div2(input_l) >> (granule_shift - 3));
            partial_r = partial_r.wrapping_add(f_pow2_div2(input_r) >> (granule_shift - 3));
            partial_x =
                partial_x.wrapping_add(f_mult_div2(input_l, input_r) >> (granule_shift - 3));
            smp += 1;
        }

        let q = granule_shift - 2 * in_margin;
        fixp_add_norm(partial_l, q, &mut bb_nrg_left_m, &mut bb_nrg_left_e);
        fixp_add_norm(partial_r, q, &mut bb_nrg_right_m, &mut bb_nrg_right_e);
        fixp_add_norm(partial_x, q, &mut bb_xnrg_m, &mut bb_xnrg_e);

        if smp >= fl {
            break;
        }
    }

    let mut nrg_left_e = 0;
    let mut nrg_right_e = 0;
    let mut nrg_x_e = 0;
    let nrg_left_m =
        fixp_add(h.prev_left_m, h.prev_left_e, bb_nrg_left_m, bb_nrg_left_e, &mut nrg_left_e);
    let nrg_right_m =
        fixp_add(h.prev_right_m, h.prev_right_e, bb_nrg_right_m, bb_nrg_right_e, &mut nrg_right_e);
    let nrg_x_m = fixp_add(h.prev_xnrg_m, h.prev_xnrg_e, bb_xnrg_m, bb_xnrg_e, &mut nrg_x_e);

    let mut lin_bb_cld_e = 0;
    let lin_bb_cld_m =
        f_mult(h.lin_bb_cld_weight_m, f_div_norm(nrg_left_m, nrg_right_m, &mut lin_bb_cld_e));
    lin_bb_cld_e = lin_bb_cld_e
        .wrapping_add(h.lin_bb_cld_weight_e)
        .wrapping_add(nrg_left_e)
        .wrapping_sub(nrg_right_e);

    let mut bb_corr_e = 0;
    let bb_corr_m = f_mult(
        nrg_x_m,
        inv_sqrt_norm2_e(
            f_mult(nrg_left_m, nrg_right_m),
            nrg_left_e.wrapping_add(nrg_right_e),
            &mut bb_corr_e,
        ),
    );
    bb_corr_e = bb_corr_e.wrapping_add(nrg_x_e);

    h.prev_left_m = bb_nrg_left_m;
    h.prev_left_e = bb_nrg_left_e;
    h.prev_right_m = bb_nrg_right_m;
    h.prev_right_e = bb_nrg_right_e;
    h.prev_xnrg_m = bb_xnrg_m;
    h.prev_xnrg_e = bb_xnrg_e;

    // bbCld = 10.f*log10(lin_bbCld)
    // lin_Cld = pow(10,bbCld/20) = sqrt(lin_bbCld)
    // lin_Cld2 = lin_Cld*lin_Cld = lin_bbCld
    let mut lin_cld_e = 0;
    let lin_cld_m = sqrt_fixp_e(lin_bb_cld_m, lin_bb_cld_e, &mut lin_cld_e);
    let mut sqrt_lin_cld_e = 0;
    let sqrt_lin_cld_m = sqrt_fixp_e(lin_cld_m, lin_cld_e, &mut sqrt_lin_cld_e);

    // calculate how much right and how much left signal, to avoid signal cancellations
    let mut g_m = [0; 2];
    let mut g_e = 0;
    calculate_ratio(
        sqrt_lin_cld_m,
        sqrt_lin_cld_e,
        lin_cld_m,
        lin_cld_e,
        bb_corr_m,
        bb_corr_e,
        &mut g_m,
        &mut g_e,
    );

    // calculate downmix gains
    let mut h1_m = [0; 2];
    let mut h1_e = 0;
    calculate_dmx_gains(
        lin_cld_m,
        lin_cld_e,
        lin_bb_cld_m,
        lin_bb_cld_e,
        bb_corr_m,
        bb_corr_e,
        &g_m,
        g_e,
        &mut h1_m,
        &mut h1_e,
    );

    // adapt output gains
    h1_m[L] = f_mult(h1_m[L], h.gain_weight_m[L]);
    h1_m[R] = f_mult(h1_m[R], h.gain_weight_m[R]);
    h1_e = h1_e.wrapping_add(h.gain_weight_e);

    let mut gain_left_m = h.prev_gain_m[L];
    let mut gain_right_m = h.prev_gain_m[R];

    let intermediate_gain_e =
        (h.sinus_window_e as i32).wrapping_add(h1_e).wrapping_sub(h.prev_gain_e);
    let out_shift =
        (h.prev_gain_e.wrapping_add(1).wrapping_sub(DFRACT_BITS - SAMPLE_BITS)).wrapping_neg();
    let n = fl as usize;
    // `inputTime[ch][smp + InputDelay]`: the encoder's delay (at most 5) stays inside the time
    // signal buffers; where the reference has no defined result, the port reads 0.
    let delayed = |x: &[IntPcm], s: usize| -> FixpPcm {
        usize::try_from(s as i64 + input_delay as i64)
            .ok()
            .and_then(|i| x.get(i))
            .copied()
            .unwrap_or(0)
    };

    for s in 0..fl.max(0) as usize {
        let w0 = h.sinus_window_m[s];
        let w1 = h.sinus_window_m[n - s];
        let intermediate_gain_left_m = scale_value(
            f_mult(w0, h1_m[L]).wrapping_add(f_mult(w1, h.prev_h1_m[L])),
            intermediate_gain_e,
        );
        let intermediate_gain_right_m = scale_value(
            f_mult(w0, h1_m[R]).wrapping_add(f_mult(w1, h.prev_h1_m[R])),
            intermediate_gain_e,
        );

        gain_left_m = intermediate_gain_left_m.wrapping_add(f_mult(ONE_MINUS_ALPHA, gain_left_m));
        gain_right_m =
            intermediate_gain_right_m.wrapping_add(f_mult(ONE_MINUS_ALPHA, gain_right_m));

        let tmp = f_mult_div2(gain_left_m, delayed(in_l, s))
            .wrapping_add(f_mult_div2(gain_right_m, delayed(in_r, s)));
        output_time_dmx[s] = saturate_shift(tmp, out_shift, SAMPLE_BITS) as IntPcm;
    }

    h.prev_gain_m[L] = gain_left_m;
    h.prev_gain_m[R] = gain_right_m;

    h.prev_h1_m[L] = h1_m[L];
    h.prev_h1_m[R] = h1_m[R];
    h.prev_h1_e = h1_e;

    SACENC_OK
}

/// `G_SCALE_FACTOR`
const G_SCALE_FACTOR: i32 = 2;

/// `calculateRatio(sqrt_linCld_m, sqrt_linCld_e, lin_Cld_m, lin_Cld_e, Icc_m, Icc_e, G_m, G_e)`
/// (static in C)
pub fn calculate_ratio(
    sqrt_lin_cld_m: FixpDbl,
    sqrt_lin_cld_e: i32,
    lin_cld_m: FixpDbl,
    lin_cld_e: i32,
    icc_m: FixpDbl,
    icc_e: i32,
    g_m: &mut [FixpDbl; 2],
    g_e: &mut i32,
) {
    if icc_m >= 0 {
        g_m[0] = fl2fxconst_dbl((1.0f32 / (1 << G_SCALE_FACTOR) as f32) as f64);
        g_m[1] = g_m[0];
        *g_e = G_SCALE_FACTOR;
    } else {
        let max_gain_factor = fl2fxconst_dbl((2.0f32 / (1 << G_SCALE_FACTOR) as f32) as f64);
        let q: FixpDbl;

        // r = (lin_Cld + 1 + 2*Icc*sqrt_linCld) / (lin_Cld + 1 - 2*Icc*sqrt_linCld)
        //   = (tmp1 + tmp2) / (tmp1 - tmp2)
        let mut tmp1_e = 0;
        let tmp1_m = fixp_add(lin_cld_m, lin_cld_e, fl2fxconst_dbl(0.5), 1, &mut tmp1_e);

        let tmp2_m = f_mult(icc_m, sqrt_lin_cld_m);
        let tmp2_e = 1i32.wrapping_add(icc_e).wrapping_add(sqrt_lin_cld_e);
        let mut numerator_e = 0;
        let numerator_m = fixp_add(tmp1_m, tmp1_e, tmp2_m, tmp2_e, &mut numerator_e);
        let mut denominator_e = 0;
        let denominator_m =
            fixp_add(tmp1_m, tmp1_e, tmp2_m.wrapping_neg(), tmp2_e, &mut denominator_e);

        if (numerator_m > 0) && (denominator_m > 0) {
            let mut r_e = 0;
            let r_m = f_div_norm(numerator_m, denominator_m, &mut r_e);
            r_e = r_e.wrapping_add(numerator_e).wrapping_sub(denominator_e);

            // r_4 = sqrt( sqrt( r ) )
            let mut r4_e = 0;
            let mut r4_m = sqrt_fixp_e(r_m, r_e, &mut r4_e);
            let r4_e_in = r4_e;
            r4_m = sqrt_fixp_e(r4_m, r4_e_in, &mut r4_e);

            r4_e = r4_e.wrapping_sub(G_SCALE_FACTOR);

            // q = min(r4_m, max_gain_factor)
            q = if (r4_e >= 0) && (r4_m >= max_gain_factor.wrapping_shr(r4_e as u32)) {
                max_gain_factor
            } else {
                scale_value(r4_m, r4_e)
            };
        } else {
            q = 0;
        }

        g_m[0] = max_gain_factor.wrapping_sub(q);
        g_m[1] = q;

        *g_e = G_SCALE_FACTOR;
    }
}

/// `H1_SCALE_FACTOR`
const H1_SCALE_FACTOR: i32 = 2;

/// `calculateDmxGains(lin_Cld_m, lin_Cld_e, lin_Cld2_m, lin_Cld2_e, Icc_m, Icc_e, G_m, G_e,
/// H1_m, pH1_e)` (static in C)
pub fn calculate_dmx_gains(
    lin_cld_m: FixpDbl,
    lin_cld_e: i32,
    lin_cld2_m: FixpDbl,
    lin_cld2_e: i32,
    icc_m: FixpDbl,
    icc_e: i32,
    g_m: &[FixpDbl; 2],
    g_e: i32,
    h1_m: &mut [FixpDbl; 2],
    p_h1_e: &mut i32,
) {
    let max_gain_factor = fl2fxconst_dbl((2.0f32 / (1 << H1_SCALE_FACTOR) as f32) as f64);

    // nrgRight = sqrt(1/(lin_Cld2 + 1)
    let mut nrg_right_e = 0;
    let mut nrg_right_m =
        fixp_add(lin_cld2_m, lin_cld2_e, fl2fxconst_dbl(0.5), 1, &mut nrg_right_e);
    let e_in = nrg_right_e;
    nrg_right_m = inv_sqrt_norm2_e(nrg_right_m, e_in, &mut nrg_right_e);

    // nrgLeft = lin_Cld * nrgRight
    let nrg_left_m = f_mult(lin_cld_m, nrg_right_m);
    let nrg_left_e = lin_cld_e.wrapping_add(nrg_right_e);

    // crossNrg = sqrt(nrgLeft*nrgRight)
    let mut cross_nrg_e = 0;
    let cross_nrg_m = sqrt_fixp_e(
        f_mult(nrg_left_m, nrg_right_m),
        nrg_left_e.wrapping_add(nrg_right_e),
        &mut cross_nrg_e,
    );

    // inverse_weight = sqrt((nrgLeft + nrgRight) / ( (G[0]*G[0]*nrgLeft) +
    // (G[1]*G[1]*nrgRight) + 2*G[0]*G[1]*Icc*crossNrg)) = sqrt(inv_weight_num /
    // inv_weight_denom)
    let mut inv_weight_num_e = 0;
    let inv_weight_num_m =
        fixp_add(nrg_right_m, nrg_right_e, nrg_left_m, nrg_left_e, &mut inv_weight_num_e);

    let mut inv_weight_denom_e = 0;
    let mut inv_weight_denom_m = fixp_add(
        f_mult(f_pow2(g_m[0]), nrg_left_m),
        (2i32.wrapping_mul(g_e)).wrapping_add(nrg_left_e),
        f_mult(f_pow2(g_m[1]), nrg_right_m),
        (2i32.wrapping_mul(g_e)).wrapping_add(nrg_right_e),
        &mut inv_weight_denom_e,
    );

    let e_in = inv_weight_denom_e;
    inv_weight_denom_m = fixp_add(
        f_mult(f_mult(f_mult(g_m[0], g_m[1]), cross_nrg_m), icc_m),
        1i32.wrapping_add(2i32.wrapping_mul(g_e)).wrapping_add(cross_nrg_e).wrapping_add(icc_e),
        inv_weight_denom_m,
        e_in,
        &mut inv_weight_denom_e,
    );

    let inverse_weight_limited: FixpDbl;
    if inv_weight_denom_m > 0 {
        let mut inverse_weight_e = 0;
        let mut inverse_weight_m =
            f_div_norm(inv_weight_num_m, inv_weight_denom_m, &mut inverse_weight_e);
        let e_in = inverse_weight_e.wrapping_add(inv_weight_num_e).wrapping_sub(inv_weight_denom_e);
        inverse_weight_m = sqrt_fixp_e(inverse_weight_m, e_in, &mut inverse_weight_e);
        inverse_weight_e = inverse_weight_e.wrapping_sub(H1_SCALE_FACTOR);

        // inverse_weight_limited = min(max_gain_factor, inverse_weight)
        inverse_weight_limited = if (inverse_weight_e >= 0)
            && (inverse_weight_m >= max_gain_factor.wrapping_shr(inverse_weight_e as u32))
        {
            max_gain_factor
        } else {
            scale_value(inverse_weight_m, inverse_weight_e)
        };
    } else {
        inverse_weight_limited = max_gain_factor;
    }

    h1_m[0] = f_mult(g_m[0], inverse_weight_limited);
    h1_m[1] = f_mult(g_m[1], inverse_weight_limited);

    *p_h1_e = g_e + H1_SCALE_FACTOR;
}

/// `fdk_sacenc_close_enhancedTimeDomainDmx(phEnhancedTimeDmx)`
pub fn fdk_sacenc_close_enhanced_time_domain_dmx(
    ph_enhanced_time_dmx: Option<&mut HandleEnhancedTimeDomainDmx>,
) -> FdkSacencError {
    match ph_enhanced_time_dmx {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            *h = None;
            SACENC_OK
        }
    }
}
